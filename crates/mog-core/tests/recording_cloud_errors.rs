//! Recording error mapping (ADR 0103, PR C): captured live bodies map to plain-word kinds.

use mog_core::error::CoreError;
use mog_core::recording_cloud::errors::*;

// Bodies captured live on the DEFAULT workspace (2026-11-19, gateway chat route).
const NOT_FOUND: &str = "'no-such-model-xyz' does not exist.";
const MALFORMED: &str = "{\n  \"error\": {\n    \"code\": 400,\n    \"message\": \"The provided file is malformed or corrupted and could not be decoded.\",\n    \"status\": \"INVALID_ARGUMENT\"\n  }\n}\n";

#[test]
fn rec_errors_map_captured_model_errors() {
    use ModelCall::*;
    assert_eq!(
        from_model_reply(Transcribe, 404, "NOT_FOUND", NOT_FOUND),
        ErrorKind::TranscribeEndpointMissing
    );
    assert_eq!(
        from_model_reply(Notes, 404, "NOT_FOUND", NOT_FOUND),
        ErrorKind::NotesEndpointMissing
    );
    assert_eq!(
        from_model_reply(Notes, 403, "PERMISSION_DENIED", "no"),
        ErrorKind::NotesEndpointMissing
    );
    assert_eq!(
        from_model_reply(Transcribe, 400, "BAD_REQUEST", MALFORMED),
        ErrorKind::AudioUnreadable
    );
    assert_eq!(
        from_model_reply(Transcribe, 413, "", ""),
        ErrorKind::TooLong
    );
    assert_eq!(
        from_model_reply(Transcribe, 500, "INTERNAL_ERROR", "boom"),
        ErrorKind::TranscribeFailed
    );
    assert_eq!(
        from_model_reply(Notes, 400, "BAD_REQUEST", "other"),
        ErrorKind::NotesFailed
    );
}

#[test]
fn rec_errors_map_files_status_and_transport_faults() {
    assert_eq!(
        from_files_status(403, ErrorKind::UploadFailed),
        ErrorKind::VolumePermission
    );
    assert_eq!(
        from_files_status(404, ErrorKind::UploadFailed),
        ErrorKind::VolumeNotFound
    );
    assert_eq!(
        from_files_status(500, ErrorKind::UploadFailed),
        ErrorKind::UploadFailed
    );
    let c = |reason| CoreError::Unavailable {
        component: "R7",
        reason,
    };
    assert_eq!(
        from_core(&c("auth_expired"), ErrorKind::UploadFailed),
        ErrorKind::SignedOut
    );
    assert_eq!(
        from_core(&c("offline"), ErrorKind::UploadFailed),
        ErrorKind::Offline
    );
    assert_eq!(
        from_core(&c("rate_limited"), ErrorKind::UploadFailed),
        ErrorKind::RateLimited
    );
    assert_eq!(
        from_core(&c("non_json_reply"), ErrorKind::UploadFailed),
        ErrorKind::UploadFailed
    );
}

#[test]
fn rec_errors_each_kind_has_plain_copy_and_one_action() {
    assert_eq!(
        message(ErrorKind::TranscribeEndpointMissing),
        "The transcribe model isn't available. Pick another model in Settings."
    );
    assert_eq!(
        message(ErrorKind::NotesEndpointMissing),
        "The notes model isn't available. Pick another model in Settings."
    );
    assert_eq!(
        message(ErrorKind::AudioUnreadable),
        "The recording could not be read."
    );
    assert_eq!(
        message(ErrorKind::TooLong),
        "This recording is too long to transcribe."
    );
    assert_eq!(
        message(ErrorKind::MyFilesUnavailable),
        "My Files isn't available on this workspace. Pick a volume, or a catalog and schema."
    );
    assert_eq!(
        action(ErrorKind::NotesEndpointMissing),
        Some(ErrorAction::OpenSettings)
    );
    assert_eq!(action(ErrorKind::UploadFailed), Some(ErrorAction::Retry));
    assert_eq!(action(ErrorKind::TooLong), None);
    let wire = serde_json::to_value(ErrorKind::TranscribeEndpointMissing).unwrap();
    assert_eq!(wire, "transcribe_endpoint_missing");
    assert_eq!(
        serde_json::to_value(Stage::WritingNotes).unwrap(),
        "writing_notes"
    );
}
