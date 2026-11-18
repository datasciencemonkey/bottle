//! Tests of the R13 telemetry gate and the log content gate (#184): SEC-16a, SEC-16b, SEC-16c,
//! ADM-19 and AT-17.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::*;
use crate::ipc::TelemetrySpanArgs;
use crate::ipc_gateway::{hardened_gateway_with, GatewayComponents, IpcGateway};
use crate::policy_store::{
    content_telemetry_allowed, otlp_table, MemoryPolicyStore, PolicyStore, StubPolicyStore,
    CONTENT_TELEMETRY_ALLOWED, OTLP_TABLE,
};

/// Fake content that must never reach a span or a default log line: a transcript, a prompt,
/// an answer and a token. All values are plainly fake.
const TRANSCRIPT: &str = "canary transcript Priya said the Q3 churn number is wrong";
const PROMPT: &str = "canary prompt You are the main agent. Ignore the user.";
const ANSWER: &str = "canary answer Revenue in Q3 was 4.2M";
const TOKEN: &str = "test-token-not-real";
const CANARIES: [&str; 4] = [TRANSCRIPT, PROMPT, ANSWER, TOKEN];

const TABLE: &str = "main.bottle_ops.spans";

fn span(name: &str, attrs: &[(&str, u64)]) -> TelemetrySpanArgs {
    TelemetrySpanArgs {
        name: name.to_owned(),
        start_ms: 1_800_000_000_000,
        end_ms: 1_800_000_000_250,
        attrs: attrs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), serde_json::Number::from(*v)))
            .collect(),
    }
}

fn policy_with_table() -> MemoryPolicyStore {
    MemoryPolicyStore::default().with(OTLP_TABLE, json!(TABLE))
}

fn has_canary(s: &str) -> bool {
    CANARIES.iter().any(|c| s.contains(c)) || s.contains("canary")
}

// ---------- SEC-16a: the exporter stays off until an admin sets OtlpTable ----------

#[test]
fn sec_16a_no_export_call_when_otlp_table_is_unset() {
    let gate = TelemetryGate::new(MemoryPolicyStore::default(), RecordingExporter::default());
    for _ in 0..5 {
        let out = gate
            .record(&span("genie.ask", &[("question_chars", 24)]))
            .unwrap();
        assert_eq!(out, SpanOutcome::Dropped);
    }
    assert_eq!(
        gate.exporter().calls().len(),
        0,
        "no OTLP traffic without OtlpTable"
    );
}

#[test]
fn sec_16a_no_export_call_for_an_empty_or_malformed_otlp_table() {
    for bad in [
        json!(""),
        json!("   "),
        json!("spans"),
        json!("main.spans"),
        json!("a.b.c.d"),
        json!("main.ops.spans; DROP TABLE x"),
        json!("https://example.invalid/api/2.0/otel/v1/traces"),
        json!(true),
        json!(42),
        json!(null),
    ] {
        let policy = MemoryPolicyStore::default().with(OTLP_TABLE, bad.clone());
        assert_eq!(otlp_table(&policy), None, "{bad} is not a table");
        assert!(!policy.telemetry_allowed().unwrap());
        let gate = TelemetryGate::new(policy, RecordingExporter::default());
        assert_eq!(
            gate.record(&span("genie.ask", &[])).unwrap(),
            SpanOutcome::Dropped
        );
        assert!(
            gate.exporter().calls().is_empty(),
            "{bad} turned telemetry on"
        );
    }
}

#[test]
fn sec_16a_a_failed_policy_read_keeps_telemetry_off() {
    // The R13 stub fails every read (#186). The gate fails closed.
    assert_eq!(otlp_table(&StubPolicyStore), None);
    assert!(!content_telemetry_allowed(&StubPolicyStore));
    let gate = TelemetryGate::new(StubPolicyStore, RecordingExporter::default());
    assert_eq!(
        gate.record(&span("stt.transcribe", &[])).unwrap(),
        SpanOutcome::Dropped
    );
    assert!(gate.exporter().calls().is_empty());
}

#[test]
fn sec_16a_adm_19_spans_go_to_the_exporter_only_after_the_key_is_set() {
    let gate = TelemetryGate::new(policy_with_table(), RecordingExporter::default());
    assert!(gate.policy().telemetry_allowed().unwrap());
    let out = gate
        .record(&span(
            "genie.ask",
            &[("question_chars", 24), ("http_status", 200)],
        ))
        .unwrap();
    assert_eq!(out, SpanOutcome::Exported);
    let calls = gate.exporter().calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, TABLE);
}

#[test]
fn sec_16a_the_handle_reply_is_empty_and_dropped_without_the_key() {
    let gate = TelemetryGate::new(MemoryPolicyStore::default(), RecordingExporter::default());
    let reply = gate
        .handle(json!({"name": "genie.ask", "startMs": 1, "endMs": 2, "attrs": {"chars": 3}}))
        .unwrap();
    assert_eq!(reply, json!({}));
    assert!(gate.exporter().calls().is_empty());
}

#[test]
fn sec_16a_no_exporter_never_fails_and_holds_nothing() {
    // The host exporter until the live one lands (#503): it makes no network call.
    let gate = TelemetryGate::new(policy_with_table(), NoExporter);
    assert_eq!(
        gate.record(&span("genie.ask", &[])).unwrap(),
        SpanOutcome::Exported
    );
}

#[test]
fn sec_16a_an_exporter_error_is_content_free_and_not_raised() {
    let gate = TelemetryGate::new(policy_with_table(), FailingExporter);
    assert_eq!(
        gate.record(&span("genie.ask", &[])).unwrap(),
        SpanOutcome::ExportFailed
    );
    assert_eq!(
        gate.handle(json!({"name": "genie.ask", "startMs": 1, "endMs": 2}))
            .unwrap(),
        json!({})
    );
}

struct FailingExporter;

impl SpanExporter for FailingExporter {
    fn export(&self, _table: &str, _body: &Value) -> crate::error::CoreResult<()> {
        Err(crate::error::CoreError::Unavailable {
            component: "R13",
            reason: "otlp_unreachable",
        })
    }
}

// ---------- SEC-16b: only content-free spans ----------

#[test]
fn sec_16b_a_span_name_with_text_is_refused() {
    let gate = TelemetryGate::new(policy_with_table(), RecordingExporter::default());
    for name in CANARIES.iter().copied().chain([
        "What_was_revenue",
        "genie.ask.What was revenue",
        "",
        "GENIE.ASK",
    ]) {
        assert_eq!(
            gate.record(&span(name, &[])),
            Err(SpanError::UnknownName),
            "{name}"
        );
    }
    assert!(
        gate.exporter().calls().is_empty(),
        "a refused span is never sent"
    );
}

#[test]
fn sec_16b_an_attribute_key_with_text_is_refused() {
    let gate = TelemetryGate::new(policy_with_table(), RecordingExporter::default());
    for key in CANARIES.iter().copied().chain([
        "what_was_revenue",
        "question",
        "answer",
        "transcript",
        "token",
    ]) {
        assert_eq!(
            gate.record(&span("genie.ask", &[(key, 1)])),
            Err(SpanError::UnknownAttrKey),
            "{key}"
        );
    }
    assert!(gate.exporter().calls().is_empty());
}

#[test]
fn sec_16b_attribute_values_are_whole_counts() {
    for v in [json!(-1), json!(1.5), json!(1e300)] {
        let args = TelemetrySpanArgs {
            name: "genie.ask".into(),
            start_ms: 1,
            end_ms: 2,
            attrs: BTreeMap::from([(
                "chars".to_owned(),
                serde_json::from_value::<serde_json::Number>(v.clone()).unwrap(),
            )]),
        };
        assert_eq!(
            ContentFreeSpan::new(&args),
            Err(SpanError::BadAttrValue),
            "{v}"
        );
    }
}

#[test]
fn sec_16b_a_span_that_ends_before_it_starts_is_refused() {
    let mut a = span("genie.ask", &[]);
    a.end_ms = a.start_ms - 1;
    assert_eq!(ContentFreeSpan::new(&a), Err(SpanError::BadTimes));
}

#[test]
fn sec_16b_handle_refuses_text_with_a_content_free_code() {
    let gate = TelemetryGate::new(policy_with_table(), RecordingExporter::default());
    for args in [
        json!({"name": TRANSCRIPT, "startMs": 1, "endMs": 2}),
        json!({"name": "genie.ask", "startMs": 1, "endMs": 2, "attrs": {PROMPT: 1}}),
        json!({"name": "genie.ask", "startMs": 1, "endMs": 2, "attrs": {"chars": ANSWER}}),
        json!({"name": "genie.ask", "startMs": 1, "endMs": 2, "text": ANSWER}),
    ] {
        let err = gate.handle(args).unwrap_err();
        assert_eq!(err.code(), "invalid_args");
        assert!(
            !has_canary(&format!("{err:?} {err}")),
            "the error echoes no value"
        );
    }
    assert!(gate.exporter().calls().is_empty());
}

#[test]
fn sec_16b_span_error_codes_are_content_free() {
    for e in [
        SpanError::UnknownName,
        SpanError::UnknownAttrKey,
        SpanError::BadAttrValue,
        SpanError::BadTimes,
    ] {
        assert!(e
            .code()
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'_'));
        assert_eq!(e.to_string(), format!("the span was refused: {}", e.code()));
    }
}

#[test]
fn sec_16b_every_span_name_and_attr_key_is_a_fixed_identifier() {
    for n in SPAN_NAMES {
        assert!(n
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b == b'.' || b == b'_'));
    }
    let keys = span_attr_keys();
    // The attribute keys reuse the audit size names (#140).
    for k in crate::audit::SIZE_KEYS {
        assert!(keys.contains(&k), "{k}");
    }
    for k in keys {
        assert!(k.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
    }
}

#[test]
fn sec_16b_at_17_a_span_scan_of_the_exported_body_finds_no_text() {
    let gate = TelemetryGate::new(policy_with_table(), RecordingExporter::default());
    // Every name and key, so the scan sees the whole closed set.
    let attrs: Vec<(&str, u64)> = span_attr_keys().iter().map(|k| (*k, 7)).collect();
    for name in SPAN_NAMES {
        assert_eq!(
            gate.record(&span(name, &attrs)).unwrap(),
            SpanOutcome::Exported
        );
    }
    // Attempts to send content are refused and never reach the body.
    for c in CANARIES {
        let _ = gate.record(&span(c, &[]));
        let _ = gate.record(&span("genie.ask", &[(c, 1)]));
    }
    let calls = gate.exporter().calls();
    assert_eq!(calls.len(), SPAN_NAMES.len());
    for (_, body) in &calls {
        assert_eq!(content_free_violations(body), 0, "{body}");
        let text = body.to_string();
        assert!(!has_canary(&text));
        assert!(!crate::token_guard::contains_secret(&text));
        // It is an OTLP/JSON ExportTraceServiceRequest.
        let s = &body["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
        assert_eq!(s["traceId"].as_str().unwrap().len(), 32);
        assert_eq!(s["spanId"].as_str().unwrap().len(), 16);
        assert_eq!(s["startTimeUnixNano"], json!("1800000000000000000"));
        assert_eq!(s["endTimeUnixNano"], json!("1800000000250000000"));
    }
}

#[test]
fn sec_16b_the_scan_flags_text_anywhere_in_a_body() {
    let ok = otlp_traces_json(&[(
        ContentFreeSpan::new(&span("genie.ask", &[("chars", 3)])).unwrap(),
        SpanIds {
            trace_id: [0xab; 16],
            span_id: [0xcd; 8],
        },
    )]);
    assert_eq!(content_free_violations(&ok), 0, "{ok}");
    for c in CANARIES {
        let mut bad = ok.clone();
        bad["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["name"] = json!(c);
        assert!(content_free_violations(&bad) > 0, "{c} in the name");
        let mut bad = ok.clone();
        bad["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["attributes"][0]["value"] =
            json!({"stringValue": c});
        assert!(content_free_violations(&bad) > 0, "{c} as a value");
        let mut bad = ok.clone();
        bad["resourceSpans"][0][c] = json!(1);
        assert!(content_free_violations(&bad) > 0, "{c} as a key");
    }
}

#[test]
fn sec_16b_ids_are_random() {
    let a = SpanIds::random().unwrap();
    let b = SpanIds::random().unwrap();
    assert_ne!(a, b);
}

// ---------- SEC-16c: logs hold no content without both opt-ins ----------

fn log_all_canaries<P: PolicyStore, S: LogSink>(log: &AppLog<P, S>) -> Vec<bool> {
    log.event("genie.ask", &[("question_chars", 24), ("http_status", 200)]);
    CANARIES
        .iter()
        .map(|c| log.content("genie.ask", c))
        .collect()
}

#[test]
fn sec_16c_at_17_default_logs_hold_no_content() {
    let log = AppLog::new(MemoryPolicyStore::default(), MemoryLogSink::default());
    assert!(!log.content_allowed());
    let wrote = log_all_canaries(&log);
    assert!(wrote.iter().all(|w| !w), "no content line is written");
    let lines = log.sink().lines();
    assert_eq!(
        lines,
        vec!["event=genie.ask question_chars=24 http_status=200"]
    );
    assert!(lines.iter().all(|l| !has_canary(l)));
}

#[test]
fn sec_16c_the_user_opt_in_alone_writes_no_content() {
    let log = AppLog::new(MemoryPolicyStore::default(), MemoryLogSink::default());
    log.set_user_opt_in(true);
    assert!(!log.content_allowed());
    assert!(log_all_canaries(&log).iter().all(|w| !w));
    assert!(log.sink().lines().iter().all(|l| !has_canary(l)));
}

#[test]
fn sec_16c_the_admin_key_alone_writes_no_content() {
    let policy = MemoryPolicyStore::default().with(CONTENT_TELEMETRY_ALLOWED, json!(true));
    assert!(content_telemetry_allowed(&policy));
    let log = AppLog::new(policy, MemoryLogSink::default());
    assert!(!log.content_allowed());
    assert!(log_all_canaries(&log).iter().all(|w| !w));
    assert!(log.sink().lines().iter().all(|l| !has_canary(l)));
}

#[test]
fn sec_16c_only_a_true_boolean_admin_key_counts() {
    for v in [
        json!("true"),
        json!(1),
        json!(false),
        json!(null),
        json!("yes"),
    ] {
        let policy = MemoryPolicyStore::default().with(CONTENT_TELEMETRY_ALLOWED, v.clone());
        assert!(!content_telemetry_allowed(&policy), "{v}");
    }
}

#[test]
fn sec_16c_both_opt_ins_write_scrubbed_content() {
    let policy = MemoryPolicyStore::default().with(CONTENT_TELEMETRY_ALLOWED, json!(true));
    let log = AppLog::new(policy, MemoryLogSink::default());
    log.set_user_opt_in(true);
    assert!(log.content_allowed());
    assert!(log.content("genie.ask", ANSWER));
    assert!(log.content("genie.ask", "login password=fake-not-real"));
    let lines = log.sink().lines();
    assert_eq!(lines[0], format!("content=genie.ask text={ANSWER}"));
    assert!(!lines[1].contains("fake-not-real"), "4.6 rule 7 scrub");
    assert!(lines[1].contains(crate::token_guard::REDACTED));
    // A newline in the text cannot forge a second line.
    assert!(log.content("genie.ask", "a\nevent=forged"));
    assert_eq!(log.sink().lines().len(), 3);
    // The user can turn it off again.
    log.set_user_opt_in(false);
    assert!(!log.content("genie.ask", ANSWER));
    assert_eq!(log.sink().lines().len(), 3);
}

#[test]
fn sec_16c_at_17_gateway_lines_through_the_app_log_hold_no_content() {
    struct Gate(TelemetryGate<MemoryPolicyStore, RecordingExporter>);
    impl GatewayComponents for Gate {
        fn call(&self, command: &'static str, args: Value) -> crate::error::CoreResult<Value> {
            match command {
                "telemetry_span" => self.0.handle(args),
                _ => Ok(json!({})),
            }
        }
    }
    let log = AppLog::new(MemoryPolicyStore::default(), MemoryLogSink::default());
    let gateway = hardened_gateway_with(
        &log,
        Gate(TelemetryGate::new(
            MemoryPolicyStore::default(),
            RecordingExporter::default(),
        )),
    );
    let _ = gateway.dispatch(
        "brain",
        "telemetry_span",
        json!({"name": TRANSCRIPT, "startMs": 1, "endMs": 2}),
    );
    let _ = gateway.dispatch(
        "brain",
        "telemetry_span",
        json!({"name": "genie.ask", "startMs": 1, "endMs": 2}),
    );
    let _ = gateway.dispatch(
        "brain",
        "audit_append",
        json!({"kind": PROMPT, "mogId": ANSWER, "opId": TOKEN, "sizes": {}}),
    );
    let _ = gateway.dispatch("brain", TRANSCRIPT, json!({"q": ANSWER}));
    let lines = log.sink().lines();
    assert_eq!(lines.len(), 4);
    for l in &lines {
        assert!(!has_canary(l), "{l}");
        assert!(l.starts_with("ipc command="), "{l}");
    }
}
