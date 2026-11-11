"""Backend end-to-end checks (native + docker). Usage: e2e_basic.py PORT ROOT_DIR; needs HANDOVER_TOKEN."""
import asyncio, json, os, re, subprocess, sys, time, urllib.request
import websockets

PORT = int(sys.argv[1]); ROOT = sys.argv[2]
TOKEN = os.environ["HANDOVER_TOKEN"]
BASE = f"http://127.0.0.1:{PORT}"
CP = "__handover_control__:"

def http(method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method,
        data=json.dumps(body).encode() if body is not None else None,
        headers={"Content-Type": "application/json", "X-Handover-Token": TOKEN})
    try:
        with urllib.request.urlopen(req) as r: return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e:
        raw = e.read()
        try: return e.code, json.loads(raw or b"{}")
        except ValueError: return e.code, {"raw": raw.decode(errors="replace")}

def ws_url(iid): return f"ws://127.0.0.1:{PORT}/ws/pty/{iid}?token={TOKEN}"

async def read_until(ws, needle, timeout=8):
    buf, end = "", time.time() + timeout
    while time.time() < end:
        try: m = await asyncio.wait_for(ws.recv(), timeout=max(0.1, end - time.time()))
        except (asyncio.TimeoutError, websockets.ConnectionClosed): break
        if not m.startswith(CP): buf += m
        if needle in buf: return buf
    return buf

def alive(pid):
    try: os.kill(pid, 0); return True
    except ProcessLookupError: return False

async def native(r):
    d = os.path.join(ROOT, "proj"); os.makedirs(d, exist_ok=True)
    subprocess.run(["git", "init", "-q", d], check=True)
    _, p = http("POST", "/api/projects", {"path": d, "sandbox_mode": "native"}); pid_ = p["id"]
    http("PUT", f"/api/projects/{pid_}/config", {"sandbox_mode": "native", "handoff_method": "summary"})
    cfg_path = os.path.join(d, ".handover/config.yml")
    text = open(cfg_path).read()
    open(cfg_path, "w").write(text.replace("handoff_method: summary", "handoff_method: git"))
    _, c = http("GET", f"/api/projects/{pid_}/config")
    r["config file edit wins"] = c.get("handoff_method") == "git"
    st, _ = http("GET", f"/api/projects/{pid_}/git-diff?from=--output=/tmp/pwned_handover&to=HEAD")
    r["git-diff rejects --output"] = st == 400 and not os.path.exists("/tmp/pwned_handover")
    # Settings come from the project's config (config.yml edits included).
    text = open(cfg_path).read()
    open(cfg_path, "w").write(text.replace("custom_env_vars: {}", "custom_env_vars:\n  HANDOVER_SMOKE: env_ok_42"))
    _, inst = http("POST", "/api/instances/start", {"project_id": pid_})
    iid = inst["id"]
    async with websockets.connect(ws_url(iid)) as ws:
        await ws.send('echo "VAR=$HANDOVER_SMOKE PI""D=$$"\r')
        out = await read_until(ws, "VAR=env_ok_42")
        r["native env vars applied"] = "VAR=env_ok_42" in out
        out += await read_until(ws, "\n", 1)
        m = re.search(r"PID=(\d+)", out); shell_pid = int(m.group(1)) if m else None
        await ws.send("python3 -c \"print('→'*20000 + 'END_ARROWS')\"\r")
        out = await read_until(ws, "END_ARROWS", 10)
        r["no U+FFFD in multibyte output"] = "END_ARROWS" in out and "�" not in out
        http("POST", f"/api/projects/{pid_}/broadcast", {"prompt": "echo BR''OAD\ncast_ok"})
        out = await read_until(ws, "BROAD cast_ok", 5)
        r["broadcast submits (\\r, newlines collapsed)"] = "BROAD cast_ok" in out
        st, ho = http("POST", "/api/handoff", {"project_id": pid_, "from_instance_id": iid,
            "to_instance_id": iid, "method": "git", "task_description": "e2e"})
        r["git handoff reports a kind"] = st == 200 and ho.get("kind") in ("git_committed", "git_no_changes")
        http("DELETE", f"/api/instances/{iid}")
        await asyncio.sleep(1.5)
        r["delete kills shell"] = shell_pid is not None and not alive(shell_pid)

async def docker(r):
    d = os.path.join(ROOT, "dproj"); os.makedirs(d, exist_ok=True)
    _, p = http("POST", "/api/projects", {"path": d, "sandbox_mode": "docker"}); pid_ = p["id"]
    http("PUT", f"/api/projects/{pid_}/config", {"sandbox_mode": "docker", "mem_limit": "1g",
        "custom_env_vars": {"HANDOVER_SMOKE": "dock_ok_7"}})
    st, inst = http("POST", "/api/instances/start", {"project_id": pid_})
    r["config mem_limit applied"] = inst.get("mem_limit") == "1g"
    r["docker instance starts"] = st == 200 and bool(inst.get("container_id"))
    iid = inst["id"]
    async with websockets.connect(ws_url(iid)) as ws:
        await ws.send('echo "V=$HANDOVER_SMOKE N=$(node --version) C=$(claude --version | cut -d" " -f1)"; echo "DO""NE"\r')
        out = await read_until(ws, "DONE", 15)
        r["env var reaches container"] = "V=dock_ok_7" in out
        r["node 24 in sandbox"] = "N=v24." in out
        r["claude on PATH"] = "C=2." in out
        st, stats = http("GET", f"/api/instances/{iid}/stats")
        r["container stats"] = st == 200 and stats.get("mem_limit_mb", 0) > 900
        st, _ = http("GET", f"/api/projects/{pid_}/resources")
        r["resources endpoint"] = st == 200
    http("DELETE", f"/api/instances/{iid}")
    await asyncio.sleep(3)
    left = subprocess.run(["docker", "ps", "-aq", "--filter", f"label=handover.instance_id={iid}"],
                          capture_output=True, text=True).stdout.split()
    r["delete removes container"] = left == []

async def main():
    r = {}
    await native(r)
    await docker(r)
    for k, v in r.items(): print(("PASS " if v else "FAIL ") + k)
    sys.exit(0 if all(r.values()) else 1)
asyncio.run(main())
