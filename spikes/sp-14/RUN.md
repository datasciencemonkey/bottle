# SP-14 (#167): fine-grained CLI OAuth scopes

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-14-167-fine-grained-cli-oauth-scopes): prerequisites, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root:

```
databricks auth login --host https://<host> --profile bottle-sp14-w19 --scopes ai-gateway,genie,knowledge,offline_access
uv run --no-project --with httpx spikes/sp-14/run.py --profile bottle-sp14-w19 --workspace-id <id> --domain-id <id> --out sp14-w19.json
```

Pass: all MVP calls (MCP initialize, tools/list, model call, knowledge pages) succeed with the narrow profile.

The script prints one `SP-14 PASS: ...` or `SP-14 FAIL: ...` line and writes a JSON report to `--out`. The report has no token and no user content.

Time: about 20 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-14`
