# SP-16 (#157): token type reaching Databricks Apps

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-16-157-token-type-reaching-databricks-apps): prerequisites, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root:

```
uv run --no-project --with httpx spikes/sp-16/run.py --profile <your-profile> --app-url https://<app-url> --mode on --out sp16-on.json
```

Pass: token type and scope known with user authorization on and off.

The script prints one `SP-16 PASS: ...` or `SP-16 FAIL: ...` line and writes a JSON report to `--out`. The report has no token and no user content.

Time: about 25 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-16`
