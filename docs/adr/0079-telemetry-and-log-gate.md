# ADR 0079: The telemetry gate and the log content gate

- Status: Accepted
- Date: 2026-11-17
- Issue: #184 (SEC-16a, SEC-16b, SEC-16c, ADM-19, AT-17; D42)
- Owner: impl-184 for security-lead
- Contract change: #502. Live exporter: #503 (needs:human).

## Context

D42 says telemetry is off until an admin sets `OtlpTable`. Spans are content-free. Content logging needs a user opt-in and an admin opt-in. Appendix C has `OtlpTable` (unset) and `ContentTelemetryAllowed` (false). Appendix B sends spans to `/api/2.0/otel/v1/traces` on the home workspace. The 4.5 row says `telemetry_span` returns `{}` and drops the span when `OtlpTable` is unset.

On main:

- R13 PolicyStore is a stub. The MDM reader is #186.
- `TelemetrySpanArgs` has a free `name` string and number attributes under free keys.
- `telemetry_span` is not routed. The R12 gateway log is `NoLog`. No production code uses `println!`, `log::`, `tracing::` or `console`.
- There is no app setting for the user opt-in.

Contract files belong to contract-lead.

## Decisions

1. **Fail closed.** `policy_store::otlp_table` returns a table only for a string `catalog.schema.table`. Each part has 1-255 ASCII letters, digits or `_`. Any other value, or a read error, means "off". `ContentTelemetryAllowed` counts only as a boolean `true`. `PolicyStore::telemetry_allowed` is now a default method over `get`.
2. **Closed sets for spans.** A span name comes from 18 fixed names (`SPAN_NAMES`). An attribute key comes from the audit size names of #140 plus `attempt`, `http_status`, `ok`, `queue_ms` and `retries`. A value is a whole count up to 2^53 - 1. The free contract fields stay as they are. The gate refuses all other values with `invalid_args`, and the error holds no value. The brain client (`src/brain/telemetry.ts`) checks the same sets.
3. **The gate reads the policy for each span.** It calls the exporter only when `OtlpTable` is valid. Otherwise it makes no exporter call, so there is no network traffic (SEC-16a).
4. **A second scan before export.** `otlp_traces_json` builds an OTLP/JSON `ExportTraceServiceRequest`. Each span is its own trace with random IDs. `content_free_violations` counts each key or string that is not a fixed OTLP key, a closed-set name, a hex ID or a decimal number. The gate exports only a body with zero violations (SEC-16b).
5. **Exporter interface.** `SpanExporter` is the seam. `NoExporter` sends nothing and is the host exporter until #503. `RecordingExporter` is the test double. An export error never fails a user action: the brain still gets `{}`.
6. **One log gate.** `AppLog` (Rust) and `createBrainLog` (TS) write two kinds of lines:
   - event lines with a fixed name and whole counts;
   - content lines, only when the user opt-in and `ContentTelemetryAllowed` are both on.
   A content line is put on one line. The Rust log also applies the 4.6 rule-7 scrub. The brain holds no token (W-01b, 4.6 rule 9). `AppLog` implements `GatewayLog`, so the R12 lines can go to it.
7. **Scans keep it so.** A Rust test scans the production code of `mog-core`, `mog-core-ffi` and `src-tauri`. It fails on `println!`, `eprintln!`, `print!`, `eprint!`, `dbg!`, `log::`, `tracing::` or `std::io::stderr`. A TS test fails on `console` outside `src/brain/log.ts`. `testkit.rs` is exempt because it is test support.

## Consequences

- #502 routes `telemetry_span` to `TelemetryGate::handle` and adds the `app.log.contentOptIn` setting. Until then the brain call returns `not_implemented`, and no span leaves the device.
- #503 adds the live exporter over R7 HttpTransport and the live check with an admin.
- A new span name or attribute key needs a change in both closed sets (Rust and TS) and a test.
- The 7-day local log file (Appendix B) is not part of this ADR. A file sink implements `LogSink`.
