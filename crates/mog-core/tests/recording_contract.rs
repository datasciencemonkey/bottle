//! The call recording contract of ADR 0103: the settings defaults, the Rust knob gate on
//! `recording_start`, the error-kind codes and the content-free `Debug` of the transcript types.

use std::sync::{Arc, Mutex};

use mog_core::error::CoreError;
use mog_core::ipc_gateway::{GatewayComponents, ERROR_CODES};
use mog_core::recording::{
    MemorySettingsStore, RecordingComponent, RecordingSettingsStore, SYSTEM_AUDIO_SETTINGS_URL,
};
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
    assert_eq!(s.transcribe_endpoint.as_str(), "system.ai.gemini-3-8-flash");
    assert_eq!(s.notes_endpoint.as_str(), "system.ai.claude-sonnet-5-5");
    assert_eq!(RECORDING_DEFAULT_ENDPOINT, "system.ai.claude-sonnet-5-5");
    assert_eq!(s.length_min.get(), 60);
    assert_eq!(
        s.preset_instructions.as_str(),
        RECORDING_DEFAULT_INSTRUCTIONS
    );
    assert!(RECORDING_DEFAULT_INSTRUCTIONS.contains("## Action items"));
    let wire = serde_json::to_value(&s).unwrap();
    assert_eq!(wire["storage"], Value::Null);
    assert_eq!(
        wire["transcribeEndpoint"],
        json!("system.ai.gemini-3-8-flash")
    );
    assert_eq!(wire["lengthMin"], json!(60));
}

#[test]
fn cap_06_recording_start_is_refused_when_the_knob_is_off() {
    let (c, _) = component(RecordingSettings::default());
    assert_eq!(
        code(c.call("recording_start", json!({"profile": "p"}))),
        "recording_off"
    );
    // The consent notice does not open the gate alone.
    let s = RecordingSettings {
        consent_ack: true,
        ..RecordingSettings::default()
    };
    let (c, _) = component(s);
    assert_eq!(
        code(c.call(
            "recording_start",
            json!({"profile": "p", "instructions": "Notes."})
        )),
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
        code(c.call("recording_start", json!({"profile": "p"}))),
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
    assert_eq!(
        code(c.call("recording_start", json!({"profile": "p"}))),
        "not_implemented"
    );
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
    assert_eq!(c.call("recording_settings_read", json!({})).unwrap(), reply);
    assert_eq!(
        code(c.call("recording_start", json!({"profile": "p"}))),
        "recording_off"
    );
}

#[test]
fn sec_02b_a_bad_settings_patch_is_invalid_args() {
    let (c, _) = component(RecordingSettings::default());
    assert_eq!(
        code(c.call(
            "recording_settings_write",
            json!({"patch": {"lengthMin": 2}})
        )),
        "invalid_args"
    );
}

#[test]
fn cap_06_every_other_recording_command_is_a_typed_stub() {
    let (c, _) = component(RecordingSettings::default());
    for (name, args) in [
        ("recording_storage_probe", json!({"profile": "p"})),
        (
            "recording_volume_create",
            json!({"profile": "p", "catalog": "main", "schema": "calls"}),
        ),
        ("recording_stop", json!({})),
        ("recording_state", json!({})),
        ("recording_list", json!({"profile": "p"})),
        ("recording_detail", json!({"profile": "p", "id": "rec-1"})),
        ("recording_retry", json!({"profile": "p", "id": "rec-1"})),
        ("recording_delete", json!({"profile": "p", "id": "rec-1"})),
        (
            "recording_open",
            json!({"profile": "p", "id": "rec-1", "target": "folder"}),
        ),
    ] {
        assert_eq!(code(c.call(name, args)), "not_implemented", "{name}");
    }
    // `recording_open_system_audio_settings` needs no profile and no `connect_capture` link
    // (CAP-10c, #593 follow-up), so it is not one of the stubs above: it is a real host
    // handler with its own `rec_open_system_audio_settings_*` tests below, which inject a fake
    // opener (or run only off macOS) so nothing here ever really opens System Settings.
}

#[test]
fn recording_cloud_commands_require_the_home_profile() {
    // Rust has no home profile of its own (it never picks one): the brain sends it on every
    // cloud-bound recording command. Missing it is `invalid_args`, same as any other required
    // field, before the command ever reaches `not_implemented`.
    let (c, _) = component(RecordingSettings::default());
    for (name, args) in [
        ("recording_storage_probe", json!({})),
        (
            "recording_volume_create",
            json!({"catalog": "main", "schema": "calls"}),
        ),
        ("recording_list", json!({})),
        ("recording_detail", json!({"id": "rec-1"})),
        ("recording_retry", json!({"id": "rec-1"})),
        ("recording_delete", json!({"id": "rec-1"})),
        ("recording_open", json!({"id": "rec-1", "target": "folder"})),
    ] {
        assert_eq!(code(c.call(name, args)), "invalid_args", "{name}");
    }
    // `recording_start` gates on the knob first (cap_06_recording_start_is_refused_when...),
    // but a missing profile is still invalid_args even with the knob on, ahead of the gate.
    let s = RecordingSettings {
        enabled: true,
        consent_ack: true,
        ..RecordingSettings::default()
    };
    let (c, _) = component(s);
    assert_eq!(code(c.call("recording_start", json!({}))), "invalid_args");
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
    assert_eq!(RecordingErrorKind::RateLimited.ipc_code(), "rate_limited");
    assert_eq!(
        RecordingErrorKind::TranscribeEndpointMissing.ipc_code(),
        "transcribe_endpoint_missing"
    );
    assert_eq!(
        RecordingErrorKind::AudioUnreadable.ipc_code(),
        "audio_unreadable"
    );
    assert_eq!(RecordingErrorKind::ALL.len(), 16);
}

// CAP-10c (#593 follow-up): `recording_open_system_audio_settings` needs no profile and no
// `connect_capture` link, unlike `recording_start`. `RecordingComponent::set_open_url` injects
// the opener, so the two tests below run on every OS with a fake that opens nothing, checking
// the exact URL and the mapping of a refusal to `host_failed` — the real NSWorkspace call
// behind the default opener is not exercised by any test (it would really open System Settings
// on the machine that runs it); a human reviewer checks that one line by reading it.

#[test]
fn rec_open_system_audio_settings_asks_the_injected_opener_for_the_right_url() {
    let (c, _) = component(RecordingSettings::default());
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let s = seen.clone();
    c.set_open_url(Arc::new(move |url: &str| {
        *s.lock().unwrap() = Some(url.to_owned());
        true
    }));
    assert_eq!(
        c.call("recording_open_system_audio_settings", json!({})),
        Ok(json!({}))
    );
    assert_eq!(
        seen.lock().unwrap().as_deref(),
        Some(SYSTEM_AUDIO_SETTINGS_URL)
    );
}

#[test]
fn rec_open_system_audio_settings_maps_a_false_opener_to_host_failed() {
    let (c, _) = component(RecordingSettings::default());
    c.set_open_url(Arc::new(|_url: &str| false));
    assert_eq!(
        code(c.call("recording_open_system_audio_settings", json!({}))),
        "host_failed"
    );
}

/// A later `set_open_url` is ignored (mirrors `set_cloud`): the first opener wins, same as the
/// first `connect_capture`/`set_cloud` call.
#[test]
fn rec_open_system_audio_settings_keeps_its_first_injected_opener() {
    let (c, _) = component(RecordingSettings::default());
    c.set_open_url(Arc::new(|_url: &str| true));
    c.set_open_url(Arc::new(|_url: &str| false));
    assert_eq!(
        c.call("recording_open_system_audio_settings", json!({})),
        Ok(json!({}))
    );
}

/// With nothing injected, the component's default opener refuses outright (CAP-10c review: it
/// must never call the real `NSWorkspace` off the main thread on its own). Safe on every OS,
/// including macOS: the default never touches `open_url_on_this_thread`.
#[test]
fn rec_open_system_audio_settings_refuses_with_no_opener_injected() {
    let (c, _) = component(RecordingSettings::default());
    assert_eq!(
        code(c.call("recording_open_system_audio_settings", json!({}))),
        "host_failed"
    );
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
            parts_uploaded: None,
            parts_total: None,
            on_device_only: false,
        },
        instructions: MARKER.into(),
        segments: vec![seg.clone()],
        transcript_md: Some(MARKER.into()),
        notes_md: Some(MARKER.into()),
    };
    for shown in [
        format!("{seg:?}"),
        format!("{detail:?}"),
        format!("{detail:#?}"),
    ] {
        assert!(!shown.contains(MARKER), "{shown}");
    }
    // The wire still holds the text for the hub to show.
    assert_eq!(serde_json::to_value(&seg).unwrap()["text"], json!(MARKER));
}
