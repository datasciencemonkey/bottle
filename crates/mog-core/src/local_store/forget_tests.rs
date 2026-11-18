//! Tests of the Forget groups of the local catalog (#123, MEM-32a, MEM-32c, MEM-01, U9, AT-16;
//! ADR 0081). They use the in-memory key vault double, a temp folder and fixed epoch-ms values:
//! no OS key store, no Lakebase, no live Genie, no token. Every id and text is fake.
//!
//! The optional pointers `genie_exchange.session_id` and `memory_item.source_exchange_id` are
//! `ON DELETE SET NULL` (ADR 0033). So Forget must delete the citing items while those pointers
//! still lead to their parents (the #35 review note on #123).

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::catalog::{self, Access, Kind};
use super::key_vault::MemoryKeyVault;
use super::vec_codec::quantize_int8;
use super::{mem_local_exec, LocalStore, SqlCipherLocalStore};
use crate::error::CoreError;

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";
const S1: &str = "21111111-2222-4333-8444-555555555501";
const S2: &str = "21111111-2222-4333-8444-555555555502";
const SB: &str = "21111111-2222-4333-8444-5555555555b1";
const U1: &str = "61111111-2222-4333-8444-555555555501";
const U2: &str = "61111111-2222-4333-8444-555555555502";
const UB: &str = "61111111-2222-4333-8444-5555555555b1";
const E1: &str = "31111111-2222-4333-8444-555555555501";
const E2: &str = "31111111-2222-4333-8444-555555555502";
const EB: &str = "31111111-2222-4333-8444-5555555555b1";
/// I1 cites E1, I2 cites S1 by `source_session_ids`, I3 cites U1, I4 cites nothing in S1,
/// I5 cites E2 (session S2). IB is MOG B's item.
const I1: &str = "51111111-2222-4333-8444-555555555501";
const I2: &str = "51111111-2222-4333-8444-555555555502";
const I3: &str = "51111111-2222-4333-8444-555555555503";
const I4: &str = "51111111-2222-4333-8444-555555555504";
const I5: &str = "51111111-2222-4333-8444-555555555505";
const IB: &str = "51111111-2222-4333-8444-5555555555b1";
const F1: &str = "71111111-2222-4333-8444-555555555501";
const F2: &str = "71111111-2222-4333-8444-555555555502";

const T0: i64 = 1_767_225_600_000;

/// The text of each item. Each one has a word that no other row has, for the FTS checks.
const TEXTS: [(&str, &str); 6] = [
    (I1, "Q3 revenue rose zebrafinch percent."),
    (I2, "Decision: ship the walrusdock release."),
    (I3, "Open question about the quokkabay budget."),
    (I4, "Use the EU region for marmotpeak tables."),
    (I5, "Churn fell in the ibexvale segment."),
    (IB, "MOG B keeps the narwhalcove plan."),
];

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-r10f-{tag}-{}-{n}", std::process::id()));
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

fn rows(v: &Value) -> &Vec<Value> {
    v["rows"].as_array().expect("rows array")
}

fn count(s: &SqlCipherLocalStore, sql: &str) -> i64 {
    s.with_conn(|c| c.query_row(sql, [], |r| r.get(0))).unwrap()
}

fn exchange(mog: &str, id: &str, req: &str, session: Option<&str>, conv: &str) -> Value {
    json!({"mogId": mog, "exchangeId": id, "requestId": req, "sessionId": session,
           "backendKind": "genie_one_mcp", "profileName": "test-profile",
           "workspaceHost": "example.cloud.databricks.com", "workspaceId": 1234,
           "homeWorkspace": true, "conversationId": conv, "questionUser": "What was Q3 revenue?",
           "questionSentSha256": "0".repeat(64), "status": "completed",
           "answerSummary": "Revenue rose.", "finalAnswerMd": "**Revenue** rose.",
           "askedAt": T0, "rowHlc": 1, "deviceId": "test-device"})
}

fn item(mog: &str, id: &str, source_exchange: Option<&str>) -> Value {
    let text = TEXTS.iter().find(|(i, _)| *i == id).expect("text").1;
    json!({"mogId": mog, "itemId": id, "itemType": "fact", "text": text, "source": "you",
           "confidence": 0.9, "sourceExchangeId": source_exchange, "seenAt": T0, "rowHlc": 1,
           "deviceId": "test-device", "now": T0})
}

/// MOG A: sessions S1 (utterances U1, U2; exchange E1) and S2 (exchange E2), items I1-I5 with
/// embed jobs and an int8 vector each. MOG B: session SB, utterance UB, exchange EB, item IB.
fn seeded(tag: &str) -> (TempDir, SqlCipherLocalStore) {
    let t = TempDir::new(tag);
    let s = SqlCipherLocalStore::new(t.0.join("memory"), Arc::new(MemoryKeyVault::new()));
    s.open().expect("open");
    for mog in [MOG_A, MOG_B] {
        exec(
            &s,
            "mem.mog.upsert",
            json!({"mogId": mog, "displayName": "Sales", "backendKind": "genie_one_mcp",
                   "rowHlc": 1, "deviceId": "test-device"}),
        );
    }
    s.with_conn(|c| {
        for (sid, mog) in [(S1, MOG_A), (S2, MOG_A), (SB, MOG_B)] {
            c.execute(
                "INSERT INTO session (session_id, mog_id, kind, started_at, row_hlc, \
                 origin_device_id) VALUES (?1, ?2, 'meeting', ?3, 1, 'test-device')",
                rusqlite::params![sid, mog, T0],
            )?;
        }
        for (uid, sid, mog, seq, text) in [
            (U1, S1, MOG_A, 1, "we said the pelicanridge number twice"),
            (U2, S1, MOG_A, 2, "and the flamingobay launch slips"),
            (UB, SB, MOG_B, 1, "MOG B talks about the pelicanridge number"),
        ] {
            c.execute(
                "INSERT INTO utterance (utterance_id, session_id, mog_id, seq, speaker_track, \
                 started_at, duration_ms, text_clean, route, stt_provider, row_hlc, \
                 origin_device_id) VALUES (?1, ?2, ?3, ?4, 'you', ?5, 100, ?6, 'context', \
                 'local', 1, 'test-device')",
                rusqlite::params![uid, sid, mog, seq, T0, text],
            )?;
        }
        Ok(())
    })
    .unwrap();
    exec(
        &s,
        "mem.exchange.upsert",
        exchange(MOG_A, E1, "41111111-2222-4333-8444-555555555501", Some(S1), "conv-s1"),
    );
    exec(
        &s,
        "mem.exchange.upsert",
        exchange(MOG_A, E2, "41111111-2222-4333-8444-555555555502", Some(S2), "conv-s2"),
    );
    exec(
        &s,
        "mem.exchange.upsert",
        exchange(MOG_B, EB, "41111111-2222-4333-8444-5555555555b1", Some(SB), "conv-b"),
    );
    for (mog, id, src) in [
        (MOG_A, I1, Some(E1)),
        (MOG_A, I2, None),
        (MOG_A, I3, None),
        (MOG_A, I4, None),
        (MOG_A, I5, Some(E2)),
        (MOG_B, IB, Some(EB)),
    ] {
        exec(&s, "mem.item.write", item(mog, id, src));
    }
    let v: Vec<f32> = (0..256).map(|i| if i == 3 { 1.0 } else { 0.0 }).collect();
    s.with_conn(|c| {
        c.execute(
            "UPDATE memory_item SET source_session_ids = json_array(?1) WHERE item_id = ?2",
            rusqlite::params![S1, I2],
        )?;
        c.execute(
            "UPDATE memory_item SET source_utterance_ids = json_array(?1) WHERE item_id = ?2",
            rusqlite::params![U1, I3],
        )?;
        c.execute(
            "UPDATE memory_item SET source_session_ids = json_array(?1) WHERE item_id = ?2",
            rusqlite::params![SB, IB],
        )?;
        c.execute(
            "INSERT INTO memory_item_vec_256 (rowid, mog_id, embedding_model, embedding) \
             SELECT rid, mog_id, 'qwen3@256', vec_int8(?1) FROM memory_item",
            rusqlite::params![quantize_int8(&v)],
        )
    })
    .unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM outbox WHERE kind = 'embed'"), 6);
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 6);
    (t, s)
}

fn item_ids(s: &SqlCipherLocalStore) -> Vec<String> {
    s.with_conn(|c| {
        let mut st = c.prepare("SELECT item_id FROM memory_item ORDER BY item_id")?;
        let out = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>();
        out
    })
    .unwrap()
}

fn outbox_refs(s: &SqlCipherLocalStore) -> Vec<String> {
    s.with_conn(|c| {
        let mut st = c.prepare("SELECT ref_id FROM outbox ORDER BY ref_id")?;
        let out = st
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>();
        out
    })
    .unwrap()
}

/// The hits of `mem.recall.fts` for one word in one MOG.
fn recall(s: &SqlCipherLocalStore, mog: &str, word: &str) -> usize {
    rows(&exec(
        s,
        "mem.recall.fts",
        json!({"mogId": mog, "query": word, "limit": 50}),
    ))
    .len()
}

fn fts_utterance(s: &SqlCipherLocalStore, word: &str) -> i64 {
    count(
        s,
        &format!("SELECT count(*) FROM utterance_fts WHERE utterance_fts MATCH '{word}'"),
    )
}

fn forget_session(mog: &str, session: &str, forget_id: &str) -> Value {
    json!({"stmtId": "mem.forget.session",
           "params": {"mogId": mog, "sessionId": session, "forgetId": forget_id,
                      "requestedAt": T0 + 1, "deviceId": "test-device"}})
}

// ---- MEM-32a: Forget a session ---------------------------------------------------------------

#[test]
fn mem_32a_at_16_forget_session_deletes_its_rows_and_each_citing_item() {
    let (_t, s) = seeded("32a-session");
    // Brain access: T10 runs it through mem_local_exec on a user action.
    let out = mem_local_exec(&s, forget_session(MOG_A, S1, F1)).expect("forget session");
    let local: i64 = rows(&out)
        .iter()
        .map(|r| r["changes"].as_i64().unwrap())
        .sum();
    // I1 + I2 + I3, 3 embed jobs, E1, U1 + U2, S1, and the forget_log row.
    assert_eq!(local, 3 + 3 + 1 + 2 + 1 + 1);

    // No row of the scope remains.
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM session WHERE session_id = '{S1}'")),
        0
    );
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM utterance WHERE session_id = '{S1}'")),
        0
    );
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM genie_exchange WHERE exchange_id = '{E1}'")),
        0
    );
    // Each citing item is gone: by exchange (I1), by session id (I2) and by utterance (I3).
    assert_eq!(item_ids(&s), vec![I4, I5, IB]);
    // The #35 note: no item is left with a pointer that the delete set to NULL.
    assert_eq!(
        count(&s, "SELECT count(*) FROM memory_item WHERE source_exchange_id IS NULL AND item_id IN ('51111111-2222-4333-8444-555555555501')"),
        0
    );
    // The embed jobs and the vectors of the deleted items are gone too.
    assert_eq!(outbox_refs(&s), vec![I4, I5, IB]);
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 3);
    // FTS recall can no longer find a forgotten text; the other texts still match.
    for w in ["zebrafinch", "walrusdock", "quokkabay"] {
        assert_eq!(recall(&s, MOG_A, w), 0, "{w}");
    }
    assert_eq!(recall(&s, MOG_A, "marmotpeak"), 1);
    assert_eq!(recall(&s, MOG_A, "ibexvale"), 1);
    assert_eq!(fts_utterance(&s, "flamingobay"), 0);
    // Only MOG B's utterance still holds the shared word.
    assert_eq!(fts_utterance(&s, "pelicanridge"), 1);
    // The other session and MOG B keep their rows.
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM genie_exchange WHERE exchange_id IN ('{E2}', '{EB}')")),
        2
    );
    assert_eq!(count(&s, "SELECT count(*) FROM session"), 2);
    // One forget_log row for the v1 sync, with the session as its target.
    let log = s
        .with_conn(|c| {
            c.query_row(
                "SELECT mog_id, scope, target_ids, requested_at, device_id FROM forget_log \
                 WHERE forget_id = ?1",
                [F1],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                },
            )
        })
        .unwrap();
    assert_eq!(
        log,
        (
            MOG_A.to_owned(),
            "session".to_owned(),
            format!("[\"{S1}\"]"),
            T0 + 1,
            "test-device".to_owned()
        )
    );
}

#[test]
fn mem_32a_mem_32c_session_conversations_are_collected_before_the_delete() {
    let (_t, s) = seeded("32a-conv");
    let q = json!({"mogId": MOG_A, "sessionId": S1});
    let got = exec(&s, "mem.forget.session_conversations", q.clone());
    assert_eq!(
        rows(&got),
        &vec![json!({"backendKind": "genie_one_mcp", "profile": "test-profile",
                     "host": "example.cloud.databricks.com", "workspaceId": 1234,
                     "spaceId": null, "conversationId": "conv-s1"})]
    );
    // MEM-01: another MOG finds nothing for the same session id.
    let other = exec(
        &s,
        "mem.forget.session_conversations",
        json!({"mogId": MOG_B, "sessionId": S1}),
    );
    assert!(rows(&other).is_empty());
    mem_local_exec(&s, forget_session(MOG_A, S1, F1)).unwrap();
    // After the delete the session has no exchange, so the read must come first.
    assert!(rows(&exec(&s, "mem.forget.session_conversations", q)).is_empty());
}

#[test]
fn mem_32a_mem_32c_mog_conversations_are_distinct_and_mog_scoped() {
    let (_t, s) = seeded("32a-mogconv");
    // A second exchange in the same conversation lists once.
    let mut again = exchange(
        MOG_A,
        "31111111-2222-4333-8444-555555555503",
        "41111111-2222-4333-8444-555555555503",
        Some(S2),
        "conv-s2",
    );
    again["askedAt"] = json!(T0 + 10);
    exec(&s, "mem.exchange.upsert", again);
    let got = exec(&s, "mem.forget.mog_conversations", json!({"mogId": MOG_A}));
    let ids: Vec<&str> = rows(&got)
        .iter()
        .map(|r| r["conversationId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["conv-s1", "conv-s2"]);
}

// ---- MEM-32a: Forget items and Forget a MOG -----------------------------------------------------

#[test]
fn mem_32a_forget_items_deletes_the_items_their_jobs_vectors_and_fts_rows() {
    let (_t, s) = seeded("32a-items");
    let out = exec(
        &s,
        "mem.forget.items",
        json!({"mogId": MOG_A, "itemIds": [I1, I4], "forgetId": F1,
               "requestedAt": T0 + 1, "deviceId": "test-device"}),
    );
    let ch: Vec<i64> = rows(&out)
        .iter()
        .map(|r| r["changes"].as_i64().unwrap())
        .collect();
    assert_eq!(ch, vec![2, 2, 1], "items, embed jobs, log row");
    assert_eq!(item_ids(&s), vec![I2, I3, I5, IB]);
    assert_eq!(outbox_refs(&s), vec![I2, I3, I5, IB]);
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 4);
    assert_eq!(recall(&s, MOG_A, "zebrafinch"), 0);
    assert_eq!(recall(&s, MOG_A, "marmotpeak"), 0);
    assert_eq!(recall(&s, MOG_A, "walrusdock"), 1);
    // The exchange that I1 cited stays: Forget of an item deletes the item only.
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM genie_exchange WHERE exchange_id = '{E1}'")),
        1
    );
    let log = count(
        &s,
        &format!(
            "SELECT count(*) FROM forget_log WHERE forget_id = '{F1}' AND scope = 'item' \
             AND target_ids = json_array('{I1}', '{I4}')"
        ),
    );
    assert_eq!(log, 1);
}

#[test]
fn mem_32a_at_16_forget_mog_leaves_no_row_of_the_mog_except_the_log() {
    let (_t, s) = seeded("32a-mog");
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO kv (mog_id, k, v) VALUES (?1, 'last_topic', 'zebrafinch')",
            [MOG_A],
        )?;
        c.execute(
            "INSERT INTO sync_state (device_id, mog_id, app_version, local_schema_version) \
             VALUES ('test-device', ?1, '0.0.0', 2)",
            [MOG_A],
        )?;
        // A session consent record: content-free security data with its own expiry (SEC-15b).
        c.execute(
            "INSERT INTO consent_record (consent_id, mog_id, device_id, notice_version, kind, \
             session_id, recorded_at, expires_at) VALUES \
             ('81111111-2222-4333-8444-555555555501', ?1, 'test-device', '1', 'session', ?2, \
             ?3, ?4)",
            rusqlite::params![MOG_A, S1, T0, T0 + 1],
        )
    })
    .unwrap();
    exec(
        &s,
        "mem.forget.mog",
        json!({"mogId": MOG_A, "forgetId": F1, "requestedAt": T0 + 1,
               "deviceId": "test-device"}),
    );
    for table in [
        "mog",
        "session",
        "utterance",
        "genie_exchange",
        "memory_item",
        "outbox",
        "kv",
        "sync_state",
    ] {
        assert_eq!(
            count(&s, &format!("SELECT count(*) FROM {table} WHERE mog_id = '{MOG_A}'")),
            0,
            "{table}"
        );
    }
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM forget_log WHERE mog_id = '{MOG_A}' AND scope = 'mog' AND target_ids = json_array('{MOG_A}')")),
        1
    );
    assert_eq!(
        count(&s, &format!("SELECT count(*) FROM consent_record WHERE mog_id = '{MOG_A}'")),
        1,
        "consent records keep their 13-month expiry (SEC-15b)"
    );
    // Vectors and FTS rows of MOG A are gone; MOG B keeps all of its rows.
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 1);
    assert_eq!(recall(&s, MOG_A, "marmotpeak"), 0);
    assert_eq!(fts_utterance(&s, "flamingobay"), 0);
    assert_eq!(recall(&s, MOG_B, "narwhalcove"), 1);
    assert_eq!(fts_utterance(&s, "pelicanridge"), 1);
    for table in ["mog", "session", "utterance", "genie_exchange", "memory_item", "outbox"] {
        assert_eq!(
            count(&s, &format!("SELECT count(*) FROM {table} WHERE mog_id = '{MOG_B}'")),
            1,
            "{table} of MOG B"
        );
    }
}

// ---- One transaction, MOG scope, access -----------------------------------------------------------

#[test]
fn mem_32a_a_failed_forget_step_rolls_back_every_delete() {
    let (_t, s) = seeded("32a-rollback");
    // The log step is the last step. A forget_id that is already taken makes it fail.
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO forget_log (forget_id, mog_id, scope, target_ids, requested_at, \
             device_id) VALUES (?1, ?2, 'item', '[]', 1, 'test-device')",
            rusqlite::params![F2, MOG_A],
        )
    })
    .unwrap();
    let r = mem_local_exec(&s, forget_session(MOG_A, S1, F2));
    assert_eq!(
        r,
        Err(CoreError::IpcRefused {
            code: "host_failed"
        })
    );
    assert_eq!(item_ids(&s), vec![I1, I2, I3, I4, I5, IB]);
    assert_eq!(count(&s, "SELECT count(*) FROM utterance"), 3);
    assert_eq!(count(&s, "SELECT count(*) FROM genie_exchange"), 3);
    assert_eq!(count(&s, "SELECT count(*) FROM outbox"), 6);
    assert_eq!(recall(&s, MOG_A, "zebrafinch"), 1);
}

#[test]
fn mem_01_forget_with_another_mog_id_deletes_nothing_of_the_first_mog() {
    let (_t, s) = seeded("01-forget");
    mem_local_exec(&s, forget_session(MOG_B, S1, F1)).unwrap();
    exec(
        &s,
        "mem.forget.items",
        json!({"mogId": MOG_B, "itemIds": [I1, I2], "forgetId": F2,
               "requestedAt": T0, "deviceId": "test-device"}),
    );
    assert_eq!(item_ids(&s), vec![I1, I2, I3, I4, I5, IB]);
    assert_eq!(count(&s, "SELECT count(*) FROM utterance"), 3);
    assert_eq!(count(&s, &format!("SELECT count(*) FROM session WHERE session_id = '{S1}'")), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM outbox"), 6);
}

#[test]
fn mem_32a_u9_forget_groups_are_brain_and_their_steps_are_rust_only() {
    for id in ["mem.forget.items", "mem.forget.session", "mem.forget.mog"] {
        let g = catalog::find_group(id).unwrap_or_else(|| panic!("{id} is a group"));
        assert_eq!(g.access, Access::Brain, "{id}: a user action through T10");
        for st in g.steps {
            assert_eq!(st.access, Access::Core, "{}: no partial Forget from the brain", st.id);
            assert_eq!(st.kind, Kind::Write);
            let sql = st.sql.to_ascii_lowercase();
            // U9: Forget deletes by id and scope only, never by age.
            assert!(!sql.contains("_at <"), "{}: no age rule", st.id);
            assert!(!sql.contains("expires_at"), "{}: no TTL", st.id);
        }
        // The log row is the last step, so a failed delete writes no log row.
        let last = g.steps.last().unwrap();
        assert!(last.sql.contains("INSERT INTO forget_log"), "{id}");
    }
    for id in [
        "mem.forget.session_conversations",
        "mem.forget.mog_conversations",
    ] {
        let st = catalog::find(id).unwrap_or_else(|| panic!("{id}"));
        assert_eq!((st.kind, st.access), (Kind::Query, Access::Brain), "{id}");
    }
    // A lone exchange or item delete would leave citing items or jobs behind (#35 note).
    assert!(catalog::find("mem.forget.exchange").is_none());
    assert!(catalog::find("mem.forget.item").is_none());
    // A step ID through mem_local_exec is refused.
    let (_t, s) = seeded("32a-steps");
    let r = mem_local_exec(
        &s,
        json!({"stmtId": "mem.forget.session_row",
               "params": {"mogId": MOG_A, "sessionId": S1}}),
    );
    assert_eq!(
        r,
        Err(CoreError::IpcRefused {
            code: "invalid_args"
        })
    );
}
