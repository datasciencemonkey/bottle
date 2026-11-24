# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Lockdown canaries for the pinned Genie Code app-server. LOCAL ONLY (a release gate).

Run: uv run tools/genie-lockdown/lockdown.py [--record]

It drives the real `genie app-server` (pinned 0.5.0) against a FAKE Responses server on
127.0.0.1. It makes no Databricks call, no model call and picks no profile. It checks that:
  1. the binary is the pinned version and has the pinned SHA-256 (pin.rs);
  2. the tool list that genie offers the model matches the pinned snapshot and has no shell,
     web search or image tool;
  3. an `apply_patch` canary creates no file (read-only sandbox);
  4. no server-to-client request arrives (no approval, no elicitation);
  5. the thread is ephemeral, the CLI version is the pin, and remote control is disabled;
  6. the `bottle` MCP server starts with `env_vars` forwarding and `ask_genie` works;
  7. no file under GENIE_HOME or the temp cwd holds the prompt or the answer canary, during
     the live session and after it (disk canary);
  8. only the fake model host is contacted (egress canary through a recording proxy);
  9. the `genie doctor --json` enabled-flag snapshot equals the pinned snapshot.
The config is the golden `config.toml` of the Rust session host, with only the model host, the
auth command and the shim command replaced. With --record it writes the snapshots of 2 and 9.
"""
import hashlib, http.server, json, os, re, secrets, shutil, socket, subprocess, sys, tempfile, threading, time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
GOLDEN = ROOT / "crates/mog-core/src/agent_run/session/golden_config.toml"
PIN = (ROOT / "crates/mog-core/src/agent_run/pin.rs").read_text()
PINNED = re.search(r'GENIE_PINNED: &str = "([^"]+)"', PIN).group(1)
DIGEST = re.search(r'"aarch64-apple-darwin",\s*"genie",\s*"([0-9a-f]{64})"', PIN).group(1)
FORBIDDEN_TOOLS = {"shell", "local_shell", "exec_command", "shell_command", "web_search",
                   "view_image", "unified_exec", "write_stdin", "js_repl", "image_generation"}
RECORD = "--record" in sys.argv
failures: list[str] = []


def check(ok: bool, what: str, detail: str = "") -> None:
    print(("PASS  " if ok else "FAIL  ") + what + (f"  [{detail}]" if detail and not ok else ""))
    if not ok:
        failures.append(what)


def find_genie() -> Path:
    cands = [os.environ.get("GENIE_BIN", ""),
             str(Path.home() / f".genie/packages/standalone/releases/{PINNED}-aarch64-apple-darwin/bin/genie"),
             shutil.which("genie") or ""]
    for c in cands:
        if c and Path(c).is_file():
            return Path(c).resolve()
    sys.exit("genie not found (set GENIE_BIN)")


def sha256(p: Path) -> str:
    return hashlib.sha256(p.read_bytes()).hexdigest()


# ---------- the fake Responses server (the "model") ----------
class Model:
    def __init__(self, patch_target: str):
        self.requests: list[dict] = []
        self.hits: list[str] = []
        self.patch_target = patch_target

    def models_json(self) -> bytes:
        m = {"slug": "genie-code-cli-primary", "display_name": "fake", "description": "fake",
             "default_reasoning_level": "low",
             "supported_reasoning_levels": [{"effort": "low", "description": "low"}],
             "shell_type": "shell_command", "visibility": "list", "supported_in_api": True,
             "priority": 1, "base_instructions": "You are a test.",
             "supports_reasoning_summaries": False, "support_verbosity": False,
             "apply_patch_tool_type": "freeform",
             "truncation_policy": {"mode": "bytes", "limit": 10000},
             "supports_parallel_tool_calls": True, "context_window": 200000,
             "experimental_supported_tools": [], "input_modalities": ["text"]}
        return json.dumps({"models": [m]}).encode()

    def script(self, body: dict) -> list[dict]:
        items = body.get("input", [])
        outputs = {i.get("call_id"): i for i in items if i.get("type", "").endswith("_output")}
        calls = {i.get("call_id"): i for i in items if i.get("type") in ("function_call", "custom_tool_call")}
        named = {c.get("name"): cid for cid, c in calls.items()}
        if "ask_genie" not in named:
            patch = f"*** Begin Patch\n*** Add File: {self.patch_target}\n+CANARY\n*** End Patch"
            return [
                {"type": "custom_tool_call", "id": "ct1", "call_id": "call_patch", "name": "apply_patch", "input": patch},
                {"type": "function_call", "id": "fc1", "call_id": "call_ask", "namespace": "mcp__bottle",
                 "name": "ask_genie", "arguments": json.dumps({"workspace": "home", "question": "What was revenue?"})},
            ]
        if "request_user_input" not in named:
            q = {"questions": [{"header": "Pick", "id": "pick", "question": "Which?",
                                "options": [{"label": "a", "description": "a"}, {"label": "b", "description": "b"}]}]}
            return [{"type": "function_call", "id": "fc2", "call_id": "call_rui", "name": "request_user_input",
                     "arguments": json.dumps(q)}]
        out = outputs.get(named["ask_genie"], {}).get("output", "")
        m = re.search(r"callId[^a-z0-9]*([a-z0-9-]+)", json.dumps(out))
        final = {"answer": "done", "sources": [{"workspace": "home", "callId": m.group(1) if m else "none"}]}
        return [{"type": "message", "role": "assistant", "id": "m_final",
                 "content": [{"type": "output_text", "text": json.dumps(final)}]}]


def serve(model: Model):
    class H(http.server.BaseHTTPRequestHandler):
        def log_message(self, *a): pass

        def do_GET(self):
            model.hits.append(self.path.split("?")[0])
            if self.path.startswith("/v1/models"):
                b = model.models_json()
                self.send_response(200); self.send_header("content-type", "application/json")
                self.send_header("content-length", str(len(b))); self.end_headers(); self.wfile.write(b)
            else:
                self.send_response(404); self.end_headers()

        def do_POST(self):
            n = int(self.headers.get("content-length", 0))
            body = json.loads(self.rfile.read(n) or b"{}")
            model.hits.append(self.path.split("?")[0])
            model.requests.append(body)
            self.send_response(200); self.send_header("content-type", "text/event-stream"); self.end_headers()
            ev = lambda t, d: self.wfile.write(f"event: {t}\ndata: {json.dumps(d)}\n\n".encode())
            ev("response.created", {"type": "response.created", "response": {"id": "r"}})
            for it in model.script(body):
                ev("response.output_item.done", {"type": "response.output_item.done", "item": it})
            usage = {"input_tokens": 100, "input_tokens_details": {"cached_tokens": 0},
                     "output_tokens": 10, "output_tokens_details": {"reasoning_tokens": 0}, "total_tokens": 110}
            ev("response.completed", {"type": "response.completed", "response": {"id": "r", "usage": usage}})
            self.wfile.flush()

    srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), H)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def recording_proxy():
    """Any connection here is egress to a host other than the fake model."""
    seen: list[str] = []
    s = socket.socket(); s.bind(("127.0.0.1", 0)); s.listen(8)

    def loop():
        while True:
            try:
                c, _ = s.accept()
            except OSError:
                return
            try:
                first = c.recv(512).split(b"\r\n")[0].decode("latin1")
                seen.append(first.split(" ")[1] if " " in first else first)
                c.sendall(b"HTTP/1.1 502 Bad Gateway\r\ncontent-length: 0\r\n\r\n")
            finally:
                c.close()
    threading.Thread(target=loop, daemon=True).start()
    return s, seen


MCP = r'''
import sys, json, os, uuid
LOG = sys.argv[1]
ANS = open(sys.argv[2]).read()
for line in sys.stdin:
    m = json.loads(line)
    meth, mid = m.get("method"), m.get("id")
    if meth == "initialize":
        r = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}}, "serverInfo": {"name": "bottle", "version": "1"}}
    elif meth == "tools/list":
        r = {"tools": [{"name": "ask_genie", "description": "Ask one workspace.",
             "inputSchema": {"type": "object", "properties": {"workspace": {"type": "string"}, "question": {"type": "string"}},
                             "required": ["workspace", "question"]}}]}
    elif meth == "tools/call":
        cid = "c-" + uuid.uuid4().hex[:8]
        with open(LOG, "a") as f:
            f.write(json.dumps({"callId": cid, "secret_env_forwarded": bool(os.environ.get("BOTTLE_AGENT_SECRET")),
                                "socket_env_forwarded": bool(os.environ.get("BOTTLE_AGENT_SOCK"))}) + "\n")
        r = {"content": [{"type": "text", "text": json.dumps({"callId": cid, "workspace": "home", "answer": ANS})}]}
    else:
        r = None
    if mid is not None and r is not None:
        print(json.dumps({"jsonrpc": "2.0", "id": mid, "result": r}), flush=True)
'''


def scan(dirs: list[Path], needles: list[bytes]) -> list[str]:
    hits = []
    for d in dirs:
        for p in d.rglob("*") if d.exists() else []:
            if p.is_file():
                try:
                    data = p.read_bytes()
                except OSError:
                    continue
                if any(n in data for n in needles):
                    hits.append(str(p))
    return hits


class Client:
    def __init__(self, genie: Path, env: dict, cwd: str):
        self.p = subprocess.Popen([str(genie), "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                  stderr=subprocess.PIPE, env=env, text=True, cwd=cwd)
        self.msgs: list[dict] = []
        self.n = 0
        self.err = []
        threading.Thread(target=lambda: [self.err.append(len(l)) for l in self.p.stderr], daemon=True).start()

    def send(self, method, params=None, notify=False):
        m = {"jsonrpc": "2.0", "method": method}
        if params is not None: m["params"] = params
        if not notify:
            self.n += 1; m["id"] = self.n
        self.p.stdin.write(json.dumps(m) + "\n"); self.p.stdin.flush()
        return m.get("id")

    def read_until(self, pred, timeout=90):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            line = self.p.stdout.readline()
            if not line:
                return None
            m = json.loads(line)
            self.msgs.append(m)
            if "method" in m and "id" in m:  # a server request: decline, the check fails later
                self.p.stdin.write(json.dumps({"jsonrpc": "2.0", "id": m["id"], "error": {"code": -32601, "message": "refused"}}) + "\n")
                self.p.stdin.flush()
            if pred(m):
                return m
        return None


def main() -> int:
    genie = find_genie()
    ver = subprocess.run([str(genie), "--version"], capture_output=True, text=True).stdout.split()[-1]
    check(ver == PINNED, f"genie version is the pin ({PINNED})", ver)
    check(sha256(genie) == DIGEST, "genie SHA-256 is the pinned digest")
    base = Path(tempfile.mkdtemp(prefix="genie-lockdown-"))
    home, cwd, work = base / "genie-home", base / "cwd", base / "work"
    for d in (home, cwd, work): d.mkdir(mode=0o700)
    prompt_canary = "CANARY-PROMPT-" + secrets.token_hex(6)
    answer_canary = "CANARY-ANSWER-" + secrets.token_hex(6)
    patch_target = str(base / "PATCH_CANARY.txt")
    model = Model(patch_target)
    srv = serve(model)
    proxy_sock, egress = recording_proxy()
    (work / "mcp.py").write_text(MCP)
    (work / "empty.cfg").write_text("")
    (work / "answer.txt").write_text(answer_canary)
    cli = shutil.which("databricks")
    cfg = GOLDEN.read_text()
    cfg = re.sub(r'base_url = ".*"', f'base_url = "http://127.0.0.1:{srv.server_address[1]}/v1"', cfg)
    cfg = re.sub(r'(\[model_providers\.databricks\.auth\]\ncommand = )".*"\nargs = \[.*\]',
                 r'\1"/bin/echo"\nargs = ["fake-token"]', cfg)
    cfg = re.sub(r'(\[mcp_servers\.bottle\]\ncommand = )".*"\nargs = \[.*\]',
                 rf'\1"/usr/bin/python3"\nargs = ["{work / "mcp.py"}", "{work / "calls.jsonl"}", "{work / "answer.txt"}"]', cfg)
    (home / "config.toml").write_text(cfg)
    env = {"HOME": str(Path.home()), "GENIE_HOME": str(home), "PATH": "/usr/bin:/bin" + (":" + str(Path(cli).parent) if cli else ""),
           "DATABRICKS_CONFIG_FILE": str(work / "empty.cfg"),
           "BOTTLE_AGENT_SOCK": str(work / "s.sock"), "BOTTLE_AGENT_SECRET": "00" * 32, "BOTTLE_AGENT_SESSION": "s" * 32,
           "HTTP_PROXY": f"http://127.0.0.1:{proxy_sock.getsockname()[1]}", "HTTPS_PROXY": f"http://127.0.0.1:{proxy_sock.getsockname()[1]}",
           "ALL_PROXY": f"http://127.0.0.1:{proxy_sock.getsockname()[1]}", "NO_PROXY": "127.0.0.1,localhost"}
    c = Client(genie, env, str(cwd))
    c.send("initialize", {"clientInfo": {"name": "bottle", "version": "lockdown"}})
    c.read_until(lambda m: m.get("id") == 1)
    c.send("initialized", notify=True)
    tid = c.send("thread/start", {"cwd": str(cwd), "ephemeral": True, "approvalPolicy": "never"})
    tm = c.read_until(lambda m: m.get("id") == tid)
    check(tm is not None and "result" in tm, "thread/start answered", json.dumps(tm)[:300] if tm else "no reply")
    if not tm or "result" not in tm:
        print("stderr bytes:", sum(c.err)); return 1
    r = tm["result"]
    th = r["thread"]
    check(th.get("ephemeral") is True, "thread.ephemeral is true")
    check(th.get("cliVersion") == PINNED, "thread.cliVersion is the pin", str(th.get("cliVersion")))
    check(r.get("modelProvider") == "databricks", "model provider is the configured one")
    check(r.get("approvalPolicy") == "never", "approvalPolicy is never", str(r.get("approvalPolicy")))
    check(r.get("sandbox", {}).get("type") == "readOnly", "sandbox is readOnly")
    turn = c.send("turn/start", {"threadId": th["id"], "input": [{"type": "text", "text": f"Compare revenue. {prompt_canary}"}]})
    done = c.read_until(lambda m: m.get("method") == "turn/completed")
    check(done is not None and done["params"]["turn"]["status"] == "completed", "the turn completed",
          json.dumps(done)[:200] if done else "no turn/completed")
    egress_session = list(egress)
    # 3, 4, 5
    check(not Path(patch_target).exists(), "the apply_patch canary created no file")
    srv_reqs = [m for m in c.msgs if "method" in m and "id" in m]
    check(not srv_reqs, "no server-to-client request (no approval, no elicitation)", str([m["method"] for m in srv_reqs]))
    rc = [m for m in c.msgs if m.get("method") == "remoteControl/status/changed"]
    check(all(m["params"].get("status") == "disabled" for m in rc), "remote control is disabled", str(len(rc)))
    seen_items = sorted({m["params"]["item"]["type"] for m in c.msgs
                         if m.get("method", "").startswith("item/") and "type" in m.get("params", {}).get("item", {})})
    print("info: item types seen:", seen_items)
    bad_items = [m["params"]["item"]["type"] for m in c.msgs
                 if m.get("method", "").startswith("item/") and m["params"].get("item", {}).get("type") in
                 ("commandExecution", "webSearch", "fileChange")]
    check(not bad_items, "no command, file change or web search item", str(bad_items))
    for it in (model.requests[1].get("input", []) if len(model.requests) > 1 else []):
        if it.get("call_id") == "call_patch" and it.get("type", "").endswith("_output"):
            print("info: apply_patch result seen by the model:", json.dumps(it.get("output"))[:160])
    print("info: model requests:", len(model.requests))
    # 6
    calls = [json.loads(l) for l in (work / "calls.jsonl").read_text().splitlines()] if (work / "calls.jsonl").exists() else []
    check(len(calls) == 1, "the bottle MCP server got one ask_genie call", str(len(calls)))
    check(bool(calls) and calls[0]["secret_env_forwarded"] and calls[0]["socket_env_forwarded"],
          "env_vars forwarding reaches the shim (V2)")
    # 2
    first = model.requests[0] if model.requests else {}
    tools = []
    for t in first.get("tools", []):
        tools.append([t.get("type"), t.get("name")])
        for inner in t.get("tools", []):
            tools.append([f"{t.get('name')}", inner.get("name")])
    names = {n for _, n in tools}
    check(not (names & FORBIDDEN_TOOLS), "no shell, web search or image tool is offered", str(sorted(names & FORBIDDEN_TOOLS)))
    snap = HERE / f"expected-tools-{PINNED}.json"
    if RECORD:
        snap.write_text(json.dumps(sorted(tools), indent=1) + "\n")
    check(snap.exists() and sorted(tools) == json.loads(snap.read_text()), "the offered tool list equals the pinned snapshot", str(sorted(tools)))
    # 7: disk canary, live
    needles = [prompt_canary.encode(), answer_canary.encode()]
    live = scan([home, cwd], needles)
    check(not live, "disk canary: no file under GENIE_HOME or cwd holds a canary (live)", str(live))
    # 9: doctor flags
    dj = subprocess.run([str(genie), "doctor", "--json"], capture_output=True, text=True, env=env, cwd=str(cwd))
    try:
        flags = json.loads(dj.stdout)["checks"]["config.load"]["details"]["enabled feature flags"]
    except Exception as e:
        flags = f"unreadable: {e}"
    fsnap = HERE / f"expected-doctor-{PINNED}.json"
    if RECORD:
        fsnap.write_text(json.dumps({"enabledFeatureFlags": flags}, indent=1) + "\n")
    check(fsnap.exists() and json.loads(fsnap.read_text())["enabledFeatureFlags"] == flags,
          "doctor enabled-flag snapshot equals the pinned snapshot", str(flags))
    # end of session: kill, delete GENIE_HOME (what the Rust host does), scan again
    c.p.stdin.close()
    try: c.p.wait(timeout=5)
    except subprocess.TimeoutExpired: c.p.kill()
    shutil.rmtree(home); shutil.rmtree(cwd)
    # The MCP stub keeps its own answer file in `work`; that is the test's, not genie's.
    after = [h for h in scan([base], needles) if "/work/" not in h]
    check(not after, "disk canary: no file holds a canary after the session", str(after))
    # 8: egress. `genie doctor` is a separate command that Bottle never runs; its contacts are
    # printed for information only.
    check(not egress_session, "egress canary: during the session only the fake model host was contacted",
          str(egress_session))
    print("info: contacts made by `genie doctor` only:", sorted(set(egress) - set(egress_session)))
    hosts = sorted(set(model.hits))
    print("model host paths seen:", hosts)
    srv.shutdown(); proxy_sock.close(); shutil.rmtree(base, ignore_errors=True)
    print("RESULT:", "FAIL " + str(failures) if failures else "PASS (all lockdown canaries)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
