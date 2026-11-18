//! T12 consent record tests (#63). They use the in-memory key vault double, a temp folder and
//! the simulated wall clock. They never touch the real OS key store, a Databricks profile or a
//! token. Every ID and name here is plainly fake.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::*;
use crate::clock::{SimWallClock, WallClock};
use crate::local_store::key_vault::MemoryKeyVault;
use crate::local_store::{LocalStore, SqlCipherLocalStore};

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";
const DEVICE: &str = "d0d0d0d0-0000-4000-8000-00000000fa4e";
const SESSION_1: &str = "5e550000-0000-4000-8000-000000000001";
const SESSION_2: &str = "5e550000-0000-4000-8000-000000000002";
const USER: &str = "fake.user@example.invalid";

/// 2025-01-15T12:00:00Z in Unix milliseconds.
const T0: i64 = 1_736_942_400_000;
const DAY_MS: i64 = 86_400_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-t12-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    _dir: TempDir,
    store: Arc<SqlCipherLocalStore>,
    clock: SimWallClock,
    consent: ConsentRecords,
}

fn fixture(tag: &str, retention: ConsentRetention) -> Fixture {
    let dir = TempDir::new(tag);
    let store = Arc::new(SqlCipherLocalStore::new(
        dir.path().join("memory"),
        Arc::new(MemoryKeyVault::new()),
    ));
    store.open().expect("open");
    let clock = SimWallClock::at(T0);
    let consent = ConsentRecords::new(
        Arc::clone(&store),
        Arc::new(clock.clone()),
        DEVICE,
        retention,
    )
    .expect("consent records");
    Fixture {
        _dir: dir,
        store,
        clock,
        consent,
    }
}

fn attestation<'a>(mog: &'a str, session: &'a str) -> SessionAttestation<'a> {
    SessionAttestation {
        mog_id: mog,
        session_id: session,
        kind: AttestKind::OthersAgreed,
        notice_version: 3,
        user: Some(USER),
        disclosure_copied: true,
    }
}

fn count(s: &SqlCipherLocalStore, sql: &str) -> i64 {
    s.with_conn(|c| c.query_row(sql, [], |r| r.get(0))).unwrap()
}

fn seed_mog_session_and_utterance(s: &SqlCipherLocalStore, mog: &str, session: &str, at: i64) {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO mog (mog_id, display_name, backend_kind, row_hlc, origin_device_id)
             VALUES (?1, 'Fake MOG', 'genie_one_mcp', 1, ?2)",
            rusqlite::params![mog, DEVICE],
        )?;
        c.execute(
            "INSERT INTO session (session_id, mog_id, kind, started_at, row_hlc, origin_device_id)
             VALUES (?1, ?2, 'meeting', ?3, 1, ?4)",
            rusqlite::params![session, mog, at, DEVICE],
        )?;
        c.execute(
            "INSERT INTO utterance (utterance_id, session_id, mog_id, seq, speaker_track,
               started_at, duration_ms, text_clean, route, stt_provider, row_hlc, origin_device_id)
             VALUES ('fake-utt-1', ?1, ?2, 1, 'you', ?3, 1000, 'fake transcript line',
               'context', 'local_parakeet', 1, ?4)",
            rusqlite::params![session, mog, at, DEVICE],
        )?;
        Ok(())
    })
    .unwrap();
}

// ---- SEC-32a, AT-24: what a record holds ---------------------------------------------------

#[test]
fn sec_32a_at_24_notice_record_has_user_device_version_and_time() {
    let f = fixture("notice", ConsentRetention::default());
    let r = f.consent.record_notice(Some(USER), 3).unwrap();
    assert_eq!(r.kind, ConsentKind::Notice);
    assert_eq!(r.user.as_deref(), Some(USER));
    assert_eq!(r.device, DEVICE);
    assert_eq!(r.notice_version, 3);
    assert_eq!(r.accepted_at, T0);
    assert_eq!(r.mog_id, None, "a notice consent is device-wide");
    assert_eq!(r.session_id, None);
    assert_eq!(r.attestation_kind, None);
    // It is in the local store, not only in the reply.
    assert_eq!(f.consent.records().unwrap(), vec![r]);
    assert_eq!(f.consent.latest_notice_version().unwrap(), Some(3));
}

#[test]
fn sec_32a_at_24_session_record_has_session_id_and_attestation_kind() {
    let f = fixture("session", ConsentRetention::default());
    let r = f
        .consent
        .record_session(&attestation(MOG_A, SESSION_1))
        .unwrap();
    assert_eq!(r.kind, ConsentKind::Session);
    assert_eq!(r.mog_id.as_deref(), Some(MOG_A));
    assert_eq!(r.session_id.as_deref(), Some(SESSION_1));
    assert_eq!(r.attestation_kind, Some(AttestKind::OthersAgreed));
    assert_eq!(r.notice_version, 3);
    assert_eq!(r.user.as_deref(), Some(USER));
    assert_eq!(r.device, DEVICE);
    assert_eq!(r.disclosure_copied, Some(true));
    assert_eq!(r.accepted_at, T0);
    assert_eq!(f.consent.session_record(MOG_A, SESSION_1).unwrap(), Some(r));
    // MEM-01: another MOG does not see the record.
    assert_eq!(f.consent.session_record(MOG_B, SESSION_1).unwrap(), None);
}

#[test]
fn sec_32a_at_24_only_me_and_no_user_are_stored_as_given() {
    let f = fixture("onlyme", ConsentRetention::default());
    let mut a = attestation(MOG_A, SESSION_2);
    a.kind = AttestKind::OnlyMe;
    a.user = None;
    a.disclosure_copied = false;
    let r = f.consent.record_session(&a).unwrap();
    assert_eq!(r.attestation_kind, Some(AttestKind::OnlyMe));
    assert_eq!(r.user, None);
    assert_eq!(r.disclosure_copied, Some(false));
}

#[test]
fn sec_32a_record_rejects_ids_that_are_not_uuids() {
    let f = fixture("ids", ConsentRetention::default());
    assert_eq!(
        f.consent
            .record_session(&attestation("not-a-mog", SESSION_1))
            .unwrap_err(),
        CoreError::MogIdRequired { component: "R10" }
    );
    assert!(f
        .consent
        .record_session(&attestation(MOG_A, "not-a-session"))
        .is_err());
    assert_eq!(count(&f.store, "SELECT count(*) FROM consent_record"), 0);
    let bad_device = ConsentRecords::new(
        Arc::clone(&f.store),
        Arc::new(f.clock.clone()),
        "",
        ConsentRetention::default(),
    );
    assert!(bad_device.is_err());
}

#[test]
fn sec_32a_at_24_record_outlives_session_and_transcript_delete() {
    let f = fixture("outlive", ConsentRetention::default());
    seed_mog_session_and_utterance(&f.store, MOG_A, SESSION_1, T0);
    f.consent
        .record_session(&attestation(MOG_A, SESSION_1))
        .unwrap();
    // "Stop and delete this session" (SEC-14a) and a Wipe delete the transcript, then the
    // session, then the MOG. None of them may take the consent record with it.
    f.store
        .with_conn(|c| {
            c.execute("DELETE FROM utterance WHERE mog_id = ?1", [MOG_A])?;
            c.execute("DELETE FROM session WHERE mog_id = ?1", [MOG_A])?;
            c.execute("DELETE FROM mog WHERE mog_id = ?1", [MOG_A])
        })
        .unwrap();
    assert_eq!(count(&f.store, "SELECT count(*) FROM utterance"), 0);
    assert_eq!(count(&f.store, "SELECT count(*) FROM session"), 0);
    let kept = f.consent.session_record(MOG_A, SESSION_1).unwrap();
    assert!(kept.is_some(), "the consent record outlives the transcript");
}

#[test]
fn sec_32a_record_holds_no_content_columns() {
    let f = fixture("cols", ConsentRetention::default());
    let mut cols: Vec<String> = f
        .store
        .with_conn(|c| {
            let mut st = c.prepare("SELECT name FROM pragma_table_info('consent_record')")?;
            let v = st
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(v)
        })
        .unwrap();
    cols.sort();
    // Only identifiers, versions, choices and times: no transcript, answer or audio column.
    assert_eq!(
        cols,
        [
            "attestation_kind",
            "consent_id",
            "device_id",
            "disclosure_copied",
            "expires_at",
            "kind",
            "mog_id",
            "notice_version",
            "owner_principal",
            "recorded_at",
            "server_seq",
            "session_id",
            "updated_at",
        ]
    );
}

// ---- SEC-15b, AT-16: the 13-month expiry ----------------------------------------------------

#[test]
fn sec_15b_at_16_fourteen_month_record_is_gone() {
    let f = fixture("14m", ConsentRetention::default());
    let notice = f.consent.record_notice(Some(USER), 3).unwrap();
    f.consent
        .record_session(&attestation(MOG_A, SESSION_1))
        .unwrap();
    assert_eq!(notice.expires_at, add_months_utc(T0, 13));
    f.clock.set(add_months_utc(T0, 14));
    assert_eq!(f.consent.purge_expired().unwrap(), 2);
    assert!(f.consent.records().unwrap().is_empty());
    assert_eq!(f.consent.session_record(MOG_A, SESSION_1).unwrap(), None);
    assert_eq!(f.consent.latest_notice_version().unwrap(), None);
}

#[test]
fn sec_15b_twelve_month_record_is_kept() {
    let f = fixture("12m", ConsentRetention::default());
    f.consent.record_notice(Some(USER), 3).unwrap();
    f.clock.set(add_months_utc(T0, 12));
    assert_eq!(f.consent.purge_expired().unwrap(), 0);
    // One day before the 13 months end, the record is still there.
    f.clock.set(add_months_utc(T0, 13) - DAY_MS);
    assert_eq!(f.consent.purge_expired().unwrap(), 0);
    assert_eq!(f.consent.records().unwrap().len(), 1);
    // At 13 months it expires.
    f.clock.set(add_months_utc(T0, 13));
    assert_eq!(f.consent.purge_expired().unwrap(), 1);
}

#[test]
fn sec_15b_purge_keeps_newer_records() {
    let f = fixture("mixed", ConsentRetention::default());
    f.consent.record_notice(Some(USER), 3).unwrap();
    f.clock.set(add_months_utc(T0, 6));
    let newer = f
        .consent
        .record_session(&attestation(MOG_A, SESSION_1))
        .unwrap();
    f.clock.set(add_months_utc(T0, 14));
    assert_eq!(f.consent.purge_expired().unwrap(), 1);
    assert_eq!(f.consent.records().unwrap(), vec![newer]);
}

#[test]
fn sec_15b_admin_range_is_12_to_84_months() {
    assert_eq!(ConsentRetention::default().get(), 13);
    assert_eq!(DEFAULT_RETENTION_MONTHS, 13);
    for bad in [0, 1, 11, 85, 120, u32::MAX] {
        assert_eq!(
            ConsentRetention::months(bad).unwrap_err(),
            CoreError::Unavailable {
                component: "T12",
                reason: "consent_retention_out_of_range"
            },
            "{bad} months"
        );
    }
    for good in [12, 13, 36, 84] {
        assert_eq!(ConsentRetention::months(good).unwrap().get(), good);
    }
}

#[test]
fn sec_15b_84_month_policy_keeps_a_14_month_record() {
    let f = fixture("84m", ConsentRetention::months(84).unwrap());
    let r = f.consent.record_notice(Some(USER), 3).unwrap();
    assert_eq!(r.expires_at, add_months_utc(T0, 84));
    f.clock.set(add_months_utc(T0, 14));
    assert_eq!(f.consent.purge_expired().unwrap(), 0);
    f.clock.set(add_months_utc(T0, 84));
    assert_eq!(f.consent.purge_expired().unwrap(), 1);
}

#[test]
fn sec_15b_12_month_policy_expires_at_12_months() {
    let f = fixture("12p", ConsentRetention::months(12).unwrap());
    f.consent.record_notice(Some(USER), 3).unwrap();
    f.clock.set(add_months_utc(T0, 12) - 1);
    assert_eq!(f.consent.purge_expired().unwrap(), 0);
    f.clock.set(add_months_utc(T0, 12));
    assert_eq!(f.consent.purge_expired().unwrap(), 1);
}

#[test]
fn sec_15b_mem_33a_purge_deletes_no_user_content() {
    let f = fixture("u9", ConsentRetention::default());
    seed_mog_session_and_utterance(&f.store, MOG_A, SESSION_1, T0);
    f.consent
        .record_session(&attestation(MOG_A, SESSION_1))
        .unwrap();
    f.clock.set(add_months_utc(T0, 40));
    assert_eq!(f.consent.purge_expired().unwrap(), 1);
    // U9: the transcript, the session and the MOG are older still, and stay.
    assert_eq!(count(&f.store, "SELECT count(*) FROM utterance"), 1);
    assert_eq!(count(&f.store, "SELECT count(*) FROM session"), 1);
    assert_eq!(count(&f.store, "SELECT count(*) FROM mog"), 1);
}

#[test]
fn sec_15b_calendar_months_are_utc_and_clamp_the_day() {
    // 2025-01-31T00:00:00Z + 1 month = 2025-02-28T00:00:00Z.
    let jan_31 = 1_738_281_600_000;
    let feb_28 = 1_740_700_800_000;
    assert_eq!(add_months_utc(jan_31, 1), feb_28);
    // 2024-02-29T06:00:00Z + 12 months = 2025-02-28T06:00:00Z.
    let leap = 1_709_186_400_000;
    assert_eq!(add_months_utc(leap, 12), 1_740_722_400_000);
    // 2025-01-15T12:00:00Z + 13 months = 2026-02-15T12:00:00Z; - 13 months goes back.
    assert_eq!(add_months_utc(T0, 13), 1_771_156_800_000);
    assert_eq!(add_months_utc(add_months_utc(T0, 13), -13), T0);
    // 1969-12-15T00:00:00Z + 1 month = 1970-01-15T00:00:00Z.
    assert_eq!(add_months_utc(-1_468_800_000, 1), 1_209_600_000);
}

#[test]
fn sec_15b_sim_wall_clock_moves_only_on_set_or_advance() {
    let c = SimWallClock::at(T0);
    assert_eq!(c.now_ms(), T0);
    assert_eq!(c.now_ms(), T0);
    c.advance_ms(DAY_MS);
    assert_eq!(c.now_ms(), T0 + DAY_MS);
    c.set(5);
    assert_eq!(c.clone().now_ms(), 5);
}

// ---- MEM-01: the mog_id guard and device-wide records --------------------------------------

#[test]
fn mem_01_device_wide_reads_return_only_consent_fields() {
    // A device-wide read crosses MOGs on purpose (the hub's consent list), so it must return
    // only content-free consent fields.
    let f = fixture("wide", ConsentRetention::default());
    f.consent
        .record_session(&attestation(MOG_A, SESSION_1))
        .unwrap();
    f.consent
        .record_session(&attestation(MOG_B, SESSION_2))
        .unwrap();
    f.consent.record_notice(None, 4).unwrap();
    let all = f.consent.records().unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(f.consent.latest_notice_version().unwrap(), Some(4));
}

// ---- #431: the device's records in the app data folder ---------------------------------------

#[test]
fn sec_32a_device_id_is_made_once_and_kept() {
    let dir = TempDir::new("device-once");
    let first = device_id_in(dir.path()).unwrap();
    assert!(is_uuid(&first), "{first}");
    assert_eq!(first.as_bytes()[14], b'4', "a UUID v4");
    let file = dir.path().join(DEVICE_ID_FILE);
    assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), first);
    // The next start reads the same ID; it never makes a second one.
    assert_eq!(device_id_in(dir.path()).unwrap(), first);
    // Another device folder gets another ID.
    let other = TempDir::new("device-other");
    assert_ne!(device_id_in(other.path()).unwrap(), first);
}

#[test]
fn sec_32a_a_damaged_device_id_fails_closed_and_is_kept() {
    let dir = TempDir::new("device-bad");
    let file = dir.path().join(DEVICE_ID_FILE);
    for bad in ["", "not-a-uuid", "test-token-not-real", "d0d0d0d0-0000-4000-8000"] {
        std::fs::write(&file, bad).unwrap();
        assert!(device_id_in(dir.path()).is_err(), "{bad:?}");
        // Nothing is overwritten: the old records keep their device.
        assert_eq!(std::fs::read_to_string(&file).unwrap(), bad);
    }
}

#[test]
fn sec_32a_at_24_device_records_open_in_the_memory_folder_and_outlive_a_restart() {
    let dir = TempDir::new("device-open");
    let vault: Arc<MemoryKeyVault> = Arc::new(MemoryKeyVault::new());
    let clock = SimWallClock::at(T0);
    let (store, records) = open_device_records(
        dir.path(),
        vault.clone(),
        Arc::new(clock.clone()),
        ConsentRetention::default(),
    )
    .unwrap();
    assert_eq!(store.db_path(), dir.path().join(MEMORY_DIR).join("memory.db"));
    let r = records.record_notice(None, 2).unwrap();
    let device = std::fs::read_to_string(dir.path().join(DEVICE_ID_FILE)).unwrap();
    assert_eq!(r.device, device.trim());
    assert_eq!(r.accepted_at, T0);
    store.close();
    drop(records);
    // A restart opens the same file with the same key and the same device.
    let (_store, again) = open_device_records(
        dir.path(),
        vault,
        Arc::new(clock),
        ConsentRetention::default(),
    )
    .unwrap();
    assert_eq!(again.records().unwrap(), vec![r]);
    assert_eq!(again.latest_notice_version().unwrap(), Some(2));
}

#[test]
fn sec_03_device_records_do_not_open_without_the_key() {
    // The OS key store is not there (an unsigned macOS build, Linux): nothing opens, and the
    // consent_* commands answer consent_storage_failed. No key and no record is made.
    struct NoVault;
    impl crate::local_store::key_vault::KeyVault for NoVault {
        fn load(&self) -> CoreResult<Option<crate::local_store::key_vault::LocalKey>> {
            Err(CoreError::KeychainUnavailable { component: "R10" })
        }
        fn store(&self, _k: &crate::local_store::key_vault::LocalKey) -> CoreResult<()> {
            Err(CoreError::KeychainUnavailable { component: "R10" })
        }
    }
    let dir = TempDir::new("device-nokey");
    let got = open_device_records(
        dir.path(),
        Arc::new(NoVault),
        Arc::new(SimWallClock::at(T0)),
        ConsentRetention::default(),
    );
    assert!(got.is_err());
    assert!(!dir.path().join(MEMORY_DIR).join("memory.db").exists());
}
