"""Backend lifecycle checks: idle exit, shells keep it alive, teardown on SIGTERM."""
import json, os, signal, subprocess, sys, time, urllib.request, asyncio
import websockets

BIN, ROOT = sys.argv[1], sys.argv[2]
TOKEN = "lifecycle-token"
ok = True

def check(name, cond):
    global ok
    print(("PASS " if cond else "FAIL ") + name); ok &= bool(cond)

def start(port, idle):
    env = dict(os.environ, HANDOVER_TOKEN=TOKEN, HANDOVER_STATE_PATH=f"{ROOT}/state-{port}.json")
    args = [BIN, "--host", "127.0.0.1", "--port", str(port)] + (["--idle-exit-secs", str(idle)] if idle else [])
    p = subprocess.Popen(args, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    for _ in range(100):
        try:
            urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=1); return p
        except Exception: time.sleep(0.1)
    raise SystemExit("backend did not start")

def api(port, method, path, body=None):
    req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"X-Handover-Token": TOKEN, "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=10) as r: return json.loads(r.read() or b"null")

def wait_exit(p, secs):
    try: p.wait(secs); return True
    except subprocess.TimeoutExpired: return False

async def run_marker(port, iid):
    async with websockets.connect(f"ws://127.0.0.1:{port}/ws/pty/{iid}?token={TOKEN}") as ws:
        await ws.send("echo LIFE""MARK\r")
        buf = ""
        while "LIFEMARK" not in buf.replace("LIFE""MARK\r", "", 1) or buf.count("LIFEMARK") < 2:
            buf += await asyncio.wait_for(ws.recv(), 10)

# 1. Idle backend exits on its own.
p = start(18801, 2)
check("idle backend exits by itself", wait_exit(p, 10))

# 2. A running shell keeps it alive; idle-shutdown is refused; list shows the instance.
p = start(18802, 2)
os.makedirs(f"{ROOT}/life", exist_ok=True)
pid = api(18802, "POST", "/api/projects", {"path": f"{ROOT}/life", "sandbox_mode": "native"})["id"]
iid = api(18802, "POST", "/api/instances/start", {"project_id": pid, "sandbox_mode": "native"})["id"]
asyncio.run(run_marker(18802, iid))
check("shell running: no idle exit", not wait_exit(p, 6))
check("idle shutdown refused while shell runs", api(18802, "POST", "/api/shutdown", {"only_if_idle": True})["stopping"] is False)
projects = api(18802, "GET", "/api/projects")
check("project list includes the open instance", [i["id"] for i in projects[0]["instances"]] == [iid])
check("ping reports one shell", api(18802, "GET", "/api/ping")["shells"] == 1)
api(18802, "DELETE", f"/api/instances/{iid}")
check("after last shell ends, idle exit happens", wait_exit(p, 12))

# 3. A docker Instance with no shell attached still blocks idle exit.
p = start(18804, 2)
os.makedirs(f"{ROOT}/life3", exist_ok=True)
pid = api(18804, "POST", "/api/projects", {"path": f"{ROOT}/life3", "sandbox_mode": "docker"})["id"]
inst = api(18804, "POST", "/api/instances/start", {"project_id": pid, "sandbox_mode": "docker"})
check("container without shell: no idle exit", not wait_exit(p, 6))
check("ping reports it busy", api(18804, "GET", "/api/ping")["busy"] == 1)
api(18804, "DELETE", f"/api/instances/{inst['id']}")
check("after container closes, idle exit happens", wait_exit(p, 15))

# 4. SIGTERM tears down docker containers.
p = start(18803, None)
os.makedirs(f"{ROOT}/life2", exist_ok=True)
pid = api(18803, "POST", "/api/projects", {"path": f"{ROOT}/life2", "sandbox_mode": "docker"})["id"]
inst = api(18803, "POST", "/api/instances/start", {"project_id": pid, "sandbox_mode": "docker"})
name = f"handover-{inst['id']}"
running = subprocess.run(["docker", "ps", "-q", "--filter", f"name={name}"], capture_output=True, text=True).stdout.strip()
check("docker container running", running != "")
p.send_signal(signal.SIGTERM)
check("SIGTERM exits", wait_exit(p, 20))
left = subprocess.run(["docker", "ps", "-aq", "--filter", f"name={name}"], capture_output=True, text=True).stdout.strip()
check("container removed on exit", left == "")
sys.exit(0 if ok else 1)
