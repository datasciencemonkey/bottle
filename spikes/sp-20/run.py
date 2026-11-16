#!/usr/bin/env python3
"""SP-20 (#187), with SP-20b: UC Pages API, scopes, idempotent retry, domains and MCP visibility.

Automated per profile (run once with an all-apis profile, once with a knowledge-only profile):
 1. GET /api/2.0/domains (Beta, scope `domains`, COL-100): status and count only.
 2. POST /api/2.0/knowledge/pages with state DRAFT and request_id as a QUERY parameter (API docs),
    then the same POST again: the second answer must name the same Page (idempotent).
 3. The same pair with request_id in the BODY (B3 text lists it with the body fields). Both forms
    are tried because the docs and the requirements differ (#187 comment by spike-lead).
 4. Cleanup: delete the body-form Pages. Keep the query-form Page for the manual DRAFT-grounding
    test unless --no-keep. Later: --cleanup-from <report> deletes only Pages that this script made
    (name from the report AND display_name with the probe marker).
Optional --mcp-check (SP-20b): make one Genie One MCP conversation with a marker question and
record its ID, so a person can look for it in the Genie One history.

Pass (per profile): 200 with state DRAFT and an idempotent retry. Grounding (owner only) is manual.
Fallback: keep Page drafts local with Copy as markdown; ask the account team.

Run:
  uv run --no-project --with httpx spikes/sp-20/run.py --profile <your-profile> --domain-id <id> --out sp20.json
"""
import argparse
import json
import os
import sys
import uuid

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "common"))
import spikelib as sl  # noqa: E402

SPIKE = "SP-20"
PAGES = "/api/2.0/knowledge/pages"
MARK = "Bottle SP-20 probe"
TERM = "Bottle quuxrate"
DEFINITION = (f"{TERM} means the count of blue widgets divided by the count of red widgets in one week. "
              f"{TERM} is a test definition from the Bottle SP-20 spike. It has no business meaning.")
MCP_QUESTION = "Bottle SP-20b probe: what does Bottle quuxrate mean?"


def page_body(domain_id, tag, request_id=None):
    b = {"domain": f"domains/{domain_id}", "display_name": f"{MARK} {tag}", "body": DEFINITION,
         "synonyms": ["quuxrate", "bottle quux rate"], "description": "Test Page from the Bottle SP-20 spike.",
         "state": "DRAFT"}
    if request_id:
        b["request_id"] = request_id
    return b


def create(api, domain_id, form, request_id, tag):
    if form == "query":
        r = api.call("POST", f"{PAGES}?request_id={request_id}", body=page_body(domain_id, tag))
    else:
        r = api.call("POST", PAGES, body=page_body(domain_id, tag, request_id))
    s = {"form": form, "status": r.status, "elapsed_ms": r.elapsed_ms}
    if 200 <= r.status < 300:
        j = r.json() or {}
        s.update({"name": j.get("name"), "state": j.get("state"), "owner_ids_count": len(j.get("owner_ids") or []),
                  "synonyms_count": len(j.get("synonyms") or []), "domain": j.get("domain"),
                  "response_shape": sl.json_shape({k: v for k, v in j.items() if k != "body"})})
    else:
        s["error_shape"] = sl.error_shape(r)
    return s


def pair(api, domain_id, form):
    rid = str(uuid.uuid4())
    tag = f"{form} {rid[:8]}"
    first = create(api, domain_id, form, rid, tag)
    second = create(api, domain_id, form, rid, tag)
    same = bool(first.get("name")) and first.get("name") == second.get("name")
    names = sorted({x["name"] for x in (first, second) if x.get("name")})
    return {"request_id": rid, "first": first, "second": second, "idempotent": same, "created_names": names}


def domains_check(api):
    r = api.call("GET", "/api/2.0/domains?page_size=50")
    s = {"status": r.status, "ok": 200 <= r.status < 300}
    if s["ok"]:
        s["count"] = len((r.json() or {}).get("domains", []))
    else:
        s["error_shape"] = sl.error_shape(r)
    return s


def delete_owned(api, names):
    """Delete Pages by name only if GET shows our marker in display_name."""
    out = []
    for name in names:
        if not (isinstance(name, str) and name.startswith("pages/")):
            out.append({"name": name, "deleted": False, "reason": "not a page name"})
            continue
        g = api.call("GET", f"/api/2.0/knowledge/{name}")
        dn = ((g.json() or {}).get("display_name", "") if g.status == 200 else "")
        if not dn.startswith(MARK):
            out.append({"name": name, "deleted": False, "reason": f"GET {g.status}; marker missing"})
            continue
        d = api.call("DELETE", f"/api/2.0/knowledge/{name}")
        out.append({"name": name, "deleted": 200 <= d.status < 300, "status": d.status})
    return out


def evaluate(r):
    q = r.get("query_form", {})
    first = q.get("first", {})
    created = first.get("status") == 200 and first.get("state") == "DRAFT"
    passed = created and q.get("idempotent") is True
    return {"passed": passed, "created_draft": created, "idempotent_query_form": q.get("idempotent"),
            "idempotent_body_form": r.get("body_form", {}).get("idempotent"),
            "body_form_status": r.get("body_form", {}).get("first", {}).get("status"),
            "domains_readable": r.get("domains", {}).get("ok"),
            "domains_fallback": None if r.get("domains", {}).get("ok") else "Paste a domain ID or URL (PG-20).",
            "fallback": None if passed else "Keep Page drafts local with Copy as markdown; ask the account team."}


def run(api, args, mcp_factory=None):
    rep = {"domains": domains_check(api)}
    rep["query_form"] = pair(api, args.domain_id, "query")
    rep["body_form"] = pair(api, args.domain_id, "body")
    to_delete = list(rep["body_form"]["created_names"])
    if args.no_keep or len(rep["query_form"]["created_names"]) > 1:
        to_delete += rep["query_form"]["created_names"]
        rep["kept_for_grounding"] = []
    else:
        rep["kept_for_grounding"] = rep["query_form"]["created_names"]
    rep["cleanup"] = delete_owned(api, to_delete)
    if args.mcp_check and mcp_factory:
        rep["mcp_check"] = mcp_factory()
    rep["evaluation"] = evaluate(rep)
    return rep


def mcp_check(api, workspace_id):
    mcp = sl.McpClient(api, sl.mcp_url("", workspace_id))
    init = mcp.initialize()
    if not init["ok"]:
        return {"ok": False, "step": "initialize", "detail": init.get("error") or init.get("jsonrpc_error")}
    ask = mcp.call_tool("genie_ask", {"question": MCP_QUESTION})
    cid = sl.find_key(ask.pop("result", None), "conversation_id") if ask["ok"] else None
    out = {"ok": bool(cid), "conversation_id": cid, "question_marker": "Bottle SP-20b probe"}
    if cid:
        lst = api.call("GET", "/api/2.0/genie/one/conversations?page_size=100")
        out["genie_one_api_list_status"] = lst.status
        if lst.status == 200:
            out["listed_in_genie_one_api"] = any(c.get("conversation_id") == cid
                                                 for c in (lst.json() or {}).get("conversations", []))
    return out


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profile", required=True, help="Databricks CLI profile (you choose it)")
    p.add_argument("--out", required=True)
    p.add_argument("--domain-id", help="ID of the home domain (from GET /api/2.0/domains or Catalog Explorer)")
    p.add_argument("--no-keep", action="store_true", help="delete the DRAFT Page at the end")
    p.add_argument("--mcp-check", action="store_true", help="SP-20b: make one MCP conversation")
    p.add_argument("--workspace-id")
    p.add_argument("--cleanup-from", help="delete the kept Page named in this report")
    args = p.parse_args(argv)

    def go():
        host = sl.get_host(args.profile)
        token = sl.get_token(args.profile)
        api = sl.Api(host, token, sl.HttpxTransport(timeout=60))
        kind = sl.token_kind(token.reveal())
        if args.cleanup_from:
            with open(args.cleanup_from, encoding="utf-8") as f:
                names = json.load(f).get("kept_for_grounding", [])
            res = delete_owned(api, names)
            sl.write_report(args.out, {"spike": SPIKE, "cleanup": res}, secrets=[token.reveal()])
            sl.say(f"{SPIKE} cleanup: {res}")
            return 0 if all(x["deleted"] for x in res) else 1
        if not args.domain_id:
            d = domains_check(api)
            sl.say(f"{SPIKE}: GET /api/2.0/domains status={d['status']} count={d.get('count')}")
            raise sl.SpikeError("pass --domain-id (the script never picks a domain)")
        wid = sl.resolve_workspace_id(api, args.workspace_id) if args.mcp_check else None
        rep = run(api, args, (lambda: mcp_check(api, wid)) if args.mcp_check else None)
        rep.update({"spike": SPIKE, "issue": 187, "profile_label": args.profile,
                    "token_scopes": sl.scopes_of(kind), "term": TERM})
        sl.write_report(args.out, rep, secrets=[token.reveal()])
        ev = rep["evaluation"]
        sl.say(f"  kept for grounding: {rep['kept_for_grounding']}  term: {TERM}")
        sl.say(sl.verdict(SPIKE, ev["passed"],
                          f"scopes={sl.scopes_of(kind)} draft={ev['created_draft']} "
                          f"idempotent(query)={ev['idempotent_query_form']} "
                          f"body_form_status={ev['body_form_status']} idempotent(body)={ev['idempotent_body_form']} "
                          f"domains_readable={ev['domains_readable']}"))
        token.wipe()
        return 0 if ev["passed"] else 1

    return sl.main_guard(go)


if __name__ == "__main__":
    sys.exit(main())
