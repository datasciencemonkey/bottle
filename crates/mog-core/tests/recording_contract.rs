//! The call recording contract of ADR 0103: the settings defaults, the Rust knob gate on
//! `recording_start`, the error-kind codes and the content-free `Debug` of the transcript types.

use std::sync::Arc;

use mog_core::error::CoreError;
use mog_core::ipc_gateway::{GatewayComponents, ERROR_CODES};
use mog_core::recording::{MemorySettingsStore, RecordingComponent, RecordingSettingsStore};
use mog_core::types::{
    RecordingDetail, RecordingErrorKind, RecordingSegment, RecordingSettings, RecordingStatus,
    RecordingSummary, RECORDING_DEFAULT_ENDPOINT, RECORDING_DEFAULT_INSTRUCTIONS,
};
use serde_json::{json, Value};

fn component(settings: RecordingSettings) -> (RecordingComponent, Arc<MemorySettingsStore>) {
    let store = Arc::new(MemorySettingsStore::new(settings));
    (RecordingComponent::new(store.clone()), store)
}

fn code(r: Result<Value, CoreError>) -> &'static str {
    match r {
        Err(CoreError::IpcRefused { code }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn cap_06_the_knob_is_off_by_default_with_the_brief_defaults() {
    let s = RecordingSettings::default();
    assert!(!s.enabled);
    assert!(!s.consent_ack);
    assert!(s.storage.is_none());
    assert!(s.warehouse_id.is_none());
    assert_eq!(s.notes_endpoint.as_str(), "databricks-claude-sonnet-5-5");
    assert_eq!(RECORDING_DEFAULT_ENDPOINT, "databricks-claude-sonnet-5-5");
    assert_eq!(s.length_min.get(), 60);
    assert_eq!(s.preset_instructions.as_str(), RECORDING_DEFAULT_INSTRUCTIONS);
    assert!(RECORDING_DEFAULT_INSTRUCTIONS.contains("## Action items"));
    let wire = serde_json::to_value(&s).unwrap();
    assert_eq!(wire["storage"], Value::Null);
    assert_eq!(wire["warehouseId"], Value::Null);
    assert_eq!(wire["lengthMin"], json!(60));
}

#[test]
fn cap_06_recording_start_is_refused_when_the_knob_is_off() {
    let (c, _) = component(RecordingSettings::default());
    assert_eq!(code(c.call("recording_start", json!({}))), "recording_off");
    // The consent notice does not open the gate alone.
    let s = RecordingSettings {
        consent_ack: true,
        ..RecordingSettings::default()
    };
    let (c, _) = component(s);
    assert_eq!(
        code(c.call("recording_start", json!({"instructions": "Notes."}))),
        "recording_off"
    );
}

#[test]
fn cap_06_recording_start_needs_the_consent_notice_after_the_knob() {
    let s = RecordingSettings {
        enabled: true,
        ..RecordingSettings::default()
    };
    let (c, _) = component(s);
    assert_eq!(
        code(c.call("recording_start", json!({}))),
        "consent_required"
    );
}

#[test]
fn cap_06_an_open_gate_reaches_the_capture_stub() {
    let s = RecordingSettings {
        enabled: true,
        consent_ack: true,
        ..RecordingSettings::default()
    };
    let (c, _) = component(s);
    assert_eq!(code(c.call("recording_start", json!({}))), "not_implemented");
}

#[test]
fn cap_06_a_settings_write_that_turns_the_knob_off_closes_the_gate() {
    let s = RecordingSettings {
        enabled: true,
        consent_ack: true,
        ..RecordingSettings::default()
    };
    let (c, store) = component(s);
    let reply = c
        .call(
            "recording_settings_write",
            json!({"patch": {"enabled": false, "lengthMin": 30}}),
        )
        .expect("write");
    assert_eq!(reply["enabled"], json!(false));
    assert_eq!(reply["lengthMin"], json!(30));
    assert!(!store.load().unwrap().enabled);
    assert_eq!(
        c.call("recording_settings_read", json!({})).unwrap(),
        reply
    );
    assert_eq!(code(c.call("recording_start", json!({}))), "recording_off");
}

#[test]
fn sec_02b_a_bad_settings_patch_is_invalid_args() {
    let (c, _) = component(RecordingSettings::default());
    assert_eq!(
        code(c.call("recording_settings_write", json!({"patch": {"lengthMin": 2}}))),
        "invalid_args"
    );
}

#[test]
fn cap_06_every_other_recording_command_is_a_typed_stub() {
    let (c, _) = component(RecordingSettings::default());
    for (name, args) in [
        ("recording_storage_probe", json!({})),
        (
            "recording_volume_create",
            json!({"catalog": "main", "schema": "calls"}),
        ),
        ("recording_warehouses", json!({})),
        ("recording_stop", json!({})),
        ("recording_state", json!({})),
        ("recording_list", json!({})),
        ("recording_detail", json!({"id": "rec-1"})),
        ("recording_retry", json!({"id": "rec-1"})),
        ("recording_open", json!({"id": "rec-1", "target": "folder"})),
    ] {
        assert_eq!(code(c.call(name, args)), "not_implemented", "{name}");
    }
}

#[test]
fn sec_03_every_error_kind_maps_to_an_ipc_code() {
    for kind in RecordingErrorKind::ALL {
        let c = kind.ipc_code();
        assert!(ERROR_CODES.contains(&c), "{kind:?} -> {c}");
    }
    assert_eq!(RecordingErrorKind::RecordingOff.ipc_code(), "recording_off");
    assert_eq!(RecordingErrorKind::Network.ipc_code(), "offline");
    assert_eq!(RecordingErrorKind::Auth.ipc_code(), "auth_expired");
    assert_eq!(RecordingErrorKind::Internal.ipc_code(), "host_failed");
    assert_eq!(RecordingErrorKind::ALL.len(), 16);
}

const MARKER: &str = "the secret plan for the merger";

#[test]
fn sec_16c_transcript_debug_never_holds_the_text() {
    let seg = RecordingSegment {
        start: 1.0,
        end: 2.5,
        speaker: "1".into(),
        text: MARKER.into(),
        part: 1,
    };
    let detail = RecordingDetail {
        summary: RecordingSummary {
            id: "rec-1".parse().unwrap(),
            name: "2026-11-19_1430".into(),
            started_at_ms: 1,
            duration_s: 60,
            status: RecordingStatus::Done,
            error_kind: None,
            error_stage: None,
            engine: None,
            run_id: None,
        },
        instructions: MARKER.into(),
        segments: vec![seg.clone()],
        transcript_md: Some(MARKER.into()),
        notes_md: Some(MARKER.into()),
    };
    for shown in [format!("{seg:?}"), format!("{detail:?}"), format!("{detail:#?}")] {
        assert!(!shown.contains(MARKER), "{shown}");
    }
    // The wire still holds the text for the hub to show.
    assert_eq!(serde_json::to_value(&seg).unwrap()["text"], json!(MARKER));
}
