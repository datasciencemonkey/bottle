#!/usr/bin/env python3
"""SP-16 (#157): token type and scope that reach a Databricks App on /api/invocations.

Deploy spikes/sp-16/app first (see RUN.md). Then call it twice: once with user authorization on,
once with it off. Each call prints PASS/FAIL for that mode. --merge reads both reports and prints
the spike verdict.

Pass: token type and scope known (for both modes). Fallback: origin-bound lease per app; v1 gating.

Run:
  uv run --no-project --with httpx spikes/sp-16/run.py --profile <your-profile> \
      --app-name bottle-sp16 --mode on --out sp16-on.json
  uv run --no-project spikes/sp-16/run.py --merge sp16-on.json sp16-off.json --out sp16.json
"""
import argparse
import json
import os
import subprocess
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "common"))
import spikelib as sl  # noqa: E402

SPIKE = "SP-16"


def app_url_for_profile(app_name, profile, runner, given_url=None):
    """4.6 rule 1: the app must be in the profile's workspace. `databricks apps get <name>
    --profile <p>` answers only for an app in that workspace; its `url` is the only URL we use.
    A --app-url that differs from it is refused (review finding 10)."""
    res = sl.run_cli(["apps", "get", app_name, "--profile", profile, "-o", "json"], runner=runner, timeout=120)
    if not res.ok:
        raise sl.SpikeError(f"databricks apps get {app_name} failed: " + sl.redact(res.stderr.strip())[:300])
    url = (res.json() or {}).get("url") or ""
    if not url:
        raise sl.SpikeError("databricks apps get returned no url (is the app deployed?)")
    sl.check_app_url(url)
    if given_url and given_url.rstrip("/") != url.rstrip("/"):
        raise sl.SpikeError("--app-url is not the URL of this app in the profile's workspace")
    return url.rstrip("/")


class AppApi:
    """Calls the app URL (4.6 rule 1: https://*.databricksapps.com only) with the CLI token."""

    def __init__(self, app_url, token, transport):
        sl.check_app_url(app_url)
        self.base = app_url.rstrip("/")
        self.token = token
        self.t = transport

    def invoke(self):
        h = {"Content-Type": "application/json", "User-Agent": "bottle-spike/0.1"}
        h.update(self.token.auth_header())
        body = json.dumps({"id": "bottle-sp16-probe", "input": "ping"}).encode()
        return self.t.request("POST", self.base + "/api/invocations", headers=h, body=body)


def mode_result(mode, resp, caller_kind):
    row = {"mode": mode, "status": resp.status, "elapsed_ms": resp.elapsed_ms,
           "caller_token": caller_kind}
    if resp.status == 200:
        try:
            info = resp.json()
        except ValueError:
            info = None
        if isinstance(info, dict) and "tokens" in info:
            row["app_saw"] = {"identity_header_names": info.get("identity_header_names", []),
                              "tokens": info.get("tokens", {})}
    else:
        row["error_shape"] = sl.error_shape(resp)
    return row


def known(row):
    """Is the token type and scope known for this mode?"""
    saw = row.get("app_saw")
    if not saw:
        return False, "no summary from the app"
    fwd = saw["tokens"].get("x-forwarded-access-token", {"present": False})
    if row["mode"] == "off":
        return True, "user token present" if fwd.get("present") else "no user token forwarded"
    if not fwd.get("present"):
        return False, "user authorization is on but no x-forwarded-access-token arrived"
    if fwd.get("format") == "jwt":
        if sl.scopes_of(fwd):
            return True, f"jwt typ={fwd.get('header_typ')} scope={sl.scopes_of(fwd)}"
        return False, "jwt without a scope claim"
    return False, f"token format {fwd.get('format')}: scope unknown"


def evaluate(rows):
    by = {r["mode"]: r for r in rows}
    out = {m: dict(zip(("known", "detail"), known(by[m]))) if m in by else {"known": False, "detail": "not run"}
           for m in ("on", "off")}
    passed = all(v["known"] for v in out.values())
    return {"passed": passed, "modes": out,
            "fallback": None if passed else "Origin-bound lease per app; v1 gating."}


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--profile", help="Databricks CLI profile (you choose it)")
    p.add_argument("--app-name", help="name of the deployed probe app in the profile's workspace")
    p.add_argument("--app-url", help="optional; must equal the url from `databricks apps get`")
    p.add_argument("--mode", choices=["on", "off"], help="user authorization state of the app now")
    p.add_argument("--merge", nargs=2, metavar="REPORT", help="combine the on and off reports")
    p.add_argument("--out", required=True)
    args = p.parse_args(argv)

    def go():
        if args.merge:
            rows = []
            for path in args.merge:
                with open(path, encoding="utf-8") as f:
                    rows.append(json.load(f)["result"])
            ev = evaluate(rows)
            sl.write_report(args.out, {"spike": SPIKE, "issue": 157, "results": rows, "evaluation": ev})
            sl.say(sl.verdict(SPIKE, ev["passed"], json.dumps(ev["modes"])))
            return 0 if ev["passed"] else 1
        if not (args.profile and args.app_name and args.mode):
            raise sl.SpikeError("--profile, --app-name and --mode are required (or use --merge)")
        url = app_url_for_profile(args.app_name, args.profile, subprocess.run, args.app_url)
        token = sl.get_token(args.profile, subprocess.run)
        api = AppApi(url, token, sl.HttpxTransport(timeout=60))
        row = mode_result(args.mode, api.invoke(), sl.token_kind(token.reveal()))
        ok, detail = known(row)
        sl.write_report(args.out, {"spike": SPIKE, "issue": 157, "result": row}, secrets=[token.reveal()])
        sl.say(f"  app saw headers: {row.get('app_saw', {}).get('identity_header_names')}")
        sl.say(sl.verdict(f"{SPIKE}[{args.mode}]", ok, detail))
        token.wipe()
        return 0 if ok else 1

    return sl.main_guard(go)


if __name__ == "__main__":
    sys.exit(main())
