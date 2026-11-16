#!/usr/bin/env python3
"""SP-11 (#121): list and delete a Genie One MCP thread through the Genie One API (D76).

Steps:
 1. Create a thread with Genie One MCP (`genie_ask`, small JSON-RPC client; this is not the SP-09
    client). Or skip this step with --conversation-id for a thread you made yourself in an MCP client.
 2. List it:   databricks api get /api/2.0/genie/one/conversations?page_size=100 (paged)
 3. Delete it: databricks api delete /api/2.0/genie/one/conversations/<id>
 4. Check:     list again (must be absent) and GET it (must fail).
The script deletes only the conversation it created in step 1, or the one you name with
--conversation-id together with --i-created-it.

Pass: the MCP thread lists and deletes. Fallback: "Delete in Genie One" deep link with a disclosure.
Needs: Genie One API preview on (admin, ADM-14), scope `genie` for the API and `ai-gateway` for MCP.

Run:
  uv run --no-project --with httpx spikes/sp-11/run.py --profile <your-profile> --out sp11.json
"""
import argparse
import os
import subprocess
import sys
import time
from urllib.parse import quote

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "common"))
import spikelib as sl  # noqa: E402

SPIKE = "SP-11"
LIST = "/api/2.0/genie/one/conversations"
QUESTION = "Bottle SP-11 probe: which catalogs can I see? Answer in one line."


def create_via_mcp(api, workspace_id, question, poll_s=90, sleep=time.sleep):
    """Return (conversation_id or None, steps)."""
    mcp = sl.McpClient(api, sl.mcp_url("", workspace_id))
    steps = {"initialize": mcp.initialize()}
    if not steps["initialize"]["ok"]:
        return None, steps
    steps["tools_list"] = mcp.tools_list()
    ask = mcp.call_tool("genie_ask", {"question": question})
    res = ask.pop("result", None)
    cid = sl.find_key(res, "conversation_id") if ask["ok"] else None
    rid = sl.find_key(res, "response_id") if ask["ok"] else None
    ask["conversation_id_found"] = bool(cid)
    ask["status_field"] = sl.find_key(res, "status") if ask["ok"] else None
    steps["genie_ask"] = ask
    if cid and rid and poll_s:
        # CHECK: genie_poll_response argument names are not in the docs page; these follow genie_ask.
        deadline = time.monotonic() + poll_s
        polls = 0
        while time.monotonic() < deadline:
            st = mcp.call_tool("genie_poll_response", {"conversation_id": cid, "response_id": rid})
            polls += 1
            status = sl.find_key(st.pop("result", None), "status") if st["ok"] else None
            steps["poll"] = {"ok": st["ok"], "polls": polls, "last_status": status}
            if not st["ok"] or (status or "").lower() in ("completed", "failed", "incomplete", "cancelled"):
                break
            sleep(3)
    return cid, steps


def find_in_list(fetch_page, cid, max_pages=5):
    """fetch_page(token) -> step dict with json {conversations, next_page_token}. Returns (found, pages, ok)."""
    token, pages = None, 0
    while pages < max_pages:
        path = LIST + "?page_size=100" + (f"&page_token={quote(token)}" if token else "")
        step = fetch_page(path)
        pages += 1
        if not step["ok"]:
            return False, pages, False
        data = step.get("json") or {}
        if any(c.get("conversation_id") == cid for c in data.get("conversations", [])):
            return True, pages, True
        token = data.get("next_page_token")
        if not token:
            break
    return False, pages, True


def evaluate(r):
    listed = r.get("listed_before") is True
    deleted = r.get("delete", {}).get("ok") is True
    gone = r.get("listed_after") is False and r.get("get_after_ok") is False
    passed = listed and deleted and gone
    return {"passed": passed, "listed": listed, "deleted": deleted, "absent_after": gone,
            "ui_decision": "Genie One API delete utility (G-45a)" if passed
            else "\"Delete in Genie One\" deep link with a disclosure (G-45b)"}


def run(args, api_call, cli_get, cli_delete):
    report = {"created_by_script": False}
    cid = args.conversation_id
    if cid:
        if not args.i_created_it:
            raise sl.SpikeError("--conversation-id needs --i-created-it (the script deletes only your own probe thread)")
        report["source"] = "given"
    else:
        api = api_call()
        wid = sl.resolve_workspace_id(api, args.workspace_id)
        cid, steps = create_via_mcp(api, wid, args.question, poll_s=args.poll_s)
        report["mcp"] = steps
        report["source"] = "mcp"
        if not cid:
            report["evaluation"] = evaluate(report)
            report["evaluation"]["note"] = "no conversation_id from genie_ask"
            return report
        report["created_by_script"] = True
        time.sleep(args.settle_s)
    report["conversation_id"] = cid
    found, pages, ok = find_in_list(cli_get, cid, args.max_pages)
    report["listed_before"] = found if ok else None
    report["list_pages_read"] = pages
    report["get_before_ok"] = cli_get(f"{LIST}/{quote(cid)}")["ok"]
    d = cli_delete(f"{LIST}/{quote(cid)}")
    report["delete"] = {"ok": d["ok"], "error": d.get("error"), "elapsed_ms": d["elapsed_ms"],
                        "json": d.get("json")}
    time.sleep(args.settle_s)
    found2, _, ok2 = find_in_list(cli_get, cid, args.max_pages)
    report["listed_after"] = found2 if ok2 else None
    g = cli_get(f"{LIST}/{quote(cid)}")
    report["get_after_ok"] = g["ok"]
    report["get_after_error"] = g.get("error")
    report["evaluation"] = evaluate(report)
    return report


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profile", required=True, help="Databricks CLI profile (you choose it)")
    p.add_argument("--out", required=True, help="path of the JSON report")
    p.add_argument("--workspace-id", help="workspace ID for ?o= (default: read from a response header)")
    p.add_argument("--conversation-id", help="skip the MCP step and use this thread")
    p.add_argument("--i-created-it", action="store_true", help="confirm the given thread is your probe thread")
    p.add_argument("--question", default=QUESTION)
    p.add_argument("--poll-s", type=int, default=90)
    p.add_argument("--settle-s", type=float, default=3.0)
    p.add_argument("--max-pages", type=int, default=5)
    args = p.parse_args(argv)
    secrets = []

    def go():
        def api_call():
            host = sl.get_host(args.profile, subprocess.run)
            token = sl.get_token(args.profile, subprocess.run)
            secrets.append(token.reveal())
            return sl.Api(host, token, sl.HttpxTransport(timeout=60))

        def cli_get(path):
            return sl.cli_api("get", path, args.profile, runner=subprocess.run)

        def cli_delete(path):
            return sl.cli_api("delete", path, args.profile, runner=subprocess.run)

        report = run(args, api_call, cli_get, cli_delete)
        report.update({"spike": SPIKE, "issue": 121})
        sl.write_report(args.out, report, secrets=secrets)
        ev = report["evaluation"]
        sl.say(sl.verdict(SPIKE, ev["passed"], f"listed={ev['listed']} deleted={ev['deleted']} "
                                               f"absent_after={ev['absent_after']} -> {ev['ui_decision']}"))
        return 0 if ev["passed"] else 1

    return sl.main_guard(go)


if __name__ == "__main__":
    sys.exit(main())
