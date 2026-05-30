import asyncio
import glob
import grp
import os
import pwd
import shutil
import tempfile

import docker

# Label used to identify containers we own, so we can sweep orphans on boot.
INSTANCE_LABEL = "handover.instance_id"

# Every sandbox is a plain bash environment; the user runs claude/codex/etc.
# themselves once attached.
BASE_IMAGE = "handover-base:latest"

# Writable HOME inside the container (created in the base image). The sandbox
# runs as the host's non-root UID, so HOME can't be /root.
SANDBOX_HOME = "/home/sandbox"

# Prefix for the temp dirs holding generated passwd/group files, so they can
# be identified and swept on startup.
NSS_DIR_PREFIX = "handover-nss-"

# Host API keys forwarded into the container so CLIs skip the login screen.
FORWARDED_API_KEYS = ("ANTHROPIC_API_KEY", "OPENAI_API_KEY")

# Read-only staging dir where host auth/config is bind-mounted, then copied
# into the writable HOME at container start (see _COPY_CONFIG_CMD). We copy
# rather than mount directly because write-heavy CLIs (codex, cursor, ...)
# must write into their config dir (sqlite/logs/token refresh), which a
# read-only mount forbids. Copies are throwaway, so host files are untouched.
CONFIG_STAGING_DIR = "/mnt/host-config"

# Host auth/config dirs, keyed by their path relative to HOME inside the
# container. Only those that exist on the host are staged.
CONFIG_MOUNTS = {
    "~/.claude": ".claude",
    "~/.codex": ".codex",
    "~/.cursor": ".cursor",
    "~/.config/cursor": ".config/cursor",
    "~/.antigravity": ".antigravity",
    "~/.gemini": ".gemini",
    "~/.config/claude-code": ".config/claude-code",
    "~/.config/claude": ".config/claude",
}

# Host auth/config *files*, same keying as CONFIG_MOUNTS. Claude reads its
# main config/credentials from ~/.claude.json specifically.
CONFIG_FILE_MOUNTS = {
    "~/.claude.json": ".claude.json",
}

# Copies the staged read-only config into the writable HOME, then makes it
# writable by the (owning) sandbox user. Run once at container start.
_COPY_CONFIG_CMD = (
    f"if [ -d {CONFIG_STAGING_DIR} ]; then "
    f"cp -a {CONFIG_STAGING_DIR}/. $HOME/ 2>/dev/null; "
    f"chmod -R u+rwX $HOME 2>/dev/null; fi; true"
)

# Resource ceilings applied to every sandbox container.
DEFAULT_MEM_LIMIT = "2g"
NANO_CPUS = 1_000_000_000  # 1.0 CPU

_BYTES_PER_MB = 1024 * 1024


def _cpu_percent(stats: dict) -> float:
    """CPU usage percent from a Docker stats snapshot.

    Computed the same way ``docker stats`` does: the container's CPU-time
    delta over the system CPU-time delta, scaled by the number of CPUs. With
    100% meaning one fully-used core (so a 4-core host can report up to 400%).
    Returns 0.0 if either delta is non-positive (e.g. the first sample after
    start, when there's no prior reading to diff against).
    """
    cpu = stats.get("cpu_stats", {})
    precpu = stats.get("precpu_stats", {})
    cpu_usage = cpu.get("cpu_usage", {})
    precpu_usage = precpu.get("cpu_usage", {})

    cpu_delta = cpu_usage.get("total_usage", 0) - precpu_usage.get("total_usage", 0)
    system_delta = cpu.get("system_cpu_usage", 0) - precpu.get("system_cpu_usage", 0)
    if cpu_delta <= 0 or system_delta <= 0:
        return 0.0

    # online_cpus is absent on older daemons; fall back to the per-CPU array.
    online_cpus = cpu.get("online_cpus") or len(cpu_usage.get("percpu_usage") or [])
    if not online_cpus:
        return 0.0

    return round((cpu_delta / system_delta) * online_cpus * 100.0, 1)


def _mem_used_mb(stats: dict) -> float:
    """Memory in use (MB), with page cache excluded to match ``docker stats``."""
    mem = stats.get("memory_stats", {})
    usage = mem.get("usage", 0)
    # Newer cgroup v2 reports cache under "inactive_file"; v1 under "cache".
    detail = mem.get("stats", {})
    cache = detail.get("inactive_file", detail.get("cache", 0))
    return round(max(usage - cache, 0) / _BYTES_PER_MB, 1)


def _mem_limit_mb(stats: dict) -> float:
    """Container memory limit (MB)."""
    limit = stats.get("memory_stats", {}).get("limit", 0)
    return round(limit / _BYTES_PER_MB, 1)


class DockerRuntime:
    """Manages the lifecycle of per-instance sandbox containers.

    The docker SDK is fully synchronous, so every blocking call is offloaded
    to a worker thread via ``asyncio.to_thread`` to keep the FastAPI event
    loop responsive.
    """

    def __init__(self):
        self.client = docker.from_env()
        # Generated once and reused for every container (the host UID's name
        # doesn't change during the server's lifetime).
        self._nss_dir = self._build_nss_files()

    @staticmethod
    def _build_nss_files():
        """Write minimal passwd/group files so the container can resolve the
        host UID's username (avoids the "I have no name!" prompt).

        We mount *these generated copies* rather than the host's real
        /etc/passwd and /etc/group: the bind mounts use SELinux relabeling
        ("z"), and relabeling the host's actual system auth files
        (passwd_file_t -> container_file_t) under enforcing SELinux can break
        host login/sudo. Relabeling throwaway copies is harmless.
        """
        uid = os.getuid()
        gid = os.getgid()
        user = pwd.getpwuid(uid)
        group = grp.getgrgid(gid)

        nss_dir = tempfile.mkdtemp(prefix=NSS_DIR_PREFIX)
        with open(os.path.join(nss_dir, "passwd"), "w") as f:
            f.write("root:x:0:0:root:/root:/bin/bash\n")
            # Home points at the writable sandbox HOME, not the host's home,
            # which doesn't exist inside the container.
            f.write(
                f"{user.pw_name}:x:{uid}:{gid}:"
                f"{user.pw_gecos}:{SANDBOX_HOME}:/bin/bash\n"
            )
        with open(os.path.join(nss_dir, "group"), "w") as f:
            f.write("root:x:0:\n")
            if gid != 0:
                f.write(f"{group.gr_name}:x:{gid}:\n")

        return nss_dir

    async def start_container(
        self,
        project_path,
        instance_id,
        mem_limit=DEFAULT_MEM_LIMIT,
        custom_env_vars=None,
    ):
        """Launch a detached, TTY-enabled bash sandbox.

        The project directory is mounted at ``/workspace``. ``mem_limit`` caps
        both RAM and swap (no swap headroom beyond RAM). ``custom_env_vars`` is
        an optional dict of extra environment variables (e.g. an OpenRouter /
        custom-proxy base URL + key) merged into the container's environment.
        Returns the new container's ID.
        """
        # Run as the host user so files created in /workspace are owned by
        # the host user instead of root.
        uid = os.getuid()
        gid = os.getgid()

        # Point HOME at the in-image writable home so the CLIs find the auth
        # configs we mount below, and forward any host API keys so the CLIs
        # can skip the login screen entirely.
        environment = {"HOME": SANDBOX_HOME}
        for key in FORWARDED_API_KEYS:
            value = os.environ.get(key)
            if value:
                environment[key] = value

        # User-supplied vars (OpenRouter/custom-proxy config, etc.) are merged
        # last so they override forwarded host keys on collision.
        if custom_env_vars:
            environment.update(
                {str(k): str(v) for k, v in custom_env_vars.items()}
            )

        # "z" relabels each bind mount for SELinux (Fedora/RHEL), without which
        # the container can't read/write the mount even when it owns the files.
        volumes = {
            project_path: {"bind": "/workspace", "mode": "rw,z"},
            # Generated passwd/group copies so the container resolves the host
            # UID's username (see _build_nss_files). Never the host's real
            # /etc files, to avoid SELinux-relabeling them.
            os.path.join(self._nss_dir, "passwd"): {
                "bind": "/etc/passwd",
                "mode": "ro,z",
            },
            os.path.join(self._nss_dir, "group"): {
                "bind": "/etc/group",
                "mode": "ro,z",
            },
        }
        # Stage host auth/config dirs and files read-only under a staging dir;
        # they're copied into the writable HOME at container start so a prior
        # `claude login` / `codex login` / etc. session carries in AND the
        # write-heavy CLIs can update their own state.
        for host_path, rel in CONFIG_MOUNTS.items():
            expanded = os.path.expanduser(host_path)
            if os.path.isdir(expanded):
                volumes[expanded] = {
                    "bind": f"{CONFIG_STAGING_DIR}/{rel}",
                    "mode": "ro,z",
                }
        for host_path, rel in CONFIG_FILE_MOUNTS.items():
            expanded = os.path.expanduser(host_path)
            if os.path.isfile(expanded):
                volumes[expanded] = {
                    "bind": f"{CONFIG_STAGING_DIR}/{rel}",
                    "mode": "ro,z",
                }

        def _run():
            container = self.client.containers.run(
                BASE_IMAGE,
                # Keep the container's main process alive so we can attach
                # interactive shells to it via `docker exec`.
                command="/bin/bash",
                detach=True,
                tty=True,
                stdin_open=True,
                user=f"{uid}:{gid}",
                # A readable, instance-scoped name aids debugging / cleanup.
                name=f"handover-{instance_id}",
                labels={
                    INSTANCE_LABEL: instance_id,
                },
                working_dir="/workspace",
                environment=environment,
                volumes=volumes,
                mem_limit=mem_limit,
                memswap_limit=mem_limit,
                nano_cpus=NANO_CPUS,
                # Linux fix so code in the container can reach the host.
                extra_hosts={"host.docker.internal": "host-gateway"},
            )
            # Copy the staged host config into the writable HOME before the
            # frontend attaches, so the CLIs see auth and can write their state.
            container.exec_run(
                cmd=["/bin/bash", "-c", _COPY_CONFIG_CMD],
                user=f"{uid}:{gid}",
                environment={"HOME": SANDBOX_HOME},
            )
            return container.id

        return await asyncio.to_thread(_run)

    async def stop_container(self, container_id):
        """Stop and remove a sandbox container.

        Safe to call for a container that is already gone.
        """

        def _stop():
            try:
                container = self.client.containers.get(container_id)
            except docker.errors.NotFound:
                return
            container.stop()
            container.remove(force=True)

        await asyncio.to_thread(_stop)

    async def pause_container(self, container_id):
        """Freeze a container's processes (Resource Governor suspend).

        Paused containers keep their RAM but consume no CPU. Safe/idempotent:
        a missing or already-paused container is a no-op.
        """

        def _pause():
            try:
                container = self.client.containers.get(container_id)
            except docker.errors.NotFound:
                return
            try:
                container.pause()
            except docker.errors.APIError:
                # Already paused (or not in a pausable state) — ignore.
                pass

        await asyncio.to_thread(_pause)

    async def resume_container(self, container_id):
        """Resume (unpause) a previously paused container. Safe/idempotent."""

        def _unpause():
            try:
                container = self.client.containers.get(container_id)
            except docker.errors.NotFound:
                return
            try:
                container.unpause()
            except docker.errors.APIError:
                # Not paused — ignore.
                pass

        await asyncio.to_thread(_unpause)

    async def get_container_stats(self, container_id):
        """Return a single live CPU/memory snapshot for a container.

        Shape: ``{"mem_used_mb": float, "mem_limit_mb": float,
        "cpu_percent": float}``. Returns all-zeros when the container is
        missing, not running, or paused (a paused container reports no CPU
        delta and its stats are meaningless to display as "live" usage).
        """
        zeros = {"mem_used_mb": 0.0, "mem_limit_mb": 0.0, "cpu_percent": 0.0}

        def _stats():
            try:
                container = self.client.containers.get(container_id)
            except docker.errors.NotFound:
                return zeros
            # `status` is "running", "paused", "exited", ... Only a running
            # (unpaused) container has meaningful live usage.
            if container.status != "running":
                return zeros

            # stream=False returns one snapshot carrying both the current
            # (`cpu_stats`) and previous (`precpu_stats`) samples, so the CPU
            # delta can be computed from a single call.
            try:
                stats = container.stats(stream=False)
            except docker.errors.APIError:
                return zeros

            return {
                "mem_used_mb": _mem_used_mb(stats),
                "mem_limit_mb": _mem_limit_mb(stats),
                "cpu_percent": _cpu_percent(stats),
            }

        return await asyncio.to_thread(_stats)

    async def cleanup_orphans(self):
        """Force-remove every container we previously created.

        Called on startup so the app always begins from a clean slate, with
        no leftover sandboxes from a prior (possibly crashed) run. Returns the
        number of containers removed.
        """

        def _cleanup():
            containers = self.client.containers.list(
                all=True,
                filters={"label": INSTANCE_LABEL},
            )
            for container in containers:
                try:
                    container.remove(force=True)
                except docker.errors.NotFound:
                    pass
            return len(containers)

        return await asyncio.to_thread(_cleanup)

    async def cleanup_nss_temp_dirs(self):
        """Remove leftover passwd/group temp dirs from prior runs.

        The current process's dir (``self._nss_dir``) is kept. Returns the
        number of stale dirs removed.
        """

        def _cleanup():
            pattern = os.path.join(tempfile.gettempdir(), f"{NSS_DIR_PREFIX}*")
            removed = 0
            for path in glob.glob(pattern):
                if path == self._nss_dir or not os.path.isdir(path):
                    continue
                shutil.rmtree(path, ignore_errors=True)
                removed += 1
            return removed

        return await asyncio.to_thread(_cleanup)
