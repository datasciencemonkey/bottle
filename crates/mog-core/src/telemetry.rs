//! R13 telemetry gate and the log content gate (#184, D42, ADR 0079).
//!
//! - SEC-16a: [`TelemetryGate`] calls its [`SpanExporter`] only when the MDM key `OtlpTable`
//!   holds a valid table name. Otherwise it drops the span and makes no network call.
//! - SEC-16b: a span is a [`ContentFreeSpan`]: a name from [`SPAN_NAMES`], times, and whole
//!   counts under keys from [`span_attr_keys`]. Any other name, key or value is refused, and
//!   [`content_free_violations`] scans the OTLP body once more before export.
//! - SEC-16c: [`AppLog`] writes content-free event lines. It writes content only when the admin
//!   key `ContentTelemetryAllowed` and the user opt-in are both on, and it scrubs that content
//!   with the 4.6 rule-7 patterns.
//!
//! TODO(#502): the host routes `telemetry_span` to [`TelemetryGate::handle`] and sets the user
//! opt-in from the `app.log.contentOptIn` app setting.
//! TODO(#503): the live OTLP exporter over R7 HttpTransport. Until then the host uses
//! [`NoExporter`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::audit::SIZE_KEYS;
use crate::error::{CoreError, CoreResult};
use crate::ipc::TelemetrySpanArgs;
use crate::ipc_gateway::GatewayLog;
use crate::policy_store::{content_telemetry_allowed, otlp_table, PolicyStore};
use crate::token_guard::scrub;

const ID: &str = "R13";

/// The span names the app may send. A closed set, so a name can never carry text.
pub const SPAN_NAMES: [&str; 18] = [
    "agent.tool",
    "agent.turn",
    "app.start",
    "card.render",
    "cleanup.run",
    "context.assemble",
    "genie.ask",
    "genie.poll",
    "insert.text",
    "memory.recall",
    "memory.write",
    "page.propose",
    "ptt.capture",
    "ptt.preview",
    "router.run",
    "session.capture",
    "stt.transcribe",
    "wipe.run",
];

/// The attribute keys a span may hold besides the audit size names ([`SIZE_KEYS`], #140).
pub const EXTRA_ATTR_KEYS: [&str; 5] = ["attempt", "http_status", "ok", "queue_ms", "retries"];

/// The largest attribute value: the largest integer that JSON numbers hold exactly.
pub const MAX_ATTR_VALUE: u64 = (1 << 53) - 1;

/// Every attribute key a span may hold: [`SIZE_KEYS`] and [`EXTRA_ATTR_KEYS`].
pub fn span_attr_keys() -> Vec<&'static str> {
    SIZE_KEYS
        .iter()
        .chain(EXTRA_ATTR_KEYS.iter())
        .copied()
        .collect()
}

fn known(set: &[&'static str], s: &str) -> Option<&'static str> {
    set.iter().find(|x| **x == s).copied()
}

fn attr_key(s: &str) -> Option<&'static str> {
    known(&SIZE_KEYS, s).or_else(|| known(&EXTRA_ATTR_KEYS, s))
}

/// Why a span was refused. No variant carries the refused value (SEC-16b, ARC-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanError {
    UnknownName,
    UnknownAttrKey,
    BadAttrValue,
    BadTimes,
}

impl SpanError {
    /// A stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            SpanError::UnknownName => "unknown_name",
            SpanError::UnknownAttrKey => "unknown_attr_key",
            SpanError::BadAttrValue => "bad_attr_value",
            SpanError::BadTimes => "bad_times",
        }
    }
}

impl std::fmt::Display for SpanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the span was refused: {}", self.code())
    }
}

impl std::error::Error for SpanError {}

/// One checked span. Every string in it comes from a closed set (SEC-16b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentFreeSpan {
    pub name: &'static str,
    pub start_ms: u64,
    pub end_ms: u64,
    pub attrs: BTreeMap<&'static str, u64>,
}

impl ContentFreeSpan {
    /// Check the `telemetry_span` arguments. Every check runs before any export.
    pub fn new(args: &TelemetrySpanArgs) -> Result<Self, SpanError> {
        let name = known(&SPAN_NAMES, &args.name).ok_or(SpanError::UnknownName)?;
        if args.end_ms < args.start_ms || nanos(args.end_ms).is_none() {
            return Err(SpanError::BadTimes);
        }
        let attrs = args
            .attrs
            .iter()
            .map(|(k, v)| {
                let k = attr_key(k).ok_or(SpanError::UnknownAttrKey)?;
                let v = v
                    .as_u64()
                    .filter(|v| *v <= MAX_ATTR_VALUE)
                    .ok_or(SpanError::BadAttrValue)?;
                Ok((k, v))
            })
            .collect::<Result<_, _>>()?;
        Ok(ContentFreeSpan {
            name,
            start_ms: args.start_ms,
            end_ms: args.end_ms,
            attrs,
        })
    }
}

fn nanos(ms: u64) -> Option<u64> {
    ms.checked_mul(1_000_000)
}

/// The OTLP trace and span IDs of one span. Each brain span is its own trace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpanIds {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
}

impl SpanIds {
    /// Random IDs from the OS.
    pub fn random() -> CoreResult<Self> {
        let mut b = [0u8; 24];
        getrandom::fill(&mut b).map_err(|_| CoreError::Unavailable {
            component: ID,
            reason: "no_random",
        })?;
        let mut ids = SpanIds {
            trace_id: [0; 16],
            span_id: [0; 8],
        };
        ids.trace_id.copy_from_slice(&b[..16]);
        ids.span_id.copy_from_slice(&b[16..]);
        Ok(ids)
    }
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .fold(String::with_capacity(b.len() * 2), |mut s, x| {
            let _ = write!(s, "{x:02x}");
            s
        })
}

const SERVICE_KEY: &str = "service.name";
const SERVICE: &str = "bottle";

/// The OTLP/JSON `ExportTraceServiceRequest` body of the spans. OTLP/JSON writes 64-bit
/// integers as decimal strings.
pub fn otlp_traces_json(spans: &[(ContentFreeSpan, SpanIds)]) -> Value {
    let spans: Vec<Value> = spans
        .iter()
        .map(|(s, ids)| {
            let attributes: Vec<Value> = s
                .attrs
                .iter()
                .map(|(k, v)| json!({"key": k, "value": {"intValue": v.to_string()}}))
                .collect();
            json!({
                "traceId": hex(&ids.trace_id),
                "spanId": hex(&ids.span_id),
                "name": s.name,
                "kind": 1,
                "startTimeUnixNano": nanos(s.start_ms).unwrap_or(0).to_string(),
                "endTimeUnixNano": nanos(s.end_ms).unwrap_or(0).to_string(),
                "attributes": attributes,
            })
        })
        .collect();
    json!({"resourceSpans": [{
        "resource": {"attributes": [{"key": SERVICE_KEY, "value": {"stringValue": SERVICE}}]},
        "scopeSpans": [{"scope": {"name": SERVICE}, "spans": spans}],
    }]})
}

/// The object keys of an OTLP body from [`otlp_traces_json`].
const OTLP_KEYS: [&str; 16] = [
    "resourceSpans",
    "resource",
    "attributes",
    "key",
    "value",
    "stringValue",
    "intValue",
    "scopeSpans",
    "scope",
    "name",
    "spans",
    "traceId",
    "spanId",
    "kind",
    "startTimeUnixNano",
    "endTimeUnixNano",
];

fn content_free_string(s: &str) -> bool {
    let is_hex = |n: usize| {
        s.len() == n
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    };
    known(&SPAN_NAMES, s).is_some()
        || attr_key(s).is_some()
        || s == SERVICE
        || s == SERVICE_KEY
        || is_hex(32)
        || is_hex(16)
        || (!s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
}

/// The span scan (SEC-16b): the count of object keys and strings in `body` that are not a
/// fixed OTLP key, a closed-set name, an ID or a number. Zero means the body is content-free.
pub fn content_free_violations(body: &Value) -> usize {
    match body {
        Value::String(s) => usize::from(!content_free_string(s)),
        Value::Array(a) => a.iter().map(content_free_violations).sum(),
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| {
                usize::from(!OTLP_KEYS.contains(&k.as_str())) + content_free_violations(v)
            })
            .sum(),
        Value::Null | Value::Bool(_) | Value::Number(_) => 0,
    }
}

/// Where spans go once telemetry is on. The live exporter is #503.
pub trait SpanExporter: Send + Sync {
    /// Send one OTLP/JSON body for the Unity Catalog table `table`.
    fn export(&self, table: &str, body: &Value) -> CoreResult<()>;
}

/// The host exporter until #503: it drops every body and makes no network call.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoExporter;

impl SpanExporter for NoExporter {
    fn export(&self, _table: &str, _body: &Value) -> CoreResult<()> {
        Ok(())
    }
}

/// The test double: it keeps each call.
#[derive(Debug, Default)]
pub struct RecordingExporter {
    calls: Mutex<Vec<(String, Value)>>,
}

impl RecordingExporter {
    /// The `(table, body)` of each call, in order.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl SpanExporter for RecordingExporter {
    fn export(&self, table: &str, body: &Value) -> CoreResult<()> {
        self.calls
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((table.to_owned(), body.clone()));
        Ok(())
    }
}

/// What happened to a checked span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanOutcome {
    /// Telemetry is off (no valid `OtlpTable`): no exporter call (SEC-16a).
    Dropped,
    /// The exporter took the body.
    Exported,
    /// The exporter failed. The brain still gets `{}`: telemetry never fails a user action.
    ExportFailed,
}

/// The R13 telemetry gate in front of an exporter (SEC-16a, SEC-16b).
#[derive(Debug)]
pub struct TelemetryGate<P, E> {
    policy: P,
    exporter: E,
}

impl<P: PolicyStore, E: SpanExporter> TelemetryGate<P, E> {
    pub fn new(policy: P, exporter: E) -> Self {
        TelemetryGate { policy, exporter }
    }

    pub fn policy(&self) -> &P {
        &self.policy
    }

    pub fn exporter(&self) -> &E {
        &self.exporter
    }

    /// Check one span, then export it only if `OtlpTable` is set. A refused span is never sent.
    pub fn record(&self, args: &TelemetrySpanArgs) -> Result<SpanOutcome, SpanError> {
        let span = ContentFreeSpan::new(args)?;
        // The policy is read for each span, so an admin change applies at once.
        let Some(table) = otlp_table(&self.policy) else {
            return Ok(SpanOutcome::Dropped);
        };
        let Ok(ids) = SpanIds::random() else {
            return Ok(SpanOutcome::ExportFailed);
        };
        let body = otlp_traces_json(&[(span, ids)]);
        // A second guard: the body is built from closed sets, so this does not happen.
        if content_free_violations(&body) != 0 {
            return Ok(SpanOutcome::Dropped);
        }
        Ok(match self.exporter.export(&table, &body) {
            Ok(()) => SpanOutcome::Exported,
            Err(_) => SpanOutcome::ExportFailed,
        })
    }

    /// The `telemetry_span` command: `{}`, or `invalid_args` for a span with text (4.5 row).
    pub fn handle(&self, args: Value) -> CoreResult<Value> {
        let refused = CoreError::IpcRefused {
            code: "invalid_args",
        };
        let args: TelemetrySpanArgs = serde_json::from_value(args).map_err(|_| refused.clone())?;
        self.record(&args).map_err(|_| refused)?;
        Ok(json!({}))
    }
}

/// Where [`AppLog`] writes its lines.
pub trait LogSink: Send + Sync {
    /// Keep one line. The line has no line break.
    fn write(&self, line: &str);
}

/// The in-memory sink: the test double.
#[derive(Debug, Default)]
pub struct MemoryLogSink {
    lines: Mutex<Vec<String>>,
}

impl MemoryLogSink {
    pub fn lines(&self) -> Vec<String> {
        self.lines.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl LogSink for MemoryLogSink {
    fn write(&self, line: &str) {
        self.lines
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(line.to_owned());
    }
}

/// The app log with the SEC-16c content gate. The user opt-in starts off.
#[derive(Debug)]
pub struct AppLog<P, S> {
    policy: P,
    sink: S,
    user_opt_in: AtomicBool,
}

impl<P: PolicyStore, S: LogSink> AppLog<P, S> {
    pub fn new(policy: P, sink: S) -> Self {
        AppLog {
            policy,
            sink,
            user_opt_in: AtomicBool::new(false),
        }
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    /// The user half of the content opt-in. TODO(#502): from the `app.log.contentOptIn` setting.
    pub fn set_user_opt_in(&self, on: bool) {
        self.user_opt_in.store(on, Ordering::SeqCst);
    }

    /// True only with the user opt-in and `ContentTelemetryAllowed` (SEC-16c).
    pub fn content_allowed(&self) -> bool {
        self.user_opt_in.load(Ordering::SeqCst) && content_telemetry_allowed(&self.policy)
    }

    /// A content-free line: a fixed name and whole counts under fixed keys.
    pub fn event(&self, name: &'static str, fields: &[(&'static str, u64)]) {
        let mut line = format!("event={name}");
        for (k, v) in fields {
            let _ = write!(line, " {k}={v}");
        }
        self.sink.write(&line);
    }

    /// A content line, written only if [`AppLog::content_allowed`]. The text is put on one line
    /// and scrubbed with the 4.6 rule-7 patterns. Returns true if the line was written.
    pub fn content(&self, name: &'static str, text: &str) -> bool {
        if !self.content_allowed() {
            return false;
        }
        let one_line: String = text
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        self.sink
            .write(&format!("content={name} text={}", scrub(&one_line)));
        true
    }
}

/// The R12 gateway lines go to the app log. They are content-free (SEC-03); the scrub is a
/// second guard.
impl<P: PolicyStore, S: LogSink> GatewayLog for AppLog<P, S> {
    fn record(&self, line: &str) {
        let one_line: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        self.sink.write(&scrub(&one_line));
    }
}

impl<P: PolicyStore, S: LogSink> GatewayLog for &AppLog<P, S> {
    fn record(&self, line: &str) {
        (**self).record(line);
    }
}

#[cfg(test)]
mod tests;
