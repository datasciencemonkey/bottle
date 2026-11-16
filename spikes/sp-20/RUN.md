# SP-20 (#187): UC Pages API, scopes, DRAFT grounding and MCP visibility (SP-20b)

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-20-187-uc-pages-api-scopes-draft-grounding-and-mcp-visibility-with-sp-20b): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with httpx spikes/sp-20/run.py --profile <your-profile> --domain-id <id> --out sp20.json
```

Pass: 200 with state DRAFT and an idempotent retry with request_id in the body (B3); the query form is recorded; Genie One uses the DRAFT for the owner only (manual step).

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 40 min in two parts of 20 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-20`
