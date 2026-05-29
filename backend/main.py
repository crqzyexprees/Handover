import asyncio
import os
import time
import uuid

import docker
import git
import pexpect
import psutil
from fastapi import FastAPI, HTTPException, WebSocket, WebSocketDisconnect
from fastapi.middleware.cors import CORSMiddleware
from fastapi.responses import JSONResponse
from pydantic import BaseModel

from services.docker_runtime import DockerRuntime
from services import handoff_engine

app = FastAPI(title="Handover Backend")

_docker_runtime: DockerRuntime | None = None


def get_docker_runtime() -> DockerRuntime:
    """Lazy-init Docker so the API can start when the daemon is down."""
    global _docker_runtime
    if _docker_runtime is not None:
        return _docker_runtime
    try:
        _docker_runtime = DockerRuntime()
    except Exception as exc:
        raise HTTPException(
            status_code=503,
            detail=f"Docker is not available: {exc}",
        ) from exc
    return _docker_runtime

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# ---------------------------------------------------------------------------
# In-memory registries
#
# These hold state for the lifetime of the server process only. They will be
# replaced / backed by something more durable (and Docker-aware) later.
# ---------------------------------------------------------------------------
projects: dict[str, dict] = {}
project_configs: dict[str, dict] = {}
instances: dict[str, dict] = {}

# Live PTY sessions keyed by instance_id. Populated when a /ws/pty/{id}
# WebSocket attaches and cleared when it detaches. Lets endpoints like
# /unload tear an instance's shell down even while it's still streaming
# (notably native-mode shells, which have no container to stop).
pty_sessions: dict[str, pexpect.spawn] = {}

# Resource Governor state.
# The project the user is currently looking at — never suspended by the
# governor. Set when a project is activated.
focused_project_id: str | None = None
# project_id -> monotonic timestamp of last activation, for LRU selection.
project_last_active: dict[str, float] = {}

# RAM thresholds (percent of system memory in use).
GOVERNOR_INTERVAL_SECONDS = 15
RAM_SUSPEND_THRESHOLD = 85  # suspend the single LRU project above this
RAM_EMERGENCY_THRESHOLD = 95  # suspend ALL non-focused projects above this

# Global app configuration, set via the Onboarding Wizard. Lives for the
# process lifetime (to be persisted later).
app_config: dict = {
    "clis_enabled": ["claude", "codex"],
    "auto_suspend_delay": 300,
    "container_mem_limit": "2g",
}


# ---------------------------------------------------------------------------
# Request models
# ---------------------------------------------------------------------------
class ProjectCreate(BaseModel):
    path: str
    sandbox_mode: str = "docker"


class ProjectConfig(BaseModel):
    project_name: str | None = None
    sandbox_mode: str = "docker"
    mem_limit: str = "2g"
    handoff_method: str = "git"


class InstanceStart(BaseModel):
    project_id: str
    # "docker" (sandboxed container, default) or "native" (a shell directly on
    # the host, with access to whatever CLIs the user has installed).
    sandbox_mode: str = "docker"
    # Per-container memory cap (also caps swap). Docker mode only. Defaults to
    # 2g, enough headroom for the Node CLI TUIs (claude/codex) the user may run.
    mem_limit: str | None = None


class HandoffRequest(BaseModel):
    from_instance_id: str
    to_instance_id: str
    project_id: str
    method: str
    # What the user originally asked the AIs to do. Used both as the handoff
    # checkpoint content and to build the prompt injected into the new AI.
    task_description: str


# Default memory cap when the request doesn't specify one. 2g leaves enough
# headroom for the Node CLI TUIs the user may launch inside the shell.
DEFAULT_MEM_LIMIT = "2g"

# Supported sandbox modes.
SANDBOX_MODES = ("docker", "native")
HANDOFF_METHODS = ("git", "summary")


def _basename_from_path(path: str) -> str:
    normalized = path.replace("\\", "/").rstrip("/")
    if not normalized:
        return "Unknown"
    return normalized.rsplit("/", 1)[-1] or "Unknown"


def _default_project_config(project_id: str) -> dict:
    project = projects.get(project_id)
    path = project.get("path", "") if project else ""
    return {
        "project_name": _basename_from_path(path),
        "sandbox_mode": "docker",
        "mem_limit": DEFAULT_MEM_LIMIT,
        "handoff_method": "git",
    }


@app.on_event("startup")
async def cleanup_orphaned_containers():
    # Start the Resource Governor regardless of Docker state (it no-ops if
    # Docker is down, since there are no containers to pause).
    _spawn_background(resource_governor())
    print(
        f"[startup] resource governor started "
        f"(every {GOVERNOR_INTERVAL_SECONDS}s; suspend>{RAM_SUSPEND_THRESHOLD}%, "
        f"emergency>{RAM_EMERGENCY_THRESHOLD}%)",
        flush=True,
    )

    # Remove any sandbox containers and stale passwd/group temp dirs left over
    # from a previous run so we always start from a clean slate.
    try:
        runtime = get_docker_runtime()
    except HTTPException:
        print("[startup] Docker unavailable; skipping container cleanup")
        return
    removed = await runtime.cleanup_orphans()
    print(f"[startup] removed {removed} orphaned sandbox container(s)")
    removed_nss = await runtime.cleanup_nss_temp_dirs()
    print(f"[startup] removed {removed_nss} stale nss temp dir(s)")


@app.get("/")
async def root():
    return {"status": "ok", "service": "handover-backend"}


# ---------------------------------------------------------------------------
# Onboarding Wizard: health checks + global app config
# ---------------------------------------------------------------------------
@app.get("/api/health/docker")
async def docker_health():
    """Report whether the Docker daemon is reachable.

    The ping runs in a worker thread so a hung/unreachable daemon can't block
    the event loop.
    """

    def _ping():
        docker.from_env().ping()

    try:
        await asyncio.to_thread(_ping)
    except Exception:
        return {
            "status": "error",
            "message": "Docker is not running or not installed.",
        }
    return {"status": "ok"}


@app.get("/api/config")
async def get_config():
    return app_config


@app.put("/api/config")
async def update_config(body: dict):
    # Merge the incoming JSON into the global config (supports partial updates).
    app_config.update(body)
    return {"status": "ok"}


# ---------------------------------------------------------------------------
# Project endpoints
# ---------------------------------------------------------------------------
@app.get("/api/projects")
async def list_projects():
    result = []
    for project_id, project in projects.items():
        entry = dict(project)
        cfg = project_configs.get(project_id)
        if cfg:
            entry["config"] = cfg
            entry["name"] = cfg.get("project_name") or entry.get(
                "name", _basename_from_path(project["path"])
            )
        elif "name" not in entry:
            entry["name"] = _basename_from_path(project["path"])
        result.append(entry)
    return result


@app.post("/api/projects")
async def create_project(body: ProjectCreate):
    if body.sandbox_mode not in SANDBOX_MODES:
        raise HTTPException(
            status_code=400,
            detail=f"sandbox_mode must be one of {SANDBOX_MODES}",
        )
    project_id = str(uuid.uuid4())
    name = _basename_from_path(body.path)
    project = {
        "id": project_id,
        "path": body.path,
        "name": name,
        "sandbox_mode": body.sandbox_mode,
        # Resource Governor lifecycle state: "active" or "suspended".
        "state": "active",
    }
    projects[project_id] = project
    project_last_active[project_id] = time.monotonic()
    config = _default_project_config(project_id)
    config["sandbox_mode"] = body.sandbox_mode
    project_configs[project_id] = config
    return {**project, "config": config}


@app.get("/api/projects/{project_id}/config")
async def get_project_config(project_id: str):
    if project_id not in projects:
        raise HTTPException(status_code=404, detail="Project not found")
    if project_id not in project_configs:
        project_configs[project_id] = _default_project_config(project_id)
    return project_configs[project_id]


@app.put("/api/projects/{project_id}/config")
async def save_project_config(project_id: str, body: ProjectConfig):
    if project_id not in projects:
        raise HTTPException(status_code=404, detail="Project not found")

    if body.sandbox_mode not in SANDBOX_MODES:
        raise HTTPException(
            status_code=400,
            detail=f"sandbox_mode must be one of {SANDBOX_MODES}",
        )
    if body.handoff_method not in HANDOFF_METHODS:
        raise HTTPException(
            status_code=400,
            detail=f"handoff_method must be one of {HANDOFF_METHODS}",
        )

    project = projects[project_id]
    display_name = (body.project_name or "").strip() or _basename_from_path(project["path"])
    saved = {
        "project_name": display_name,
        "sandbox_mode": body.sandbox_mode,
        "mem_limit": body.mem_limit or DEFAULT_MEM_LIMIT,
        "handoff_method": body.handoff_method,
    }
    project_configs[project_id] = saved
    project["name"] = display_name
    return saved


@app.post("/api/projects/{project_id}/activate")
async def activate_project(project_id: str):
    global focused_project_id
    if project_id not in projects:
        raise HTTPException(status_code=404, detail="Project not found")
    project = projects[project_id]

    # If the governor suspended this project, resume its containers before
    # focusing it (so clicking a suspended project transparently wakes it).
    if project.get("state") == "suspended":
        try:
            runtime = get_docker_runtime()
            for container_id in _project_container_ids(project_id):
                await runtime.resume_container(container_id)
        except HTTPException:
            pass  # Docker unavailable — nothing to resume.
        project["state"] = "active"

    # Mark this as the focused project (governor never suspends it) and bump
    # its LRU timestamp.
    focused_project_id = project_id
    project_last_active[project_id] = time.monotonic()
    return {"status": "ok"}


@app.post("/api/projects/{project_id}/resume")
async def resume_project(project_id: str):
    """Unpause a governor-suspended project's containers and reactivate it.

    Called from the frontend when the user clicks a suspended project.
    """
    project = projects.get(project_id)
    if project is None:
        raise HTTPException(status_code=404, detail="Project not found")

    runtime = get_docker_runtime()
    for container_id in _project_container_ids(project_id):
        await runtime.resume_container(container_id)

    project["state"] = "active"
    # Resuming counts as recent use, so it isn't immediately re-suspended.
    project_last_active[project_id] = time.monotonic()
    return {"status": "ok", "state": "active"}


# Strong references to fire-and-forget background tasks. The event loop keeps
# only weak refs to tasks, so without this a task can be garbage-collected
# mid-flight (and silently cancelled).
_background_tasks: set[asyncio.Task] = set()


def _spawn_background(coro) -> None:
    """Schedule a coroutine without blocking the caller, holding a strong ref."""
    task = asyncio.create_task(coro)
    _background_tasks.add(task)
    task.add_done_callback(_background_tasks.discard)


def _close_pty_session(session: pexpect.spawn) -> None:
    """Force-close a pexpect child, swallowing any teardown errors.

    Runs in a worker thread (close(force=True) waits on the child), and is
    deliberately defensive: the session may already be dead or mid-read.
    """
    try:
        if session.isalive():
            session.close(force=True)
    except Exception:
        pass


async def _stop_container_bg(container_id: str) -> None:
    """Best-effort container teardown for fire-and-forget background tasks."""
    try:
        await get_docker_runtime().stop_container(container_id)
    except Exception:
        pass


# ---------------------------------------------------------------------------
# Resource Governor
#
# Background loop that pauses (freezes) the Docker containers of idle projects
# when system RAM gets tight, so multiple open projects don't exhaust memory.
# Paused containers keep their RAM but use no CPU; resuming is instant.
# ---------------------------------------------------------------------------
def _project_container_ids(project_id: str) -> list[str]:
    """Container IDs for a project's docker-mode instances.

    Native instances have no container (container_id is None) and are skipped.
    """
    return [
        instance["container_id"]
        for instance in instances.values()
        if instance.get("project_id") == project_id
        and instance.get("container_id")
    ]


def _suspendable_projects() -> list[str]:
    """Non-focused, active projects that own >=1 container, LRU-first."""
    candidates = [
        project_id
        for project_id, project in projects.items()
        if project_id != focused_project_id
        and project.get("state") != "suspended"
        and _project_container_ids(project_id)
    ]
    return sorted(
        candidates, key=lambda pid: project_last_active.get(pid, 0.0)
    )


async def _suspend_project(project_id: str) -> bool:
    """Pause every container of a project and mark it suspended."""
    container_ids = _project_container_ids(project_id)
    if not container_ids:
        return False
    try:
        runtime = get_docker_runtime()
    except HTTPException:
        return False
    for container_id in container_ids:
        await runtime.pause_container(container_id)
    projects[project_id]["state"] = "suspended"
    print(
        f"[governor] suspended project {project_id} "
        f"({len(container_ids)} container(s) paused)",
        flush=True,
    )
    return True


async def _governor_tick() -> None:
    """One RAM check; suspend project(s) if memory is over threshold."""
    percent = psutil.virtual_memory().percent

    if percent >= RAM_EMERGENCY_THRESHOLD:
        # Emergency: pause every non-focused project.
        targets = _suspendable_projects()
        if targets:
            print(
                f"[governor] RAM {percent:.0f}% >= "
                f"{RAM_EMERGENCY_THRESHOLD}% — emergency pausing "
                f"{len(targets)} project(s)"
            )
        for project_id in targets:
            await _suspend_project(project_id)
    elif percent >= RAM_SUSPEND_THRESHOLD:
        # Pause only the single least-recently-used eligible project.
        targets = _suspendable_projects()
        if targets:
            print(
                f"[governor] RAM {percent:.0f}% >= "
                f"{RAM_SUSPEND_THRESHOLD}% — suspending LRU project "
                f"{targets[0]}"
            )
            await _suspend_project(targets[0])


async def resource_governor() -> None:
    """Run a governor tick every GOVERNOR_INTERVAL_SECONDS, forever."""
    while True:
        await asyncio.sleep(GOVERNOR_INTERVAL_SECONDS)
        try:
            await _governor_tick()
        except Exception as exc:  # never let the loop die
            print(f"[governor] tick error: {exc}")


@app.post("/api/projects/{project_id}/unload")
async def unload_project(project_id: str):
    """Tear down a project and every instance / container it owns.

    Idempotent: unloading an unknown (already-unloaded) project is a no-op so
    the frontend's close button never errors.
    """
    project = projects.get(project_id)
    if project is None:
        return {"status": "ok", "message": "Already unloaded"}

    # Snapshot the IDs first since we mutate `instances` while iterating.
    instance_ids = [
        instance_id
        for instance_id, instance in instances.items()
        if instance.get("project_id") == project_id
    ]

    project_configs.pop(project_id, None)

    for instance_id in instance_ids:
        instance = instances[instance_id]

        # Detach the live shell (if any) before destroying its container, so
        # the streaming WebSocket sees EOF and tears down cleanly. For native
        # instances this is the only thing that kills the host-side shell.
        session = pty_sessions.pop(instance_id, None)
        if session is not None:
            await asyncio.to_thread(_close_pty_session, session)

        container_id = instance.get("container_id")
        if container_id:
            await get_docker_runtime().stop_container(container_id)

        del instances[instance_id]

    del projects[project_id]
    return {"status": "ok", "message": "Project unloaded"}


# ---------------------------------------------------------------------------
# Instance endpoints
# ---------------------------------------------------------------------------
@app.post("/api/instances/start")
async def start_instance(body: InstanceStart):
    project = projects.get(body.project_id)
    if project is None:
        raise HTTPException(status_code=404, detail="Project not found")

    if body.sandbox_mode not in SANDBOX_MODES:
        raise HTTPException(
            status_code=400,
            detail=f"sandbox_mode must be one of {SANDBOX_MODES}",
        )

    instance_id = str(uuid.uuid4())
    instance = {
        "id": instance_id,
        "project_id": body.project_id,
        "sandbox_mode": body.sandbox_mode,
    }

    if body.sandbox_mode == "native":
        # No Docker — the WebSocket spawns a shell directly on the host.
        # Record container_id as None so the registry shape stays uniform.
        instance["container_id"] = None
    else:
        mem_limit = body.mem_limit or DEFAULT_MEM_LIMIT
        instance["container_id"] = await get_docker_runtime().start_container(
            project_path=project["path"],
            instance_id=instance_id,
            mem_limit=mem_limit,
        )
        instance["mem_limit"] = mem_limit

    instances[instance_id] = instance
    return instance


@app.delete("/api/instances/{instance_id}")
async def delete_instance(instance_id: str):
    """Tear down a single instance and return immediately.

    Idempotent (an unknown id is a no-op so closing a tab never errors). The
    container is stopped in the background so the response doesn't block on
    Docker's stop grace period (~10s), which was causing the frontend to hang.
    """
    instance = instances.pop(instance_id, None)
    if instance is None:
        return {"status": "ok"}

    # Detach any live shell so the streaming WebSocket tears down. Closing can
    # briefly block (waitpid), so run it off the event loop and don't wait.
    session = pty_sessions.pop(instance_id, None)
    if session is not None:
        _spawn_background(asyncio.to_thread(_close_pty_session, session))

    # Stop/remove the container without blocking the response.
    container_id = instance.get("container_id")
    if container_id:
        _spawn_background(_stop_container_bg(container_id))

    return {"status": "ok"}


@app.post("/api/instances/{instance_id}/focus")
async def focus_instance(instance_id: str):
    if instance_id not in instances:
        raise HTTPException(status_code=404, detail="Instance not found")
    return {"status": "ok"}


@app.post("/api/handoff")
async def execute_handoff(body: HandoffRequest):
    if body.method not in HANDOFF_METHODS:
        raise HTTPException(
            status_code=400,
            detail=f"method must be one of {HANDOFF_METHODS}",
        )

    project = projects.get(body.project_id)
    if project is None:
        raise HTTPException(status_code=404, detail="Project not found")

    from_instance = instances.get(body.from_instance_id)
    if from_instance is None:
        raise HTTPException(status_code=404, detail="from_instance not found")

    to_instance = instances.get(body.to_instance_id)
    if to_instance is None:
        raise HTTPException(status_code=404, detail="to_instance not found")

    if from_instance.get("project_id") != body.project_id:
        raise HTTPException(
            status_code=400,
            detail="from_instance does not belong to project_id",
        )
    if to_instance.get("project_id") != body.project_id:
        raise HTTPException(
            status_code=400,
            detail="to_instance does not belong to project_id",
        )

    project_path = project["path"]
    task_description = body.task_description

    if body.method == "git":
        try:
            await handoff_engine.git_handoff(
                project_path, f"Handoff checkpoint: {task_description}"
            )
        except (git.InvalidGitRepositoryError, git.NoSuchPathError):
            return JSONResponse(
                status_code=400,
                content={
                    "status": "error",
                    "message": (
                        "Project is not a Git repository. "
                        "Initialize git first or use Summary handoff."
                    ),
                },
            )
        injection_prompt = (
            "You are taking over this project. The previous AI just committed "
            "their work. Please review the latest git commit using "
            "'git log -1 -p' to understand the current state of the codebase. "
            f"Your overall goal is: {task_description}. Continue working on "
            "this goal from where the previous AI left off."
        )
    else:
        await handoff_engine.summary_handoff(project_path, task_description)
        injection_prompt = (
            "You are taking over this project. Read the file "
            "'.handover/handoffs/latest.md' to see the current state. "
            f"Your overall goal is: {task_description}. Continue from there."
        )

    # Inject the prompt into the new AI's live shell. Send a newline first to
    # land on a fresh prompt, briefly wait, then send the handoff prompt.
    child = pty_sessions.get(body.to_instance_id)
    if child is not None and child.isalive():
        await asyncio.to_thread(child.sendline, "\n")
        await asyncio.sleep(0.5)
        await asyncio.to_thread(child.sendline, injection_prompt)

    return {"status": "ok", "message": "Handoff complete"}


# ---------------------------------------------------------------------------
# PTY WebSocket
# ---------------------------------------------------------------------------
@app.websocket("/ws/pty/{instance_id}")
async def websocket_pty(websocket: WebSocket, instance_id: str):
    # Reject connections for instances we don't know about. Closing before
    # accept() causes Starlette to reject the handshake outright.
    instance = instances.get(instance_id)
    if instance is None:
        await websocket.close(code=4404)
        return

    mode = instance.get("sandbox_mode", "docker")
    container_id = instance.get("container_id")
    if mode == "docker" and not container_id:
        await websocket.close(code=4404)
        return

    await websocket.accept()

    env = os.environ.copy()
    env["TERM"] = "xterm-256color"

    if mode == "native":
        # Spawn the user's own shell directly on the host (no Docker), so the
        # terminal has access to every CLI they've installed. Run it as an
        # interactive login shell so it sources ~/.bashrc / ~/.zshrc and picks
        # up their full PATH and environment. Start in the project directory.
        shell = os.environ.get("SHELL") or "/bin/bash"
        if not os.path.isfile(shell):
            shell = "/bin/bash"
        project = projects.get(instance.get("project_id"))
        cwd = (
            project["path"]
            if project and os.path.isdir(project["path"])
            else os.path.expanduser("~")
        )
        child = pexpect.spawn(
            shell,
            ["-l", "-i"],
            cwd=cwd,
            encoding="utf-8",
            timeout=None,
            maxread=1024,
            env=env,
        )
    else:
        # Docker mode: drop the user into a bash prompt inside the sandbox
        # container. pexpect allocates a PTY and `docker exec -it` wires that
        # PTY through to the container's bash.
        child = pexpect.spawn(
            "docker",
            ["exec", "-it", container_id, "/bin/bash"],
            encoding="utf-8",
            timeout=None,
            maxread=1024,
            env=env,
        )

    # Track the live session so /unload can tear it down even while it's
    # actively streaming.
    pty_sessions[instance_id] = child

    async def pty_to_ws():
        try:
            while True:
                # Blocking pexpect read offloaded to a thread so the
                # event loop stays responsive.
                data = await asyncio.to_thread(
                    child.read_nonblocking, 1024, None
                )
                if not data:
                    break
                await websocket.send_text(data)
        except (pexpect.EOF, pexpect.exceptions.EOF):
            pass
        except Exception:
            pass

    async def ws_to_pty():
        try:
            while True:
                data = await websocket.receive_text()
                child.send(data)
        except WebSocketDisconnect:
            pass
        except Exception:
            pass

    reader = asyncio.create_task(pty_to_ws())
    writer = asyncio.create_task(ws_to_pty())

    try:
        _, pending = await asyncio.wait(
            {reader, writer}, return_when=asyncio.FIRST_COMPLETED
        )
        for task in pending:
            task.cancel()
    finally:
        # Drop our session entry, unless a newer WS for the same instance has
        # already replaced it (or /unload already removed and closed it).
        if pty_sessions.get(instance_id) is child:
            del pty_sessions[instance_id]
        try:
            if child.isalive():
                child.close(force=True)
        except Exception:
            pass
        try:
            await websocket.close()
        except Exception:
            pass


if __name__ == "__main__":
    import uvicorn
    uvicorn.run(app, host="127.0.0.1", port=8765)
