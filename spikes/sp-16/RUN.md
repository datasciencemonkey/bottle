# SP-16 (#157): token type reaching Databricks Apps

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-16-157-token-type-reaching-databricks-apps): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with httpx spikes/sp-16/run.py --profile <your-profile> --app-name bottle-sp16 --mode on --out sp16-on.json
```

Pass: token type and scope known with user authorization on and off. The app URL comes from `databricks apps get` in the profile's workspace (4.6 rule 1).

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 25 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-16`
