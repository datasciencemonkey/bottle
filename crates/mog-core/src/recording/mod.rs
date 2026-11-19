//! Call recording (ADR 0103): the seams that the capture and cloud PRs implement, and the one
//! IPC component of the `recording_*` commands.
//!
//! Rust owns the capture, the files, the network, the tokens and the pipeline. The component
//! checks the knob here, in Rust: with `enabled: false`, `recording_start` answers
//! `recording_off`, and no capture path starts. Then the first-use notice: with
//! `consent_ack: false`, it answers `consent_required`.
//!
//! This module is not a contract file. The wire types are in `crate::types` and the rows in
//! `crate::ipc`. TODO(#65): PR B wires [`RecordingCapture`] into `recording_start`,
//! `recording_stop` and `recording_state`; PR C wires [`RecordingCloud`] into the other
//! commands and swaps [`MemorySettingsStore`] for a JSON file in the app config dir. No compute
//! runs in the workspace: two model serving calls transcribe and write the notes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::error::{CoreError, CoreResult};
use crate::ipc::{
    RecordingIdArgs, RecordingListArgs, RecordingOpenArgs, RecordingSettingsWriteArgs,
    RecordingStartArgs, RecordingStorageProbeArgs, RecordingVolumeCreateArgs,
};
use crate::ipc_gateway::GatewayComponents;
use crate::types::{
    Instructions, ProfileName, RecordingDetail, RecordingErrorKind, RecordingId, RecordingSegment,
    RecordingSettings, RecordingState, RecordingStorage, RecordingSummary, StorageProbe, UcName,
};

/// The result of a recording seam: a fixed error kind, never upstream text.
pub type RecordingResult<T> = Result<T, RecordingErrorKind>;

impl RecordingErrorKind {
    /// Every kind, in the order of the enum.
    pub const ALL: [RecordingErrorKind; 16] = [
        RecordingErrorKind::RecordingOff,
        RecordingErrorKind::ConsentRequired,
        RecordingErrorKind::RecordingBusy,
        RecordingErrorKind::MicBusy,
        RecordingErrorKind::NoStorage,
        RecordingErrorKind::VolumePermission,
        RecordingErrorKind::TranscribeEndpointMissing,
        RecordingErrorKind::NotesEndpointMissing,
        RecordingErrorKind::TooLong,
        RecordingErrorKind::AudioUnreadable,
        RecordingErrorKind::UploadFailed,
        RecordingErrorKind::RateLimited,
        RecordingErrorKind::Network,
        RecordingErrorKind::Auth,
        RecordingErrorKind::SystemAudioDenied,
        RecordingErrorKind::Internal,
    ];

    /// The IPC refusal code of this kind (`ipc_gateway::ERROR_CODES`). Network, auth and
    /// internal faults use the older codes `offline`, `auth_expired` and `host_failed`.
    pub fn ipc_code(self) -> &'static str {
        match self {
            RecordingErrorKind::RecordingOff => "recording_off",
            RecordingErrorKind::ConsentRequired => "consent_required",
            RecordingErrorKind::RecordingBusy => "recording_busy",
            RecordingErrorKind::MicBusy => "mic_busy",
            RecordingErrorKind::NoStorage => "no_storage",
            RecordingErrorKind::VolumePermission => "volume_permission",
            RecordingErrorKind::TranscribeEndpointMissing => "transcribe_endpoint_missing",
            RecordingErrorKind::NotesEndpointMissing => "notes_endpoint_missing",
            RecordingErrorKind::TooLong => "too_long",
            RecordingErrorKind::AudioUnreadable => "audio_unreadable",
            RecordingErrorKind::UploadFailed => "upload_failed",
            RecordingErrorKind::RateLimited => "rate_limited",
            RecordingErrorKind::Network => "offline",
            RecordingErrorKind::Auth => "auth_expired",
            RecordingErrorKind::SystemAudioDenied => "system_audio_denied",
            RecordingErrorKind::Internal => "host_failed",
        }
    }
}

impl From<RecordingErrorKind> for CoreError {
    fn from(kind: RecordingErrorKind) -> Self {
        CoreError::IpcRefused {
            code: kind.ipc_code(),
        }
    }
}

// ---------- seams ----------

/// Where the recording settings live. PR C keeps them in a JSON file in the app config dir
/// (no secret). [`MemorySettingsStore`] is the stub and the test double.
pub trait RecordingSettingsStore: Send + Sync {
    fn load(&self) -> CoreResult<RecordingSettings>;
    fn save(&self, settings: &RecordingSettings) -> CoreResult<()>;
}

/// One finished audio file on the local disk: Opus in Ogg, 16 kbps mono. A recording has one
/// file, `<name>.opus`, unless a later limit forces a split, in which case each file is
/// `<name>.partNN.opus` (ADR 0103 decision 8).
#[derive(Debug, Clone, PartialEq)]
pub struct FinishedPart {
    /// 1-based.
    pub index: u32,
    pub path: PathBuf,
    /// The start of the part from the start of the recording.
    pub offset_s: f64,
    pub duration_s: f64,
}

/// The one recording session of the device (PR B): the mic and the system audio, mixed to one
/// 16 kHz mono stream, encoded as one Ogg Opus file at 16 kbps (`<name>.opus`, or
/// `<name>.partNN.opus` per part if a later limit forces a split). It shares the one mic owner
/// with PTT.
pub trait RecordingCapture: Send + Sync {
    /// Start the recording `id` into the temp folder `dir`. It stops by itself after
    /// `max_len`. A second start answers `RecordingBusy`.
    fn start(
        &self,
        id: &RecordingId,
        dir: &Path,
        max_len: Duration,
    ) -> RecordingResult<RecordingState>;
    /// Stop the recording (the user, the length limit, sleep, quit or a lost device) and give
    /// the finished parts.
    fn stop(&self) -> RecordingResult<Vec<FinishedPart>>;
    /// The state now.
    fn state(&self) -> RecordingState;
}

/// One recording job: the folder, the parts and the instructions. `profile` is the home
/// profile the brain sent at `recording_start` (`src/brain/home-profile.ts`): the pipeline
/// uses it for every cloud call of this job, including after `recording_stop`.
#[derive(Debug, Clone, PartialEq)]
pub struct PipelineRequest {
    pub profile: ProfileName,
    pub storage: RecordingStorage,
    pub id: RecordingId,
    pub name: String,
    pub parts: Vec<FinishedPart>,
    pub instructions: Instructions,
    pub settings: RecordingSettings,
}

/// The workspace side (PR C): the Files API for storage, and model serving for the transcript and
/// the notes. Every call goes through HttpTransport with a TokenBroker token of the home
/// profile. Rust has no home profile of its own: the brain owns it (`src/brain/home-profile.ts`)
/// and sends it on every cloud-bound recording command, so every method here takes it as its
/// own `&ProfileName` argument. No method returns upstream text or a token. No method starts
/// compute in the workspace.
pub trait RecordingCloud: Send + Sync {
    /// Find the My Files root (`GET /api/2.0/fs/directories`, read-only) on `profile`'s host.
    fn probe_storage(&self, profile: &ProfileName) -> RecordingResult<StorageProbe>;
    /// Create the managed volume `bottle_audio` in `catalog.schema`, after the user confirmed.
    fn create_volume(
        &self,
        profile: &ProfileName,
        catalog: &UcName,
        schema: &UcName,
    ) -> RecordingResult<RecordingStorage>;
    /// Upload one part to `<root>/Bottle/Recordings/<name>/`. The caller deletes the local part
    /// only after this succeeds.
    fn upload_part(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        name: &str,
        part: &FinishedPart,
    ) -> RecordingResult<()>;
    /// Transcribe one part with the transcribe endpoint (the AI Gateway route
    /// `mlflow.chat_completions`, the audio inline as an `audio_url` data URL). Gives the
    /// segments with the part start added to their times. `context` holds the last lines of the
    /// previous part, if any.
    fn transcribe_part(
        &self,
        profile: &ProfileName,
        endpoint: &crate::types::EndpointName,
        part: &FinishedPart,
        context: &[RecordingSegment],
    ) -> RecordingResult<Vec<RecordingSegment>>;
    /// The notes Markdown from one chat call with the instructions and the joined transcript.
    fn write_notes(
        &self,
        profile: &ProfileName,
        endpoint: &crate::types::EndpointName,
        instructions: &Instructions,
        transcript_md: &str,
    ) -> RecordingResult<String>;
    /// Write the instructions, `job.json`, the transcript files and the notes to the volume
    /// folder, and give the summary of the job. `request.profile` is the same profile; the
    /// explicit argument keeps every `RecordingCloud` method the same shape.
    fn write_job(
        &self,
        profile: &ProfileName,
        request: &PipelineRequest,
        status: crate::types::RecordingStatus,
        detail: Option<&RecordingDetail>,
    ) -> RecordingResult<RecordingSummary>;
    /// The recordings in the volume, newest first, from each `<name>.job.json`.
    fn list(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
    ) -> RecordingResult<Vec<RecordingSummary>>;
    fn detail(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
    ) -> RecordingResult<RecordingDetail>;
    /// Start again at the failed step.
    fn retry(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
    ) -> RecordingResult<RecordingSummary>;
    /// The Catalog Explorer URL of the recording folder, built from `profile`'s host. It stays
    /// in Rust: the host opens it with the OS opener, and it never crosses IPC.
    fn folder_link(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
    ) -> RecordingResult<String>;
}

// ---------- the settings stub ----------

/// The settings in memory. The stub until PR C, and the test double.
#[derive(Default)]
pub struct MemorySettingsStore(Mutex<RecordingSettings>);

impl MemorySettingsStore {
    pub fn new(settings: RecordingSettings) -> Self {
        Self(Mutex::new(settings))
    }
}

impl RecordingSettingsStore for MemorySettingsStore {
    fn load(&self) -> CoreResult<RecordingSettings> {
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
    fn save(&self, settings: &RecordingSettings) -> CoreResult<()> {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = settings.clone();
        Ok(())
    }
}

// ---------- the IPC component ----------

/// The `recording_*` commands behind the hardened gateway. The gateway checked the window and
/// the arguments already.
pub struct RecordingComponent {
    settings: Arc<dyn RecordingSettingsStore>,
}

impl RecordingComponent {
    pub fn new(settings: Arc<dyn RecordingSettingsStore>) -> Self {
        Self { settings }
    }

    /// The knob and the first-use notice, checked in Rust before any capture path.
    pub fn gate(&self) -> CoreResult<RecordingSettings> {
        let s = self.settings.load()?;
        if !s.enabled {
            return Err(RecordingErrorKind::RecordingOff.into());
        }
        if !s.consent_ack {
            return Err(RecordingErrorKind::ConsentRequired.into());
        }
        Ok(s)
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

fn reply<T: serde::Serialize>(v: &T) -> CoreResult<Value> {
    serde_json::to_value(v).map_err(|_| refused("host_failed"))
}

impl GatewayComponents for RecordingComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "recording_settings_read" => reply(&self.settings.load()?),
            "recording_settings_write" => {
                let a: RecordingSettingsWriteArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                let next = self.settings.load()?.apply(a.patch);
                self.settings.save(&next)?;
                reply(&next)
            }
            "recording_start" => {
                let _a: RecordingStartArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                self.gate()?;
                // TODO(#65): PR B starts the RecordingCapture session here, with `_a.profile`
                // carried into the `PipelineRequest` the pipeline uses after `recording_stop`.
                Err(refused("not_implemented"))
            }
            // The cloud-bound commands (ADR 0103 follow-up): typed stubs that still require the
            // brain's home profile (`src/brain/home-profile.ts`), so a missing or malformed
            // `profile` is `invalid_args` here too, ahead of PR C wiring `RecordingCloud` in.
            "recording_storage_probe" => {
                let _a: RecordingStorageProbeArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                Err(refused("not_implemented"))
            }
            "recording_volume_create" => {
                let _a: RecordingVolumeCreateArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                Err(refused("not_implemented"))
            }
            "recording_list" => {
                let _a: RecordingListArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                Err(refused("not_implemented"))
            }
            "recording_detail" | "recording_retry" => {
                let _a: RecordingIdArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                Err(refused("not_implemented"))
            }
            "recording_open" => {
                let _a: RecordingOpenArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                Err(refused("not_implemented"))
            }
            // TODO(#65): PR B (stop, state) and PR C (the rest) fill these in.
            _ => Err(refused("not_implemented")),
        }
    }
}
