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
use mog_core::recording_capture::session::{FinishedRecording, PartsHandoff};
use mog_core::recording_cloud::cloud::{
    is_audio_file_of, CloudDeps, LocalPart, ProfileWorkspace, RecordingCloudService,
};
use mog_core::recording_cloud::contract::CloudAdapter;
use mog_core::recording_cloud::opener::{open_link, SystemOpener, UrlOpener};
use mog_core::recording_cloud::settings_store::{JsonSettingsStore, SETTINGS_FILE};
use mog_core::types::{Instructions, RecordingId};
use tauri::Manager;

use crate::ipc::BrainChannel;
use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;
use crate::recording_host::HandoffSlot;

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

/// The cloud end of PR B's hand-off: each finished recording runs the pipeline exactly once,
/// on its own thread, with the profile and the instructions of its `recording_start`.
pub struct PipelineHandoff {
    run: Arc<dyn Fn(PipelineRequest, u64) + Send + Sync>,
    settings: Arc<dyn RecordingSettingsStore>,
    seen: Mutex<HashSet<String>>,
}

impl PipelineHandoff {
    /// A hand-off that calls `run(request, started_at_ms)` on a worker thread.
    pub fn new(
        run: Arc<dyn Fn(PipelineRequest, u64) + Send + Sync>,
        settings: Arc<dyn RecordingSettingsStore>,
    ) -> Self {
        Self {
            run,
            settings,
            seen: Mutex::new(HashSet::new()),
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
        let run = self.run.clone();
        let dir = rec.dir.clone();
        let started = rec.started_at_ms.max(0) as u64;
        let _ = std::thread::Builder::new()
            .name("bottle-rec-pipeline".into())
            .spawn(move || {
                run(req, started);
                remove_dir_if_empty(&dir);
            });
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
    let temp_parts: mog_core::recording_cloud::contract::TempParts =
        Arc::new(move |name: &str| temp_parts_of(temp_root.as_deref(), name));
    let app = handle.clone();
    let on_job = Arc::new(move |s: &mog_core::types::RecordingSummary| {
        if let (Some(ch), Ok(v)) = (app.try_state::<BrainChannel>(), serde_json::to_value(s)) {
            ch.send(events::RECORDING_JOB, v);
        }
    });
    let adapter = Arc::new(CloudAdapter::new(
        service.clone(),
        store.clone(),
        temp_parts,
        on_job,
    ));
    // PR B's finished recordings run the pipeline once each (waiting ones go out now).
    if let Some(slot) = handle.try_state::<Arc<HandoffSlot>>() {
        let a = adapter.clone();
        slot.connect(Arc::new(PipelineHandoff::new(
            Arc::new(move |req, started| {
                let _ = a.run_pipeline(&req, started);
            }),
            store.clone(),
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
