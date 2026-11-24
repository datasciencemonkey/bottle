//! The host side of the T12 consent records (#431, ADR 0052, 0060, 0071).
//!
//! At start the host prepares the one R10 local store in `<app local data dir>/memory`, and reads
//! the device ID in `<app local data dir>/device-id`. It does not read the OS key store: the
//! store opens on its first use, after the hub shows why macOS asks (ADR 0105). The key is read
//! once per launch and kept in memory (`CachedKeyVault`)
//! (`mog_core::consent::prepare_device_records`). It manages the store as `Arc<SqlCipherLocalStore>`
//! and the records as `Arc<ConsentRecords>`, the type `ipc::route_from` reads for
//! `consent_notice_record` and `consent_records_list`. Only the brain may call them (ADR 0060).
//! No view gets the store: no `fs` plugin, no path, no SQL (ARC-4).
//!
//! If the folder is not there, nothing is managed. If the key store refuses (the user chose
//! Deny, or no key store), each use fails: `consent_*` answers `consent_storage_failed`, and the
//! brain keeps the notice gate closed: no capture before a stored acceptance (MB-08a).
//! Known gap: 9.3 step 5 and SEC-32a need the signed-in `user` in each record. The host still
//! builds the component with `NoSignedInUser` (contract file `src-tauri/src/ipc.rs`), so a
//! record has no user. TODO(#478): the host reads a managed user from T7 ProfileService (#17).
//!
//! R14 WipeService (#124, ADR 0087) shares the same store. The host builds it with the consent
//! records as a content-free expiry, runs `run_security_expiries` once after the first open (9.2:
//! security data keeps its expiry), and manages it as `Arc<LocalWipeService>` for the `mem_wipe`
//! route. This start call deletes no user content (U9). There is no timer.
//! `mem_local_exec` reads the same managed store (#426), so there is one store per app.
//! TODO(#525): `src-tauri/src/ipc.rs` routes `mem_wipe` to `mog_core::wipe::mem_wipe`.
//! TODO(#448): add the audit log as an expiry once the host keeps one.

use std::path::Path;
use std::sync::Arc;

use mog_core::clock::{SystemWallClock, WallClock};
use mog_core::consent::{prepare_device_records, ConsentRetention};
use mog_core::error::{CoreError, CoreResult};
use mog_core::local_store::key_vault::{os_key_vault, KeyVault};
use mog_core::wipe::{AgentWipeHook, AgentWipeSlot, LocalWipeService, SecurityExpiry, WipeService};
use tauri::{Manager, Runtime};

/// Open the records in `dir` and put the store and the records in managed state. On an error
/// nothing is managed.
pub fn install_with<R: Runtime>(
    app: &tauri::AppHandle<R>,
    dir: &Path,
    vault: Arc<dyn KeyVault>,
    clock: Arc<dyn WallClock>,
) -> CoreResult<()> {
    let (store, records) =
        prepare_device_records(dir, vault, Arc::clone(&clock), ConsentRetention::default())?;
    let records = Arc::new(records);
    // Plan A3.7: every Wipe scope first ends the agent session and drops its history.
    // The slot is empty until `agent_session_host::install` binds the orchestrator host.
    let agent_slot = Arc::new(AgentWipeSlot::default());
    let wipe = Arc::new(
        LocalWipeService::new(Arc::clone(&store), clock)
            .with_expiry(Arc::clone(&records) as Arc<dyn SecurityExpiry>)
            .with_agent_hook(Arc::clone(&agent_slot) as Arc<dyn AgentWipeHook>),
    );
    // 9.2: the content-free expiries run once, right after the first open (ADR 0105). A failure
    // leaves the records for the next run. The hook holds the service weakly: no cycle.
    let weak = Arc::downgrade(&wipe);
    store.on_first_open(Box::new(move || {
        if let Some(w) = weak.upgrade() {
            let _ = w.run_security_expiries();
        }
    }));
    app.manage(store);
    app.manage(records);
    app.manage(wipe);
    app.manage(agent_slot);
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
