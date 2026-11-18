//! Tests of the fixed local statement catalog and `mem_local_exec` (#37). They use the in-memory
//! key vault double and a temp folder: no OS key store, no Lakebase, no profile, no token.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::catalog::{self, Access, Kind, ParamType, STATEMENTS};
use super::key_vault::MemoryKeyVault;
use super::mog_guard::require_mog_id_filter;
use super::{mem_local_exec, LocalStore, SqlCipherLocalStore, StubLocalStore};
use crate::error::CoreError;

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";
const ITEM_1: &str = "11111111-2222-4333-8444-555555555501";
const EXCH_1: &str = "11111111-2222-4333-8444-555555555502";
const REQ_1: &str = "11111111-2222-4333-8444-555555555503";
const FORGET_1: &str = "11111111-2222-4333-8444-555555555504";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-r10c-{tag}-{}-{n}", std::process::id()));
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

fn seed_mog(s: &SqlCipherLocalStore, mog: &str) {
    exec(
        s,
        "mem.mog.upsert",
        json!({"mogId": mog, "displayName": "Sales", "backendKind": "genie_one_mcp",
               "rowHlc": 1, "deviceId": "test-device"}),
    );
}

fn item(mog: &str, text: &str) -> Value {
    json!({"mogId": mog, "itemId": ITEM_1, "itemType": "decision", "text": text,
           "source": "you", "confidence": 0.9, "seenAt": 1_000, "rowHlc": 1,
           "deviceId": "test-device"})
}

fn exchange(mog: &str, status: &str) -> Value {
    json!({"mogId": mog, "exchangeId": EXCH_1, "requestId": REQ_1,
           "backendKind": "genie_one_mcp", "profileName": "test-profile",
           "workspaceHost": "example.cloud.databricks.com", "homeWorkspace": true,
           "questionSentSha256": "0".repeat(64), "status": status,
           "answerSummary": "Revenue rose 4 %.", "keyNumbers": [{"label": "rev", "value": 4}],
           "finalAnswerMd": "**Revenue** rose 4 %.", "askedAt": 2_000, "rowHlc": 1,
           "deviceId": "test-device"})
}

fn invalid_args() -> Result<Value, CoreError> {
    Err(CoreError::IpcRefused {
        code: "invalid_args",
    })
}

// ---- MEM-34a: only fixed catalog statements run --------------------------------------------

#[test]
fn mem_34a_exec_rejects_an_unknown_stmt_id() {
    let t = TempDir::new("34a-unknown");
    let s = open(&t);
    for id in ["mem.nope", "", "mem.item.get ", "MEM.ITEM.GET"] {
        assert_eq!(
            s.exec(id, json!({"mogId": MOG_A})),
            Err(CoreError::UnknownStatement { component: "R10" }),
            "{id:?}"
        );
    }
}

#[test]
fn mem_34a_mem_local_exec_rejects_an_unknown_stmt_id() {
    let t = TempDir::new("34a-route");
    let s = open(&t);
    for id in [
        "mem.nope",
        "SELECT * FROM memory_item",
        "mem.item.get; DROP TABLE memory_item",
    ] {
        let args = json!({"stmtId": id, "params": {"mogId": MOG_A}});
        assert_eq!(mem_local_exec(&s, args), invalid_args(), "{id:?}");
    }
    // The arguments themselves follow `MemExecArgs` (deny_unknown_fields).
    for bad in [
        json!({"stmtId": "mem.item.get"}),
        json!({"stmtId": "mem.item.get", "params": {}, "sql": "SELECT 1"}),
        json!({"params": {"mogId": MOG_A}}),
        json!([]),
    ] {
        assert_eq!(mem_local_exec(&s, bad.clone()), invalid_args(), "{bad}");
    }
}

#[test]
fn mem_34a_mem_local_exec_refuses_rust_only_statements() {
    let t = TempDir::new("34a-core");
    let s = open(&t);
    seed_mog(&s, MOG_A);
    exec(&s, "mem.item.upsert", item(MOG_A, "Use the EU region."));
    let core: Vec<_> = STATEMENTS
        .iter()
        .filter(|st| st.access == Access::Core)
        .collect();
    assert!(
        core.iter().any(|st| st.id.starts_with("mem.wipe.")),
        "Wipe runs only from R14 in Rust"
    );
    for st in core {
        let args = json!({"stmtId": st.id, "params": {"mogId": MOG_A, "before": i64::MAX}});
        assert_eq!(mem_local_exec(&s, args), invalid_args(), "{}", st.id);
    }
    // The item is still there: the brain could not wipe it.
    let got = exec(
        &s,
        "mem.item.get",
        json!({"mogId": MOG_A, "itemId": ITEM_1}),
    );
    assert_eq!(rows(&got).len(), 1);
    // Rust callers can run the Core statements.
    let wiped = exec(
        &s,
        "mem.wipe.items_before",
        json!({"mogId": MOG_A, "before": i64::MAX}),
    );
    assert_eq!(changes(&wiped), 1);
}

#[test]
fn mem_34a_params_are_schema_validated() {
    let t = TempDir::new("34a-params");
    let s = open(&t);
    seed_mog(&s, MOG_A);
    let bad_params = Err(CoreError::InvalidStatementParams { component: "R10" });
    let mut unknown = item(MOG_A, "x");
    unknown["sql"] = json!("DELETE FROM memory_item");
    let mut missing = item(MOG_A, "x");
    missing.as_object_mut().unwrap().remove("text");
    let mut wrong_type = item(MOG_A, "x");
    wrong_type["confidence"] = json!("high");
    let mut too_long = item(MOG_A, "x");
    too_long["text"] = json!("a".repeat(601));
    let mut bad_uuid = item(MOG_A, "x");
    bad_uuid["itemId"] = json!("not-a-uuid");
    let mut bad_code = item(MOG_A, "x");
    bad_code["itemType"] = json!("decision'; --");
    let mut float_int = item(MOG_A, "x");
    float_int["seenAt"] = json!(1.5);
    for (name, p) in [
        ("unknown key", unknown),
        ("missing key", missing),
        ("wrong type", wrong_type),
        ("too long", too_long),
        ("bad uuid", bad_uuid),
        ("bad code", bad_code),
        ("float for int", float_int),
    ] {
        assert_eq!(s.exec("mem.item.upsert", p), bad_params, "{name}");
    }
    // A missing or bad mogId is the MEM-01 refusal.
    let mut no_mog = item(MOG_A, "x");
    no_mog.as_object_mut().unwrap().remove("mogId");
    assert_eq!(
        s.exec("mem.item.upsert", no_mog),
        Err(CoreError::MogIdRequired { component: "R10" })
    );
    // Params must be an object.
    assert_eq!(
        s.exec("mem.item.get", json!([MOG_A])),
        Err(CoreError::MogIdRequired { component: "R10" })
    );
    // An out-of-range limit is refused.
    assert_eq!(
        s.exec(
            "mem.recall.fts",
            json!({"mogId": MOG_A, "query": "region", "limit": 100_000})
        ),
        bad_params
    );
    // Through the route, every refusal is `invalid_args`.
    let args = json!({"stmtId": "mem.item.upsert", "params": {"mogId": MOG_A}});
    assert_eq!(mem_local_exec(&s, args), invalid_args());
    let args = json!({"stmtId": "mem.item.get", "params": {"itemId": ITEM_1}});
    assert_eq!(mem_local_exec(&s, args), invalid_args());
}

#[test]
fn mem_34a_catalog_ids_are_unique_and_named() {
    let mut seen = BTreeSet::new();
    for st in STATEMENTS {
        assert!(seen.insert(st.id), "duplicate id {}", st.id);
        let parts: Vec<&str> = st.id.split('.').collect();
        assert!(parts.len() >= 3 && parts[0] == "mem", "{}", st.id);
        assert!(
            parts
                .iter()
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_lowercase() || c == '_')),
            "{}",
            st.id
        );
        assert_eq!(catalog::find(st.id).map(|f| f.id), Some(st.id));
    }
    // The ids that later issues use (#38, #114, #199, #123, #124, #102).
    for id in [
        "mem.exchange.upsert",
        "mem.item.upsert",
        "mem.recall.fts",
        "mem.forget.session_conversations",
        "mem.wipe.items_before",
        "mem.outbox.enqueue",
        "mem.outbox.due",
        "mem.outbox.done",
    ] {
        assert!(seen.contains(id), "missing {id}");
    }
    assert!(catalog::find("mem.nope").is_none());
}

/// The `:name` parameters of a statement, in order of first use.
fn sql_params(sql: &str) -> BTreeSet<String> {
    let c: Vec<char> = sql.chars().collect();
    let mut out = BTreeSet::new();
    let mut i = 0;
    let mut in_str = false;
    while i < c.len() {
        if c[i] == '\'' {
            in_str = !in_str;
        } else if !in_str && c[i] == ':' && c.get(i + 1).is_some_and(|n| n.is_ascii_alphabetic()) {
            let start = i + 1;
            i += 1;
            while i < c.len() && c[i].is_ascii_alphanumeric() {
                i += 1;
            }
            out.insert(c[start..i].iter().collect());
            continue;
        }
        i += 1;
    }
    out
}

#[test]
fn mem_34a_every_declared_param_is_bound_and_every_bound_param_is_declared() {
    for st in STATEMENTS {
        let declared: BTreeSet<String> = st.params.iter().map(|p| p.name.to_owned()).collect();
        assert_eq!(
            declared.len(),
            st.params.len(),
            "{}: duplicate param",
            st.id
        );
        assert_eq!(sql_params(st.sql), declared, "{}", st.id);
        let mog = st.params.iter().find(|p| p.name == "mogId");
        assert!(
            mog.is_some_and(|p| p.required && p.ty == ParamType::Uuid),
            "{}: mogId must be a required UUID",
            st.id
        );
        // No positional or other parameter styles.
        assert!(!st.sql.contains('?') && !st.sql.contains('@') && !st.sql.contains('$'));
        // One statement per entry.
        assert!(
            !st.sql.trim_end().trim_end_matches(';').contains(';'),
            "{}",
            st.id
        );
        let starts = st.sql.trim_start().to_ascii_lowercase();
        let is_query = starts.starts_with("select") || starts.starts_with("with");
        assert_eq!(is_query, st.kind == Kind::Query, "{}", st.id);
    }
}

#[test]
fn mem_34a_every_statement_prepares_on_the_migrated_schema() {
    let t = TempDir::new("34a-prepare");
    let s = open(&t);
    for st in STATEMENTS {
        s.with_conn(|c| c.prepare(st.sql).map(|_| ()))
            .unwrap_or_else(|e| panic!("{}: {e:?}", st.id));
    }
}

#[test]
fn mem_34a_ts_statement_ids_match_the_brain_catalog() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/brain/components/memory-store.ts");
    let ts = std::fs::read_to_string(&path).expect("memory-store.ts");
    let start = ts.find("LOCAL_STMT_IDS = [").expect("LOCAL_STMT_IDS list");
    let body = &ts[start..start + ts[start..].find(']').expect("end of list")];
    let ts_ids: BTreeSet<&str> = body.split('\'').skip(1).step_by(2).collect();
    let brain: BTreeSet<&str> = STATEMENTS
        .iter()
        .filter(|st| st.access == Access::Brain)
        .map(|st| st.id)
        .chain(
            catalog::GROUPS
                .iter()
                .filter(|g| g.access == Access::Brain)
                .map(|g| g.id),
        )
        .collect();
    assert_eq!(ts_ids, brain);
}

// ---- MEM-01: every catalog statement filters by mog_id --------------------------------------

#[test]
fn mem_01_every_catalog_statement_filters_by_mog_id() {
    assert!(!STATEMENTS.is_empty());
    for st in STATEMENTS {
        assert_eq!(require_mog_id_filter(st.sql), Ok(()), "{}", st.id);
    }
}

#[test]
fn mem_01_reads_writes_and_deletes_stay_inside_their_mog() {
    let t = TempDir::new("01-scope");
    let s = open(&t);
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    exec(&s, "mem.item.upsert", item(MOG_A, "Use the EU region."));
    // B cannot read A's item.
    let got = exec(
        &s,
        "mem.item.get",
        json!({"mogId": MOG_B, "itemId": ITEM_1}),
    );
    assert!(rows(&got).is_empty());
    // B cannot overwrite A's item through the upsert conflict path.
    let up = exec(&s, "mem.item.upsert", item(MOG_B, "Use the US region."));
    assert_eq!(changes(&up), 0);
    let got = exec(
        &s,
        "mem.item.get",
        json!({"mogId": MOG_A, "itemId": ITEM_1}),
    );
    assert_eq!(rows(&got)[0]["text"], "Use the EU region.");
    // B's recall does not see A's item.
    let hits = exec(
        &s,
        "mem.recall.fts",
        json!({"mogId": MOG_B, "query": "region", "limit": 10}),
    );
    assert!(rows(&hits).is_empty());
    // B cannot forget A's item.
    let del = exec(
        &s,
        "mem.forget.items",
        json!({"mogId": MOG_B, "itemIds": [ITEM_1], "forgetId": FORGET_1,
               "requestedAt": 20, "deviceId": "test-device"}),
    );
    assert_eq!(changes(&del), 0);
    // Same for exchanges.
    exec(&s, "mem.exchange.upsert", exchange(MOG_A, "pending"));
    let up = exec(&s, "mem.exchange.upsert", exchange(MOG_B, "failed"));
    assert_eq!(changes(&up), 0);
    let got = exec(
        &s,
        "mem.exchange.get",
        json!({"mogId": MOG_B, "requestId": REQ_1}),
    );
    assert!(rows(&got).is_empty());
    let got = exec(
        &s,
        "mem.exchange.get",
        json!({"mogId": MOG_A, "requestId": REQ_1}),
    );
    assert_eq!(rows(&got)[0]["status"], "pending");
}

// ---- Round trip of the brain statements (MEM-41: no Lakebase anywhere) ----------------------

#[test]
fn mem_41_brain_statements_run_on_the_local_store_with_no_lakebase() {
    let t = TempDir::new("41-roundtrip");
    let s = open(&t);
    let run = |id: &str, params: Value| {
        mem_local_exec(&s, json!({"stmtId": id, "params": params}))
            .unwrap_or_else(|e| panic!("{id}: {e:?}"))
    };
    run(
        "mem.mog.upsert",
        json!({"mogId": MOG_A, "displayName": "Sales", "backendKind": "genie_one_mcp",
               "rowHlc": 1, "deviceId": "test-device"}),
    );
    // Item write (the brain's only item write, #114), a second write counts the mention, then
    // get and recall.
    let mut first = item(MOG_A, "Use the EU region.");
    first["now"] = json!(1_000);
    assert_eq!(changes(&run("mem.item.write", first)), 1);
    let mut again = item(MOG_A, "Use the EU region for new tables.");
    again["seenAt"] = json!(5_000);
    again["now"] = json!(5_000);
    assert_eq!(changes(&run("mem.item.write", again)), 1);
    let got = run("mem.item.get", json!({"mogId": MOG_A, "itemId": ITEM_1}));
    let row = &rows(&got)[0];
    assert_eq!(row["itemId"], ITEM_1);
    assert_eq!(row["type"], "decision");
    assert_eq!(row["text"], "Use the EU region for new tables.");
    assert_eq!(row["mentionCount"], 2);
    assert_eq!(row["firstSeenAt"], 1_000);
    assert_eq!(row["lastSeenAt"], 5_000);
    let hits = run(
        "mem.recall.fts",
        json!({"mogId": MOG_A, "query": "tables region", "limit": 5}),
    );
    assert_eq!(rows(&hits).len(), 1);
    assert_eq!(rows(&hits)[0]["itemId"], ITEM_1);
    assert!(rows(&hits)[0]["score"].is_number());

    // Exchange upsert (pending, then completed) and get.
    run("mem.exchange.upsert", exchange(MOG_A, "pending"));
    let mut done = exchange(MOG_A, "completed");
    done["completedAt"] = json!(3_000);
    done["conversationId"] = json!("conv-1");
    run("mem.exchange.upsert", done);
    let got = run(
        "mem.exchange.get",
        json!({"mogId": MOG_A, "requestId": REQ_1}),
    );
    let row = &rows(&got)[0];
    assert_eq!(row["status"], "completed");
    assert_eq!(row["completedAt"], 3_000);
    assert_eq!(row["conversationId"], "conv-1");
    assert_eq!(row["finalAnswerMd"], "**Revenue** rose 4 %.");
    assert_eq!(row["homeWorkspace"], 1);

    // Outbox: enqueue, due, retry (not due any more), done.
    run(
        "mem.outbox.enqueue",
        json!({"mogId": MOG_A, "kind": "embed", "op": "item.embed", "refId": ITEM_1,
               "now": 10}),
    );
    let due = run(
        "mem.outbox.due",
        json!({"mogId": MOG_A, "kind": "embed", "now": 10, "limit": 10}),
    );
    assert_eq!(rows(&due).len(), 1);
    let oid = rows(&due)[0]["oid"].as_i64().unwrap();
    assert_eq!(rows(&due)[0]["refId"], ITEM_1);
    let retry = run(
        "mem.outbox.retry",
        json!({"mogId": MOG_A, "oid": oid, "nextAttemptAt": 100, "errorCode": "offline"}),
    );
    assert_eq!(changes(&retry), 1);
    let due = run(
        "mem.outbox.due",
        json!({"mogId": MOG_A, "kind": "embed", "now": 50, "limit": 10}),
    );
    assert!(rows(&due).is_empty());
    assert_eq!(
        changes(&run("mem.outbox.done", json!({"mogId": MOG_A, "oid": oid}))),
        1
    );

    // Forget (#123): the item, its jobs and the log row, in one transaction.
    let out = run(
        "mem.forget.items",
        json!({"mogId": MOG_A, "itemIds": [ITEM_1], "forgetId": FORGET_1,
               "requestedAt": 20, "deviceId": "test-device"}),
    );
    assert_eq!(changes(&out), 1);
    let out = run(
        "mem.forget.mog",
        json!({"mogId": MOG_A, "forgetId": "11111111-2222-4333-8444-555555555505",
               "requestedAt": 21, "deviceId": "test-device"}),
    );
    assert_eq!(
        rows(&out).last().unwrap()["changes"],
        1,
        "the forget_log row of the MOG"
    );
    let got = run(
        "mem.exchange.get",
        json!({"mogId": MOG_A, "requestId": REQ_1}),
    );
    assert!(rows(&got).is_empty());
    let hits = run(
        "mem.recall.fts",
        json!({"mogId": MOG_A, "query": "region", "limit": 5}),
    );
    assert!(rows(&hits).is_empty());
}

#[test]
fn mem_41_a_store_that_is_not_there_answers_not_implemented() {
    let args = json!({"stmtId": "mem.item.get", "params": {"mogId": MOG_A, "itemId": ITEM_1}});
    assert_eq!(
        mem_local_exec(&StubLocalStore, args),
        Err(CoreError::IpcRefused {
            code: "not_implemented"
        })
    );
}

#[test]
fn mem_41_a_closed_store_answers_host_failed() {
    let t = TempDir::new("41-closed");
    let s = SqlCipherLocalStore::new(t.0.join("memory"), Arc::new(MemoryKeyVault::new()));
    let args = json!({"stmtId": "mem.item.get", "params": {"mogId": MOG_A, "itemId": ITEM_1}});
    assert_eq!(
        mem_local_exec(&s, args),
        Err(CoreError::IpcRefused {
            code: "host_failed"
        })
    );
}

// ---- MEM-34b: params are data, never SQL ----------------------------------------------------

#[test]
fn mem_34b_recall_query_text_is_data_not_fts_or_sql_syntax() {
    let t = TempDir::new("34b-fts");
    let s = open(&t);
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    exec(&s, "mem.item.upsert", item(MOG_A, "Use the EU region."));
    for q in [
        "region\" OR mog_id:*",
        "region'; DROP TABLE memory_item; --",
        "NEAR(region use) *",
        "mog_id : region",
    ] {
        let hits = exec(
            &s,
            "mem.recall.fts",
            json!({"mogId": MOG_B, "query": q, "limit": 5}),
        );
        assert!(rows(&hits).is_empty(), "{q}");
        let hits = exec(
            &s,
            "mem.recall.fts",
            json!({"mogId": MOG_A, "query": q, "limit": 5}),
        );
        assert_eq!(rows(&hits).len(), 1, "{q}");
    }
    // A query with no word is refused, not run.
    assert_eq!(
        s.exec(
            "mem.recall.fts",
            json!({"mogId": MOG_A, "query": "\"*\" -- ;", "limit": 5})
        ),
        Err(CoreError::InvalidStatementParams { component: "R10" })
    );
    // The table is still there.
    let got = exec(
        &s,
        "mem.item.get",
        json!({"mogId": MOG_A, "itemId": ITEM_1}),
    );
    assert_eq!(rows(&got).len(), 1);
}

#[test]
fn mem_34b_errors_carry_no_sql_or_content() {
    let t = TempDir::new("34b-errors");
    let s = open(&t);
    // The FK to `mog` fails: no MOG row. The error is the fixed code only.
    let e = s
        .exec("mem.item.upsert", item(MOG_A, "secret plan text"))
        .unwrap_err();
    let shown = format!("{e} {e:?}");
    for leak in ["secret plan", "INSERT", "memory_item", "FOREIGN"] {
        assert!(!shown.contains(leak), "{shown}");
    }
}

// ---- MEM-39a: the store is behind an interface ----------------------------------------------

/// A test double for R10: it records each call and answers one fixed row.
#[derive(Default)]
struct RecordingStore(std::sync::Mutex<Vec<(String, Value)>>);

impl LocalStore for RecordingStore {
    fn open(&self) -> crate::error::CoreResult<()> {
        Ok(())
    }
    fn exec(&self, stmt_id: &str, params: Value) -> crate::error::CoreResult<Value> {
        self.0.lock().unwrap().push((stmt_id.to_owned(), params));
        Ok(json!({"rows": [{"double": true}]}))
    }
}

#[test]
fn mem_39a_a_test_double_replaces_the_store_behind_mem_local_exec() {
    let double = RecordingStore::default();
    let params = json!({"mogId": MOG_A, "itemId": ITEM_1});
    let out = mem_local_exec(&double, json!({"stmtId": "mem.item.get", "params": params}))
        .expect("double answers");
    assert_eq!(out, json!({"rows": [{"double": true}]}));
    assert_eq!(
        *double.0.lock().unwrap(),
        vec![("mem.item.get".to_owned(), params)]
    );
    // The route checks the catalog first: the double never sees an unknown or Rust-only id.
    for id in ["mem.nope", "mem.wipe.items_before"] {
        let args = json!({"stmtId": id, "params": {"mogId": MOG_A}});
        assert_eq!(mem_local_exec(&double, args), invalid_args());
    }
    assert_eq!(double.0.lock().unwrap().len(), 1);
}

// ---- #431: the consent statements of #63 are a device-wide entry set of the catalog -----------

const CONSENT_IDS: [&str; 6] = [
    "consent.session.insert",
    "consent.session.get",
    "consent.notice.insert",
    "consent.notice.latest",
    "consent.list",
    "consent.purge",
];

#[test]
fn sec_32a_mem_01_consent_statements_are_in_the_catalog_with_their_scope() {
    let ids: Vec<&str> = catalog::CONSENT.iter().map(|c| c.stmt.id).collect();
    assert_eq!(ids, CONSENT_IDS);
    let others: BTreeSet<&str> = STATEMENTS
        .iter()
        .map(|s| s.id)
        .chain(catalog::GROUPS.iter().map(|g| g.id))
        .collect();
    for c in catalog::CONSENT {
        let st = &c.stmt;
        assert!(!others.contains(st.id), "{} is also a mem entry", st.id);
        assert_eq!(catalog::find_consent(st.id).map(|f| f.stmt.id), Some(st.id));
        // Rust only: T12 in the brain reaches the records through consent_* (#430), never SQL.
        assert_eq!(st.access, Access::Core, "{}", st.id);
        match c.scope {
            catalog::Scope::Mog => {
                assert_eq!(require_mog_id_filter(st.sql), Ok(()), "{}", st.id);
                let mog = st.params.iter().find(|p| p.name == "mogId");
                assert!(
                    mog.is_some_and(|p| p.required && p.ty == ParamType::Uuid),
                    "{}",
                    st.id
                );
            }
            catalog::Scope::DeviceWide(why) => {
                assert!(!why.is_empty(), "{} needs a reason", st.id)
            }
        }
        // Every consent statement, guarded or not, touches the consent_record table only.
        let lower = st.sql.to_ascii_lowercase();
        for t in [
            "mog ",
            "session ",
            "utterance",
            "genie_exchange",
            "memory_item",
            "sync_state",
            "forget_log",
            "outbox",
            "kv ",
        ] {
            assert!(
                !lower.contains(&format!("from {t}"))
                    && !lower.contains(&format!("into {t}"))
                    && !lower.contains(&format!("join {t}")),
                "{} touches {t}",
                st.id
            );
        }
        assert!(lower.contains("consent_record"), "{}", st.id);
        assert!(!lower.contains("drop "), "{}", st.id);
    }
    // The session statements filter by mog_id; the notice, the list and the purge cannot.
    let mog: Vec<&str> = catalog::CONSENT
        .iter()
        .filter(|c| c.scope == catalog::Scope::Mog)
        .map(|c| c.stmt.id)
        .collect();
    assert_eq!(mog, ["consent.session.insert", "consent.session.get"]);
    assert!(catalog::find_consent("consent.nope").is_none());
}

#[test]
fn sec_32a_mem_34a_consent_params_are_declared_bound_and_typed() {
    for c in catalog::CONSENT {
        let st = &c.stmt;
        let declared: BTreeSet<String> = st.params.iter().map(|p| p.name.to_owned()).collect();
        assert_eq!(
            declared.len(),
            st.params.len(),
            "{}: duplicate param",
            st.id
        );
        assert_eq!(sql_params(st.sql), declared, "{}", st.id);
        assert!(!st.sql.contains('?') && !st.sql.contains('@') && !st.sql.contains('$'));
        assert!(
            !st.sql.trim_end().trim_end_matches(';').contains(';'),
            "{}",
            st.id
        );
        let starts = st.sql.trim_start().to_ascii_lowercase();
        assert_eq!(
            starts.starts_with("select"),
            st.kind == Kind::Query,
            "{}",
            st.id
        );
    }
}

#[test]
fn sec_32a_mem_34a_consent_statements_prepare_on_the_migrated_schema() {
    let t = TempDir::new("431-prepare");
    let s = open(&t);
    for c in catalog::CONSENT {
        s.with_conn(|conn| conn.prepare(c.stmt.sql).map(|_| ()))
            .unwrap_or_else(|e| panic!("{}: {e:?}", c.stmt.id));
    }
}

#[test]
fn sec_32a_mem_34a_exec_and_mem_local_exec_never_run_a_consent_statement() {
    let t = TempDir::new("431-refuse");
    let s = open(&t);
    seed_mog(&s, MOG_A);
    for id in CONSENT_IDS {
        let params = json!({"mogId": MOG_A, "cutoff": i64::MAX});
        assert!(
            matches!(
                s.exec(id, params.clone()),
                Err(CoreError::UnknownStatement { .. })
            ),
            "{id}"
        );
        let args = json!({"stmtId": id, "params": params});
        assert_eq!(mem_local_exec(&s, args), invalid_args(), "{id}");
        assert_eq!(catalog::access_of(id), None, "{id}");
    }
}
