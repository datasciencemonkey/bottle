# SP-13 (#56): strict json_schema router on small models

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-13-56-strict-json_schema-router-on-small-models): prerequisites, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root:

```
uv run --no-project --with httpx spikes/sp-13/run.py --profile <your-profile> --out sp13.json
```

Pass: valid JSON >= 99.5 % and intent accuracy >= 90 % for each model, on the 500-utterance golden set.

The script prints one `SP-13 PASS: ...` or `SP-13 FAIL: ...` line and writes a JSON report to `--out`. The report has no token and no user content.

Time: about 20 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-13`
