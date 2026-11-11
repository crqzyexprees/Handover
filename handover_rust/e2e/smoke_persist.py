import asyncio, json, os, re, sys, time, urllib.request
import websockets
PORT = int(sys.argv[1]); ROOT = sys.argv[2]; MODE = sys.argv[3] if len(sys.argv) > 3 else "native"
BASE = f"http://127.0.0.1:{PORT}"; WS = f"ws://127.0.0.1:{PORT}"
CP = "__handover_control__:"
def http(method, path, body=None):
    req = urllib.request.Request(BASE + path, method=method,
        data=json.dumps(body).encode() if body is not None else None, headers={"Content-Type": "application/json", "X-Handover-Token": os.environ["HANDOVER_TOKEN"]})
    try:
        with urllib.request.urlopen(req) as r: return r.status, json.loads(r.read() or b"{}")
    except urllib.error.HTTPError as e: return e.code, json.loads(e.read() or b"{}")
u16 = lambda s: len(s.encode("utf-16-le")) // 2

class Client:
    """Mimics TerminalSession: tracks shell session id and output offset."""
    def __init__(self): self.sid = None; self.offset = 0; self.out = ""; self.controls = []; self.ws = None
    async def connect(self, iid, resume=True):
        q = f"?session={self.sid}&since={self.offset}&" if resume and self.sid is not None else "?"
        q += f"token={os.environ['HANDOVER_TOKEN']}"
        self.ws = await websockets.connect(f"{WS}/ws/pty/{iid}{q}")
    async def read_until(self, pred, timeout=8):
        end = time.time() + timeout
        while time.time() < end:
            try: m = await asyncio.wait_for(self.ws.recv(), max(0.1, end - time.time()))
            except asyncio.TimeoutError: break
            except websockets.ConnectionClosed: break
            if m.startswith(CP):
                c = json.loads(m[len(CP):]); self.controls.append(c)
                if c["type"] == "attached": self.sid = c["session"]; self.offset = c["from"]
            else:
                self.out += m; self.offset += u16(m)
            if pred(self): return True
        return pred(self)

async def main():
    r = {}
    d = os.path.join(ROOT, f"persist_{MODE}"); os.makedirs(d, exist_ok=True)
    _, p = http("POST", "/api/projects", {"path": d, "sandbox_mode": MODE})
    _, inst = http("POST", "/api/instances/start", {"project_id": p["id"], "sandbox_mode": MODE})
    iid = inst["id"]

    c = Client(); await c.connect(iid)
    await c.read_until(lambda c: c.sid is not None)
    await c.ws.send('export KEEP=alive_1; echo "P""ID=$$"; (sleep 2; echo "LA""TE_OUT") &\r')
    await c.read_until(lambda c: re.search(r"PID=\d+", c.out))
    pid1 = re.search(r"PID=(\d+)", c.out).group(1); sid1 = c.sid
    await c.ws.close()                                  # disconnect while job is running
    await asyncio.sleep(3)

    c.out = ""; await c.connect(iid)                    # resume
    await c.read_until(lambda c: "LATE_OUT" in c.out, 5)
    r["output while disconnected is replayed"] = "LATE_OUT" in c.out
    r["replay excludes already-seen output"] = "PID=" not in c.out
    await c.ws.send('echo "K=$KEEP P""ID=$$"\r')
    await c.read_until(lambda c: "K=alive_1" in c.out)
    r["same shell after reconnect (env kept)"] = "K=alive_1" in c.out and f"PID={pid1}" in c.out
    r["same session id"] = c.sid == sid1

    c2 = Client(); await c2.connect(iid)                # second window takes over
    await c2.read_until(lambda c: "K=alive_1" in c.out, 5)
    await c.read_until(lambda c: any(x["type"] == "replaced" for x in c.controls), 3)
    r["older client told it was replaced"] = any(x["type"] == "replaced" for x in c.controls)
    r["fresh client gets full history"] = "K=alive_1" in c2.out
    await c2.ws.close()

    http("POST", f"/api/projects/{p['id']}/broadcast", {"prompt": "echo BR''OAD_detached"})
    await asyncio.sleep(1.5)
    c3 = Client(); await c3.connect(iid)
    await c3.read_until(lambda c: "BROAD_detached" in c.out, 5)
    r["broadcast reaches shell with no client attached"] = "BROAD_detached" in c3.out

    await c3.ws.send("exit\r")
    await c3.read_until(lambda c: any(x["type"] == "exit" for x in c.controls), 5)
    r["client told when shell exits"] = any(x["type"] == "exit" for x in c3.controls)
    old_sid = c3.sid
    await asyncio.sleep(0.5)
    c3.controls = []; await c3.connect(iid)
    await c3.read_until(lambda c: any(x["type"] == "attached" for x in c.controls))
    r["reattach after exit starts a new shell"] = c3.sid is not None and c3.sid != old_sid

    http("DELETE", f"/api/instances/{iid}")
    await c3.read_until(lambda c: any(x["type"] == "exit" for x in c.controls), 5)
    r["delete ends the shell"] = any(x["type"] == "exit" for x in c3.controls)
    for k, v in r.items(): print(("PASS " if v else "FAIL ") + f"[{MODE}] " + k)
    sys.exit(0 if all(r.values()) else 1)
asyncio.run(main())
