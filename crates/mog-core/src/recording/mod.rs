//! Call recording (ADR 0103): the seams that the capture and cloud PRs implement, and the one
//! IPC component of the `recording_*` commands.
//!
//! Rust owns the capture, the files, the network, the tokens and the pipeline. The component
//! checks the knob here, in Rust: with `enabled: false`, `recording_start` answers
//! `recording_off`, and no capture path starts. Then the first-use notice: with
//! `consent_ack: false`, it answers `consent_required`.
//!
//! This module is not a contract file. The wire types are in `crate::types` and the rows in
//! `crate::ipc`. PR B wires [`RecordingCapture`] into `recording_start`, `recording_stop` and
//! `recording_state` through [`RecordingComponent::connect_capture`]. TODO(#65): PR C wires [`RecordingCloud`] into the other
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
    /// The link that `recording_open` opens for `target`: the folder link, or the link of the
    /// transcript or notes file (which opens the folder, because Databricks documents no file
    /// deep link). Only a finished recording has these files.
    fn target_link(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
        target: crate::ipc::RecordingOpenTarget,
    ) -> RecordingResult<String>;
    /// The recordings that are only on this device, newest first. They need no storage: this
    /// is what the list shows while Settings has no usable storage.
    fn list_local(&self) -> RecordingResult<Vec<RecordingSummary>>;
    /// Delete a recording that never reached the volume: its audio files, temp files and
    /// sidecar on this device. `Ok(false)`: no such recording on this device. A recording that
    /// captures, runs, or has parts in the volume gives `recording_busy`. It never writes to
    /// the volume. `storage` is `None` when Settings has none: the volume cannot be read then,
    /// so the device must prove that no part was ever uploaded, or the call is `recording_busy`.
    /// Files that remain after the delete give an error.
    fn delete(
        &self,
        profile: &ProfileName,
        storage: Option<&RecordingStorage>,
        id: &RecordingId,
    ) -> RecordingResult<bool>;
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
/// Opens a Catalog Explorer link for a profile (the host checks the host and uses the OS
/// opener; PR C).
pub type LinkOpener = Arc<dyn Fn(&ProfileName, &str) -> CoreResult<()> + Send + Sync>;

/// Opens a URL with the OS and reports whether it actually opened (CAP-10c, the
/// `recording_open_system_audio_settings` fix-it). `RecordingComponent::set_open_url` injects
/// one; a test injects a fake that opens nothing, so the mapping to `host_failed` and the exact
/// URL can be checked without ever really opening anything.
pub type UrlOpenFn = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The macOS Privacy & Security pane that covers System Audio Recording: the one
/// `recording_open_system_audio_settings` opens. Apple groups System Audio Recording with
/// Screen Recording under one "Screen & System Audio Recording" pane, confirmed by hand
/// (`docs/spikes/SP-01.md`); its stable URL anchor is `Privacy_ScreenCapture`. Apple documents
/// no separate `Privacy_AudioCapture` anchor, so this is the one that actually opens the right
/// pane.
pub const SYSTEM_AUDIO_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

/// Open `url` with the OS, right here on the calling thread (CAP-10c). macOS only
/// (`NSWorkspace`, which AppKit expects on the main thread — the src-tauri host dispatches
/// there, through `AppHandle::run_on_main_thread`, before calling this; `recording_*` commands
/// otherwise run off the main thread, `ipc.rs::runs_off_main_thread`). Every other OS opens
/// nothing. This is `RecordingComponent`'s default opener when nothing injects one.
pub fn open_url_on_this_thread(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        use objc2_foundation::{NSString, NSURL};
        let ns_url = NSString::from_str(url);
        match NSURL::URLWithString(&ns_url) {
            Some(u) => NSWorkspace::sharedWorkspace().openURL(&u),
            None => false,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = url;
        false
    }
}

fn default_open_url() -> UrlOpenFn {
    Arc::new(open_url_on_this_thread)
}

pub struct RecordingComponent {
    settings: Arc<dyn RecordingSettingsStore>,
    capture: Mutex<Option<CaptureLink>>,
    /// Set once in the app `setup`, after R6 and the rate bucket exist (PR C).
    cloud: std::sync::OnceLock<(Arc<dyn RecordingCloud>, LinkOpener)>,
    /// Set once in the app `setup` (CAP-10c); [`default_open_url`] until then.
    open_url: std::sync::OnceLock<UrlOpenFn>,
}

/// What PR B connects to the component once the capture host runs (ADR 0103): the capture
/// session, the root of the per-recording temp dirs, and the knob hook (the host's knob and
/// the permission ask on OFF to ON).
#[derive(Clone)]
pub struct CaptureLink {
    pub capture: Arc<dyn RecordingCapture>,
    pub temp_root: PathBuf,
    pub on_knob: Arc<dyn Fn(bool) + Send + Sync>,
    /// Called just before each capture start with the new ID, the brain's home profile and
    /// the per-recording instructions. The capture keeps them and hands them off with the
    /// finished parts (PR C runs the pipeline with them).
    pub on_start: StartContextFn,
}

/// See [`CaptureLink::on_start`].
pub type StartContextFn =
    Arc<dyn Fn(&RecordingId, &crate::types::ProfileName, Option<&Instructions>) + Send + Sync>;

fn new_recording_id() -> RecordingId {
    let mut bytes = [0u8; 8];
    let _ = getrandom::fill(&mut bytes);
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    RecordingId::try_from(format!("rec-{hex}"))
        .unwrap_or_else(|_| RecordingId::try_from("rec-0".to_owned()).expect("a valid literal ID"))
}

impl RecordingComponent {
    pub fn new(settings: Arc<dyn RecordingSettingsStore>) -> Self {
        Self {
            settings,
            capture: Mutex::new(None),
            cloud: std::sync::OnceLock::new(),
            open_url: std::sync::OnceLock::new(),
        }
    }

    /// Connect the capture (PR B). The knob hook gets the stored knob at once, then each
    /// settings write. Before this, `recording_start`, `_stop` and `_state` answer
    /// `not_implemented`.
    pub fn connect_capture(&self, link: CaptureLink) {
        let enabled = self.settings.load().map(|s| s.enabled).unwrap_or(false);
        (link.on_knob)(enabled);
        *self.capture.lock().unwrap_or_else(|e| e.into_inner()) = Some(link);
    }

    fn link(&self) -> Option<CaptureLink> {
        self.capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Inject the opener of `recording_open_system_audio_settings` (CAP-10c), once; a later
    /// call is ignored. Until this runs, the default is [`open_url_on_this_thread`] (no main
    /// thread dispatch).
    pub fn set_open_url(&self, opener: UrlOpenFn) {
        let _ = self.open_url.set(opener);
    }

    fn open_url(&self) -> UrlOpenFn {
        match self.open_url.get() {
            Some(f) => f.clone(),
            None => default_open_url(),
        }
    }

    /// Connect the cloud side (PR C) and the link opener, once. A later call is ignored.
    pub fn set_cloud(&self, cloud: Arc<dyn RecordingCloud>, opener: LinkOpener) {
        let _ = self.cloud.set((cloud, opener));
    }

    fn cloud(&self) -> CoreResult<&Arc<dyn RecordingCloud>> {
        self.cloud
            .get()
            .map(|(c, _)| c)
            .ok_or(refused("not_implemented"))
    }

    /// The storage of the Settings, or `no_storage`.
    fn storage(&self) -> CoreResult<RecordingStorage> {
        self.settings
            .load()?
            .storage
            .ok_or_else(|| RecordingErrorKind::NoStorage.into())
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
                if let Some(link) = self.link() {
                    (link.on_knob)(next.enabled);
                }
                reply(&next)
            }
            "recording_start" => {
                let a: RecordingStartArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                let settings = self.gate()?;
                let link = self.link().ok_or_else(|| refused("not_implemented"))?;
                let id = new_recording_id();
                // The job context goes with the finished parts to the cloud side (PR C): the
                // home profile for every cloud call, and this recording's instructions.
                (link.on_start)(&id, &a.profile, a.instructions.as_ref());
                let dir = link.temp_root.join(id.as_str());
                let max_len = Duration::from_secs(u64::from(settings.length_min.get()) * 60);
                reply(&link.capture.start(&id, &dir, max_len)?)
            }
            // The cloud-bound commands (ADR 0103 follow-up): typed stubs that still require the
            // brain's home profile (`src/brain/home-profile.ts`), so a missing or malformed
            // `profile` is `invalid_args` here too, ahead of PR C wiring `RecordingCloud` in.
            // The cloud-bound commands (PR C): every one carries the brain's home profile.
            "recording_storage_probe" => {
                let a: RecordingStorageProbeArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                reply(&self.cloud()?.probe_storage(&a.profile)?)
            }
            "recording_volume_create" => {
                let a: RecordingVolumeCreateArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                reply(
                    &self
                        .cloud()?
                        .create_volume(&a.profile, &a.catalog, &a.schema)?,
                )
            }
            "recording_list" => {
                let a: RecordingListArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                let cloud = self.cloud()?;
                // No storage yet: the list is the audio that waits on this device, so the user
                // sees what Upload will send once Settings has a volume.
                match self.storage() {
                    Ok(storage) => reply(&cloud.list(&a.profile, &storage)?),
                    Err(CoreError::IpcRefused { code: "no_storage" }) => {
                        reply(&cloud.list_local()?)
                    }
                    Err(e) => Err(e),
                }
            }
            "recording_detail" | "recording_retry" => {
                let a: RecordingIdArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                let cloud = self.cloud()?;
                let storage = self.storage()?;
                if command == "recording_detail" {
                    reply(&cloud.detail(&a.profile, &storage, &a.id)?)
                } else {
                    reply(&cloud.retry(&a.profile, &storage, &a.id)?)
                }
            }
            "recording_delete" => {
                let a: RecordingIdArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                let cloud = self.cloud()?;
                let storage = match self.storage() {
                    Ok(s) => Some(s),
                    Err(CoreError::IpcRefused { code: "no_storage" }) => None,
                    Err(e) => return Err(e),
                };
                if cloud.delete(&a.profile, storage.as_ref(), &a.id)? {
                    Ok(serde_json::json!({}))
                } else {
                    Err(refused("invalid_args"))
                }
            }
            "recording_open" => {
                let a: RecordingOpenArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                let cloud = self.cloud()?;
                let url = cloud.target_link(&a.profile, &self.storage()?, &a.id, a.target)?;
                let open = self
                    .cloud
                    .get()
                    .map(|(_, o)| o)
                    .ok_or(refused("not_implemented"))?;
                open(&a.profile, &url)?;
                Ok(serde_json::json!({}))
            }
            "recording_stop" => {
                let link = self.link().ok_or_else(|| refused("not_implemented"))?;
                // The parts also reach the cloud side through the capture's hand-off.
                link.capture.stop()?;
                reply(&link.capture.state())
            }
            "recording_state" => {
                let link = self.link().ok_or_else(|| refused("not_implemented"))?;
                reply(&link.capture.state())
            }
            // The "Allow System Audio Recording" fix-it (ipc.rs): no profile, no capture link,
            // no args. macOS only; refuses on every other OS.
            "recording_open_system_audio_settings" => {
                if (self.open_url())(SYSTEM_AUDIO_SETTINGS_URL) {
                    Ok(serde_json::json!({}))
                } else {
                    Err(refused("host_failed"))
                }
            }
            // TODO(#65): PR C fills in the rest.
            _ => Err(refused("not_implemented")),
        }
    }
}
