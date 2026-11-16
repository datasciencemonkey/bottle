# SP-13 (#56): strict json_schema router on small models

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-13-56-strict-json_schema-router-on-small-models): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with httpx spikes/sp-13/run.py --profile <your-profile> --out sp13.json
```

Pass: valid JSON >= 99.5 % of 500 calls and intent accuracy >= 90 % of the 480 intent-scored items, for each model. The label policy is in golden.py.

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 20 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-13`
