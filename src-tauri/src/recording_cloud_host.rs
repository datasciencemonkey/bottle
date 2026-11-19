//! The cloud half of call recording in the app (ADR 0103, PR C): one
//! [`RecordingCloudService`] on its own R7 [`ReqwestHttpTransport`], over the one R6 of
//! [`crate::mcp_host`] (the token never leaves R6), the managed T7 profile hosts and the one
//! [`SharedRateBucket`] (G-09a). It points the settings store of the `recording_*` component at
//! `<app config dir>/recording-settings.json` (no secret). Links open with the OS opener, only
//! for a Catalog Explorer URL on the profile host.
//!
//! The service is blocking: the host runs it on a worker thread. The `recording_*` commands of
//! the contract PR (`issue/rec-0-contract`) route here once it lands. The caller passes the home
//! profile the app already uses; this module never picks one.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use mog_core::clock::SystemWallClock;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport, ThreadPause};
use mog_core::ipc::events;
use mog_core::recording::{
    FinishedPart, PipelineRequest, RecordingComponent, RecordingSettingsStore,
};
use mog_core::recording_capture::session::{FinishedRecording, PartsHandoff, Snapshot};
use mog_core::recording_cloud::cloud::{
    is_audio_file_of, CloudDeps, LocalPart, ProfileWorkspace, RecordingCloudService,
};
use mog_core::recording_cloud::contract::{CloudAdapter, LocalAudio};
use mog_core::recording_cloud::layout::is_recording_name;
use mog_core::recording_cloud::opener::{open_link, SystemOpener, UrlOpener};
use mog_core::recording_cloud::settings_store::{JsonSettingsStore, SETTINGS_FILE};
use mog_core::types::{Instructions, RecordingId};
use tauri::Manager;

use crate::ipc::BrainChannel;
use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;
use crate::recording_host::{HandoffSlot, RecordingHost};

const ID: &str = "recording_cloud_host";

fn missing(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// The managed recording cloud parts.
pub struct RecordingCloudHost {
    /// The pipeline, storage and list service.
    pub service: Arc<RecordingCloudService>,
    /// The contract seam over the service (the `recording_*` component and the hand-off).
    pub adapter: Arc<CloudAdapter>,
    opener: Arc<dyn UrlOpener>,
    /// R7 of the service: the host keeps it alive.
    _transport: Arc<ReqwestHttpTransport>,
}

impl std::fmt::Debug for RecordingCloudHost {
    /// Shows nothing: no host, profile or path.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingCloudHost").finish_non_exhaustive()
    }
}

impl RecordingCloudHost {
    /// Open the Catalog Explorer folder of recording `name` in `root` for `profile`.
    pub fn open_folder(&self, profile: &str, root: &str, name: &str) -> CoreResult<()> {
        let url = self
            .service
            .link(profile, root, name)
            .map_err(|_| missing("link_failed"))?;
        let host = self.service.host_url(profile)?;
        open_link(&*self.opener, &host, &url)
    }
}

/// The temp audio folder under the app data dir (not the cache dir, so the OS does not purge a
/// recording that waits for Retry): `<app data>/recordings/<id>/<name>.opus`.
pub const TEMP_DIR: &str = "recordings";

/// The local temp audio files of recording `name` still on the device, in part order. PR B
/// keeps each recording in `<root>/<id>/`, so every folder of `root` is searched.
pub fn temp_parts_of(root: Option<&std::path::Path>, name: &str) -> Vec<LocalPart> {
    let Some(root) = root else {
        return Vec::new();
    };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.is_dir())
        .flat_map(|d| {
            std::fs::read_dir(d)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
        })
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| is_audio_file_of(name, f))
        })
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| LocalPart {
            path,
            duration_s: 0.0,
        })
        .collect()
}

/// The recording name encoded by a local temp audio file name (`<name>.<ext>` or
/// `<name>.partNN.<ext>`), or `None` when the file is not one (ADR 0103 decision 7).
fn recording_name_of_file(file: &str) -> Option<&str> {
    let (base, ext) = file.rsplit_once('.')?;
    if !matches!(ext, "opus" | "ogg" | "flac" | "wav") {
        return None;
    }
    let name = match base.rsplit_once(".part") {
        Some((n, digits)) if digits.len() == 2 && digits.bytes().all(|c| c.is_ascii_digit()) => n,
        _ => base,
    };
    is_recording_name(name).then_some(name)
}

/// The modified time of `path`, in epoch ms (`0` when it cannot be read).
fn modified_ms(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Every recording name with temp audio still on the device, and its earliest file's modified
/// time (epoch ms): the launch scan of ADR 0103 decision 7, run lazily on every `recording_list`
/// (PR B's per-ID folders under `root`, searched the same way as [`temp_parts_of`]).
pub fn temp_names_of(root: Option<&std::path::Path>) -> Vec<(String, u64)> {
    let Some(root) = root else {
        return Vec::new();
    };
    let mut names: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let dirs = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|d| d.is_dir());
    for dir in dirs {
        let files = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        for entry in files {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|f| f.to_str())
                .and_then(recording_name_of_file)
            else {
                continue;
            };
            let t = modified_ms(&path);
            names
                .entry(name.to_owned())
                .and_modify(|cur| *cur = (*cur).min(t))
                .or_insert(t);
        }
    }
    names.into_iter().collect()
}

/// The [`LocalAudio`] seam over the temp recordings folder (`<app data>/recordings/<id>/`).
struct TempAudio {
    root: Option<std::path::PathBuf>,
    /// The names capture still owns, or that finished but are not yet handed off (review of
    /// #592, BLOCKING B3): the scan and Retry both skip these.
    busy: Arc<dyn Fn() -> HashSet<String> + Send + Sync>,
}

impl LocalAudio for TempAudio {
    fn parts(&self, name: &str) -> Vec<LocalPart> {
        temp_parts_of(self.root.as_deref(), name)
    }
    fn names(&self) -> Vec<(String, u64)> {
        temp_names_of(self.root.as_deref())
    }
    fn active(&self) -> HashSet<String> {
        (self.busy)()
    }
}

/// The recording names a hand-off has taken from PR B but has not yet reached the pipeline's
/// own in-flight guard (the gap between [`PipelineHandoff::finished`] and
/// `CloudAdapter::run_pipeline`'s `begin`). Review of #592, BLOCKING B3.
#[derive(Default)]
pub struct ActiveNames {
    names: Mutex<HashSet<String>>,
}

impl ActiveNames {
    fn mark(&self, name: &str) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned());
    }
    fn release(&self, name: &str) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }
    fn get(&self) -> HashSet<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    /// Is `name` marked active now? For tests.
    pub fn contains(&self, name: &str) -> bool {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(name)
    }
}

/// The cloud end of PR B's hand-off: each finished recording runs the pipeline exactly once,
/// on its own thread, with the profile and the instructions of its `recording_start`.
pub struct PipelineHandoff {
    run: Arc<dyn Fn(PipelineRequest, u64) + Send + Sync>,
    settings: Arc<dyn RecordingSettingsStore>,
    seen: Mutex<HashSet<String>>,
    /// Marked for a recording's name from the moment a run is dispatched until `run` returns,
    /// so the cloud-side scan and Retry never touch it in between (review of #592, BLOCKING
    /// B3).
    active: Arc<ActiveNames>,
}

impl PipelineHandoff {
    /// A hand-off that calls `run(request, started_at_ms)` on a worker thread.
    pub fn new(
        run: Arc<dyn Fn(PipelineRequest, u64) + Send + Sync>,
        settings: Arc<dyn RecordingSettingsStore>,
        active: Arc<ActiveNames>,
    ) -> Self {
        Self {
            run,
            settings,
            seen: Mutex::new(HashSet::new()),
            active,
        }
    }

    /// The pipeline request of `rec`, or `None` when it cannot run now: no audio (the empty
    /// temp dir is deleted), no home profile, or no storage in Settings. Then the audio stays
    /// in the temp dir for Retry.
    pub fn request(&self, rec: &FinishedRecording) -> Option<PipelineRequest> {
        if rec.parts.is_empty() {
            remove_dir_if_empty(&rec.dir);
            return None;
        }
        let settings = self.settings.load().ok()?;
        let profile = rec.profile.clone().filter(|p| !p.trim().is_empty())?;
        let storage = settings.storage.clone()?;
        let id = RecordingId::try_from(rec.id.clone()).ok()?;
        let instructions = rec
            .instructions
            .clone()
            .and_then(|i| Instructions::try_from(i).ok())
            .unwrap_or_else(|| settings.preset_instructions.clone());
        Some(PipelineRequest {
            profile,
            storage,
            id,
            name: rec.name.clone(),
            parts: rec
                .parts
                .iter()
                .map(|p| FinishedPart {
                    index: p.index,
                    path: p.path.clone(),
                    offset_s: p.offset_ms as f64 / 1000.0,
                    duration_s: p.samples as f64 / f64::from(SAMPLE_RATE),
                })
                .collect(),
            instructions,
            settings,
        })
    }
}

/// The capture sample rate of the parts (16 kHz mono).
const SAMPLE_RATE: u32 = 16_000;

/// Delete `dir` only when it is empty (after every part was uploaded and deleted).
pub fn remove_dir_if_empty(dir: &std::path::Path) {
    let _ = std::fs::remove_dir(dir);
}

/// Releases `name` from `active` when dropped: a panic inside the dispatched run still
/// unwinds through this guard, so the name is never left marked active forever (review of
/// #592, LOW).
struct ActiveGuard {
    active: Arc<ActiveNames>,
    name: String,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.release(&self.name);
    }
}

impl PartsHandoff for PipelineHandoff {
    fn finished(&self, rec: FinishedRecording) {
        let first = self
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(rec.id.clone());
        if !first {
            return;
        }
        let Some(req) = self.request(&rec) else {
            return;
        };
        self.active.mark(&rec.name);
        let run = self.run.clone();
        let dir = rec.dir.clone();
        let started = rec.started_at_ms.max(0) as u64;
        let guard = ActiveGuard {
            active: self.active.clone(),
            name: rec.name.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("bottle-rec-pipeline".into())
            .spawn(move || {
                let _guard = guard;
                run(req, started);
                remove_dir_if_empty(&dir);
            });
        if spawned.is_err() {
            // The thread never started, so its guard never ran: release the mark here (a
            // failed spawn must not leave the name stuck active forever).
            self.active.release(&rec.name);
        }
    }
}

/// Build the recording cloud host. [`crate::mcp_host::install`] and
/// [`crate::profiles_host::install`] run first.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) -> CoreResult<()> {
    let bucket = handle
        .try_state::<SharedRateBucket>()
        .ok_or(missing("rate_bucket_missing"))?
        .0
        .clone();
    let hosts: Arc<dyn ProfileHosts> = handle
        .try_state::<ProfilesHost>()
        .ok_or(missing("profiles_missing"))?
        .0
        .clone();
    let tokens = handle
        .try_state::<McpHost>()
        .ok_or(missing("token_broker_missing"))?
        .token_broker();
    let transport = Arc::new(ReqwestHttpTransport::new(
        hosts.clone(),
        tokens.clone(),
        bucket,
    )?);
    let service = Arc::new(RecordingCloudService::new(CloudDeps {
        http: transport.clone(),
        workspace: Arc::new(ProfileWorkspace { hosts, tokens }),
        wall: Arc::new(SystemWallClock),
        pause: Arc::new(ThreadPause),
    }));
    // The settings file of the `recording_*` component (lib.rs builds it before the config
    // dir is known). No config dir: the defaults stay (knob off) and writes fail.
    if let (Some(store), Ok(dir)) = (
        handle.try_state::<Arc<JsonSettingsStore>>(),
        handle.path().app_config_dir(),
    ) {
        store.set_path(dir.join(SETTINGS_FILE));
    }
    let store: Arc<dyn RecordingSettingsStore> = match handle.try_state::<Arc<JsonSettingsStore>>()
    {
        Some(s) => s.inner().clone(),
        None => Arc::new(JsonSettingsStore::deferred()),
    };
    let temp_root = handle.path().app_data_dir().ok().map(|d| d.join(TEMP_DIR));
    let active_names = Arc::new(ActiveNames::default());
    let busy: Arc<dyn Fn() -> HashSet<String> + Send + Sync> = {
        let active_names = active_names.clone();
        let app = handle.clone();
        Arc::new(move || {
            let mut names = active_names.get();
            // Capture owns the name of the recording running right now (review of #592,
            // BLOCKING B3): the chunker writes its open part under the final name, in the
            // same folder the scan reads.
            if let Some(host) = app.try_state::<Arc<RecordingHost>>() {
                if let Snapshot::Recording(info) = host.session().snapshot() {
                    names.insert(info.name);
                }
                // A stop that is still flushing the writer and running the hand-off (review
                // of #592): `snapshot()` alone would read `Idle` here already.
                if let Some(name) = host.session().finishing_name() {
                    names.insert(name);
                }
            }
            // A recording that finished before the cloud side connected still waits in the
            // slot; it is not active capture, but it is not abandoned either.
            if let Some(slot) = app.try_state::<Arc<HandoffSlot>>() {
                names.extend(slot.pending().into_iter().map(|rec| rec.name));
            }
            names
        })
    };
    let local: Arc<dyn LocalAudio> = Arc::new(TempAudio {
        root: temp_root,
        busy,
    });
    let app = handle.clone();
    let on_job = Arc::new(move |s: &mog_core::types::RecordingSummary| {
        if let (Some(ch), Ok(v)) = (app.try_state::<BrainChannel>(), serde_json::to_value(s)) {
            ch.send(events::RECORDING_JOB, v);
        }
    });
    let adapter = Arc::new(CloudAdapter::new(
        service.clone(),
        store.clone(),
        local,
        on_job,
        Arc::new(|| {
            use mog_core::clock::WallClock;
            mog_core::clock::SystemWallClock.now_ms()
        }),
    ));
    // PR B's finished recordings run the pipeline once each (waiting ones go out now).
    if let Some(slot) = handle.try_state::<Arc<HandoffSlot>>() {
        let a = adapter.clone();
        slot.connect(Arc::new(PipelineHandoff::new(
            Arc::new(move |req, started| {
                let _ = a.run_pipeline(&req, started);
            }),
            store.clone(),
            active_names.clone(),
        )));
    }
    let opener: Arc<dyn UrlOpener> = Arc::new(SystemOpener);
    if let Some(component) = handle.try_state::<Arc<RecordingComponent>>() {
        let (svc, op) = (service.clone(), opener.clone());
        component.set_cloud(
            adapter.clone(),
            Arc::new(move |profile: &String, url: &str| {
                let host = svc.host_url(profile)?;
                open_link(&*op, &host, url)
            }),
        );
    }
    handle.manage(RecordingCloudHost {
        service,
        adapter,
        opener,
        _transport: transport,
    });
    Ok(())
}
