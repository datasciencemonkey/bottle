"""[SP-16] Offline tests for the SP-16 probe app and spikes/sp-16/run.py."""
import importlib.util
import json
import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import FakeTransport, fake_token, load_run, resp  # noqa: E402

import spikelib as sl  # noqa: E402

r16 = load_run("sp-16", "sp16_run")
_spec = importlib.util.spec_from_file_location(
    "sp16_app", os.path.join(os.path.dirname(__file__), "..", "app", "app.py"))
app = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(app)

APP_URL = "https://bottle-sp16-1234.aws.databricksapps.com"


def headers_with(tok):
    return [("Host", "x"), ("X-Forwarded-Email", "someone@example.com"), ("X-Forwarded-User", "u-1"),
            ("X-Forwarded-Preferred-Username", "someone"), ("X-Real-Ip", "10.0.0.1"),
            ("X-Request-Id", "r-1"), ("X-Forwarded-Access-Token", tok), ("Accept", "*/*")]


def test_sp_16_app_logs_header_names_not_values(capsys):
    """[SP-16] summarize() returns header names only; emails, IPs and the token value never appear."""
    tok = fake_token("sql iam.current-user:read")
    info = app.summarize(headers_with(tok))
    text = json.dumps(info)
    assert info["identity_header_names"] == ["x-forwarded-access-token", "x-forwarded-email",
                                             "x-forwarded-preferred-username", "x-forwarded-user",
                                             "x-real-ip", "x-request-id"]
    for secret in (tok, "someone@example.com", "10.0.0.1", "u-1", "eyJ"):
        assert secret not in text
    t = info["tokens"]["x-forwarded-access-token"]
    assert t["format"] == "jwt" and t["scope"] == "sql iam.current-user:read" and t["header_typ"] == "at+jwt"
    assert "sub" not in t and "email" not in t


def test_sp_16_app_token_summary_shapes():
    """[SP-16] Bearer prefix is stripped; opaque and broken tokens give no value."""
    assert app.token_summary("Bearer " + fake_token())["format"] == "jwt"
    assert app.token_summary("dapiabc") == {"present": True, "format": "opaque"}
    assert app.token_summary("eyJ.x.y") == {"present": True, "format": "jwt-undecodable"}
    assert app.token_summary("") == {"present": False}


def test_sp_16_app_handler_never_prints_token(capsys):
    """[SP-16] The request handler prints one line with names only."""
    tok = fake_token()
    info = app.summarize(headers_with(tok))
    print("sp16 " + json.dumps(info))
    out = capsys.readouterr().out
    assert tok not in out and "eyJ" not in out


def test_sp_16_app_url_must_be_databricksapps():
    """[SP-16] The client refuses to send the token to a URL that is not *.databricksapps.com."""
    with pytest.raises(sl.SpikeError):
        r16.AppApi("https://evil.example.com", sl.Token(fake_token()), FakeTransport([]))
    t = FakeTransport([resp(200, {})])
    r16.AppApi(APP_URL, sl.Token(fake_token()), t).invoke()
    assert t.requests[0]["url"] == APP_URL + "/api/invocations"


def _row(mode, tokens):
    body = {"identity_header_names": ["x-forwarded-email"], "tokens": tokens}
    return r16.mode_result(mode, resp(200, body), {"format": "jwt"})


def test_sp_16_pass_when_both_modes_known():
    """[SP-16] Pass: on = JWT with scope; off = result recorded (no user token)."""
    on = _row("on", {"x-forwarded-access-token": app.token_summary(fake_token("sql"))})
    off = _row("off", {"authorization": {"present": False}})
    ev = r16.evaluate([on, off])
    assert ev["passed"] and "scope=['sql']" in ev["modes"]["on"]["detail"]
    assert ev["modes"]["off"]["detail"] == "no user token forwarded"


def test_sp_16_fail_cases():
    """[SP-16] Opaque token, missing user token, an HTTP error or a missing mode all fail."""
    opaque = _row("on", {"x-forwarded-access-token": {"present": True, "format": "opaque"}})
    missing = _row("on", {})
    err = r16.mode_result("on", resp(401, {"error_code": "UNAUTHENTICATED", "message": "x"}), {})
    off = _row("off", {})
    for on in (opaque, missing, err):
        assert not r16.evaluate([on, off])["passed"]
    assert not r16.evaluate([off])["passed"]
    assert "lease" in r16.evaluate([off])["fallback"]
