//! The host side of the T12 consent records (#431, ADR 0052, 0060, 0071).
//!
//! At start the host opens the one R10 local store in `<app local data dir>/memory` with the key
//! from the OS key store, and the device ID in `<app local data dir>/device-id`
//! (`mog_core::consent::open_device_records`). It manages the store as `Arc<SqlCipherLocalStore>`
//! and the records as `Arc<ConsentRecords>`, the type `ipc::route_from` reads for
//! `consent_notice_record` and `consent_records_list`. Only the brain may call them (ADR 0060).
//! No view gets the store: no `fs` plugin, no path, no SQL (ARC-4).
//!
//! If the folder or the key store is not there (for example an unsigned macOS build, #34),
//! nothing is managed. Then `consent_*` answers `consent_storage_failed`, and the brain keeps the
//! notice gate closed: no capture before a stored acceptance (MB-08a).
//! TODO(#478): the host gives the signed-in user from T7 ProfileService (#17); until then the
//! notice record has no user, as before the first sign-in (9.3 step 5).

use std::path::Path;
use std::sync::Arc;

use mog_core::clock::{SystemWallClock, WallClock};
use mog_core::consent::{open_device_records, ConsentRecords, ConsentRetention};
use mog_core::error::{CoreError, CoreResult};
use mog_core::local_store::key_vault::{os_key_vault, KeyVault};
use mog_core::local_store::SqlCipherLocalStore;
use tauri::{Manager, Runtime};

/// Open the consent records in the app data folder `dir`, kept for the default 13 months
/// (SEC-15b). TODO(#186): the admin retention from PolicyStore.
pub fn open(
    dir: &Path,
    vault: Arc<dyn KeyVault>,
    clock: Arc<dyn WallClock>,
) -> CoreResult<(Arc<SqlCipherLocalStore>, Arc<ConsentRecords>)> {
    let (store, records) = open_device_records(dir, vault, clock, ConsentRetention::default())?;
    Ok((store, Arc::new(records)))
}

/// Open the records in `dir` and put the store and the records in managed state. On an error
/// nothing is managed.
pub fn install_with<R: Runtime>(
    app: &tauri::AppHandle<R>,
    dir: &Path,
    vault: Arc<dyn KeyVault>,
    clock: Arc<dyn WallClock>,
) -> CoreResult<()> {
    let (store, records) = open(dir, vault, clock)?;
    app.manage(store);
    app.manage(records);
    Ok(())
}

/// Open the records in the app local data dir (not a roaming folder) with the OS key store.
pub fn install<R: Runtime>(app: &tauri::AppHandle<R>) -> CoreResult<()> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|_| CoreError::StoreUnavailable { component: "R10" })?;
    install_with(
        app,
        &dir,
        Arc::from(os_key_vault()),
        Arc::new(SystemWallClock),
    )
}
