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
            assert!(
                !sql.contains("final_answer_md = null"),
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
