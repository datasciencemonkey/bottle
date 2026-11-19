//! The one R5 CliRunner of the process (#16).
//!
//! [`install`] builds one [`DatabricksCliRunner::system`] and manages it as [`SharedCli`]. T7
//! ([`crate::profiles_host`]) runs `auth describe` on it, and R6 ([`crate::mcp_host`]) runs
//! `auth token --profile <p>` on it. R5 never picks a profile: the caller passes it.
//!
//! [`start`] builds the `cli_status` component ([`CliStatusHost`]) and runs the launch check on a
//! worker thread (W-02a..W-02d), so a hung CLI cannot block the app start. The record of the
//! verified path and version (no secret) lives in `<app config dir>/cli-record.json`. A missing
//! or refused CLI never fails the start: `cli_status` then says `found: false`.
//!
//! The R5 policy comes from the MDM keys `CliPath` and `CliMinVersion` through R13. The app has
//! no MDM source yet, so R13 is the stub and the default policy holds. TODO(#186): read MDM.
//!
//! The host routes `cli_status` through `HostComponents` in `src-tauri/src/ipc.rs`, a contract
//! file, and the reply has no field for the launch warnings: TODO(#570). Until then the brain
//! gets `not_implemented`, and the warnings wait in [`CliStatusComponent::warnings`].

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mog_core::cli_runner::DatabricksCliRunner;
use mog_core::cli_status::{
    cli_policy, CliRecordFile, CliStatusComponent, PlaintextStorage, RECORD_FILE,
};
use mog_core::policy_store::StubPolicyStore;
use tauri::Manager;

use crate::profiles_host::ProfilesHost;

/// The one R5 of the process, shared by T7 and R6.
#[derive(Clone)]
pub struct SharedCli(pub Arc<DatabricksCliRunner>);

/// The `cli_status` component of the app.
#[derive(Debug, Clone)]
pub struct CliStatusHost(pub Arc<CliStatusComponent>);

/// The record file for the app config dir `app_config`.
pub fn record_path(app_config: &Path) -> PathBuf {
    app_config.join(RECORD_FILE)
}

/// Build and manage the one R5. It runs no CLI child yet.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {
    // TODO(#186): the MDM `CliPath` and `CliMinVersion` from the real R13.
    let policy = cli_policy(&StubPolicyStore);
    handle.manage(SharedCli(Arc::new(DatabricksCliRunner::system(policy))));
}

/// Manage the `cli_status` component and run the launch check on a worker (W-02d).
/// [`install`] and [`crate::profiles_host::install`] run first.
pub fn start(handle: &tauri::AppHandle<tauri::Wry>) {
    let Some(cli) = handle.try_state::<SharedCli>().map(|c| c.0.clone()) else {
        return;
    };
    let record = CliRecordFile::new(handle.path().app_config_dir().ok().map(|d| record_path(&d)));
    let plaintext = handle
        .try_state::<ProfilesHost>()
        .map(|t7| t7.0.clone() as Arc<dyn PlaintextStorage>);
    let status = Arc::new(CliStatusComponent::new(cli, record, plaintext));
    handle.manage(CliStatusHost(status.clone()));
    // If the worker cannot start, `cli_status` says `found: false` until R5 runs a command.
    let _ = std::thread::Builder::new()
        .name("cli-launch-check".into())
        .spawn(move || {
            status.launch();
        });
}
