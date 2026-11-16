#!/usr/bin/env python3
"""SP-14 (#167): fine-grained CLI OAuth scopes on the published `databricks-cli` app.

First log in to a NEW bottle-* profile with narrow scopes (CLI v1.25: --scopes is comma-separated):
  databricks auth login --host https://<host> --profile bottle-sp14-w19 \
      --scopes ai-gateway,genie,knowledge,offline_access          # W-19 list (requirements)
  databricks auth login --host https://<host> --profile bottle-sp14-issue \
      --scopes ai-gateway,genie,postgres,offline_access           # issue #167 list
Then run this script once per profile. It reads the scope claim of the token in memory (only the
scope list is printed), then runs each MVP call:
  MVP: Genie One MCP initialize, MCP tools/list, a model call, a knowledge-pages call.
  Info (v1): Genie One API list (scope genie), Lakebase projects list (scope postgres).
Pass: all MVP calls succeed. Fallback: stay on all-apis.

Run:
  uv run --no-project --with httpx spikes/sp-14/run.py --profile <your-bottle-profile> --out sp14.json \
      [--domain-id <id>]   # with a domain: pages call = create a DRAFT probe Page, then delete it
"""
import argparse
import json
import os
import sys
import uuid

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "common"))
import spikelib as sl  # noqa: E402

SPIKE = "SP-14"
W19_SCOPES = ["ai-gateway", "genie", "knowledge", "offline_access"]
ISSUE_SCOPES = ["ai-gateway", "genie", "postgres", "offline_access"]
MVP_CALLS = ["mcp_initialize", "mcp_tools_list", "model_call", "knowledge_pages"]
MODEL = "system.ai.meta-llama-3-1-8b-instruct"
PROBE_MARK = "Bottle SP-14 scope probe"


def step_of(resp, name):
    s = {"call": name, "status": resp.status, "ok": 200 <= resp.status < 300, "elapsed_ms": resp.elapsed_ms}
    if not s["ok"]:
        s["error_shape"] = sl.error_shape(resp)
    return s


def pages_call(api, domain_id):
    """Create a DRAFT probe Page and delete it (needs --domain-id), else GET the list."""
    if not domain_id:
        r = api.call("GET", "/api/2.0/knowledge/pages?page_size=1")
        s = step_of(r, "knowledge_pages")
        s["mode"] = "list"
        s["note"] = "CHECK: list may need MANAGE DISCOVER or MANAGE DOMAIN; a 403 can be a permission, not a scope"
        return s
    rid = str(uuid.uuid4())
    body = {"domain": f"domains/{domain_id}", "display_name": f"{PROBE_MARK} {rid[:8]}",
            "body": "Probe Page from the Bottle SP-14 scope spike. It is deleted at once.",
            "synonyms": [], "description": "SP-14 probe", "state": "DRAFT"}
    r = api.call("POST", f"/api/2.0/knowledge/pages?request_id={rid}", body=body)
    s = step_of(r, "knowledge_pages")
    s["mode"] = "create+delete"
    if s["ok"]:
        name = (r.json() or {}).get("name", "")
        s["state"] = (r.json() or {}).get("state")
        if name.startswith("pages/"):
            d = api.call("DELETE", f"/api/2.0/knowledge/{name}")
            s["cleanup_status"] = d.status
    return s


def run_calls(api, workspace_id, domain_id=None, model=MODEL):
    steps = {}
    mcp = sl.McpClient(api, sl.mcp_url("", workspace_id))
    init = mcp.initialize()
    steps["mcp_initialize"] = {"call": "mcp_initialize", "ok": init["ok"], "status": init["status"],
                               "elapsed_ms": init["elapsed_ms"],
                               **({k: init[k] for k in ("error", "jsonrpc_error") if k in init})}
    if init["ok"]:
        tl = mcp.tools_list()
        steps["mcp_tools_list"] = {"call": "mcp_tools_list", "ok": tl["ok"], "status": tl["status"],
                                   "tools": tl.get("tools"),
                                   **({k: tl[k] for k in ("error", "jsonrpc_error") if k in tl})}
    else:
        steps["mcp_tools_list"] = {"call": "mcp_tools_list", "ok": False, "skipped": "initialize failed"}
    body = {"model": model, "max_tokens": 5, "messages": [{"role": "user", "content": "Say OK."}]}
    steps["model_call"] = step_of(api.call("POST", "/ai-gateway/mlflow/v1/chat/completions", body=body),
                                  "model_call")
    steps["knowledge_pages"] = pages_call(api, domain_id)
    steps["genie_one_api_list"] = step_of(api.call("GET", "/api/2.0/genie/one/conversations?page_size=1"),
                                          "genie_one_api_list")
    steps["postgres_projects_list"] = step_of(api.call("GET", "/api/2.0/postgres/projects?page_size=1"),
                                              "postgres_projects_list")
    return steps


def evaluate(steps, scopes):
    failing = [c for c in MVP_CALLS if not steps.get(c, {}).get("ok")]
    passed = not failing
    matches = None
    for name, want in (("W-19", W19_SCOPES), ("issue #167", ISSUE_SCOPES)):
        if sorted(want) == sorted(scopes):
            matches = name
    return {"passed": passed, "failing_mvp_calls": failing, "token_scopes": scopes,
            "scope_list_matches": matches,
            "info_calls": {k: steps[k]["ok"] for k in ("genie_one_api_list", "postgres_projects_list") if k in steps},
            "fallback": None if passed else "Stay on all-apis."}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profile", required=True, help="the bottle-* profile you logged in with --scopes")
    p.add_argument("--out", required=True)
    p.add_argument("--workspace-id")
    p.add_argument("--domain-id", help="domain ID for the DRAFT create/delete pages call")
    p.add_argument("--model", default=MODEL)
    args = p.parse_args(argv)

    def go():
        host = sl.get_host(args.profile)
        token = sl.get_token(args.profile)
        kind = sl.token_kind(token.reveal())
        scopes = sl.scopes_of(kind)
        sl.say(f"{SPIKE}: token format={kind.get('format')} scopes={scopes}")
        api = sl.Api(host, token, sl.HttpxTransport(timeout=60))
        wid = sl.resolve_workspace_id(api, args.workspace_id)
        steps = run_calls(api, wid, args.domain_id, args.model)
        ev = evaluate(steps, scopes)
        report = {"spike": SPIKE, "issue": 167, "profile_label": args.profile, "token": kind,
                  "steps": steps, "evaluation": ev}
        sl.write_report(args.out, report, secrets=[token.reveal()])
        for name, s in steps.items():
            sl.say(f"  {name:<24} ok={s.get('ok')} status={s.get('status')}")
        sl.say(sl.verdict(SPIKE, ev["passed"], f"scopes={scopes} failing={ev['failing_mvp_calls']} "
                                               f"info={json.dumps(ev['info_calls'])}"))
        token.wipe()
        return 0 if ev["passed"] else 1

    return sl.main_guard(go)


if __name__ == "__main__":
    sys.exit(main())
