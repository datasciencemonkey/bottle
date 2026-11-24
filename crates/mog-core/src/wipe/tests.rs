//! Tests of R14 WipeService (#124; MEM-33a, SEC-15a, SEC-15b, MEM-26, MEM-01, G-49, U9, AT-16;
//! ADR 0087, ADR 0033, ADR 0081). They use the in-memory key vault double, a temp folder and a
//! simulated wall clock: no OS key store, no Lakebase, no live Genie, no token. Every id and text
//! is fake.
//!
//! U9: Bottle deletes no user content on its own. Only the user's Wipe (older than 30 d, older
//! than 90 d, or everything) deletes transcripts, dictation history, answers and memory items.
//! The content-free security data (consent records, the action log) keeps its own expiry.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::*;
use crate::audit::{AuditLog, AuditRetention, MemoryAuditStore};
use crate::clock::SimWallClock;
use crate::consent::{ConsentRecords, ConsentRetention};
use crate::ipc::AuditAppendArgs;
use crate::local_store::catalog::{self, Access};
use crate::local_store::key_vault::MemoryKeyVault;
use crate::local_store::mog_guard::require_mog_id_filter;
use crate::local_store::vec_codec::quantize_int8;
use crate::local_store::{mem_local_exec, LocalStore, SqlCipherLocalStore};

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";
const DEVICE: &str = "7c3e9a10-1b2c-4d5e-8f60-718293a4b5c6";

const DAY: i64 = 86_400_000;
/// 2026-01-01T00:00:00Z. The clock of each test runs 400 d later.
const T0: i64 = 1_767_225_600_000;
const NOW: i64 = T0 + 400 * DAY;

/// One age class: the id suffix, the age in days and a word that no other row has.
struct Age {
    tag: &'static str,
    days: i64,
    word: &'static str,
}

/// Recent (10 d), middle (45 d) and old (120 d) rows in MOG A.
const AGES: [Age; 3] = [
    Age {
        tag: "01",
        days: 10,
        word: "zebrafinch",
    },
    Age {
        tag: "02",
        days: 45,
        word: "walrusdock",
    },
    Age {
        tag: "03",
        days: 120,
        word: "quokkabay",
    },
];

fn session_id(tag: &str) -> String {
    format!("21111111-2222-4333-8444-5555555555{tag}")
}
fn utterance_id(tag: &str) -> String {
    format!("61111111-2222-4333-8444-5555555555{tag}")
}
fn dictation_id(tag: &str) -> String {
    format!("62222222-2222-4333-8444-5555555555{tag}")
}
fn exchange_id(tag: &str) -> String {
    format!("31111111-2222-4333-8444-5555555555{tag}")
}
fn request_id(tag: &str) -> String {
    format!("41111111-2222-4333-8444-5555555555{tag}")
}
fn item_id(tag: &str) -> String {
    format!("51111111-2222-4333-8444-5555555555{tag}")
}
fn conv(tag: &str) -> String {
    format!("conv-test-{tag}")
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-r14-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exec(s: &SqlCipherLocalStore, id: &str, params: Value) -> Value {
    s.exec(id, params).unwrap_or_else(|e| panic!("{id}: {e:?}"))
}

fn count(s: &SqlCipherLocalStore, sql: &str) -> i64 {
    s.with_conn(|c| c.query_row(sql, [], |r| r.get(0))).unwrap()
}

fn ids(s: &SqlCipherLocalStore, sql: &str) -> Vec<String> {
    s.with_conn(|c| {
        let mut st = c.prepare(sql)?;
        let out = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>();
        out
    })
    .unwrap()
}

/// The user content of the store, by table: (items, exchanges, utterances, sessions, outbox).
fn content(s: &SqlCipherLocalStore) -> [i64; 5] {
    [
        count(s, "SELECT count(*) FROM memory_item"),
        count(s, "SELECT count(*) FROM genie_exchange"),
        count(s, "SELECT count(*) FROM utterance"),
        count(s, "SELECT count(*) FROM session"),
        count(s, "SELECT count(*) FROM outbox"),
    ]
}

fn recall(s: &SqlCipherLocalStore, mog: &str, word: &str) -> usize {
    exec(
        s,
        "mem.recall.fts",
        json!({"mogId": mog, "query": word, "limit": 50}),
    )["rows"]
        .as_array()
        .expect("rows")
        .len()
}

fn fts_utterance(s: &SqlCipherLocalStore, word: &str) -> i64 {
    count(
        s,
        &format!("SELECT count(*) FROM utterance_fts WHERE utterance_fts MATCH '{word}'"),
    )
}

fn add_mog(s: &SqlCipherLocalStore, mog: &str) {
    exec(
        s,
        "mem.mog.upsert",
        json!({"mogId": mog, "displayName": "Sales", "backendKind": "genie_one_mcp",
               "rowHlc": 1, "deviceId": "test-device"}),
    );
}

fn add_session(s: &SqlCipherLocalStore, mog: &str, sid: &str, kind: &str, at: i64) {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO session (session_id, mog_id, kind, title, summary, started_at, row_hlc, \
             origin_device_id) VALUES (?1, ?2, ?3, 'Weekly sync', 'They talked.', ?4, 1, \
             'test-device')",
            rusqlite::params![sid, mog, kind, at],
        )
    })
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn add_utterance(
    s: &SqlCipherLocalStore,
    mog: &str,
    sid: &str,
    uid: &str,
    seq: i64,
    route: &str,
    at: i64,
    text: &str,
) {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO utterance (utterance_id, session_id, mog_id, seq, speaker_track, \
             started_at, duration_ms, text_clean, text_raw, route, stt_provider, row_hlc, \
             origin_device_id) VALUES (?1, ?2, ?3, ?4, 'you', ?5, 100, ?6, ?6, ?7, 'local', 1, \
             'test-device')",
            rusqlite::params![uid, sid, mog, seq, at, text, route],
        )
    })
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn add_exchange(
    s: &SqlCipherLocalStore,
    mog: &str,
    eid: &str,
    req: &str,
    session: Option<&str>,
    conversation: &str,
    backend: &str,
    asked_at: i64,
) {
    exec(
        s,
        "mem.exchange.upsert",
        json!({"mogId": mog, "exchangeId": eid, "requestId": req, "sessionId": session,
               "backendKind": backend, "profileName": "test-profile",
               "workspaceHost": "example.cloud.databricks.com", "workspaceId": 1234,
               "homeWorkspace": true, "spaceId": "space-test-1",
               "conversationId": conversation, "questionUser": "What was Q3 revenue?",
               "questionSentSha256": "0".repeat(64), "status": "completed",
               "answerSummary": "Revenue rose.", "finalAnswerMd": "**Revenue** rose.",
               "askedAt": asked_at, "rowHlc": 1, "deviceId": "test-device"}),
    );
}

fn add_item(
    s: &SqlCipherLocalStore,
    mog: &str,
    iid: &str,
    text: &str,
    source_exchange: Option<&str>,
    seen_at: i64,
) {
    exec(
        s,
        "mem.item.write",
        json!({"mogId": mog, "itemId": iid, "itemType": "fact", "text": text,
               "source": "you", "confidence": 0.9, "sourceExchangeId": source_exchange,
               "seenAt": seen_at, "rowHlc": 1, "deviceId": "test-device", "now": seen_at}),
    );
}

/// MOG A has, for each of the 3 ages, a meeting session with one utterance (a transcript), a
/// PTT session with one dictation utterance (dictation history), an exchange with a full answer
/// in that session, and a memory item with its embed job and int8 vector. The recent item cites
/// the OLD exchange, and an old exchange with no session shares the RECENT conversation. MOG B
/// has one old row of each kind (a Wipe is for the whole device).
fn seeded(tag: &str) -> (TempDir, Arc<SqlCipherLocalStore>) {
    let t = TempDir::new(tag);
    let s = Arc::new(SqlCipherLocalStore::new(
        t.0.join("memory"),
        Arc::new(MemoryKeyVault::new()),
    ));
    s.open().expect("open");
    add_mog(&s, MOG_A);
    add_mog(&s, MOG_B);
    for a in &AGES {
        let at = NOW - a.days * DAY;
        let sid = session_id(a.tag);
        let did = session_id(&format!("d{}", &a.tag[1..]));
        add_session(&s, MOG_A, &sid, "meeting", at);
        add_utterance(
            &s,
            MOG_A,
            &sid,
            &utterance_id(a.tag),
            1,
            "context",
            at,
            &format!("we said the {} number", a.word),
        );
        add_session(&s, MOG_A, &did, "push_to_talk", at);
        add_utterance(
            &s,
            MOG_A,
            &did,
            &dictation_id(a.tag),
            1,
            "dictation",
            at,
            &format!("dictated {} note", a.word),
        );
        add_exchange(
            &s,
            MOG_A,
            &exchange_id(a.tag),
            &request_id(a.tag),
            Some(&sid),
            &conv(a.tag),
            "genie_one_mcp",
            at,
        );
    }
    // The old exchange with no session shares the recent conversation: that conversation is
    // still in use after an age Wipe, so it is not returned for a Genie delete.
    add_exchange(
        &s,
        MOG_A,
        &exchange_id("04"),
        &request_id("04"),
        None,
        &conv("01"),
        "genie_one_mcp",
        NOW - 200 * DAY,
    );
    for a in &AGES {
        // The recent item cites the old exchange: no cascade, the item stays (ADR 0033).
        let src = if a.tag == "01" {
            Some(exchange_id("03"))
        } else {
            Some(exchange_id(a.tag))
        };
        add_item(
            &s,
            MOG_A,
            &item_id(a.tag),
            &format!("Q3 revenue rose {} percent.", a.word),
            src.as_deref(),
            NOW - a.days * DAY,
        );
    }
    // MOG B: one old row of each kind, with a genie_agent conversation.
    let old = NOW - 120 * DAY;
    let sb = session_id("b1");
    add_session(&s, MOG_B, &sb, "meeting", old);
    add_utterance(
        &s,
        MOG_B,
        &sb,
        &utterance_id("b1"),
        1,
        "context",
        old,
        "MOG B talks about pelicanridge",
    );
    add_exchange(
        &s,
        MOG_B,
        &exchange_id("b1"),
        &request_id("b1"),
        Some(&sb),
        &conv("b1"),
        "genie_agent",
        old,
    );
    add_item(
        &s,
        MOG_B,
        &item_id("b1"),
        "MOG B keeps the narwhalcove plan.",
        Some(&exchange_id("b1")),
        old,
    );
    let v: Vec<f32> = (0..256).map(|i| if i == 3 { 1.0 } else { 0.0 }).collect();
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO memory_item_vec_256 (rowid, mog_id, embedding_model, embedding) \
             SELECT rid, mog_id, 'qwen3@256', vec_int8(?1) FROM memory_item",
            rusqlite::params![quantize_int8(&v)],
        )
    })
    .unwrap();
    assert_eq!(content(&s), [4, 5, 7, 7, 4]);
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 4);
    (t, s)
}

fn clock() -> SimWallClock {
    SimWallClock::at(NOW)
}

fn service(store: &Arc<SqlCipherLocalStore>, clock: &SimWallClock) -> LocalWipeService {
    LocalWipeService::new(Arc::clone(store), Arc::new(clock.clone()))
}

fn wipe(store: &Arc<SqlCipherLocalStore>, scope: WipeScope) -> WipeReport {
    service(store, &clock()).wipe(scope).expect("wipe")
}

// ---- MEM-33a, U9: no user content goes without a Wipe action --------------------------------

#[test]
fn mem_33a_u9_at_16_no_user_content_row_is_deleted_without_a_wipe_action() {
    let (_t, s) = seeded("33a-none");
    let c = clock();
    let audit = Arc::new(AuditLog::new(
        MemoryAuditStore::default(),
        AuditRetention::default(),
    ));
    let consent = Arc::new(
        ConsentRecords::new(
            Arc::clone(&s),
            Arc::new(c.clone()),
            DEVICE,
            ConsentRetention::default(),
        )
        .unwrap(),
    );
    let svc = service(&s, &c)
        .with_expiry(audit as Arc<dyn SecurityExpiry>)
        .with_expiry(consent as Arc<dyn SecurityExpiry>);
    let before = content(&s);
    // 400 more days pass, and the security expiries run many times (app start, after a Wipe).
    for d in 0..=400 {
        c.set(NOW + d * DAY);
        svc.run_security_expiries().expect("expiries");
    }
    assert_eq!(content(&s), before, "the expiries never touch user content");
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 4);
    for a in &AGES {
        assert_eq!(recall(&s, MOG_A, a.word), 1, "{}", a.word);
    }
    // MEM-26: the full answer of the oldest exchange is still there.
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM genie_exchange WHERE final_answer_md IS NOT NULL"
        ),
        5
    );
}

#[test]
fn mem_33a_u9_the_wipe_statements_and_groups_are_rust_only() {
    let wipe_entries: Vec<&str> = catalog::STATEMENTS
        .iter()
        .filter(|st| st.id.starts_with("mem.wipe."))
        .map(|st| st.id)
        .chain(
            catalog::GROUPS
                .iter()
                .filter(|g| g.id.starts_with("mem.wipe."))
                .map(|g| g.id),
        )
        .collect();
    for id in ["mem.wipe.before", "mem.wipe.everything"] {
        assert!(wipe_entries.contains(&id), "missing {id}");
    }
    let (_t, s) = seeded("33a-core");
    for id in &wipe_entries {
        assert_eq!(catalog::access_of(id), Some(Access::Core), "{id}");
        let args = json!({"stmtId": id, "params": {"mogId": MOG_A, "before": i64::MAX}});
        assert_eq!(
            mem_local_exec(&*s, args),
            Err(CoreError::IpcRefused {
                code: "invalid_args"
            }),
            "{id}"
        );
    }
    let (list, why) = (&catalog::WIPE_MOG_LIST.stmt, catalog::WIPE_MOG_LIST.why);
    assert_eq!(list.access, Access::Core);
    assert!(!why.is_empty());
    assert!(catalog::find(list.id).is_none(), "not reachable by exec");
    assert!(
        s.exec(list.id, json!({"mogId": MOG_A})).is_err(),
        "exec refuses the device-wide list"
    );
    assert_eq!(content(&s), [4, 5, 7, 7, 4], "the brain could not wipe");
}

// ---- MEM-33a: each option deletes only the matching rows -----------------------------------

#[test]
fn mem_33a_at_16_older_than_30_days_deletes_only_the_matching_rows() {
    let (_t, s) = seeded("33a-30");
    let r = wipe(&s, WipeScope::OlderThan30Days);
    assert_eq!(
        ids(&s, "SELECT item_id FROM memory_item"),
        vec![item_id("01")]
    );
    assert_eq!(
        ids(&s, "SELECT exchange_id FROM genie_exchange"),
        vec![exchange_id("01")]
    );
    assert_eq!(
        ids(
            &s,
            "SELECT utterance_id FROM utterance ORDER BY utterance_id"
        ),
        vec![utterance_id("01"), dictation_id("01")]
    );
    assert_eq!(
        ids(&s, "SELECT session_id FROM session ORDER BY session_id"),
        vec![session_id("01"), session_id("d1")]
    );
    assert_eq!(ids(&s, "SELECT ref_id FROM outbox"), vec![item_id("01")]);
    // No cascade: the recent item stays, and its pointer to the wiped exchange is cleared.
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_item WHERE source_exchange_id IS NULL"
        ),
        1
    );
    // Items, exchanges, utterances and sessions of 45, 120 and 200 d, in both MOGs.
    assert_eq!(r.local, 3 + 4 + 5 + 5 + 3);
}

#[test]
fn mem_33a_at_16_older_than_90_days_deletes_only_the_matching_rows() {
    let (_t, s) = seeded("33a-90");
    wipe(&s, WipeScope::OlderThan90Days);
    assert_eq!(
        ids(&s, "SELECT item_id FROM memory_item ORDER BY item_id"),
        vec![item_id("01"), item_id("02")]
    );
    assert_eq!(
        ids(
            &s,
            "SELECT exchange_id FROM genie_exchange ORDER BY exchange_id"
        ),
        vec![exchange_id("01"), exchange_id("02")]
    );
    assert_eq!(count(&s, "SELECT count(*) FROM utterance"), 4);
    assert_eq!(count(&s, "SELECT count(*) FROM session"), 4);
    assert_eq!(count(&s, "SELECT count(*) FROM outbox"), 2);
    // MOG B had only old rows.
    assert_eq!(
        count(
            &s,
            &format!(
                "SELECT (SELECT count(*) FROM memory_item WHERE mog_id = '{MOG_B}') + \
                 (SELECT count(*) FROM session WHERE mog_id = '{MOG_B}')"
            )
        ),
        0
    );
}

#[test]
fn mem_33a_a_row_exactly_at_the_cutoff_is_not_older_and_stays() {
    let (_t, s) = seeded("33a-edge");
    let edge = NOW - 30 * DAY;
    add_item(
        &s,
        MOG_A,
        &item_id("e1"),
        "The edge note stays.",
        None,
        edge,
    );
    add_exchange(
        &s,
        MOG_A,
        &exchange_id("e1"),
        &request_id("e1"),
        None,
        "conv-test-e1",
        "genie_one_mcp",
        edge,
    );
    wipe(&s, WipeScope::OlderThan30Days);
    assert!(ids(&s, "SELECT item_id FROM memory_item").contains(&item_id("e1")));
    assert!(ids(&s, "SELECT exchange_id FROM genie_exchange").contains(&exchange_id("e1")));
    // One millisecond older goes.
    add_item(
        &s,
        MOG_A,
        &item_id("e2"),
        "The old edge note.",
        None,
        edge - 1,
    );
    wipe(&s, WipeScope::OlderThan30Days);
    assert!(!ids(&s, "SELECT item_id FROM memory_item").contains(&item_id("e2")));
}

#[test]
fn mem_33a_a_session_stays_while_one_of_its_utterances_or_exchanges_stays() {
    let (_t, s) = seeded("33a-session");
    // An old session with a recent utterance, and an old session with a recent exchange.
    let (s1, s2) = (session_id("f1"), session_id("f2"));
    add_session(&s, MOG_A, &s1, "meeting", NOW - 120 * DAY);
    add_utterance(
        &s,
        MOG_A,
        &s1,
        &utterance_id("f1"),
        1,
        "context",
        NOW - 5 * DAY,
        "a late line",
    );
    add_session(&s, MOG_A, &s2, "meeting", NOW - 120 * DAY);
    add_exchange(
        &s,
        MOG_A,
        &exchange_id("f2"),
        &request_id("f2"),
        Some(&s2),
        "conv-test-f2",
        "genie_one_mcp",
        NOW - 5 * DAY,
    );
    wipe(&s, WipeScope::OlderThan30Days);
    let sessions = ids(&s, "SELECT session_id FROM session");
    assert!(sessions.contains(&s1), "its utterance is recent");
    assert!(sessions.contains(&s2), "its exchange is recent");
    // The recent exchange keeps its session pointer.
    assert_eq!(
        count(
            &s,
            &format!(
                "SELECT count(*) FROM genie_exchange WHERE exchange_id = '{}' AND \
                 session_id = '{s2}'",
                exchange_id("f2")
            )
        ),
        1
    );
}

// ---- MEM-33a: "everything" leaves an empty store --------------------------------------------

#[test]
fn mem_33a_at_16_everything_leaves_an_empty_store() {
    let (_t, s) = seeded("33a-all");
    let c = clock();
    let consent = ConsentRecords::new(
        Arc::clone(&s),
        Arc::new(c.clone()),
        DEVICE,
        ConsentRetention::default(),
    )
    .unwrap();
    consent.record_notice(None, 1).unwrap();
    let r = service(&s, &c).wipe(WipeScope::Everything).expect("wipe");
    assert_eq!(content(&s), [0, 0, 0, 0, 0]);
    assert_eq!(r.local, 4 + 5 + 7 + 7 + 4);
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 0);
    for a in &AGES {
        assert_eq!(recall(&s, MOG_A, a.word), 0, "{}", a.word);
        assert_eq!(fts_utterance(&s, a.word), 0, "{}", a.word);
    }
    assert_eq!(recall(&s, MOG_B, "narwhalcove"), 0);
    // Kept, with no content: the MOG rows (the parents that writers need), the consent record
    // (SEC-32a, its own SEC-15b expiry) and the schema.
    assert_eq!(count(&s, "SELECT count(*) FROM mog"), 2);
    assert_eq!(count(&s, "SELECT count(*) FROM consent_record"), 1);
    // The store still works: a new exchange saves after the Wipe.
    add_exchange(
        &s,
        MOG_A,
        &exchange_id("c1"),
        &request_id("c1"),
        None,
        "conv-test-c1",
        "genie_one_mcp",
        NOW,
    );
    assert_eq!(count(&s, "SELECT count(*) FROM genie_exchange"), 1);
}

#[test]
fn mem_33a_at_16_a_wiped_text_is_no_longer_found_by_recall() {
    let (_t, s) = seeded("33a-recall");
    for a in &AGES {
        assert_eq!(recall(&s, MOG_A, a.word), 1, "{}", a.word);
        assert_eq!(fts_utterance(&s, a.word), 2, "{}", a.word);
    }
    wipe(&s, WipeScope::OlderThan30Days);
    assert_eq!(recall(&s, MOG_A, "zebrafinch"), 1, "the recent item stays");
    assert_eq!(fts_utterance(&s, "zebrafinch"), 2);
    for word in ["walrusdock", "quokkabay"] {
        assert_eq!(recall(&s, MOG_A, word), 0, "{word}");
        assert_eq!(fts_utterance(&s, word), 0, "{word}");
    }
    assert_eq!(recall(&s, MOG_B, "narwhalcove"), 0);
    assert_eq!(fts_utterance(&s, "pelicanridge"), 0);
    // The vec0 rows and the embed jobs of the wiped items are gone too (#102 queue, KNN).
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 1);
    assert_eq!(
        count(
            &s,
            &format!(
                "SELECT count(*) FROM memory_item_vec_256 WHERE rowid = \
                 (SELECT rid FROM memory_item WHERE item_id = '{}')",
                item_id("01")
            )
        ),
        1
    );
    assert_eq!(count(&s, "SELECT count(*) FROM outbox"), 1);
}

// ---- G-49, MEM-32c (9.2 "Genie conversations created by Bottle") ---------------------------

#[test]
fn mem_33a_g_49_wipe_returns_the_conversations_that_no_kept_exchange_uses() {
    let (_t, s) = seeded("33a-conv");
    let r = wipe(&s, WipeScope::OlderThan30Days);
    let got: Vec<(String, String, String)> = r
        .conversations
        .iter()
        .map(|c| {
            (
                c.mog_id.clone(),
                c.backend_kind.clone(),
                c.conversation_id.clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (MOG_A.to_owned(), "genie_one_mcp".to_owned(), conv("02")),
            (MOG_A.to_owned(), "genie_one_mcp".to_owned(), conv("03")),
            (MOG_B.to_owned(), "genie_agent".to_owned(), conv("b1")),
        ],
        "conv-test-01 is still used by the recent exchange"
    );
    let b = &r.conversations[2];
    assert_eq!(b.profile, "test-profile");
    assert_eq!(b.host, "example.cloud.databricks.com");
    assert_eq!(b.workspace_id, Some(1234));
    assert_eq!(b.space_id.as_deref(), Some("space-test-1"));

    let (_t2, s2) = seeded("33a-conv-all");
    let all = wipe(&s2, WipeScope::Everything);
    let mut cids: Vec<String> = all
        .conversations
        .iter()
        .map(|c| c.conversation_id.clone())
        .collect();
    cids.sort();
    assert_eq!(cids, vec![conv("01"), conv("02"), conv("03"), conv("b1")]);
}

// ---- MEM-26: the full answer stays until Wipe ------------------------------------------------

#[test]
fn mem_26_mem_33a_the_full_answer_is_present_after_400_days_and_wipe_30d_removes_it() {
    let (_t, s) = seeded("26");
    let answers = |s: &SqlCipherLocalStore| {
        count(
            s,
            "SELECT count(*) FROM genie_exchange WHERE final_answer_md = '**Revenue** rose.'",
        )
    };
    assert_eq!(answers(&s), 5);
    wipe(&s, WipeScope::OlderThan30Days);
    assert_eq!(answers(&s), 1);
}

// ---- MEM-01: every Wipe delete filters by mog_id; the MOG list is device-wide ---------------

#[test]
fn mem_01_each_wipe_statement_filters_by_mog_id_and_the_mog_list_is_listed_with_a_reason() {
    for st in catalog::STATEMENTS
        .iter()
        .filter(|st| st.id.starts_with("mem.wipe."))
    {
        assert_eq!(require_mog_id_filter(st.sql), Ok(()), "{}", st.id);
    }
    for g in catalog::GROUPS
        .iter()
        .filter(|g| g.id.starts_with("mem.wipe."))
    {
        assert!(g.params.iter().any(|p| p.name == "mogId" && p.required));
        for st in g.steps {
            assert_eq!(require_mog_id_filter(st.sql), Ok(()), "{}", st.id);
        }
    }
    // The device-wide MOG list cannot pass the guard (it crosses MOGs on purpose); it reads ids
    // only and lists the MOG of every table that holds user content.
    let list = &catalog::WIPE_MOG_LIST;
    assert!(require_mog_id_filter(list.stmt.sql).is_err());
    assert!(list.stmt.sql.to_ascii_lowercase().starts_with("select"));
    for t in [
        "mog",
        "memory_item",
        "genie_exchange",
        "utterance",
        "session",
        "outbox",
    ] {
        assert!(
            list.stmt.sql.contains(&format!("FROM {t} ")) || list.stmt.sql.ends_with(t),
            "the list reads the MOG ids of {t}"
        );
    }
}

#[test]
fn mem_33a_mem_01_a_row_of_a_mog_with_no_mog_row_is_wiped_too() {
    let (_t, s) = seeded("33a-orphan");
    // An embed job of a MOG that has no row any more (outbox has no foreign key).
    let orphan = "7d1e2f30-4a5b-4c6d-8e7f-8091a2b3c4d5";
    exec(
        &s,
        "mem.outbox.enqueue",
        json!({"mogId": orphan, "kind": "embed", "op": "embed_item",
               "refId": item_id("a9"), "now": NOW - 100 * DAY}),
    );
    wipe(&s, WipeScope::Everything);
    assert_eq!(count(&s, "SELECT count(*) FROM outbox"), 0);
}

// ---- SEC-15a: the Section 9.2 table on a simulated clock -------------------------------------

/// A fake expiry that counts its runs.
#[derive(Default)]
struct Counting(AtomicUsize);

impl SecurityExpiry for Counting {
    fn name(&self) -> &'static str {
        "counting"
    }
    fn expire(&self, _now_ms: i64) -> CoreResult<usize> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Ok(0)
    }
}

fn audit_args(mog: &str, op: &str) -> AuditAppendArgs {
    AuditAppendArgs {
        kind: "ask".to_owned(),
        mog_id: mog.to_owned(),
        op_id: op.to_owned(),
        sizes: BTreeMap::new(),
    }
}

#[test]
fn sec_15a_sec_15b_at_16_the_section_9_2_table_passes_on_a_simulated_clock() {
    let (_t, s) = seeded("15a");
    let c = clock();
    let consent = Arc::new(
        ConsentRecords::new(
            Arc::clone(&s),
            Arc::new(c.clone()),
            DEVICE,
            ConsentRetention::default(),
        )
        .unwrap(),
    );
    // Consent records: 13 months, content-free (SEC-15b). One of 14 months, one of 12 months.
    c.set(crate::consent::add_months_utc(NOW, -14));
    consent.record_notice(None, 1).unwrap();
    c.set(crate::consent::add_months_utc(NOW, -12));
    consent.record_notice(None, 2).unwrap();
    c.set(NOW);
    // Main agent action log: content-free, 30 d. One of 31 d, one of exactly 30 d.
    let audit = Arc::new(AuditLog::new(
        MemoryAuditStore::default(),
        AuditRetention::default(),
    ));
    let (old_op, kept_op) = (
        "00000000-0000-4000-9000-000000000201",
        "00000000-0000-4000-9000-000000000202",
    );
    audit
        .append(
            &audit_args(MOG_A, old_op),
            &BTreeMap::new(),
            (NOW - 31 * DAY) as u64,
        )
        .unwrap();
    audit
        .append(
            &audit_args(MOG_A, kept_op),
            &BTreeMap::new(),
            (NOW - 30 * DAY) as u64,
        )
        .unwrap();
    let counting = Arc::new(Counting::default());
    let svc = service(&s, &c)
        .with_expiry(Arc::clone(&consent) as Arc<dyn SecurityExpiry>)
        .with_expiry(Arc::clone(&audit) as Arc<dyn SecurityExpiry>)
        .with_expiry(Arc::clone(&counting) as Arc<dyn SecurityExpiry>);

    let report = svc.run_security_expiries().expect("expiries");
    assert_eq!(
        report.expired,
        vec![("consent_records", 1), ("action_log", 1), ("counting", 0)]
    );
    assert_eq!(
        counting.0.load(Ordering::Relaxed),
        1,
        "each expiry runs once"
    );
    // Consent records: the 14-month record is gone, the 12-month one stays.
    let versions: Vec<u32> = consent
        .records()
        .unwrap()
        .iter()
        .map(|r| r.notice_version)
        .collect();
    assert_eq!(versions, vec![2]);
    // Action log: 31 d goes, exactly 30 d stays.
    let ops: Vec<String> = audit
        .entries(MOG_A)
        .unwrap()
        .iter()
        .map(|e| e.op_id.clone())
        .collect();
    assert_eq!(ops, vec![kept_op.to_owned()]);

    // Local dictation history, transcripts (raw and cleaned), full answers and memory items:
    // kept until Wipe. The expiries above deleted none of them, even at 120 and 200 d.
    let rows_of = |s: &SqlCipherLocalStore| {
        [
            count(
                s,
                "SELECT count(*) FROM utterance WHERE route = 'dictation'",
            ),
            count(
                s,
                "SELECT count(*) FROM utterance WHERE route <> 'dictation' AND text_raw IS NOT NULL",
            ),
            count(
                s,
                "SELECT count(*) FROM genie_exchange WHERE final_answer_md IS NOT NULL",
            ),
            count(s, "SELECT count(*) FROM memory_item"),
        ]
    };
    assert_eq!(rows_of(&s), [3, 4, 5, 4]);
    // Wipe "older than 30 d" removes each of them by its own age.
    svc.wipe(WipeScope::OlderThan30Days).expect("wipe");
    assert_eq!(rows_of(&s), [1, 1, 1, 1]);
    // Wipe "everything" removes the rest.
    svc.wipe(WipeScope::Everything).expect("wipe");
    assert_eq!(rows_of(&s), [0, 0, 0, 0]);
    // The consent record and the action log entry are content-free: a Wipe leaves them.
    assert_eq!(consent.records().unwrap().len(), 1);
    assert_eq!(audit.entries(MOG_A).unwrap().len(), 1);
}

#[test]
fn sec_15a_an_expiry_error_does_not_stop_the_other_expiries() {
    struct Failing;
    impl SecurityExpiry for Failing {
        fn name(&self) -> &'static str {
            "failing"
        }
        fn expire(&self, _now_ms: i64) -> CoreResult<usize> {
            Err(CoreError::StoreUnavailable { component: "R10" })
        }
    }
    let (_t, s) = seeded("15a-err");
    let counting = Arc::new(Counting::default());
    let svc = service(&s, &clock())
        .with_expiry(Arc::new(Failing))
        .with_expiry(Arc::clone(&counting) as Arc<dyn SecurityExpiry>);
    assert!(svc.run_security_expiries().is_err());
    assert_eq!(counting.0.load(Ordering::Relaxed), 1);
}

// ---- The scope and the mem_wipe adapter (contract #525) --------------------------------------

#[test]
fn mem_33a_the_three_options_cut_at_30_and_90_days_or_take_everything() {
    assert_eq!(
        WipeScope::OlderThan30Days.cutoff_ms(NOW),
        Some(NOW - 30 * DAY)
    );
    assert_eq!(
        WipeScope::OlderThan90Days.cutoff_ms(NOW),
        Some(NOW - 90 * DAY)
    );
    assert_eq!(WipeScope::Everything.cutoff_ms(NOW), None);
    for (wire, scope) in [
        ("older_than_30d", WipeScope::OlderThan30Days),
        ("older_than_90d", WipeScope::OlderThan90Days),
        ("everything", WipeScope::Everything),
    ] {
        assert_eq!(WipeScope::parse(wire), Some(scope));
        assert_eq!(scope.wire(), wire);
    }
    for bad in ["", "older_than_7d", "Everything", "all"] {
        assert_eq!(WipeScope::parse(bad), None, "{bad}");
    }
}

#[test]
fn mem_33a_sec_15a_mem_wipe_runs_the_wipe_then_the_expiries_and_answers_ids_only() {
    let (_t, s) = seeded("33a-ipc");
    let c = clock();
    let counting = Arc::new(Counting::default());
    let svc = service(&s, &c).with_expiry(Arc::clone(&counting) as Arc<dyn SecurityExpiry>);
    let out = mem_wipe(&svc, json!({"scope": "older_than_90d"})).expect("mem_wipe");
    assert_eq!(content(&s)[0], 2);
    assert_eq!(
        counting.0.load(Ordering::Relaxed),
        1,
        "the expiries run after a Wipe"
    );
    assert_eq!(out["local"], json!(2 + 3 + 3 + 3 + 2));
    let convs = out["conversations"].as_array().expect("conversations");
    assert_eq!(convs.len(), 2);
    let keys: Vec<&String> = convs[1].as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
        vec![
            "backendKind",
            "conversationId",
            "host",
            "mogId",
            "profile",
            "spaceId",
            "workspaceId"
        ]
    );
    // Ids only: no question, summary or answer text crosses back.
    let text = out.to_string();
    for content in ["Q3", "Revenue", "revenue", "walrusdock", "quokkabay"] {
        assert!(!text.contains(content), "{content}");
    }
    for bad in [
        json!({}),
        json!({"scope": "older_than_7d"}),
        json!({"scope": "everything", "before": 0}),
        json!([]),
    ] {
        assert_eq!(
            mem_wipe(&svc, bad.clone()),
            Err(CoreError::IpcRefused {
                code: "invalid_args"
            }),
            "{bad}"
        );
    }
    assert_eq!(content(&s)[0], 2, "a refused call deletes nothing");
}

#[test]
fn mem_33a_525_an_empty_store_wipes_to_zero_for_every_scope() {
    let t = TempDir::new("33a-empty");
    let s = Arc::new(SqlCipherLocalStore::new(
        t.0.join("memory"),
        Arc::new(MemoryKeyVault::new()),
    ));
    s.open().expect("open");
    add_mog(&s, MOG_A);
    let svc = service(&s, &clock());
    for scope in ["older_than_30d", "older_than_90d", "everything"] {
        let out = mem_wipe(&svc, json!({ "scope": scope })).expect("an empty wipe succeeds");
        assert_eq!(out, json!({"local": 0, "conversations": []}), "{scope}");
    }
}

/// A vault whose key store refuses and that counts every read (ADR 0105, the locked store).
struct RefusingVault(AtomicUsize);

impl crate::local_store::key_vault::KeyVault for RefusingVault {
    fn load(&self) -> CoreResult<Option<crate::local_store::key_vault::LocalKey>> {
        self.0.fetch_add(1, Ordering::Relaxed);
        Err(CoreError::KeychainUnavailable { component: "R10" })
    }
    fn store(&self, _key: &crate::local_store::key_vault::LocalKey) -> CoreResult<()> {
        Err(CoreError::KeychainUnavailable { component: "R10" })
    }
}

#[test]
fn mem_33a_525_a_locked_store_answers_host_failed_with_one_key_read_per_call() {
    let t = TempDir::new("33a-locked");
    let vault = Arc::new(RefusingVault(AtomicUsize::new(0)));
    let s = Arc::new(SqlCipherLocalStore::new(t.0.join("memory"), vault.clone()));
    let svc = service(&s, &clock());
    assert_eq!(
        mem_wipe(&svc, json!({"scope": "older_than_90d"})),
        Err(CoreError::IpcRefused {
            code: "host_failed"
        })
    );
    assert_eq!(vault.0.load(Ordering::Relaxed), 1, "one read for one call");
}

#[test]
fn mem_33a_the_stub_still_reports_not_implemented() {
    let s = StubWipeService;
    assert!(matches!(
        s.wipe(WipeScope::Everything),
        Err(CoreError::NotImplemented { component: "R14" })
    ));
    assert_eq!(
        mem_wipe(&s, json!({"scope": "everything"})),
        Err(CoreError::IpcRefused {
            code: "not_implemented"
        })
    );
}

/// ADR 0121: an agent turn row (`genie_agent`, no conversation id, a thread root in
/// `split_group_id`) is a plain exchange row for Wipe. Age Wipe and Wipe everything delete it,
/// and it gives no remote conversation to delete.
#[test]
fn adr_0121_wipe_removes_agent_turn_rows() {
    let (_t, s) = seeded("0121-wipe");
    let turn = request_id("f1");
    let root = request_id("f3");
    for (tag, at) in [("f1", NOW), ("f2", NOW - 100 * DAY)] {
        let req = request_id(tag);
        exec(
            &s,
            "mem.exchange.upsert",
            json!({"mogId": MOG_A, "exchangeId": exchange_id(tag), "requestId": req,
                   "backendKind": "genie_agent", "profileName": "test-profile",
                   "workspaceHost": "example.cloud.databricks.com", "workspaceId": 1234,
                   "homeWorkspace": true, "conversationId": null,
                   "questionUser": "What drove Q3?", "questionSentSha256": "0".repeat(64),
                   "status": "completed", "finalAnswerMd": "Note from your MOG",
                   "splitGroupId": root, "askedAt": at, "rowHlc": 1,
                   "deviceId": "test-device"}),
        );
    }
    let turns = "SELECT count(*) FROM genie_exchange WHERE backend_kind = 'genie_agent'";
    let before = count(&s, turns);
    let c = clock();
    let r = wipe(&s, WipeScope::OlderThan90Days);
    assert!(r.local >= 1);
    assert!(count(&s, turns) < before, "the old turn row goes");
    assert_eq!(
        count(
            &s,
            &format!(
                "SELECT count(*) FROM genie_exchange WHERE request_id = '{}'",
                request_id("f2")
            )
        ),
        0
    );
    assert_eq!(
        count(
            &s,
            &format!("SELECT count(*) FROM genie_exchange WHERE request_id = '{turn}'")
        ),
        1
    );
    service(&s, &c).wipe(WipeScope::Everything).expect("wipe");
    assert_eq!(count(&s, turns), 0);
}

/// ADR 0121 (security M1): a hub Delete leaves a tombstone that keeps the Genie conversation id.
/// Wipe everything must still return that conversation, so Bottle can delete it on the server.
#[test]
fn adr_0121_wipe_still_returns_the_conversation_of_a_locally_deleted_thread() {
    let (_t, s) = seeded("0121-tomb");
    let req = request_id("e1");
    exec(
        &s,
        "mem.exchange.upsert",
        json!({"mogId": MOG_A, "exchangeId": exchange_id("e1"), "requestId": req,
               "backendKind": "genie_one_mcp", "profileName": "test-profile",
               "workspaceHost": "example.cloud.databricks.com", "workspaceId": 1234,
               "homeWorkspace": true, "conversationId": "conv-test-deleted",
               "questionUser": "What drove Q3?", "questionSentSha256": "0".repeat(64),
               "status": "completed", "finalAnswerMd": "Revenue grew",
               "askedAt": NOW - 2 * DAY, "rowHlc": 1, "deviceId": "test-device"}),
    );
    let del = exec(
        &s,
        "mem.exchange.delete",
        json!({"mogId": MOG_A, "requestIds": [req], "now": NOW}),
    );
    assert_eq!(del["rows"][0]["changes"], 1);
    let r = wipe(&s, WipeScope::Everything);
    assert!(
        r.conversations
            .iter()
            .any(|c| c.conversation_id == "conv-test-deleted"),
        "the tombstone still gives its conversation id to Wipe"
    );
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM genie_exchange WHERE conversation_id = 'conv-test-deleted'"
        ),
        0,
        "Wipe hard-deletes the tombstone"
    );
}
