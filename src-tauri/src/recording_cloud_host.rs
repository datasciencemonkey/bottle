//! The cloud half of call recording in the app (ADR 0103, PR C): one
//! [`RecordingCloudService`] on its own R7 [`ReqwestHttpTransport`], over the one R6 of
//! [`crate::mcp_host`] (the token never leaves R6), the managed T7 profile hosts and the one
//! [`SharedRateBucket`] (G-09a). The recording Settings live in
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
use mog_core::recording_cloud::cloud::{CloudDeps, ProfileWorkspace, RecordingCloudService};
use mog_core::recording_cloud::opener::{open_link, SystemOpener, UrlOpener};
use mog_core::recording_cloud::settings_store::{SettingsFile, SETTINGS_FILE};
use tauri::Manager;

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
    /// The Settings file.
    pub settings: SettingsFile,
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
    let settings = match handle.path().app_config_dir() {
        Ok(dir) => SettingsFile::new(dir.join(SETTINGS_FILE)),
        Err(_) => SettingsFile::none(),
    };
    handle.manage(RecordingCloudHost {
        service,
        settings,
        opener: Arc::new(SystemOpener),
        _transport: transport,
    });
    Ok(())
}
