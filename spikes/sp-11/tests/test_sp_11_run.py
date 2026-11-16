"""[SP-11] Offline tests for spikes/sp-11/run.py (fake MCP server and fake CLI)."""
import json
import os
import sys
from types import SimpleNamespace

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import FAKE_HOST, FakeTransport, fake_token, load_run, resp  # noqa: E402

import spikelib as sl  # noqa: E402

r11 = load_run("sp-11", "sp11_run")


class FakeGenieOne:
    """In-memory Genie One API behind fake `databricks api get|delete`."""

    def __init__(self, ids, delete_ok=True, page=2):
        self.ids = list(ids)
        self.delete_ok = delete_ok
        self.page = page
        self.calls = []

    def get(self, path):
        self.calls.append(("GET", path))
        if path.startswith(r11.LIST + "?"):
            start = int(path.split("page_token=")[1]) if "page_token=" in path else 0
            chunk = self.ids[start:start + self.page]
            nxt = str(start + self.page) if start + self.page < len(self.ids) else None
            body = {"conversations": [{"conversation_id": i, "title": "secret title"} for i in chunk]}
            if nxt:
                body["next_page_token"] = nxt
            return {"ok": True, "json": body, "elapsed_ms": 1}
        cid = path.rsplit("/", 1)[1]
        if cid in self.ids:
            return {"ok": True, "json": {"conversation_id": cid}, "elapsed_ms": 1}
        return {"ok": False, "error": "Error: Conversation not found", "elapsed_ms": 1}

    def delete(self, path):
        self.calls.append(("DELETE", path))
        cid = path.rsplit("/", 1)[1]
        if self.delete_ok and cid in self.ids:
            self.ids.remove(cid)
            return {"ok": True, "json": {}, "elapsed_ms": 1}
        return {"ok": False, "error": "Error: PERMISSION_DENIED", "elapsed_ms": 1}


def mcp_server(cid="conv-9"):
    def answer(method, url, headers, body):
        msg = json.loads(body)
        if msg.get("method") == "notifications/initialized":
            return resp(202, raw=b"")
        if msg["method"] == "initialize":
            return resp(200, {"jsonrpc": "2.0", "id": msg["id"], "result": {"protocolVersion": "2025-06-18"}},
                        headers={"mcp-session-id": "s1"})
        if msg["method"] == "tools/list":
            return resp(200, {"jsonrpc": "2.0", "id": msg["id"], "result": {"tools": [{"name": "genie_ask"}]}})
        name = msg["params"]["name"]
        if name == "genie_ask":
            text = json.dumps({"conversation_id": cid, "response_id": "r1", "status": "IN_PROGRESS"})
        else:
            text = json.dumps({"status": "COMPLETED"})
        sse = "data: " + json.dumps({"jsonrpc": "2.0", "id": msg["id"],
                                     "result": {"content": [{"type": "text", "text": text}]}}) + "\n\n"
        return resp(200, raw=sse.encode(), headers={"content-type": "text/event-stream"})
    return answer


def args(**kw):
    base = dict(conversation_id=None, i_created_it=False, workspace_id="123", question=r11.QUESTION,
                poll_s=5, settle_s=0, max_pages=5)
    base.update(kw)
    return SimpleNamespace(**base)


def test_sp_11_mcp_create_returns_conversation_id():
    """[SP-11] genie_ask over MCP (SSE answer) gives the conversation ID; ?o= is in the URL."""
    t = FakeTransport(mcp_server())
    api = sl.Api(FAKE_HOST, sl.Token(fake_token()), t)
    cid, steps = r11.create_via_mcp(api, "123", "q", poll_s=5, sleep=lambda s: None)
    assert cid == "conv-9" and steps["genie_ask"]["conversation_id_found"]
    assert steps["poll"]["last_status"] == "COMPLETED"
    assert all(rq["url"].endswith("system.ai.genie_one_mcp?o=123") for rq in t.requests)


def test_sp_11_find_in_list_pages():
    """[SP-11] The list search follows next_page_token."""
    g = FakeGenieOne(["a", "b", "c", "d", "e"])
    assert r11.find_in_list(g.get, "e") == (True, 3, True)
    assert r11.find_in_list(g.get, "zz") == (False, 3, True)
    assert r11.find_in_list(g.get, "e", max_pages=1) == (False, 1, True)


def test_sp_11_full_run_passes_and_deletes_only_its_thread():
    """[SP-11] Pass: the MCP thread lists, deletes and is gone; no other thread is touched."""
    g = FakeGenieOne(["other-1", "conv-9", "other-2"])
    api = lambda: sl.Api(FAKE_HOST, sl.Token(fake_token()), FakeTransport(mcp_server()))  # noqa: E731
    rep = r11.run(args(), api, g.get, g.delete)
    assert rep["evaluation"]["passed"] and rep["created_by_script"]
    assert [c for c in g.calls if c[0] == "DELETE"] == [("DELETE", r11.LIST + "/conv-9")]
    assert g.ids == ["other-1", "other-2"]
    assert "secret title" not in json.dumps(rep)


def test_sp_11_fail_when_delete_denied():
    """[SP-11] A failed delete fails the spike and picks the deep-link fallback."""
    g = FakeGenieOne(["conv-9"], delete_ok=False)
    api = lambda: sl.Api(FAKE_HOST, sl.Token(fake_token()), FakeTransport(mcp_server()))  # noqa: E731
    rep = r11.run(args(), api, g.get, g.delete)
    assert not rep["evaluation"]["passed"] and "Delete in Genie One" in rep["evaluation"]["ui_decision"]


def test_sp_11_fail_when_mcp_thread_not_listed():
    """[SP-11] A thread that does not list (MCP threads hidden) fails."""
    g = FakeGenieOne(["other"])
    api = lambda: sl.Api(FAKE_HOST, sl.Token(fake_token()), FakeTransport(mcp_server()))  # noqa: E731
    rep = r11.run(args(), api, g.get, g.delete)
    assert rep["listed_before"] is False and not rep["evaluation"]["passed"]


def test_sp_11_given_id_needs_confirmation():
    """[SP-11] --conversation-id without --i-created-it is refused (never delete another thread)."""
    g = FakeGenieOne(["x"])
    with pytest.raises(sl.SpikeError):
        r11.run(args(conversation_id="x"), None, g.get, g.delete)
    rep = r11.run(args(conversation_id="x", i_created_it=True), None, g.get, g.delete)
    assert rep["evaluation"]["passed"] and rep["source"] == "given"
