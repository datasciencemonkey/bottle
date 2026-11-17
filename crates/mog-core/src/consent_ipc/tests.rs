//! `consent_*` component tests (#430). They use the in-memory key vault double, a temp folder
//! and the simulated wall clock. Every ID and name here is plainly fake.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::clock::SimWallClock;
use crate::consent::{ConsentRetention, SessionAttestation};
use crate::error::CoreError;
use crate::ipc::AttestKind;
use crate::local_store::key_vault::MemoryKeyVault;
use crate::local_store::{LocalStore, SqlCipherLocalStore};
use crate::types::ConsentRecordView;

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const DEVICE: &str = "d0d0d0d0-0000-4000-8000-00000000fa4e";
const SESSION_1: &str = "5e550000-0000-4000-8000-000000000001";
const USER: &str = "fake.user@example.invalid";

/// 2025-01-15T12:00:00Z in Unix milliseconds.
const T0: i64 = 1_736_942_400_000;
const DAY_MS: i64 = 86_400_000;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p =
            std::env::temp_dir().join(format!("bottle-t12ipc-{tag}-{}-{n}", std::process::id()));
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

struct FakeUser(Option<&'static str>);

impl SignedInUser for FakeUser {
    fn principal(&self) -> Option<String> {
        self.0.map(str::to_owned)
    }
}

struct Fixture {
    _dir: TempDir,
    store: Arc<SqlCipherLocalStore>,
    clock: SimWallClock,
    records: Arc<ConsentRecords>,
    component: ConsentComponent,
}

fn fixture(tag: &str, user: Option<&'static str>) -> Fixture {
    let dir = TempDir::new(tag);
    let store = Arc::new(SqlCipherLocalStore::new(
        dir.path().join("memory"),
        Arc::new(MemoryKeyVault::new()),
    ));
    store.open().expect("open");
    let clock = SimWallClock::at(T0);
    let records = Arc::new(
        ConsentRecords::new(
            Arc::clone(&store),
            Arc::new(clock.clone()),
            DEVICE,
            ConsentRetention::default(),
        )
        .expect("consent records"),
    );
    let component = ConsentComponent::new(Arc::clone(&records), Arc::new(FakeUser(user)));
    Fixture {
        _dir: dir,
        store,
        clock,
        records,
        component,
    }
}

fn code(r: CoreResult<Value>) -> &'static str {
    match r {
        Err(CoreError::IpcRefused { code }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn sec_32a_at_24_notice_record_fills_user_device_and_time_in_rust() {
    let f = fixture("fill", Some(USER));
    f.clock.set(T0 + 5 * DAY_MS);
    f.component
        .call("consent_notice_record", json!({"noticeVersion": 3}))
        .expect("recorded");
    let stored = f.records.records().unwrap();
    assert_eq!(stored.len(), 1);
    let r = &stored[0];
    assert_eq!(r.user.as_deref(), Some(USER), "the user comes from Rust");
    assert_eq!(r.device, DEVICE, "the device comes from Rust");
    assert_eq!(r.accepted_at, T0 + 5 * DAY_MS, "the time comes from Rust");
    assert_eq!(r.notice_version, 3);
    assert_eq!(r.mog_id, None, "a notice record is device-wide");
}

#[test]
fn sec_32a_at_24_notice_record_before_sign_in_has_no_user() {
    let f = fixture("nouser", None);
    f.component
        .call("consent_notice_record", json!({"noticeVersion": 1}))
        .expect("recorded");
    assert_eq!(f.records.records().unwrap()[0].user, None);
}

#[test]
fn sec_32a_notice_reply_is_only_the_record_id() {
    let f = fixture("reply", Some(USER));
    let reply = f
        .component
        .call("consent_notice_record", json!({"noticeVersion": 2}))
        .unwrap();
    let stored = f.records.records().unwrap();
    assert_eq!(reply, json!({ "recordId": stored[0].consent_id }));
    let text = reply.to_string();
    assert!(!text.contains(USER) && !text.contains(DEVICE), "{text}");
}

#[test]
fn sec_32a_notice_version_zero_is_invalid_args() {
    let f = fixture("zero", Some(USER));
    assert_eq!(
        code(
            f.component
                .call("consent_notice_record", json!({"noticeVersion": 0}))
        ),
        "invalid_args"
    );
    assert!(f.records.records().unwrap().is_empty());
}

#[test]
fn sec_32a_records_list_is_content_free() {
    let f = fixture("list", Some(USER));
    f.component
        .call("consent_notice_record", json!({"noticeVersion": 3}))
        .unwrap();
    f.clock.set(T0 + DAY_MS);
    f.records
        .record_session(&SessionAttestation {
            mog_id: MOG_A,
            session_id: SESSION_1,
            kind: AttestKind::OthersAgreed,
            notice_version: 3,
            user: Some(USER),
            disclosure_copied: true,
        })
        .unwrap();
    let stored = f.records.records().unwrap();
    let reply = f.component.call("consent_records_list", json!({})).unwrap();
    assert_eq!(
        reply,
        json!({"records": [
            {
                "recordId": stored[0].consent_id,
                "kind": "notice",
                "noticeVersion": 3,
                "sessionId": null,
                "acceptedAt": T0
            },
            {
                "recordId": stored[1].consent_id,
                "kind": "others_agreed",
                "noticeVersion": 3,
                "sessionId": SESSION_1,
                "acceptedAt": T0 + DAY_MS
            }
        ]})
    );
    let text = reply.to_string();
    for leak in [USER, DEVICE, MOG_A] {
        assert!(!text.contains(leak), "{leak} crossed IPC: {text}");
    }
    // The reply is the shared view type.
    let views: Vec<ConsentRecordView> =
        serde_json::from_value(reply["records"].clone()).expect("ConsentRecordView");
    assert_eq!(views.len(), 2);
}

#[test]
fn sec_15b_records_list_omits_a_purged_record() {
    let f = fixture("purge", Some(USER));
    f.component
        .call("consent_notice_record", json!({"noticeVersion": 1}))
        .unwrap();
    f.clock.set(T0 + 14 * 31 * DAY_MS);
    assert_eq!(f.records.purge_expired().unwrap(), 1);
    assert_eq!(
        f.component.call("consent_records_list", json!({})).unwrap(),
        json!({"records": []})
    );
}

#[test]
fn sec_03_a_store_failure_is_consent_storage_failed() {
    let f = fixture("fail", Some(USER));
    f.store.close();
    assert_eq!(
        code(
            f.component
                .call("consent_notice_record", json!({"noticeVersion": 1}))
        ),
        "consent_storage_failed"
    );
    assert_eq!(
        code(f.component.call("consent_records_list", json!({}))),
        "consent_storage_failed"
    );
}

#[test]
fn sec_03_other_commands_are_not_this_component() {
    let f = fixture("other", Some(USER));
    assert_eq!(
        code(f.component.call("config_list", json!({}))),
        "not_implemented"
    );
}

fn session_record(kind: Option<AttestKind>) -> crate::consent::ConsentRecord {
    crate::consent::ConsentRecord {
        consent_id: "c0c0c0c0-0000-4000-8000-000000000009".into(),
        mog_id: Some(MOG_A.into()),
        user: Some(USER.into()),
        device: DEVICE.into(),
        notice_version: 2,
        kind: crate::consent::ConsentKind::Session,
        session_id: Some(SESSION_1.into()),
        attestation_kind: kind,
        disclosure_copied: Some(false),
        accepted_at: T0,
        expires_at: T0,
    }
}

#[test]
fn sec_32a_session_kinds_map_to_the_attestation_choice_and_none_is_not_guessed() {
    use crate::types::ConsentRecordKind;
    let only_me = view_of(&session_record(Some(AttestKind::OnlyMe))).unwrap();
    assert_eq!(only_me.kind, ConsentRecordKind::OnlyMe);
    assert_eq!(only_me.session_id.as_deref(), Some(SESSION_1));
    let agreed = view_of(&session_record(Some(AttestKind::OthersAgreed))).unwrap();
    assert_eq!(agreed.kind, ConsentRecordKind::OthersAgreed);
    assert_eq!(view_of(&session_record(None)), None);
}
