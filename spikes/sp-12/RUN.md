# SP-12 (#191): Lakebase cold start and vectors

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-12-191-lakebase-cold-start-and-vectors): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with 'psycopg[binary]' spikes/sp-12/run.py --profile <your-profile> --create-only --out sp12-create.json
# admin step: turn on Lakebase Search for the new bottle-sp12-* project
uv run --no-project --with 'psycopg[binary]' spikes/sp-12/run.py --profile <your-profile> --project-from sp12-create.json --delete-project --out sp12.json
```

Pass: warm hybrid p95 <= 300 ms; cold wake <= 1 s; 1024-d lakebase_ann builds (vector and halfvec); verify-full chain verifies. Lakebase Search off gives NOT RUN. The script asks before the resize, the irreversible extension and the schema drop (or use --yes).

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 30 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-12`
