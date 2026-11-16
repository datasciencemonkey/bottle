# SP-06 (#48): Unity Gateway audio MIME types and payload size

The full steps are in [docs/spikes/B-RUNBOOK.md](../../docs/spikes/B-RUNBOOK.md#sp-06-48-unity-gateway-audio-mime-types-and-payload-size): prerequisites, admin steps, expected output, fallback, `CHECK:` items and the docs used.

You choose the profile. The script never chooses one. It gets the token with `databricks auth token --profile <p>` and keeps it in memory only.

Run from the repository root, with `uv` and the Databricks CLI on the PATH:

```
uv run --no-project --with httpx --with soundfile --with numpy spikes/sp-06/run.py --profile <your-profile> --out sp06.json
```

Pass: formats known; the 30 s case of the size format (WAV, else FLAC) accepted; 3.5 MB accepted; 4.1 MB rejected cleanly (4xx with a JSON error). No accepted size format gives NOT PROVEN.

The script prints one result line (`PASS`, `FAIL`, `NOT PROVEN` or `NOT RUN`) and writes a JSON report to `--out`. The report has no token and no user content. Read it before you post it.

Time: about 10 min.

Offline tests: `uv run --no-project --with pytest pytest spikes/sp-06`
