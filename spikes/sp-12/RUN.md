# SP-12 (#191): Lakebase cold start and vectors

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-12-191-lakebase-cold-start-and-vectors): prerequisites, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root:

```
uv run --no-project --with 'psycopg[binary]' spikes/sp-12/run.py --profile <your-profile> --out sp12.json --delete-project
```

Pass: warm hybrid p95 <= 300 ms; cold wake <= 1 s; 1024-d lakebase_ann builds (vector and halfvec); verify-full chain verifies.

The script prints one `SP-12 PASS: ...` or `SP-12 FAIL: ...` line and writes a JSON report to `--out`. The report has no token and no user content.

Time: about 30 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-12`
