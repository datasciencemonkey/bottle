"""SP-16 probe app for Databricks Apps (stdlib only).

It logs and returns only:
- the NAMES of the incoming identity headers (never their values), and
- for each token header, the token format and, for a JWT, only the header `typ`/`alg` and the
  claims `scope`, `scp`, `token_type`, `typ`, `grant_type`. The JWT is decoded in memory, its
  signature is not checked, and the value is never logged, returned or stored.

Routes: POST|GET /api/invocations, GET /api/health.
"""
import base64
import json
import os
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

IDENTITY_PREFIXES = ("x-forwarded-", "x-real-ip", "x-request-id", "x-databricks-", "authorization")
TOKEN_HEADERS = ("x-forwarded-access-token", "authorization")
SAFE_CLAIMS = ("scope", "scp", "token_type", "typ", "grant_type")


def _b64(part):
    return base64.urlsafe_b64decode(part + "=" * (-len(part) % 4))


def token_summary(value):
    if not value:
        return {"present": False}
    if value.lower().startswith("bearer "):
        value = value[7:].strip()
    parts = value.split(".")
    if len(parts) != 3:
        return {"present": True, "format": "opaque"}
    try:
        header = json.loads(_b64(parts[0]))
        claims = json.loads(_b64(parts[1]))
    except (ValueError, UnicodeDecodeError):
        return {"present": True, "format": "jwt-undecodable"}
    out = {"present": True, "format": "jwt", "header_typ": header.get("typ"), "header_alg": header.get("alg")}
    for k in SAFE_CLAIMS:
        if k in claims:
            out[k] = claims[k]
    return out


def summarize(headers):
    """headers: iterable of (name, value). Returns a dict with names and token summaries only."""
    names, tokens = [], {}
    for name, value in headers:
        low = name.lower()
        if low.startswith(IDENTITY_PREFIXES):
            names.append(low)
        if low in TOKEN_HEADERS:
            tokens[low] = token_summary(value)
    return {"identity_header_names": sorted(set(names)), "tokens": tokens,
            "user_authorization_token_present": "x-forwarded-access-token" in tokens}


class Handler(BaseHTTPRequestHandler):
    server_version = "bottle-sp16"

    def log_message(self, fmt, *args):  # default access log prints the path only; keep it quiet
        return

    def _reply(self, code, obj):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _invocations(self):
        n = int(self.headers.get("content-length") or 0)
        if n:
            self.rfile.read(min(n, 65536))  # drained, never logged
        info = summarize(self.headers.items())
        print("sp16 " + json.dumps(info), flush=True)
        self._reply(200, info)

    def do_GET(self):
        if self.path.startswith("/api/health"):
            return self._reply(200, {"ok": True})
        if self.path.startswith("/api/invocations"):
            return self._invocations()
        return self._reply(404, {"error": "not found"})

    def do_POST(self):
        if self.path.startswith("/api/invocations"):
            return self._invocations()
        return self._reply(404, {"error": "not found"})


def main():
    port = int(os.environ.get("DATABRICKS_APP_PORT", "8000"))
    ThreadingHTTPServer(("0.0.0.0", port), Handler).serve_forever()


if __name__ == "__main__":
    main()
