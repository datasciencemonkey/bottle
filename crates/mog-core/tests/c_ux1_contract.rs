//! ADR 0118 (C-UX1): the contract part of the paste target lock and the held-note flag.

use mog_core::insertion::InsertOutcome;
use mog_core::ipc::{self, IpcError};
use mog_core::types::{RecordingSettings, RecordingSettingsPatch};
use serde_json::json;

const MOG: &str = "00000000-0000-4000-8000-0000000000a1";

#[test]
fn adr_0118_target_changed_is_a_wire_outcome() {
    assert_eq!(
        serde_json::to_value(InsertOutcome::TargetChanged).unwrap(),
        json!("target_changed")
    );
}

#[test]
fn adr_0118_insert_text_needs_a_valid_lease_id() {
    for args in [
        json!({"mogId": MOG, "text": "hi"}),
        json!({"mogId": MOG, "text": "hi", "leaseId": ""}),
        json!({"mogId": MOG, "text": "hi", "leaseId": "bad lease/1"}),
        json!({"mogId": MOG, "text": "hi", "leaseId": 7}),
    ] {
        let err = ipc::handle("brain", "insert_text", args).unwrap_err();
        assert!(matches!(err, IpcError::InvalidArgs { .. }), "{err:?}");
    }
}

#[test]
fn adr_0118_held_note_shown_defaults_false_for_an_old_settings_file() {
    let mut v = serde_json::to_value(RecordingSettings::default()).unwrap();
    assert_eq!(v["heldNoteShown"], json!(false));
    v.as_object_mut().unwrap().remove("heldNoteShown");
    let old: RecordingSettings = serde_json::from_value(v).unwrap();
    assert!(!old.held_note_shown);
    let patched = old.apply(RecordingSettingsPatch {
        held_note_shown: Some(true),
        ..Default::default()
    });
    assert!(patched.held_note_shown);
}
