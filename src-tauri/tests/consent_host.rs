//! #431: the host opens the device's consent records (ADR 0052, 0060, 0071). The brain T12
//! ConsentService reaches them only through `consent_notice_record` and `consent_records_list`.
//! The tests use a key vault double and a temp folder: no OS key store, no profile, no token.

use bottle_lib::consent_host::{install_with, open};
use bottle_lib::ipc::{route, BrainLink, HostComponents};
use mog_core::clock::SimWallClock;
use mog_core::consent::{ConsentRecords, DEVICE_ID_FILE, MEMORY_DIR};
use mog_core::consent_ipc::{ConsentComponent, NoSignedInUser};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::ChannelRef;
use mog_core::local_store::key_vault::{KeyVault, LocalKey};
use mog_core::local_store::SqlCipherLocalStore;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::Manager;

/// 2026-11-17T12:00:00Z in Unix milliseconds.
const T0: i64 = 1_794_916_800_000;

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

/// A key vault in memory (the OS key store double).
#[derive(Default)]
struct TestVault(Mutex<Option<LocalKey>>);

impl KeyVault for TestVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn store(&self, key: &LocalKey) -> CoreResult<()> {
        *self.0.lock().unwrap() = Some(key.clone());
        Ok(())
    }
}

/// The key store is not there (an unsigned macOS build, Linux CI).
struct NoVault;

impl KeyVault for NoVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        Err(CoreError::KeychainUnavailable { component: "R10" })
    }
    fn store(&self, _key: &LocalKey) -> CoreResult<()> {
        Err(CoreError::KeychainUnavailable { component: "R10" })
    }
}

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "bottle-consent-host-{tag}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Route one brain call with the real consent component over `records`, as `route_from` does.
fn brain_call(records: &Arc<ConsentRecords>, command: &str, args: Value) -> Result<Value, Value> {
    let consent = ConsentComponent::new(records.clone(), Arc::new(NoSignedInUser));
    let components = HostComponents {
        from: "brain",
        brain: &NoBrain,
        windows: None,
        hotkeys: None,
        permissions: None,
        consent: Some(&consent),
        config: None,
        mcp: None,
        app_settings: None,
        capture: None,
        capture_ticket: None,
        insert: None,
    };
    route(components, command, args)
}

#[test]
fn sec_32a_at_24_a_notice_acceptance_writes_one_record_with_device_version_and_time() {
    let dir = TempDir::new("notice");
    let clock = SimWallClock::at(T0);
    let (_store, records) = open(&dir.0, Arc::new(TestVault::default()), Arc::new(clock)).unwrap();
    let reply = brain_call(
        &records,
        "consent_notice_record",
        json!({"noticeVersion": 1}),
    )
    .unwrap();
    let id = reply["recordId"].as_str().unwrap().to_owned();
    assert_eq!(reply, json!({ "recordId": id }));

    // One record in the local store, with the SEC-32a fields that Rust fills.
    let stored = records.records().unwrap();
    assert_eq!(stored.len(), 1);
    let r = &stored[0];
    assert_eq!(r.consent_id, id);
    let device = std::fs::read_to_string(dir.0.join(DEVICE_ID_FILE)).unwrap();
    assert_eq!(
        r.device,
        device.trim(),
        "the device ID made once for this device"
    );
    assert_eq!(r.notice_version, 1);
    assert_eq!(r.accepted_at, T0);
    // TODO(#478): the user comes from the managed SignedInUser once T7 (#17) gives it.
    assert_eq!(r.user, None, "known gap: no user until #478 and T7 (#17)");

    // The brain list is the same record, content-free.
    assert_eq!(
        brain_call(&records, "consent_records_list", json!({})),
        Ok(json!({"records": [{
            "recordId": id,
            "kind": "notice",
            "noticeVersion": 1,
            "sessionId": null,
            "acceptedAt": T0
        }]}))
    );
}

#[test]
fn sec_32a_the_store_and_the_device_id_live_in_the_given_data_dir() {
    let dir = TempDir::new("paths");
    let (store, _records) = open(
        &dir.0,
        Arc::new(TestVault::default()),
        Arc::new(SimWallClock::at(T0)),
    )
    .unwrap();
    assert_eq!(
        store.db_path(),
        dir.0
            .join(MEMORY_DIR)
            .join(mog_core::local_store::DB_FILE_NAME)
    );
    assert!(store.db_path().exists());
    assert!(dir.0.join(DEVICE_ID_FILE).exists());
}

#[test]
fn sec_15b_the_host_keeps_records_for_13_months() {
    let dir = TempDir::new("retention");
    let (_s, records) = open(
        &dir.0,
        Arc::new(TestVault::default()),
        Arc::new(SimWallClock::at(T0)),
    )
    .unwrap();
    assert_eq!(records.retention().get(), 13);
}

#[test]
fn sec_32a_install_manages_the_records_that_the_ipc_host_reads() {
    let dir = TempDir::new("install");
    let app = tauri::test::mock_app();
    install_with(
        app.handle(),
        &dir.0,
        Arc::new(TestVault::default()),
        Arc::new(SimWallClock::at(T0)),
    )
    .unwrap();
    // `ipc::route_from` reads exactly this type with `try_state`.
    let records = app.try_state::<Arc<ConsentRecords>>().expect("records");
    assert!(app.try_state::<Arc<SqlCipherLocalStore>>().is_some());
    assert!(brain_call(&records, "consent_records_list", json!({})).is_ok());
}

#[test]
fn sec_03_with_no_key_store_nothing_is_managed_and_consent_fails_closed() {
    let dir = TempDir::new("nokey");
    let app = tauri::test::mock_app();
    assert!(install_with(
        app.handle(),
        &dir.0,
        Arc::new(NoVault),
        Arc::new(SimWallClock::at(T0)),
    )
    .is_err());
    assert!(app.try_state::<Arc<ConsentRecords>>().is_none());
    assert!(app.try_state::<Arc<SqlCipherLocalStore>>().is_none());
    // So the host answers the fixed code, and the brain keeps the notice gate closed.
    let components = HostComponents {
        from: "brain",
        brain: &NoBrain,
        windows: None,
        hotkeys: None,
        permissions: None,
        consent: None,
        config: None,
        mcp: None,
        app_settings: None,
        capture: None,
        capture_ticket: None,
        insert: None,
    };
    assert_eq!(
        route(
            components,
            "consent_notice_record",
            json!({"noticeVersion": 1})
        ),
        Err(json!({"error": "consent_storage_failed"}))
    );
}
