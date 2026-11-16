# SP-11 (#121): Genie One API list and delete of MCP threads

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-11-121-genie-one-api-list-and-delete-of-mcp-threads): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with httpx spikes/sp-11/run.py --profile <your-profile> --out sp11.json
```

Pass: the MCP thread lists and deletes, and is gone after the delete.

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 10 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-11`
