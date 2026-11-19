//! #431: the host opens the device's consent records (ADR 0052, 0060, 0071). The brain T12
//! ConsentService reaches them only through `consent_notice_record` and `consent_records_list`.
//! The tests use a key vault double and a temp folder: no OS key store, no profile, no token.

use bottle_lib::consent_host::install_with;
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
    fn notify_hub(&self, _notice: Value) -> bool {
        false
    }
    fn subscribe_hub(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

/// Open the records in `dir` now, kept for the default 13 months (SEC-15b), as a test seeds or
/// reads them. The host itself opens the store on first use (`install_with`, ADR 0105).
fn open(
    dir: &std::path::Path,
    vault: Arc<dyn KeyVault>,
    clock: Arc<dyn mog_core::clock::WallClock>,
) -> CoreResult<(Arc<SqlCipherLocalStore>, Arc<ConsentRecords>)> {
    let (store, records) = mog_core::consent::open_device_records(
        dir,
        vault,
        clock,
        mog_core::consent::ConsentRetention::default(),
    )?;
    Ok((store, Arc::new(records)))
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
        consent: Some(&consent),
        ..HostComponents::none("brain", &NoBrain)
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
fn sec_03_with_no_key_store_the_records_fail_closed_on_first_use() {
    let dir = TempDir::new("nokey");
    let app = tauri::test::mock_app();
    // ADR 0105: install reads no key, so it succeeds; the first use fails with the fixed code,
    // and the brain keeps the notice gate closed.
    install_with(
        app.handle(),
        &dir.0,
        Arc::new(NoVault),
        Arc::new(SimWallClock::at(T0)),
    )
    .unwrap();
    let records = app.try_state::<Arc<ConsentRecords>>().expect("records");
    assert_eq!(
        brain_call(
            &records,
            "consent_notice_record",
            json!({"noticeVersion": 1})
        ),
        Err(json!({"error": "consent_storage_failed"}))
    );
    assert_eq!(
        brain_call(&records, "consent_records_list", json!({})),
        Err(json!({"error": "consent_storage_failed"}))
    );
}

/// A vault double that counts its reads.
#[derive(Default)]
struct CountingVault {
    inner: TestVault,
    loads: std::sync::atomic::AtomicUsize,
}

impl KeyVault for CountingVault {
    fn load(&self) -> CoreResult<Option<LocalKey>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        self.inner.load()
    }
    fn store(&self, key: &LocalKey) -> CoreResult<()> {
        self.inner.store(key)
    }
}

#[test]
fn mem_08b_install_reads_no_key_and_the_first_uses_share_one_read() {
    let dir = TempDir::new("lazy");
    let app = tauri::test::mock_app();
    let vault = Arc::new(CountingVault::default());
    install_with(
        app.handle(),
        &dir.0,
        Arc::clone(&vault) as Arc<dyn KeyVault>,
        Arc::new(SimWallClock::at(T0)),
    )
    .unwrap();
    // ADR 0105: setup does not touch the key store, so macOS shows no prompt before the hub.
    assert_eq!(vault.loads.load(Ordering::SeqCst), 0);
    let records = app.try_state::<Arc<ConsentRecords>>().expect("records");
    brain_call(&records, "consent_records_list", json!({})).unwrap();
    brain_call(
        &records,
        "consent_notice_record",
        json!({"noticeVersion": 1}),
    )
    .unwrap();
    brain_call(&records, "consent_records_list", json!({})).unwrap();
    assert_eq!(vault.loads.load(Ordering::SeqCst), 1);
}

#[test]
fn sec_15a_sec_15b_u9_install_runs_the_security_expiries_once_and_manages_r14() {
    use mog_core::wipe::{LocalWipeService, WipeService};
    let dir = TempDir::new("expiry");
    let vault: Arc<dyn KeyVault> = Arc::new(TestVault::default());
    // A notice record of 14 months ago and one of today (#124, SEC-15b).
    let old = mog_core::consent::add_months_utc(T0, -14);
    {
        let (store, records) =
            open(&dir.0, Arc::clone(&vault), Arc::new(SimWallClock::at(old))).unwrap();
        records.record_notice(None, 1).unwrap();
        store.close();
    }
    let app = tauri::test::mock_app();
    install_with(app.handle(), &dir.0, vault, Arc::new(SimWallClock::at(T0))).unwrap();
    // ADR 0105: the first open (not setup) ran the content-free expiries once: the 14-month
    // record is gone.
    let records = app.try_state::<Arc<ConsentRecords>>().expect("records");
    assert!(records.records().unwrap().is_empty());
    // R14 is managed for the `mem_wipe` route (TODO(#525)). Its expiries hold the records.
    let wipe = app
        .try_state::<Arc<LocalWipeService>>()
        .expect("wipe service");
    let report = wipe.run_security_expiries().unwrap();
    assert_eq!(report.expired, vec![("consent_records", 0)]);
}
