"""[SP-20] Offline tests for spikes/sp-20/run.py (fake Pages API)."""
import json
import os
import sys
from types import SimpleNamespace
from urllib.parse import parse_qs, urlsplit

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "..", "common"))
from loader import FAKE_HOST, FakeTransport, fake_token, load_run, resp  # noqa: E402

import spikelib as sl  # noqa: E402

r20 = load_run("sp-20", "sp20_run")


class FakePages:
    """Pages API: idempotent on the query request_id; body request_id is ignored (duplicates)."""

    def __init__(self, domains_status=200, body_field_rejected=False, other_pages=None):
        self.pages = dict(other_pages or {})
        self.by_rid = {}
        self.n = 0
        self.domains_status = domains_status
        self.body_field_rejected = body_field_rejected
        self.deleted = []

    def __call__(self, method, url, headers, body):
        u = urlsplit(url)
        path, q = u.path, parse_qs(u.query)
        if path == "/api/2.0/domains":
            if self.domains_status != 200:
                return resp(self.domains_status, {"error_code": "PERMISSION_DENIED", "message": "MANAGE DISCOVER"})
            return resp(200, {"domains": [{"name": "domains/d1", "domain_id": "d1"}]})
        if path == r20.PAGES and method == "POST":
            b = json.loads(body)
            if "request_id" in b and self.body_field_rejected:
                return resp(400, {"error_code": "INVALID_PARAMETER_VALUE", "message": "Unknown field request_id"})
            rid = q.get("request_id", [None])[0]
            if rid and rid in self.by_rid:
                return resp(200, self.pages[self.by_rid[rid]])
            self.n += 1
            name = f"pages/p{self.n}"
            page = dict(b, name=name, owner_ids=["u1"])
            page.pop("request_id", None)
            self.pages[name] = page
            if rid:
                self.by_rid[rid] = name
            return resp(200, page)
        name = path.removeprefix("/api/2.0/knowledge/")
        if method == "GET":
            return resp(200, self.pages[name]) if name in self.pages else resp(404, {"error_code": "NOT_FOUND"})
        if method == "DELETE":
            self.deleted.append(name)
            self.pages.pop(name, None)
            return resp(200, {})
        return resp(404, {})


def make(fake):
    return sl.Api(FAKE_HOST, sl.Token(fake_token("knowledge offline_access")), FakeTransport(fake))


def args(**kw):
    a = dict(domain_id="d1", no_keep=False, mcp_check=False)
    a.update(kw)
    return SimpleNamespace(**a)


def test_sp_20_body_is_draft_with_domain_name_and_few_synonyms():
    """[SP-20] [PG-05] [PG-06] [PG-08] The create body has state DRAFT, domains/<id> and <= 50 synonyms."""
    b = r20.page_body("d1", "t")
    assert b["state"] == "DRAFT" and b["domain"] == "domains/d1" and len(b["synonyms"]) <= 50
    assert "request_id" not in b and r20.page_body("d1", "t", "r")["request_id"] == "r"


def test_sp_20_query_form_is_idempotent_and_page_is_kept():
    """[SP-20] [PG-07] The same query request_id returns the same Page; it is kept for grounding."""
    fake = FakePages()
    rep = r20.run(make(fake), args())
    ev = rep["evaluation"]
    assert ev["passed"] and ev["created_draft"] and ev["idempotent_query_form"]
    assert ev["idempotent_body_form"] is False  # the fake ignores the body field
    assert rep["kept_for_grounding"] == rep["query_form"]["created_names"] == ["pages/p1"]
    assert sorted(fake.deleted) == ["pages/p2", "pages/p3"]  # the two body-form duplicates


def test_sp_20_no_keep_deletes_everything_it_made():
    """[SP-20] --no-keep also deletes the query-form Page."""
    fake = FakePages()
    rep = r20.run(make(fake), args(no_keep=True))
    assert rep["kept_for_grounding"] == [] and sorted(fake.deleted) == ["pages/p1", "pages/p2", "pages/p3"]


def test_sp_20_delete_owned_skips_pages_without_marker():
    """[SP-20] Cleanup never deletes a Page without the probe marker, or a non-page name."""
    fake = FakePages(other_pages={"pages/x": {"name": "pages/x", "display_name": "Real ARR page"}})
    res = r20.delete_owned(make(fake), ["pages/x", "domains/d1"])
    assert [x["deleted"] for x in res] == [False, False] and fake.deleted == []


def test_sp_20_domains_denied_gives_paste_fallback():
    """[SP-20] [PG-20] A 403 on GET /api/2.0/domains records the paste-an-ID fallback."""
    rep = r20.run(make(FakePages(domains_status=403)), args())
    assert rep["evaluation"]["domains_readable"] is False
    assert "PG-20" in rep["evaluation"]["domains_fallback"]


def test_sp_20_body_form_rejected_is_recorded_not_fatal():
    """[SP-20] A 400 for request_id in the body is recorded; the query form still decides the pass."""
    rep = r20.run(make(FakePages(body_field_rejected=True)), args())
    assert rep["evaluation"]["body_form_status"] == 400 and rep["evaluation"]["passed"]


def test_sp_20_fail_when_retry_duplicates():
    """[SP-20] Two Pages for one request_id fails; both are deleted and none is kept."""
    class NoIdem(FakePages):
        def __call__(self, method, url, headers, body):
            self.by_rid.clear()
            return super().__call__(method, url, headers, body)
    fake = NoIdem()
    rep = r20.run(make(fake), args())
    assert not rep["evaluation"]["passed"] and "Copy as markdown" in rep["evaluation"]["fallback"]
    assert rep["kept_for_grounding"] == [] and len(fake.deleted) == 4


def test_sp_20_report_has_no_definition_body_or_token():
    """[SP-20] The create step keeps the response shape without the body text."""
    rep = r20.run(make(FakePages()), args())
    text = json.dumps(rep)
    assert "blue widgets" not in text and "eyJ" not in text
