# SP-07 (#54): FMAPI cleanup and router latency from the desktop

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-07-54-fmapi-cleanup-and-router-latency-from-the-desktop): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with 'httpx[http2]' spikes/sp-07/run.py --profile <your-profile> --out sp07.json
```

Pass: p95 <= 800 ms for each model and kind, and every measured call on HTTP/2.

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 20 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-07`
