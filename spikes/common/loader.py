"""Test helper: import a spike's run.py under a unique module name (the dirs share the name run.py)."""
import importlib.util
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)


class RealCliBlocked(RuntimeError):
    pass


def _is_databricks(argv):
    first = argv[0] if isinstance(argv, (list, tuple)) and argv else argv
    return isinstance(first, str) and os.path.basename(first.split()[0] if first else "") == "databricks"


def _guard(real):
    def wrapped(argv, *a, **kw):
        if _is_databricks(argv):
            raise RealCliBlocked("a test tried to run the real databricks CLI; inject a fake runner")
        return real(argv, *a, **kw)
    wrapped.sp_guard = True
    return wrapped


# Every spike test imports this module, so no test can start the real CLI (review of PR #321).
if not getattr(subprocess.run, "sp_guard", False):
    subprocess.run = _guard(subprocess.run)
    _RealPopen = subprocess.Popen

    class GuardedPopen(_RealPopen):
        def __init__(self, argv, *a, **kw):
            if _is_databricks(argv):
                raise RealCliBlocked("a test tried to run the real databricks CLI; inject a fake runner")
            super().__init__(argv, *a, **kw)

    subprocess.Popen = GuardedPopen


def load_run(spike_dir_name, module_name):
    path = os.path.join(HERE, "..", spike_dir_name, "run.py")
    spec = importlib.util.spec_from_file_location(module_name, path)
    mod = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = mod
    spec.loader.exec_module(mod)
    return mod


class FakeTransport:
    """Records requests; answers from a list of Resp objects or a callable(method, url, headers, body)."""

    def __init__(self, answers):
        self.answers = answers
        self.requests = []

    def request(self, method, url, headers=None, body=None):
        self.requests.append({"method": method, "url": url, "headers": dict(headers or {}), "body": body})
        if callable(self.answers):
            return self.answers(method, url, headers or {}, body)
        return self.answers.pop(0)


def resp(status=200, body=None, headers=None, raw=None, elapsed_ms=10.0, http_version="HTTP/2"):
    import json

    import spikelib as sl

    data = raw if raw is not None else (json.dumps(body).encode() if body is not None else b"")
    h = {"content-type": "application/json"}
    h.update({k.lower(): v for k, v in (headers or {}).items()})
    return sl.Resp(status, h, data, elapsed_ms, http_version)


FAKE_HOST = "https://example-ws.cloud.databricks.com"


def fake_token(scope="all-apis offline_access"):
    """A syntactically valid unsigned JWT for tests. Not a real credential."""
    import base64
    import json

    def enc(o):
        return base64.urlsafe_b64encode(json.dumps(o).encode()).decode().rstrip("=")

    return enc({"alg": "RS256", "typ": "at+jwt"}) + "." + enc(
        {"scope": scope, "sub": "user-123", "email": "someone@example.com"}) + ".c2lnbmF0dXJl"
