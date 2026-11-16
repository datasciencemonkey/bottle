"""[SP-14] Offline tests for spikes/sp-14/run.py (fake workspace, no network)."""
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import FAKE_HOST, FakeTransport, fake_token, load_run, resp  # noqa: E402

import spikelib as sl  # noqa: E402

r14 = load_run("sp-14", "sp14_run")


def workspace(scopes, pages_status=200):
    """A fake workspace that answers 403 when the token lacks the scope of the route."""
    created = []

    def answer(method, url, headers, body):
        path = url[len(FAKE_HOST):]
        need = ("ai-gateway" if path.startswith("/ai-gateway") else "knowledge" if "/knowledge/" in path
                else "genie" if "/genie/" in path else "postgres" if "/postgres/" in path else None)
        if need and need not in scopes:
            return resp(403, {"error_code": "PERMISSION_DENIED", "message": f"Missing scope {need}"})
        if "mcp-services" in path:
            msg = json.loads(body)
            if "id" not in msg:
                return resp(202, raw=b"")
            result = {"protocolVersion": "2025-06-18"} if msg["method"] == "initialize" else {"tools": [{"name": "genie_ask"}]}
            return resp(200, {"jsonrpc": "2.0", "id": msg["id"], "result": result})
        if "/knowledge/pages" in path and method == "POST":
            created.append(path)
            return resp(pages_status, {"name": "pages/p1", "state": "DRAFT"})
        if "/knowledge/pages/" in path and method == "DELETE":
            return resp(200, {})
        return resp(200, {"choices": [{"message": {"content": "OK"}}]})

    answer.created = created
    return answer


def run_with(scopes, domain="d1"):
    t = FakeTransport(workspace(scopes))
    api = sl.Api(FAKE_HOST, sl.Token(fake_token(" ".join(scopes))), t)
    return r14.run_calls(api, "123", domain), t


def test_sp_14_w19_scopes_pass_all_mvp_calls():
    """[SP-14] [W-19] With ai-gateway, genie, knowledge, offline_access all MVP calls succeed."""
    steps, t = run_with(r14.W19_SCOPES)
    ev = r14.evaluate(steps, sorted(r14.W19_SCOPES))
    assert ev["passed"] and ev["scope_list_matches"] == "W-19"
    assert ev["info_calls"] == {"genie_one_api_list": True, "postgres_projects_list": False}
    assert steps["knowledge_pages"]["cleanup_status"] == 200
    assert any("request_id=" in rq["url"] for rq in t.requests)


def test_sp_14_issue_scopes_fail_on_pages():
    """[SP-14] The issue list (postgres, no knowledge) fails the pages call: fallback all-apis."""
    steps, _ = run_with(r14.ISSUE_SCOPES)
    ev = r14.evaluate(steps, sorted(r14.ISSUE_SCOPES))
    assert not ev["passed"] and ev["failing_mvp_calls"] == ["knowledge_pages"]
    assert ev["scope_list_matches"] == "issue #167" and ev["fallback"] == "Stay on all-apis."


def test_sp_14_no_ai_gateway_fails_mcp_and_model():
    """[SP-14] Without ai-gateway, MCP and the model call fail, and tools/list is skipped."""
    steps, _ = run_with(["genie", "knowledge"])
    ev = r14.evaluate(steps, ["genie", "knowledge"])
    assert ev["failing_mvp_calls"] == ["mcp_initialize", "mcp_tools_list", "model_call"]
    assert steps["mcp_tools_list"]["skipped"]


def test_sp_14_pages_list_mode_without_domain():
    """[SP-14] Without --domain-id the pages call is a GET list and notes the permission caveat."""
    steps, t = run_with(r14.W19_SCOPES, domain=None)
    assert steps["knowledge_pages"]["mode"] == "list" and "CHECK" in steps["knowledge_pages"]["note"]
    assert not any(rq["method"] == "POST" and "/knowledge/" in rq["url"] for rq in t.requests)


def test_sp_14_report_has_no_token():
    """[SP-14] The steps and evaluation hold no token value."""
    tok = fake_token(" ".join(r14.W19_SCOPES))
    api = sl.Api(FAKE_HOST, sl.Token(tok), FakeTransport(workspace(r14.W19_SCOPES)))
    steps = r14.run_calls(api, "123", "d1")
    text = json.dumps({"steps": steps, "token": sl.token_kind(tok)})
    assert tok not in text and "user-123" not in text
