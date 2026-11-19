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

use std::sync::Arc;

use mog_core::clock::SystemWallClock;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport, ThreadPause};
use mog_core::ipc::events;
use mog_core::recording::{RecordingComponent, RecordingSettingsStore};
use mog_core::recording_cloud::cloud::{
    is_audio_file_of, CloudDeps, LocalPart, ProfileWorkspace, RecordingCloudService,
};
use mog_core::recording_cloud::contract::CloudAdapter;
use mog_core::recording_cloud::opener::{open_link, SystemOpener, UrlOpener};
use mog_core::recording_cloud::settings_store::{JsonSettingsStore, SETTINGS_FILE};
use tauri::Manager;

use crate::ipc::BrainChannel;
use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;

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
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
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
        store,
        temp_parts,
        on_job,
    ));
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
