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
//! Known gap: 9.3 step 5 and SEC-32a need the signed-in `user` in each record. The host still
//! builds the component with `NoSignedInUser` (contract file `src-tauri/src/ipc.rs`), so a
//! record has no user. TODO(#478): the host reads a managed user from T7 ProfileService (#17).
//!
//! R14 WipeService (#124, ADR 0087) shares the same store. The host builds it with the consent
//! records as a content-free expiry, runs `run_security_expiries` once here at start (9.2:
//! security data keeps its expiry), and manages it as `Arc<LocalWipeService>` for the `mem_wipe`
//! route. This start call deletes no user content (U9). There is no timer.
//! TODO(#525): `src-tauri/src/ipc.rs` routes `mem_wipe` to `mog_core::wipe::mem_wipe`.
//! TODO(#448): add the audit log as an expiry once the host keeps one.

use std::path::Path;
use std::sync::Arc;

use mog_core::clock::{SystemWallClock, WallClock};
use mog_core::consent::{open_device_records, ConsentRecords, ConsentRetention};
use mog_core::error::{CoreError, CoreResult};
use mog_core::local_store::key_vault::{os_key_vault, KeyVault};
use mog_core::local_store::SqlCipherLocalStore;
use mog_core::wipe::{LocalWipeService, SecurityExpiry, WipeService};
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
    let (store, records) = open(dir, vault, Arc::clone(&clock))?;
    let wipe = LocalWipeService::new(Arc::clone(&store), clock)
        .with_expiry(Arc::clone(&records) as Arc<dyn SecurityExpiry>);
    // 9.2: the content-free expiries run at start. A failure leaves the records for the next run.
    let _ = wipe.run_security_expiries();
    app.manage(store);
    app.manage(records);
    app.manage(Arc::new(wipe));
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
