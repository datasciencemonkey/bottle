"""Shared helpers for the Bottle live-workspace spike scripts (SP-06, 07, 11, 12, 13, 14, 16, 20).

Stdlib only, so the offline tests run with pytest alone. HTTP goes through an injectable
transport: `HttpxTransport` (lazy httpx import) for live runs, a fake in tests.

Binding rules that this module enforces:
- Tokens come only from `databricks auth token --profile <p>`. They stay in memory in a `Token`
  object whose repr and str are redacted. No function writes a token to disk or prints it.
- A token goes only to the exact profile host over https in the Databricks domain list
  (requirements 4.6 rules 1 and 2).
- Every report and every printed line passes through `redact()` (4.6 rule 7).
- The scripts never pick a CLI profile: `--profile` is a required argument.
"""
import base64
import json
import math
import re
import sys
import time
from dataclasses import dataclass, field
from urllib.parse import urlsplit

# ---------------------------------------------------------------- redaction (4.6 rule 7)

_REDACT = [
    re.compile(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]+"),
    re.compile(r"eyJ[\w-]+\.[\w-]*\.?[\w-]*"),
    re.compile(r"dapi[0-9a-f]{32}(-\d+)?"),
    re.compile(r"dose[0-9a-f]+"),
    re.compile(r"AKIA[0-9A-Z]{16}"),
    re.compile(r"gh[pousr]_[A-Za-z0-9]{20,}"),
    re.compile(r"-----BEGIN[^\n]*"),
    re.compile(r"(?i)password=\S+"),
    re.compile(r'(?i)"(access_token|token|password|refresh_token|client_secret)"\s*:\s*"[^"]*"'),
]


def redact(text, secrets=()):
    """Return text with token-shaped strings and the given exact secrets replaced."""
    if text is None:
        return None
    out = str(text)
    for s in secrets:
        if s:
            out = out.replace(str(s), "<redacted>")
    for pat in _REDACT:
        out = pat.sub("<redacted>", out)
    return out


class Token:
    """An access token held in memory only. str() and repr() never show the value."""

    __slots__ = ("_value",)

    def __init__(self, value):
        if not value or not isinstance(value, str):
            raise SpikeError("empty token from the CLI")
        self._value = value

    def reveal(self):
        """The raw value. Use it only to build an Authorization header or a libpq password."""
        return self._value

    def auth_header(self):
        return {"Authorization": f"Bearer {self._value}"}

    def __repr__(self):
        return "Token(<redacted>)"

    __str__ = __repr__

    def wipe(self):
        self._value = "\0" * len(self._value)


class SpikeError(Exception):
    pass


# ---------------------------------------------------------------- CLI


@dataclass
class CliResult:
    returncode: int
    stdout: str
    stderr: str

    @property
    def ok(self):
        return self.returncode == 0

    def json(self):
        return json.loads(self.stdout) if self.stdout.strip() else {}


def run_cli(args, runner, timeout=300):
    """Run `databricks <args>` with `runner` (subprocess.run in main(), a fake in tests).

    There is no default runner, so no test can start the real CLI by mistake.
    Returns a CliResult. Never echoes stdout."""
    proc = runner(["databricks", *args], capture_output=True, text=True, timeout=timeout)
    return CliResult(proc.returncode, proc.stdout or "", proc.stderr or "")


def cli_api(method, path, profile, runner, body=None):
    """`databricks api <method> <path> --profile <p> [--json body]`. Returns a step dict."""
    args = ["api", method, path, "--profile", profile, "-o", "json"]
    if body is not None:
        args += ["--json", json.dumps(body)]
    t0 = time.perf_counter()
    res = run_cli(args, runner=runner)
    step = {"method": method.upper(), "path": path, "ok": res.ok,
            "elapsed_ms": round((time.perf_counter() - t0) * 1000, 1)}
    if res.ok:
        try:
            step["json"] = res.json()
        except json.JSONDecodeError:
            step["json"] = None
            step["note"] = "stdout was not JSON"
    else:
        step["error"] = redact(res.stderr.strip())[:500]
    return step


def get_token(profile, runner):
    """Token from `databricks auth token --profile <p>`, kept in memory."""
    res = run_cli(["auth", "token", "--profile", profile, "-o", "json"], runner=runner, timeout=120)
    if not res.ok:
        raise SpikeError("databricks auth token failed: " + redact(res.stderr.strip())[:300])
    try:
        data = res.json()
    except json.JSONDecodeError:
        raise SpikeError("databricks auth token did not return JSON") from None
    return Token(data.get("access_token"))


def get_host(profile, runner):
    """Host of a profile from `databricks auth profiles --skip-validate` (no network call)."""
    res = run_cli(["auth", "profiles", "--skip-validate", "-o", "json"], runner=runner, timeout=60)
    if not res.ok:
        raise SpikeError("databricks auth profiles failed: " + redact(res.stderr.strip())[:300])
    for p in res.json().get("profiles", []):
        if p.get("name") == profile:
            host = p.get("host") or ""
            return normalize_host(host)
    raise SpikeError(f"profile {profile!r} not found in ~/.databrickscfg")


def normalize_host(host):
    host = host.strip()
    if not host.startswith("https://"):
        host = "https://" + host.removeprefix("http://")
    parts = urlsplit(host)
    return f"https://{parts.hostname}"


DATABRICKS_DOMAINS = ("cloud.databricks.com", "gcp.databricks.com", "azuredatabricks.net",
                      "cloud.databricks.us", "databricks.azure.us")


def check_token_url(url, host):
    """4.6 rule 2: send a token only to the exact profile host, https, port 443, Databricks domain."""
    u = urlsplit(url)
    h = urlsplit(host).hostname or ""
    if u.scheme != "https" or (u.port not in (None, 443)):
        raise SpikeError("refusing to send a token to a non-https or non-443 URL")
    if u.hostname != h:
        raise SpikeError("refusing to send a token to a host other than the profile host")
    if h.startswith("accounts."):
        raise SpikeError("refusing to send a token to an accounts host")
    if not any(h == d or h.endswith("." + d) for d in DATABRICKS_DOMAINS):
        raise SpikeError("profile host is not in the Databricks domain list")


def check_app_url(url):
    """4.6 rule 1: a custom app URL must be https://*.databricksapps.com."""
    u = urlsplit(url)
    if u.scheme != "https" or u.port not in (None, 443):
        raise SpikeError("app URL must be https on port 443")
    if not (u.hostname or "").endswith(".databricksapps.com"):
        raise SpikeError("app URL must match https://*.databricksapps.com")


# ---------------------------------------------------------------- HTTP transport


@dataclass
class Resp:
    status: int
    headers: dict
    body: bytes
    elapsed_ms: float
    http_version: str = ""

    def json(self):
        return json.loads(self.body.decode("utf-8")) if self.body else None

    @property
    def content_type(self):
        return (self.headers.get("content-type") or "").lower()


class HttpxTransport:
    """Thin httpx wrapper. One warm client; http2 on request (SP-07)."""

    def __init__(self, http2=False, timeout=60.0):
        import httpx  # lazy: the tests do not need httpx

        self._httpx = httpx
        self.client = httpx.Client(http2=http2, timeout=timeout, follow_redirects=False)

    def request(self, method, url, headers=None, body=None):
        t0 = time.perf_counter()
        try:
            r = self.client.request(method, url, headers=headers or {}, content=body)
            data = r.read()
        except self._httpx.HTTPError as e:
            # Never include the request (it holds the header) in the error text.
            return Resp(0, {}, json.dumps({"transport_error": type(e).__name__}).encode(),
                        round((time.perf_counter() - t0) * 1000, 1))
        elapsed = round((time.perf_counter() - t0) * 1000, 1)
        return Resp(r.status_code, {k.lower(): v for k, v in r.headers.items()}, data, elapsed,
                    r.http_version)

    def close(self):
        self.client.close()


class Api:
    """Direct HTTPS calls to the profile host with the token in a header only."""

    def __init__(self, host, token, transport):
        self.host = host
        self.token = token
        self.t = transport

    def call(self, method, path, body=None, headers=None, raw=None):
        url = self.host + path
        check_token_url(url, self.host)
        h = {"Content-Type": "application/json", "User-Agent": "bottle-spike/0.1"}
        h.update(headers or {})
        h.update(self.token.auth_header())
        data = raw if raw is not None else (json.dumps(body).encode() if body is not None else None)
        return self.t.request(method, url, headers=h, body=data)


def error_shape(resp):
    """A content-free description of an error response: status, JSON keys, error_code, short message."""
    shape = {"status": resp.status, "content_type": resp.content_type.split(";")[0]}
    try:
        j = resp.json()
    except (ValueError, UnicodeDecodeError):
        j = None
    if isinstance(j, dict):
        shape["json_keys"] = sorted(j.keys())
        err = j.get("error") if isinstance(j.get("error"), dict) else j
        for k in ("error_code", "code", "type", "transport_error"):
            if isinstance(err, dict) and k in err:
                shape[k] = err[k]
        msg = (err.get("message") if isinstance(err, dict) else None) or j.get("message")
        if msg:
            shape["message"] = redact(str(msg))[:300]
    else:
        shape["json"] = False
        shape["body_bytes"] = len(resp.body)
    return shape


# ---------------------------------------------------------------- JWT claims (in memory)


def b64url_decode(part):
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


SAFE_CLAIMS = ("scope", "scp", "token_type", "typ", "grant_type")


def token_kind(value):
    """Classify a token without exposing it. JWT: return only header typ/alg and the safe claims."""
    if not value:
        return {"present": False}
    parts = value.split(".")
    if len(parts) == 3 and value.startswith("eyJ"):
        try:
            header = json.loads(b64url_decode(parts[0]))
            claims = json.loads(b64url_decode(parts[1]))
        except (ValueError, UnicodeDecodeError):
            return {"present": True, "format": "jwt-undecodable"}
        out = {"present": True, "format": "jwt", "header_typ": header.get("typ"),
               "header_alg": header.get("alg")}
        for k in SAFE_CLAIMS:
            if k in claims:
                out[k] = claims[k]
        return out
    return {"present": True, "format": "opaque"}


def scopes_of(kind):
    s = kind.get("scope", kind.get("scp"))
    if isinstance(s, str):
        return sorted(s.split())
    if isinstance(s, list):
        return sorted(str(x) for x in s)
    return []


# ---------------------------------------------------------------- stats


def percentile(values, p):
    """Linear-interpolation percentile (same as numpy's default)."""
    xs = sorted(values)
    if not xs:
        return None
    k = (len(xs) - 1) * p / 100.0
    lo, hi = math.floor(k), math.ceil(k)
    if lo == hi:
        return float(xs[int(k)])
    return xs[lo] + (xs[hi] - xs[lo]) * (k - lo)


def summary(values):
    if not values:
        return {"n": 0, "p50_ms": None, "p95_ms": None, "min_ms": None, "max_ms": None}
    return {"n": len(values), "p50_ms": round(percentile(values, 50), 1),
            "p95_ms": round(percentile(values, 95), 1), "min_ms": round(min(values), 1),
            "max_ms": round(max(values), 1)}


def json_shape(obj):
    """Replace every leaf value with its type name. Keeps keys, drops content."""
    if isinstance(obj, dict):
        return {k: json_shape(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [json_shape(obj[0])] if obj else []
    return type(obj).__name__


def message_text(choice_message):
    """Chat completion content as text. Handles a string and a list of blocks (gpt-oss, NFR-22b)."""
    c = (choice_message or {}).get("content")
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "".join(b.get("text", "") for b in c if isinstance(b, dict) and b.get("type") == "text")
    return ""


# ---------------------------------------------------------------- MCP (JSON-RPC over HTTP)


def parse_sse(text):
    """Return the JSON objects of the `data:` fields of a text/event-stream body."""
    out, buf = [], []
    for line in text.splitlines() + [""]:
        if line.startswith("data:"):
            buf.append(line[5:].lstrip())
        elif line == "" and buf:
            try:
                out.append(json.loads("\n".join(buf)))
            except json.JSONDecodeError:
                pass
            buf = []
    return out


def rpc_message(resp, want_id):
    """The JSON-RPC message with id want_id from a JSON or SSE response."""
    if "text/event-stream" in resp.content_type:
        msgs = parse_sse(resp.body.decode("utf-8", "replace"))
    else:
        j = resp.json() if resp.body else None
        msgs = j if isinstance(j, list) else ([j] if j else [])
    for m in msgs:
        if isinstance(m, dict) and m.get("id") == want_id:
            return m
    return None


def mcp_url(host, workspace_id):
    return f"{host}/ai-gateway/mcp-services/system.ai.genie_one_mcp?o={workspace_id}"


class McpClient:
    PROTOCOL = "2025-06-18"  # CHECK: the server may answer with another version; we record it.

    def __init__(self, api, path):
        self.api = api
        self.path = path  # path + query, for example /ai-gateway/mcp-services/...?o=123
        self.session_id = None
        self.protocol = None
        self._id = 0

    def _headers(self):
        h = {"Accept": "application/json, text/event-stream"}
        if self.session_id:
            h["Mcp-Session-Id"] = self.session_id
        if self.protocol:
            h["MCP-Protocol-Version"] = self.protocol
        return h

    def rpc(self, method, params=None):
        self._id += 1
        rid = self._id
        msg = {"jsonrpc": "2.0", "id": rid, "method": method, "params": params or {}}
        resp = self.api.call("POST", self.path, body=msg, headers=self._headers())
        if resp.headers.get("mcp-session-id"):
            self.session_id = resp.headers["mcp-session-id"]
        step = {"method": method, "status": resp.status, "elapsed_ms": resp.elapsed_ms}
        m = rpc_message(resp, rid) if 200 <= resp.status < 300 else None
        if m is None:
            step["ok"] = False
            step["error"] = error_shape(resp)
        elif "error" in m:
            step["ok"] = False
            step["jsonrpc_error"] = {"code": m["error"].get("code"),
                                     "message": redact(str(m["error"].get("message", "")))[:300]}
        else:
            step["ok"] = True
            step["result"] = m.get("result")
        return step

    def notify(self, method):
        msg = {"jsonrpc": "2.0", "method": method}
        resp = self.api.call("POST", self.path, body=msg, headers=self._headers())
        return {"method": method, "status": resp.status, "ok": resp.status in (200, 202, 204)}

    def initialize(self):
        step = self.rpc("initialize", {"protocolVersion": self.PROTOCOL, "capabilities": {},
                                       "clientInfo": {"name": "bottle-spike", "version": "0.1"}})
        if step["ok"]:
            self.protocol = (step["result"] or {}).get("protocolVersion") or self.PROTOCOL
            step["server_protocol"] = self.protocol
            step["server_info"] = (step["result"] or {}).get("serverInfo")
            step.pop("result", None)
            step["initialized_notification"] = self.notify("notifications/initialized")
        return step

    def tools_list(self):
        step = self.rpc("tools/list")
        if step["ok"]:
            step["tools"] = sorted(t.get("name") for t in (step.pop("result") or {}).get("tools", []))
        return step

    def call_tool(self, name, arguments):
        return self.rpc("tools/call", {"name": name, "arguments": arguments})


def find_key(obj, key):
    if isinstance(obj, dict):
        if key in obj and isinstance(obj[key], (str, int)):
            return str(obj[key])
        for v in obj.values():
            found = find_key(v, key)
            if found:
                return found
    elif isinstance(obj, list):
        for v in obj:
            found = find_key(v, key)
            if found:
                return found
    elif isinstance(obj, str):
        try:
            return find_key(json.loads(obj), key)
        except (json.JSONDecodeError, TypeError):
            m = re.search(r'"%s"\s*:\s*"([^"]+)"' % re.escape(key), obj)
            return m.group(1) if m else None
    return None


def workspace_id_from_headers(resp):
    """Workspace ID from the `x-databricks-org-id` response header (CHECK)."""
    return resp.headers.get("x-databricks-org-id")


def resolve_workspace_id(api, given=None):
    if given:
        return str(given)
    resp = api.call("GET", "/api/2.0/preview/scim/v2/Me")
    wid = workspace_id_from_headers(resp)
    if not wid:
        raise SpikeError("could not read the workspace ID from the response headers; pass --workspace-id")
    return wid


# ---------------------------------------------------------------- report


def write_report(path, report, secrets=()):
    text = json.dumps(report, indent=2, sort_keys=True, default=str)
    text = redact(text, secrets)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text + "\n")
    return text


def verdict(spike, passed, detail, word=None):
    """One result line. word overrides PASS/FAIL, for example "NOT PROVEN" or "NOT RUN"."""
    word = word or ("PASS" if passed else "FAIL")
    return redact(f"{spike} {word}: {detail}")


def confirm(message, yes=False, ask=input):
    """Print a warning; continue only on --yes or a typed "yes"."""
    print(redact(f"WARNING: {message}"), flush=True)
    if yes:
        print("  --yes given: continuing.", flush=True)
        return True
    try:
        answer = ask("  Type yes to continue: ")
    except EOFError:
        answer = ""
    if answer.strip().lower() != "yes":
        raise SpikeError("not confirmed; stopped before the step")
    return True


def say(msg, secrets=()):
    print(redact(msg, secrets), flush=True)


@dataclass
class Timer:
    t0: float = field(default_factory=time.perf_counter)

    def ms(self):
        return round((time.perf_counter() - self.t0) * 1000, 1)


def main_guard(fn):
    """Run fn(); print only a redacted error line on failure."""
    try:
        return fn()
    except SpikeError as e:
        print(redact(f"ERROR: {e}"), file=sys.stderr)
        return 2
    except Exception as e:  # noqa: BLE001 - print a redacted line, never a raw traceback
        print(redact(f"ERROR: {type(e).__name__}: {e}"), file=sys.stderr)
        return 3


def load_common():
    """No-op marker so run.py files can import spikelib after a sys.path insert."""
    return True
