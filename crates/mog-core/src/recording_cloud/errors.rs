//! Plain-word errors of the recording pipeline (ADR 0103): each [`ErrorKind`] has one message
//! and at most one action. The mapping reads only the HTTP status, the `error_code` and fixed
//! markers of the upstream message; no upstream text is kept, logged or shown.

use serde::{Deserialize, Serialize};

use crate::error::CoreError;

/// What went wrong, in a form the views can show in plain words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// The user cannot read or write the volume.
    VolumePermission,
    /// The volume (or its catalog or schema) does not exist.
    VolumeNotFound,
    /// My Files does not resolve on this workspace.
    MyFilesUnavailable,
    /// The audio is larger than one model request can carry.
    TooLong,
    /// The transcribe model endpoint is missing or not accessible.
    TranscribeEndpointMissing,
    /// The notes model endpoint is missing or not accessible.
    NotesEndpointMissing,
    /// The model could not decode the audio.
    AudioUnreadable,
    /// The CLI sign-in expired.
    SignedOut,
    /// The workspace is not reachable.
    Offline,
    /// The model kept answering 429 after the retries.
    RateLimited,
    /// Upload failed for another reason.
    UploadFailed,
    /// Transcription failed for another reason.
    TranscribeFailed,
    /// The notes step failed for another reason.
    NotesFailed,
    /// A local Settings value is not valid.
    BadSettings,
    /// The local temp audio of a recording is gone, so the upload cannot run again.
    AudioMissing,
}

/// The one action of an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorAction {
    /// Open Bottle Settings.
    OpenSettings,
    /// Try the failed stage again.
    Retry,
}

/// The pipeline stage that failed (Retry starts here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    /// Upload of the parts and sidecars.
    Uploading,
    /// The transcribe model call.
    Transcribing,
    /// The notes model call.
    WritingNotes,
}

/// A failed pipeline step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordingError {
    /// What went wrong.
    pub kind: ErrorKind,
    /// The stage, when a pipeline stage failed.
    pub stage: Option<Stage>,
}

impl RecordingError {
    /// An error with no stage (Settings, storage).
    pub fn new(kind: ErrorKind) -> Self {
        Self { kind, stage: None }
    }
    /// An error at `stage`.
    pub fn at(kind: ErrorKind, stage: Stage) -> Self {
        Self {
            kind,
            stage: Some(stage),
        }
    }
}

impl std::fmt::Display for RecordingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(message(self.kind))
    }
}

impl std::error::Error for RecordingError {}

/// The plain-words message of `kind`.
pub fn message(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::VolumePermission => {
            "Bottle can't write to this volume. Pick another volume in Settings."
        }
        ErrorKind::VolumeNotFound => {
            "Bottle can't find this volume. Pick another volume in Settings."
        }
        ErrorKind::MyFilesUnavailable => {
            "My Files isn't available on this workspace. Pick a volume, or a catalog and schema."
        }
        ErrorKind::TooLong => "This recording is too long to transcribe.",
        ErrorKind::TranscribeEndpointMissing => {
            "The transcribe model isn't available. Pick another model in Settings."
        }
        ErrorKind::NotesEndpointMissing => {
            "The notes model isn't available. Pick another model in Settings."
        }
        ErrorKind::AudioUnreadable => "The recording could not be read.",
        ErrorKind::SignedOut => "Your Databricks sign-in expired. Sign in again, then retry.",
        ErrorKind::Offline => "Bottle can't reach your workspace. Check the network, then retry.",
        ErrorKind::RateLimited => "The model is busy. Retry in a few minutes.",
        ErrorKind::UploadFailed => "The upload didn't finish. Retry.",
        ErrorKind::TranscribeFailed => "The transcript didn't finish. Retry.",
        ErrorKind::NotesFailed => "The notes didn't finish. Retry.",
        ErrorKind::BadSettings => "A recording setting isn't valid. Check Settings.",
        ErrorKind::AudioMissing => "The audio of this recording is gone from this device.",
    }
}

/// The one action of `kind`, or `None` when no action helps.
pub fn action(kind: ErrorKind) -> Option<ErrorAction> {
    match kind {
        ErrorKind::TooLong | ErrorKind::AudioMissing | ErrorKind::AudioUnreadable => None,
        ErrorKind::VolumePermission
        | ErrorKind::VolumeNotFound
        | ErrorKind::MyFilesUnavailable
        | ErrorKind::TranscribeEndpointMissing
        | ErrorKind::NotesEndpointMissing
        | ErrorKind::BadSettings => Some(ErrorAction::OpenSettings),
        ErrorKind::SignedOut
        | ErrorKind::Offline
        | ErrorKind::RateLimited
        | ErrorKind::UploadFailed
        | ErrorKind::TranscribeFailed
        | ErrorKind::NotesFailed => Some(ErrorAction::Retry),
    }
}

/// A transport fault of R7 (content-free code) as a kind; `fallback` for any other fault.
pub fn from_core(e: &CoreError, fallback: ErrorKind) -> ErrorKind {
    match e.code() {
        "auth_expired" | "token_unusable" => ErrorKind::SignedOut,
        "offline" | "timeout" => ErrorKind::Offline,
        "rate_limited" => ErrorKind::RateLimited,
        _ => fallback,
    }
}

/// A Files API reply with a non-2xx status as a kind.
pub fn from_files_status(status: u16, fallback: ErrorKind) -> ErrorKind {
    match status {
        403 => ErrorKind::VolumePermission,
        404 => ErrorKind::VolumeNotFound,
        _ => fallback,
    }
}

fn has(hay: &str, needle: &str) -> bool {
    hay.to_ascii_lowercase()
        .contains(&needle.to_ascii_lowercase())
}

/// Which model call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelCall {
    /// The transcribe call.
    Transcribe,
    /// The notes call.
    Notes,
}

/// A non-2xx reply of the model gateway as a kind (live bodies, 2026-11-19):
/// 404 `NOT_FOUND` "'x' does not exist." (also 403) is a missing endpoint; a 400 with
/// "malformed or corrupted" / "could not be decoded" is unreadable audio; a 400/413 about the
/// size is too long.
pub fn from_model_reply(call: ModelCall, status: u16, error_code: &str, msg: &str) -> ErrorKind {
    let (missing, fallback) = match call {
        ModelCall::Transcribe => (
            ErrorKind::TranscribeEndpointMissing,
            ErrorKind::TranscribeFailed,
        ),
        ModelCall::Notes => (ErrorKind::NotesEndpointMissing, ErrorKind::NotesFailed),
    };
    match status {
        403 | 404 => missing,
        413 => ErrorKind::TooLong,
        400 if error_code == "ENDPOINT_NOT_FOUND" => missing,
        400 if call == ModelCall::Transcribe
            && (has(msg, "malformed")
                || has(msg, "could not be decoded")
                || has(msg, "unsupported audio")) =>
        {
            ErrorKind::AudioUnreadable
        }
        400 if has(msg, "too large") || has(msg, "payload size") || has(msg, "exceeds") => {
            ErrorKind::TooLong
        }
        _ => fallback,
    }
}
