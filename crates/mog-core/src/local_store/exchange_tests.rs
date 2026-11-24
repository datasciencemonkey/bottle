//! Tests of the Genie exchange statements of the local catalog (#38, MEM-25a, MEM-26, MEM-32a,
//! MEM-01; ADR 0061). They use the in-memory key vault double, a temp folder and a simulated
//! clock (fixed epoch-ms values): no OS key store, no Lakebase, no live Genie, no token.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::catalog::{Access, STATEMENTS};
use super::key_vault::MemoryKeyVault;
use super::{LocalStore, SqlCipherLocalStore};

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";
const SESSION_1: &str = "21111111-2222-4333-8444-555555555501";
const EXCH_1: &str = "31111111-2222-4333-8444-555555555501";
const REQ_1: &str = "41111111-2222-4333-8444-555555555501";
const EXCH_2: &str = "31111111-2222-4333-8444-555555555502";
const REQ_2: &str = "41111111-2222-4333-8444-555555555502";
const ITEM_1: &str = "51111111-2222-4333-8444-555555555501";

/// The simulated clock: 2026-01-01T00:00:00Z in epoch ms, and one day.
const T0: i64 = 1_767_225_600_000;
const DAY: i64 = 86_400_000;

const DEEP_LINK: &str =
    "https://example.cloud.databricks.com/genie/rooms/test-space/chats/test-conv?o=1234";
const ANSWER_MD: &str =
    "**Revenue** rose 4.2 % to $1.3M in Q3.\n\n| region | rev |\n|---|---|\n| EU | 1.3M |";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-r10x-{tag}-{}-{n}", std::process::id()));
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

fn open(t: &TempDir) -> SqlCipherLocalStore {
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
    s
}

fn exec(s: &SqlCipherLocalStore, id: &str, params: Value) -> Value {
    s.exec(id, params).unwrap_or_else(|e| panic!("{id}: {e:?}"))
}

fn rows(v: &Value) -> &Vec<Value> {
    v["rows"].as_array().expect("rows array")
}

fn changes(v: &Value) -> i64 {
    rows(v)[0]["changes"].as_i64().expect("changes")
}

/// A completed home-workspace exchange with every answer part, asked at `asked_at`.
fn home(exchange_id: &str, request_id: &str, asked_at: i64) -> Value {
    json!({"mogId": MOG_A, "exchangeId": exchange_id, "requestId": request_id,
           "backendKind": "genie_one_mcp", "profileName": "test-profile",
           "workspaceHost": "example.cloud.databricks.com", "workspaceId": 1234,
           "homeWorkspace": true, "conversationId": "test-conv",
           "questionUser": "What was Q3 revenue?", "questionSentSha256": "0".repeat(64),
           "status": "completed", "answerSummary": "Revenue rose 4.2 % to $1.3M in Q3.",
           "keyNumbers": [{"text": "4.2 %", "label": "growth"}, {"text": "$1.3M"}],
           "finalAnswerMd": ANSWER_MD, "deepLink": DEEP_LINK, "askedAt": asked_at,
           "completedAt": asked_at + 5_000, "rowHlc": asked_at, "deviceId": "test-device"})
}

fn get(s: &SqlCipherLocalStore, mog: &str, request_id: &str) -> Vec<Value> {
    rows(&exec(
        s,
        "mem.exchange.get",
        json!({"mogId": mog, "requestId": request_id}),
    ))
    .clone()
}

// ---- MEM-25a: summary, key numbers and pointer ----------------------------------------------

#[test]
fn mem_25a_exchange_row_has_summary_key_numbers_and_pointer() {
    let t = TempDir::new("25a-home");
    let s = open(&t);
    let mut x = home(EXCH_1, REQ_1, T0);
    x["sessionId"] = Value::Null;
    assert_eq!(changes(&exec(&s, "mem.exchange.upsert", x)), 1);

    let got = get(&s, MOG_A, REQ_1);
    assert_eq!(got.len(), 1);
    let r = &got[0];
    assert_eq!(r["exchangeId"], EXCH_1);
    assert_eq!(r["status"], "completed");
    assert_eq!(r["answerSummary"], "Revenue rose 4.2 % to $1.3M in Q3.");
    let kn: Value = serde_json::from_str(r["keyNumbers"].as_str().expect("key numbers")).unwrap();
    assert_eq!(
        kn,
        json!([{"text": "4.2 %", "label": "growth"}, {"text": "$1.3M"}])
    );
    // The pointer: the deep link and the ids to resume the conversation.
    assert_eq!(r["deepLink"], DEEP_LINK);
    assert_eq!(r["conversationId"], "test-conv");
    assert_eq!(r["workspaceHost"], "example.cloud.databricks.com");
    assert_eq!(r["finalAnswerMd"], ANSWER_MD);
    // The extra pointer fields that Forget (#123) and split cards (G-50) use.
    assert!(r.get("sessionId").is_some(), "get returns sessionId");
    assert!(r.get("splitGroupId").is_some(), "get returns splitGroupId");
    assert_eq!(r["questionUser"], "What was Q3 revenue?");
}

#[test]
fn mem_25a_non_home_exchange_keeps_pointer_and_full_answer_only() {
    let t = TempDir::new("25a-nonhome");
    let s = open(&t);
    let mut x = home(EXCH_1, REQ_1, T0);
    let o = x.as_object_mut().unwrap();
    o.insert("homeWorkspace".into(), json!(false));
    o.remove("answerSummary");
    o.remove("keyNumbers");
    o.remove("questionUser");
    assert_eq!(changes(&exec(&s, "mem.exchange.upsert", x)), 1);
    let r = &get(&s, MOG_A, REQ_1)[0];
    assert_eq!(r["deepLink"], DEEP_LINK);
    assert_eq!(r["finalAnswerMd"], ANSWER_MD);
    assert_eq!(r["answerSummary"], Value::Null);
    assert_eq!(r["keyNumbers"], "[]");
    assert_eq!(r["homeWorkspace"], 0);

    // D60: a summary for another workspace is refused by the table CHECK.
    let mut bad = home(EXCH_2, REQ_2, T0);
    bad["homeWorkspace"] = json!(false);
    assert!(s.exec("mem.exchange.upsert", bad).is_err());
}

// ---- MEM-26: the full answer stays until Wipe ------------------------------------------------

#[test]
fn mem_26_a_later_upsert_never_erases_the_answer() {
    let t = TempDir::new("26-keep");
    let s = open(&t);
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));

    // A late or repeated write for the same request with no answer parts (for example a
    // `pending` event that arrives after the answer, or a resume).
    let mut late = home(EXCH_2, REQ_1, T0);
    let o = late.as_object_mut().unwrap();
    for k in [
        "answerSummary",
        "keyNumbers",
        "finalAnswerMd",
        "deepLink",
        "completedAt",
    ] {
        o.remove(k);
    }
    o.insert("status".into(), json!("uncertain"));
    o.insert("rowHlc".into(), json!(T0 + 10));
    assert_eq!(changes(&exec(&s, "mem.exchange.upsert", late)), 1);

    let r = &get(&s, MOG_A, REQ_1)[0];
    assert_eq!(r["exchangeId"], EXCH_1, "the first exchange id stays");
    assert_eq!(
        r["status"], "completed",
        "a late event never reopens a finished exchange"
    );
    assert_eq!(r["finalAnswerMd"], ANSWER_MD);
    assert_eq!(r["answerSummary"], "Revenue rose 4.2 % to $1.3M in Q3.");
    assert_ne!(r["keyNumbers"], "[]");
    assert_eq!(r["deepLink"], DEEP_LINK);
    assert_eq!(r["completedAt"], T0 + 5_000);
}

#[test]
fn mem_26_answer_present_after_400_days_and_wipe_30d_removes_it() {
    let t = TempDir::new("26-400d");
    let s = open(&t);
    // Exchange 1 is asked at T0; exchange 2 five days before "now".
    let now = T0 + 400 * DAY;
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));
    exec(
        &s,
        "mem.exchange.upsert",
        home(EXCH_2, REQ_2, now - 5 * DAY),
    );

    // 400 d later, with no Wipe, the full answer is still there and has no expiry.
    let r = &get(&s, MOG_A, REQ_1)[0];
    assert_eq!(r["finalAnswerMd"], ANSWER_MD);
    let expires: Option<i64> = s
        .with_conn(|c| {
            c.query_row(
                "SELECT expires_at FROM genie_exchange WHERE exchange_id = ?1",
                [EXCH_1],
                |row| row.get(0),
            )
        })
        .unwrap();
    assert_eq!(expires, None, "no TTL on an exchange row (U9)");

    // Settings -> Wipe "older than 90 d", then "older than 30 d" (R14, Core statements).
    let wipe = |days: i64| {
        changes(&exec(
            &s,
            "mem.wipe.exchanges_before",
            json!({"mogId": MOG_A, "before": now - days * DAY}),
        ))
    };
    assert_eq!(
        wipe(90),
        1,
        "older than 90 d removes only the 400 d exchange"
    );
    assert!(get(&s, MOG_A, REQ_1).is_empty());
    assert_eq!(get(&s, MOG_A, REQ_2).len(), 1);
    assert_eq!(wipe(30), 0, "the 5 d exchange is younger than 30 d");
    assert_eq!(get(&s, MOG_A, REQ_2).len(), 1);

    // The same check for "older than 30 d" alone.
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));
    assert_eq!(wipe(30), 1, "older than 30 d removes the 400 d exchange");
    assert!(get(&s, MOG_A, REQ_1).is_empty());
    assert_eq!(get(&s, MOG_A, REQ_2)[0]["finalAnswerMd"], ANSWER_MD);
}

#[test]
fn mem_26_no_statement_or_trigger_purges_exchanges_by_age() {
    for st in STATEMENTS {
        let sql = st.sql.to_ascii_lowercase();
        let deletes = sql.contains("delete from genie_exchange");
        if deletes {
            match st.id {
                // Forget: the exchanges of one session or one MOG, on a user action (MEM-32a).
                "mem.forget.session_exchanges" | "mem.forget.mog_exchanges" => {
                    assert!(!sql.contains("_at"), "{}: Forget has no age rule", st.id);
                }
                // Wipe: Settings only, through R14 (U9).
                "mem.wipe.exchanges_before" => assert_eq!(st.access, Access::Core),
                other => panic!("{other} deletes exchanges"),
            }
        }
        if sql.contains("genie_exchange") {
            assert!(!sql.contains("expires_at"), "{}: no TTL (U9)", st.id);
            // The hub delete tombstone erases the text on purpose (ADR 0121).
            assert!(
                st.id == "mem.exchange.delete" || !sql.contains("final_answer_md = null"),
                "{}: never clears the answer",
                st.id
            );
        }
    }
    let t = TempDir::new("26-triggers");
    let s = open(&t);
    let n: i64 = s
        .with_conn(|c| {
            c.query_row(
                "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' \
                 AND sql LIKE '%genie_exchange%'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(n, 0, "no trigger touches genie_exchange");
}

// ---- MEM-32a: Forget can find the dependents before it deletes -------------------------------

#[test]
fn mem_32a_exchange_dependents_and_session_exchanges_are_found() {
    let t = TempDir::new("32a-deps");
    let s = open(&t);
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO session (session_id, mog_id, kind, started_at, row_hlc, origin_device_id) \
             VALUES (?1, ?2, 'push_to_talk', ?3, 1, 'test-device')",
            rusqlite::params![SESSION_1, MOG_A, T0],
        )
    })
    .unwrap();
    let mut x = home(EXCH_1, REQ_1, T0);
    x["sessionId"] = json!(SESSION_1);
    exec(&s, "mem.exchange.upsert", x);
    exec(&s, "mem.exchange.upsert", home(EXCH_2, REQ_2, T0));
    exec(
        &s,
        "mem.item.upsert",
        json!({"mogId": MOG_A, "itemId": ITEM_1, "itemType": "genie_answer",
               "text": "Q3 revenue rose 4.2 %.", "source": "genie", "confidence": 0.9,
               "sourceExchangeId": EXCH_1, "seenAt": T0, "rowHlc": 1, "deviceId": "test-device"}),
    );

    let deps = exec(
        &s,
        "mem.exchange.dependents",
        json!({"mogId": MOG_A, "exchangeId": EXCH_1}),
    );
    assert_eq!(rows(&deps), &vec![json!({"itemId": ITEM_1})]);

    let of_session = exec(
        &s,
        "mem.exchange.by_session",
        json!({"mogId": MOG_A, "sessionId": SESSION_1}),
    );
    assert_eq!(
        rows(&of_session),
        &vec![json!({"exchangeId": EXCH_1, "requestId": REQ_1})]
    );
    assert_eq!(get(&s, MOG_A, REQ_1)[0]["sessionId"], SESSION_1);

    // MEM-01: another MOG finds nothing.
    let other = exec(
        &s,
        "mem.exchange.dependents",
        json!({"mogId": MOG_B, "exchangeId": EXCH_1}),
    );
    assert!(rows(&other).is_empty());
    let other = exec(
        &s,
        "mem.exchange.by_session",
        json!({"mogId": MOG_B, "sessionId": SESSION_1}),
    );
    assert!(rows(&other).is_empty());

    // Both are brain reads, so T10 runs Forget (#123) through mem_local_exec.
    for id in ["mem.exchange.dependents", "mem.exchange.by_session"] {
        let st = super::catalog::find(id).expect(id);
        assert_eq!(st.access, Access::Brain, "{id}");
    }
}

// ---- #103: the hub Conversations list reads the newest exchanges of one MOG ------------------

#[test]
fn hub_103_exchange_recent_lists_newest_first_inside_one_mog() {
    let t = TempDir::new("103-recent");
    let s = open(&t);
    const EXCH_3: &str = "31111111-2222-4333-8444-555555555503";
    const REQ_3: &str = "41111111-2222-4333-8444-555555555503";
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));
    let mut failed = home(EXCH_2, REQ_2, T0 + DAY);
    failed["status"] = json!("failed");
    for k in [
        "answerSummary",
        "keyNumbers",
        "finalAnswerMd",
        "deepLink",
        "completedAt",
    ] {
        failed[k] = Value::Null;
    }
    exec(&s, "mem.exchange.upsert", failed);
    let mut other = home(EXCH_3, REQ_3, T0 + 2 * DAY);
    other["mogId"] = json!(MOG_B);
    exec(&s, "mem.exchange.upsert", other);

    let got = exec(
        &s,
        "mem.exchange.recent",
        json!({"mogId": MOG_A, "limit": 50}),
    );
    let r = rows(&got);
    // Newest first; MEM-01: the MOG B row is not in the MOG A list.
    let ids: Vec<&str> = r.iter().map(|x| x["requestId"].as_str().unwrap()).collect();
    assert_eq!(ids, vec![REQ_2, REQ_1]);
    assert_eq!(r[0]["status"], "failed");
    assert_eq!(r[0]["finalAnswerMd"], Value::Null);
    let a = &r[1];
    assert_eq!(a["status"], "completed");
    assert_eq!(a["questionUser"], "What was Q3 revenue?");
    assert_eq!(a["finalAnswerMd"], ANSWER_MD);
    assert_eq!(a["profileName"], "test-profile");
    assert_eq!(a["workspaceHost"], "example.cloud.databricks.com");
    assert_eq!(a["askedAt"], T0);
    assert_eq!(a["completedAt"], T0 + 5_000);
    // #523: the stored `deep_link` (the URL Genie One returned) reaches the hub, so "Open in
    // Databricks" needs no new column and no migration. A failed row has none.
    assert_eq!(a["deepLink"], DEEP_LINK);
    assert_eq!(r[0]["deepLink"], Value::Null);
    let kn: Value = serde_json::from_str(a["keyNumbers"].as_str().expect("key numbers")).unwrap();
    assert_eq!(
        kn,
        json!([{"text": "4.2 %", "label": "growth"}, {"text": "$1.3M"}])
    );

    // The limit caps the rows; the newest stay.
    let one = exec(
        &s,
        "mem.exchange.recent",
        json!({"mogId": MOG_A, "limit": 1}),
    );
    assert_eq!(rows(&one).len(), 1);
    assert_eq!(rows(&one)[0]["requestId"], REQ_2);

    // A forgotten (soft-deleted) exchange is not listed.
    s.with_conn(|c| {
        c.execute(
            "UPDATE genie_exchange SET deleted_at = ?1 WHERE request_id = ?2",
            rusqlite::params![T0 + 3 * DAY, REQ_2],
        )
    })
    .unwrap();
    let after = exec(
        &s,
        "mem.exchange.recent",
        json!({"mogId": MOG_A, "limit": 50}),
    );
    assert_eq!(rows(&after).len(), 1);

    // A read the brain runs through mem_local_exec, like mem.exchange.get.
    let st = super::catalog::find("mem.exchange.recent").expect("recent");
    assert_eq!(st.access, Access::Brain);
    assert_eq!(st.kind, super::catalog::Kind::Query);
}

/// #(genie-followup): the hub thread view groups exchanges by `conversationId`, so the recent
/// list must carry it. A follow-up (#584, ADR 0097) keeps the same id across exchanges.
#[test]
fn genie_followup_exchange_recent_carries_the_conversation_id() {
    let t = TempDir::new("followup-recent");
    let s = open(&t);
    const EXCH_3: &str = "31111111-2222-4333-8444-555555555504";
    const REQ_3: &str = "41111111-2222-4333-8444-555555555504";
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));
    let mut followup = home(EXCH_3, REQ_3, T0 + 1_000);
    followup["requestId"] = json!(REQ_3);
    followup["exchangeId"] = json!(EXCH_3);
    // Same conversation id as EXCH_1 (`home()` always uses "test-conv"): a real follow-up.
    exec(&s, "mem.exchange.upsert", followup);

    let got = exec(
        &s,
        "mem.exchange.recent",
        json!({"mogId": MOG_A, "limit": 50}),
    );
    let r = rows(&got);
    assert_eq!(r.len(), 2);
    for row in r {
        assert_eq!(row["conversationId"], "test-conv");
    }
}

// ---- ADR 0121: agent turn rows and the local Conversations delete -------------------------------

const TURN_REQ: &str = "41111111-2222-4333-8444-555555555510";
const TURN_EXCH: &str = "31111111-2222-4333-8444-555555555510";
const CALL_REQ: &str = "41111111-2222-4333-8444-555555555511";
const CALL_EXCH: &str = "31111111-2222-4333-8444-555555555511";
const ROOT: &str = "41111111-2222-4333-8444-555555555512";

/// A turn row (`genie_agent`) with the thread root in `split_group_id`, and a call row of that turn.
fn seed_turn(s: &SqlCipherLocalStore) {
    let mut turn = home(TURN_EXCH, TURN_REQ, T0);
    {
        let o = turn.as_object_mut().unwrap();
        o.insert("backendKind".into(), json!("genie_agent"));
        o.insert("splitGroupId".into(), json!(ROOT));
        o.insert("conversationId".into(), Value::Null);
        o.insert("answerSummary".into(), Value::Null);
    }
    exec(s, "mem.exchange.upsert", turn);
    let mut call = home(CALL_EXCH, CALL_REQ, T0 + 1_000);
    call["splitGroupId"] = json!(TURN_REQ);
    exec(s, "mem.exchange.upsert", call);
}

#[test]
fn adr_0121_recent_returns_backend_kind_and_split_group_id() {
    let t = TempDir::new("0121-recent");
    let s = open(&t);
    seed_turn(&s);
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0 + 2_000));
    let got = exec(
        &s,
        "mem.exchange.recent",
        json!({"mogId": MOG_A, "limit": 50}),
    );
    let by = |req: &str| {
        rows(&got)
            .iter()
            .find(|r| r["requestId"] == req)
            .unwrap()
            .clone()
    };
    let turn = by(TURN_REQ);
    assert_eq!(turn["backendKind"], "genie_agent");
    assert_eq!(turn["splitGroupId"], ROOT);
    let call = by(CALL_REQ);
    assert_eq!(call["backendKind"], "genie_one_mcp");
    assert_eq!(call["splitGroupId"], TURN_REQ);
    let plain = by(REQ_1);
    assert_eq!(plain["splitGroupId"], Value::Null);
}

fn delete(s: &SqlCipherLocalStore, mog: &str, ids: Value) -> i64 {
    changes(&exec(
        s,
        "mem.exchange.delete",
        json!({"mogId": mog, "requestIds": ids, "now": T0 + 10 * DAY}),
    ))
}

/// The row of one request, deleted or not: id, conversation id, deleted_at and the text parts.
fn raw(s: &SqlCipherLocalStore, request_id: &str) -> (Option<String>, Option<i64>, bool, String) {
    s.with_conn(|c| {
        c.query_row(
            "SELECT conversation_id, deleted_at, question_user IS NULL AND answer_summary IS NULL \
             AND final_answer_md IS NULL, key_numbers FROM genie_exchange WHERE request_id = ?1",
            [request_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
    })
    .unwrap()
}

#[test]
fn adr_0121_delete_removes_only_the_named_ids_of_that_mog() {
    let t = TempDir::new("0121-delete");
    let s = open(&t);
    seed_turn(&s);
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0 + 2_000));
    let mut other = home(EXCH_2, REQ_2, T0 + 3_000);
    other["mogId"] = json!(MOG_B);
    exec(&s, "mem.exchange.upsert", other);

    // MEM-01: another MOG's id in the list matches nothing.
    assert_eq!(delete(&s, MOG_A, json!([REQ_2])), 0);
    assert_eq!(get(&s, MOG_B, REQ_2).len(), 1);
    // Two named ids go (as tombstones); the rest stay.
    assert_eq!(delete(&s, MOG_A, json!([TURN_REQ, CALL_REQ])), 2);
    assert!(get(&s, MOG_A, TURN_REQ).is_empty());
    assert!(get(&s, MOG_A, CALL_REQ).is_empty());
    assert_eq!(get(&s, MOG_A, REQ_1).len(), 1);
    assert_eq!(get(&s, MOG_B, REQ_2).len(), 1);
    // A repeat (a tombstone), an empty list and an unknown id delete nothing.
    assert_eq!(delete(&s, MOG_A, json!([TURN_REQ])), 0);
    assert_eq!(delete(&s, MOG_A, json!([])), 0);
    // A local delete leaves no forget_log row.
    let n: i64 = s
        .with_conn(|c| c.query_row("SELECT count(*) FROM forget_log", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n, 0);
    // The statement is a brain write that passes the MEM-01 guard.
    let st = super::catalog::find("mem.exchange.delete").expect("delete");
    assert_eq!(st.access, Access::Brain);
    assert_eq!(st.kind, super::catalog::Kind::Write);
    super::mog_guard::require_mog_id_filter(st.sql).expect("mog_guard");
}

#[test]
fn adr_0121_delete_is_a_tombstone_that_keeps_the_ids_and_upsert_cannot_revive_it() {
    let t = TempDir::new("0121-tomb");
    let s = open(&t);
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));
    assert_eq!(delete(&s, MOG_A, json!([REQ_1])), 1);
    // The row stays with its conversation id; the text is erased; the reads skip it.
    let (conv, at, erased, kn) = raw(&s, REQ_1);
    assert_eq!(conv.as_deref(), Some("test-conv"));
    assert_eq!(at, Some(T0 + 10 * DAY));
    assert!(erased, "question, summary and answer are NULL");
    assert_eq!(kn, "[]");
    assert!(get(&s, MOG_A, REQ_1).is_empty());
    let list = exec(
        &s,
        "mem.exchange.recent",
        json!({"mogId": MOG_A, "limit": 50}),
    );
    assert!(rows(&list).is_empty());
    // A late upsert of the same request keeps the tombstone and the erased text.
    exec(&s, "mem.exchange.upsert", home(EXCH_1, REQ_1, T0));
    let (conv, at, erased, kn) = raw(&s, REQ_1);
    assert_eq!(conv.as_deref(), Some("test-conv"));
    assert_eq!(at, Some(T0 + 10 * DAY));
    assert!(erased);
    assert_eq!(kn, "[]");
    assert!(get(&s, MOG_A, REQ_1).is_empty());
}

#[test]
fn adr_0121_delete_matches_no_non_uuid_id_and_checks_the_list_shape() {
    let t = TempDir::new("0121-shape");
    let s = open(&t);
    seed_turn(&s);
    // Values that are no request id match nothing, even in a list with a real one removed.
    assert_eq!(
        delete(
            &s,
            MOG_A,
            json!(["%", "*", "' OR 1=1 --", "", "null", "c:test-conv"])
        ),
        0
    );
    assert_eq!(get(&s, MOG_A, TURN_REQ).len(), 1);
    assert_eq!(get(&s, MOG_A, CALL_REQ).len(), 1);
    // The list is 200 strings of 36 characters at most; any other shape is refused.
    let too_many: Vec<String> = (0..201).map(|_| REQ_1.to_owned()).collect();
    let ok: Vec<String> = (0..200).map(|_| REQ_1.to_owned()).collect();
    assert_eq!(delete(&s, MOG_A, json!(ok)), 0);
    for bad in [
        json!(too_many),
        json!([REQ_1, "x".repeat(37)]),
        json!({"a": REQ_1}),
        json!(REQ_1),
        json!([[REQ_1]]),
        json!([7]),
    ] {
        let r = s.exec(
            "mem.exchange.delete",
            json!({"mogId": MOG_A, "requestIds": bad, "now": T0}),
        );
        assert!(r.is_err(), "{bad}");
    }
    assert!(s
        .exec("mem.exchange.delete", json!({"mogId": MOG_A}))
        .is_err());
    assert_eq!(get(&s, MOG_A, TURN_REQ).len(), 1);
}

#[test]
fn adr_0121_a_citing_memory_item_survives_a_deleted_exchange() {
    let t = TempDir::new("0121-cite");
    let s = open(&t);
    seed_turn(&s);
    exec(
        &s,
        "mem.item.write",
        json!({"mogId": MOG_A, "itemId": ITEM_1, "itemType": "fact", "text": "Q3 is up",
               "source": "you", "confidence": 0.9, "sourceExchangeId": CALL_EXCH,
               "seenAt": T0, "rowHlc": 1, "deviceId": "test-device", "now": T0}),
    );
    assert_eq!(delete(&s, MOG_A, json!([CALL_REQ])), 1);
    let src: Option<String> = s
        .with_conn(|c| {
            c.query_row(
                "SELECT source_exchange_id FROM memory_item WHERE item_id = ?1",
                [ITEM_1],
                |r| r.get(0),
            )
        })
        .unwrap();
    // The exchange row stays as a tombstone, so the pointer stays too.
    assert_eq!(src.as_deref(), Some(CALL_EXCH));
}
