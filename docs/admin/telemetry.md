# Telemetry and logs

Bottle telemetry is off by default (SEC-16a, D42, ADR 0079).

## MDM keys

| Key | Default | Effect |
|---|---|---|
| `OtlpTable` | unset | The Unity Catalog table for spans, as `catalog.schema.table`. Telemetry starts only after you set it (ADM-19). |
| `ContentTelemetryAllowed` | `false` | The admin half of the content-log opt-in. Content goes in the log only when this key is `true` and the user also opts in (SEC-16c). |

The macOS domain is `com.databricks.bottle`. The Windows key is `HKLM\Software\Policies\Databricks\Bottle`.

## What a span holds

- A fixed name, for example `genie.ask` or `stt.transcribe`.
- The start and end time.
- Whole counts, for example `question_chars`, `http_status` or `attempt`.

A span holds no transcript, prompt, question, answer, memory text, token or URL. Bottle refuses any other name or key. Bottle scans each body before it sends it (SEC-16b).

An `OtlpTable` value that is not a three-part name keeps telemetry off. Bottle sends spans to `/api/2.0/otel/v1/traces` on the home workspace (Appendix B).

## Logs

By default, log lines hold only fixed names and counts. Bottle writes content to the log only when both opt-ins are on. Even then, the core log removes secret patterns from the content, and the brain log drops a line that has one.

The live export is not built yet (#503). The 7-day log file and the telemetry-on notice are #517.
