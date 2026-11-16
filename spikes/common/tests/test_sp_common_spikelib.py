"""Offline tests for spikes/common/spikelib.py. Each test names the spike that relies on it."""
import json
import os
import sys
from types import SimpleNamespace

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from loader import FAKE_HOST, FakeTransport, fake_token, resp  # noqa: E402

import spikelib as sl  # noqa: E402


def fake_runner(outputs):
    calls = []

    def run(argv, **kw):
        calls.append(argv)
        key = " ".join(argv[1:3])
        rc, out, err = outputs[key]
        return SimpleNamespace(returncode=rc, stdout=out, stderr=err)

    run.calls = calls
    return run


def test_sp_16_redact_removes_token_shapes():
    """[SP-16] redact() removes JWTs, PATs, Bearer values, passwords and exact secrets (4.6 rule 7)."""
    tok = fake_token()
    pat_shape = "da" + "pi" + "0123456789abcdef" * 2  # built at runtime: a fake PAT shape, not a credential
    text = (f"Authorization: Bearer {tok} {pat_shape} password=hunter2 "
            f'{{"access_token": "abc"}} plain-secret-xyz')
    out = sl.redact(text, secrets=["plain-secret-xyz"])
    for bad in (tok, pat_shape, "hunter2", '"abc"', "plain-secret-xyz", "eyJ"):
        assert bad not in out


def test_sp_16_token_repr_is_redacted():
    """[SP-16] A Token never shows its value in str, repr or an f-string."""
    t = sl.Token(fake_token())
    assert "eyJ" not in repr(t) and "eyJ" not in str(t) and "eyJ" not in f"{t}"
    t.wipe()
    assert set(t.reveal()) == {"\0"}


def test_sp_14_token_kind_keeps_only_safe_claims():
    """[SP-14] token_kind() returns typ and scope, never sub or email or the value."""
    kind = sl.token_kind(fake_token("ai-gateway genie offline_access"))
    assert kind["format"] == "jwt" and kind["header_typ"] == "at+jwt"
    assert sl.scopes_of(kind) == ["ai-gateway", "genie", "offline_access"]
    assert "sub" not in kind and "email" not in kind
    assert sl.token_kind("dose0123abcd") == {"present": True, "format": "opaque"}
    assert sl.token_kind(None) == {"present": False}


def test_sp_14_get_token_and_host_use_the_cli_only():
    """[SP-14] The token comes from `databricks auth token`; the host from `auth profiles --skip-validate`."""
    tok = fake_token()
    run = fake_runner({
        "auth token": (0, json.dumps({"access_token": tok, "token_type": "Bearer"}), ""),
        "auth profiles": (0, json.dumps({"profiles": [{"name": "p1", "host": "https://example-ws.cloud.databricks.com/?o=1"}]}), ""),
    })
    t = sl.get_token("p1", runner=run)
    assert t.reveal() == tok
    assert sl.get_host("p1", runner=run) == FAKE_HOST
    assert ["databricks", "auth", "profiles", "--skip-validate", "-o", "json"] in run.calls
    assert ["databricks", "auth", "token", "--profile", "p1", "-o", "json"] in run.calls
    with pytest.raises(sl.SpikeError):
        sl.get_host("other", runner=run)


def test_sp_14_get_token_error_is_redacted():
    """[SP-14] A CLI failure message is redacted before it is raised."""
    run = fake_runner({"auth token": (1, "", "Error: bad token " + fake_token())})
    with pytest.raises(sl.SpikeError) as e:
        sl.get_token("p1", runner=run)
    assert "eyJ" not in str(e.value)


def test_sp_14_check_token_url_rules():
    """[SP-14] A token goes only to the exact https profile host in the Databricks domain list."""
    sl.check_token_url(FAKE_HOST + "/api/2.0/x", FAKE_HOST)
    for bad in ("http://example-ws.cloud.databricks.com/x", "https://evil.example.com/x",
                "https://example-ws.cloud.databricks.com:8443/x"):
        with pytest.raises(sl.SpikeError):
            sl.check_token_url(bad, FAKE_HOST)
    with pytest.raises(sl.SpikeError):
        sl.check_token_url("https://accounts.cloud.databricks.com/x", "https://accounts.cloud.databricks.com")
    with pytest.raises(sl.SpikeError):
        sl.check_token_url("https://my.example.com/x", "https://my.example.com")


def test_sp_16_check_app_url():
    """[SP-16] The app URL must be https://*.databricksapps.com (4.6 rule 1)."""
    sl.check_app_url("https://bottle-sp16-123.aws.databricksapps.com")
    for bad in ("http://x.databricksapps.com", "https://x.databricksapps.com.evil.io", "https://x.example.com"):
        with pytest.raises(sl.SpikeError):
            sl.check_app_url(bad)


def test_sp_07_percentile_linear_interpolation():
    """[SP-07] percentile() matches numpy linear interpolation."""
    xs = list(range(1, 101))
    assert sl.percentile(xs, 50) == pytest.approx(50.5)
    assert sl.percentile(xs, 95) == pytest.approx(95.05)
    assert sl.percentile([7], 95) == 7
    assert sl.percentile([], 95) is None
    assert sl.summary([1, 2, 3])["p50_ms"] == 2


def test_sp_07_message_text_handles_list_of_blocks():
    """[SP-07] [NFR-22b] gpt-oss list-of-blocks content gives only the text blocks."""
    msg = {"content": [{"type": "reasoning", "summary": [{"text": "think"}]}, {"type": "text", "text": "Hi"}]}
    assert sl.message_text(msg) == "Hi"
    assert sl.message_text({"content": "Hello"}) == "Hello"
    assert sl.message_text(None) == ""


def test_sp_12_json_shape_drops_values():
    """[SP-12] json_shape() keeps keys and types and drops values."""
    shape = sl.json_shape({"name": "projects/x", "status": {"cu": 0.5, "hosts": [{"host": "h"}]}, "tags": []})
    assert shape == {"name": "str", "status": {"cu": "float", "hosts": [{"host": "str"}]}, "tags": []}


def test_sp_11_parse_sse_and_rpc_message():
    """[SP-11] The MCP client reads JSON-RPC from an SSE body and from a JSON body."""
    sse = 'event: message\ndata: {"jsonrpc":"2.0","id":3,"result":{"ok":true}}\n\n'
    r = resp(200, raw=sse.encode(), headers={"content-type": "text/event-stream"})
    assert sl.rpc_message(r, 3) == {"jsonrpc": "2.0", "id": 3, "result": {"ok": True}}
    assert sl.rpc_message(resp(200, {"jsonrpc": "2.0", "id": 1, "result": {}}), 1)["result"] == {}
    assert sl.rpc_message(r, 9) is None


def test_sp_11_mcp_initialize_and_session_header():
    """[SP-11] initialize stores the session ID and sends it, with the protocol version, on later calls."""
    answers = [
        resp(200, {"jsonrpc": "2.0", "id": 1, "result": {"protocolVersion": "2025-06-18",
                                                          "serverInfo": {"name": "genie"}}},
             headers={"mcp-session-id": "s-1"}),
        resp(202, raw=b""),
        resp(200, {"jsonrpc": "2.0", "id": 2, "result": {"tools": [{"name": "genie_ask"}, {"name": "genie_poll_response"}]}}),
    ]
    t = FakeTransport(answers)
    mcp = sl.McpClient(sl.Api(FAKE_HOST, sl.Token(fake_token()), t), "/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=1")
    init = mcp.initialize()
    assert init["ok"] and init["initialized_notification"]["ok"]
    tl = mcp.tools_list()
    assert tl["tools"] == ["genie_ask", "genie_poll_response"]
    last = t.requests[-1]["headers"]
    assert last["Mcp-Session-Id"] == "s-1" and last["MCP-Protocol-Version"] == "2025-06-18"
    assert "text/event-stream" in last["Accept"]


def test_sp_11_mcp_records_jsonrpc_error():
    """[SP-11] A JSON-RPC error is recorded with code and redacted message."""
    t = FakeTransport([resp(200, {"jsonrpc": "2.0", "id": 1, "error": {"code": -32001, "message": "busy"}})])
    mcp = sl.McpClient(sl.Api(FAKE_HOST, sl.Token(fake_token()), t), "/mcp")
    step = mcp.rpc("tools/call", {})
    assert not step["ok"] and step["jsonrpc_error"] == {"code": -32001, "message": "busy"}


def test_sp_11_find_key_in_nested_and_text():
    """[SP-11] find_key() finds conversation_id in structuredContent or in a JSON text block."""
    assert sl.find_key({"structuredContent": {"conversation_id": "c1"}}, "conversation_id") == "c1"
    res = {"content": [{"type": "text", "text": json.dumps({"conversation_id": "c2", "status": "IN_PROGRESS"})}]}
    assert sl.find_key(res, "conversation_id") == "c2"
    assert sl.find_key({"content": [{"type": "text", "text": 'x "conversation_id": "c3" y'}]}, "conversation_id") == "c3"
    assert sl.find_key({"a": 1}, "conversation_id") is None


def test_sp_20_error_shape_is_content_free():
    """[SP-20] error_shape() keeps status, keys, error_code and a redacted short message."""
    shape = sl.error_shape(resp(403, {"error_code": "PERMISSION_DENIED", "message": "need scope " + fake_token()}))
    assert shape["status"] == 403 and shape["error_code"] == "PERMISSION_DENIED"
    assert "eyJ" not in shape["message"]
    html = sl.error_shape(resp(502, raw=b"<html>x</html>", headers={"content-type": "text/html"}))
    assert html["json"] is False and html["body_bytes"] == 14


def test_sp_20_api_adds_bearer_only_for_profile_host():
    """[SP-20] Api.call() adds the Bearer header and refuses another host."""
    t = FakeTransport([resp(200, {})])
    tok = fake_token()
    api = sl.Api(FAKE_HOST, sl.Token(tok), t)
    api.call("GET", "/api/2.0/domains")
    assert t.requests[0]["headers"]["Authorization"] == "Bearer " + tok
    api.host = "https://evil.example.com"
    with pytest.raises(sl.SpikeError):
        api.call("GET", "/x")


def test_sp_14_write_report_scrubs_secrets(tmp_path):
    """[SP-14] write_report() removes the exact token even if it slipped into the report."""
    tok = fake_token()
    text = sl.write_report(str(tmp_path / "r.json"), {"oops": tok, "ok": 1}, secrets=[tok])
    assert tok not in text and tok not in (tmp_path / "r.json").read_text()
    assert sl.verdict("SP-14", True, "x").startswith("SP-14 PASS")


def test_sp_14_no_cli_helper_has_a_default_runner():
    """[SP-14] run_cli, cli_api, get_token, get_host and the SP-12 Cli need an explicit runner,
    so no test can start the real databricks CLI by default (review of PR #321)."""
    import inspect

    from loader import load_run

    for fn in (sl.run_cli, sl.cli_api, sl.get_token, sl.get_host):
        p = inspect.signature(fn).parameters["runner"]
        assert p.default is inspect.Parameter.empty, fn.__name__
    cli = load_run("sp-12", "sp12_guard").Cli
    assert inspect.signature(cli.__init__).parameters["runner"].default is inspect.Parameter.empty
    with pytest.raises(TypeError):
        sl.get_token("p1")  # noqa: the missing runner is the point


def test_sp_14_tests_cannot_run_the_real_cli():
    """[SP-14] Under the test loader, subprocess.run and Popen refuse the databricks binary."""
    import subprocess

    from loader import RealCliBlocked

    with pytest.raises(RealCliBlocked):
        subprocess.run(["databricks", "--version"], capture_output=True)
    with pytest.raises(RealCliBlocked):
        subprocess.run(["/opt/homebrew/bin/databricks", "auth", "token"], capture_output=True)
    with pytest.raises(RealCliBlocked):
        subprocess.Popen(["databricks", "auth", "token"])
    with pytest.raises(RealCliBlocked):
        sl.run_cli(["auth", "token", "--profile", "p"], runner=subprocess.run)
    assert subprocess.run(["true"]).returncode == 0  # other binaries still run
