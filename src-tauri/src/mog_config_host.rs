//! The host side of the MOG config store (#25, MB-02, ADR 0035).
//!
//! The store lives in `<app config dir>/mogs`. The host finds the app config dir with the Tauri
//! path API in Rust, so no window gets the `fs` plugin (ARC-4). `mog_core::mog_config` does the
//! checks and the atomic writes.
//!
//! The Tauri invoke handler (`ipc.rs`, contract) still calls `mog_core::ipc::handle`, which ends
//! `config_*` in `not_implemented`. Contract change #333 routes the host through
//! `mog_core::ipc_gateway::hardened_gateway_with`; then `config_*` go to
//! `mog_core::mog_config::MogConfigComponent` over this store: TODO(#333).

use mog_core::mog_config::MogConfigStore;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{Manager, Runtime};

/// The folder of the config files in the app config dir.
pub const STORE_DIR: &str = "mogs";

/// The store folder for the app config dir `app_config`.
pub fn store_dir(app_config: &Path) -> PathBuf {
    app_config.join(STORE_DIR)
}

/// Put one `Arc<MogConfigStore>` in managed state. The folder is made on the first write.
pub fn install<R: Runtime>(app: &tauri::AppHandle<R>) -> tauri::Result<()> {
    let dir = store_dir(&app.path().app_config_dir()?);
    app.manage(Arc::new(MogConfigStore::new(dir)));
    Ok(())
}
