//! Recording error mapping (ADR 0103, PR C): captured live bodies map to plain-word kinds.

use mog_core::error::CoreError;
use mog_core::recording_cloud::errors::*;


// Bodies captured live on the DEFAULT workspace (2026-11-19), names unchanged.
const UNRESOLVED: &str = "[UNRESOLVED_ROUTINE] Cannot resolve routine `ai_transcribe` on search path [`system`.`session`, `system`.`builtin`]. SQLSTATE: 42883";
const ENDPOINT: &str = "[AI_FUNCTION_COMPILATION_ERROR] An error occurred while compiling the AI function: INVALID_ARGUMENT: 'no-such-endpoint-xyz' does not exist or is not accessible. SQLSTATE: 42883; line 1 pos 7";
const PERM: &str = "PERMISSION_DENIED: User does not have READ VOLUME on Volume adr_demos.app.inputs";
const NO_PATH: &str = "[CF_PATH_DOES_NOT_EXIST_FOR_READ_FILES] Cannot read files when the input path `/Volumes/c/s/v/nope.flac` does not exist. Please make sure the input path exists and re-try. SQLSTATE: 42000";
const NO_CATALOG: &str = "[NO_SUCH_CATALOG_EXCEPTION] Catalog 'nope_cat' was not found. Please verify the catalog name and then retry the query or command again. SQLSTATE: 42704";

#[test]
fn rec_errors_map_captured_sql_errors() {
    let f = ErrorKind::TranscribeFailed;
    assert_eq!(
        from_sql_error("BAD_REQUEST", UNRESOLVED, f),
        ErrorKind::AiTranscribeDisabled
    );
    assert_eq!(
        from_sql_error("BAD_REQUEST", ENDPOINT, ErrorKind::NotesFailed),
        ErrorKind::EndpointMissing
    );
    assert_eq!(
        from_sql_error("BAD_REQUEST", PERM, f),
        ErrorKind::VolumePermission
    );
    assert_eq!(
        from_sql_error("BAD_REQUEST", NO_PATH, f),
        ErrorKind::VolumeNotFound
    );
    assert_eq!(
        from_sql_error("BAD_REQUEST", NO_CATALOG, f),
        ErrorKind::VolumeNotFound
    );
    assert_eq!(from_sql_error("INTERNAL_ERROR", "boom", f), f);
}

#[test]
fn rec_errors_map_too_long_and_files_status() {
    assert_eq!(
        from_transcribe_message("Audio file exceeds the maximum duration of 1 hour."),
        ErrorKind::TooLong
    );
    assert_eq!(
        from_transcribe_message("Unsupported or unrecognized audio format."),
        ErrorKind::TranscribeFailed
    );
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
}

#[test]
fn rec_errors_transport_faults_and_serverless() {
    let auth = CoreError::Unavailable {
        component: "R7",
        reason: "auth_expired",
    };
    let off = CoreError::Unavailable {
        component: "R7",
        reason: "offline",
    };
    assert_eq!(from_core(&auth, ErrorKind::UploadFailed), ErrorKind::SignedOut);
    assert_eq!(from_core(&off, ErrorKind::UploadFailed), ErrorKind::Offline);
    assert!(serverless_unusable(
        400,
        "INVALID_PARAMETER_VALUE",
        "Serverless compute is not enabled for this workspace."
    ));
    assert!(!serverless_unusable(
        400,
        "INVALID_PARAMETER_VALUE",
        "Run 1 does not exist."
    ));
    assert!(!serverless_unusable(500, "", "serverless"));
}

#[test]
fn rec_errors_each_kind_has_plain_copy_and_one_action() {
    assert_eq!(
        message(ErrorKind::NoWarehouse),
        "Pick a SQL warehouse in Settings."
    );
    assert_eq!(action(ErrorKind::NoWarehouse), Some(ErrorAction::OpenSettings));
    assert_eq!(
        message(ErrorKind::AiTranscribeDisabled),
        "AI Transcribe is off for this workspace. Ask your workspace admin to turn on AI Transcribe in Previews."
    );
    assert_eq!(
        message(ErrorKind::EndpointMissing),
        "The notes model isn't available. Pick another model in Settings."
    );
    assert_eq!(action(ErrorKind::UploadFailed), Some(ErrorAction::Retry));
    let wire = serde_json::to_value(ErrorKind::AiTranscribeDisabled).unwrap();
    assert_eq!(wire, "ai_transcribe_disabled");
}
