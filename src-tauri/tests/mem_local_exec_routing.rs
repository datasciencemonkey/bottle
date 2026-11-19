//! #426 (#37): the Tauri host sends `mem_local_exec` to `mog_core::local_store::mem_local_exec`
//! over the one managed R10 store. Only the brain may call it (SEC-02a, ARC-4). The host never
//! calls `LocalStore::exec` directly, so a Rust-only (`Access::Core`) statement such as a Wipe
//! step is refused (MEM-34a). A pin (MEM-28b "remember this") needs a user action; no proof of
//! one crosses from the brain yet, so the host refuses every pin (security-lead, PR #540 review).
//! A rejection is a fixed code with no memory content in it (SEC-03). Every value is plainly fake.

use bottle_lib::ipc::{route, runs_off_main_thread, BrainLink, HostComponents};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::ChannelRef;
use mog_core::local_store::catalog::{self, Access};
use mog_core::local_store::key_vault::{KeyVault, LocalKey};
use mog_core::local_store::{LocalStore, SqlCipherLocalStore};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const MOG: &str = "00000000-0000-4000-8000-0000000000a1";
const ITEM: &str = "00000000-0000-4000-8000-0000000000b1";
const EXCH: &str = "00000000-0000-4000-8000-0000000000c1";
const REQ: &str = "00000000-0000-4000-8000-0000000000d1";
/// The fake memory text. No rejection may hold it.
const MARK: &str = "test-memory-not-real";

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

/// An R10 double: it records each statement it runs and answers one row.
#[derive(Default)]
struct FakeStore {
    seen: Mutex<Vec<(String, Value)>>,
}

impl FakeStore {
    fn seen(&self) -> Vec<(String, Value)> {
        self.seen.lock().unwrap().clone()
    }
}

impl LocalStore for FakeStore {
    fn open(&self) -> CoreResult<()> {
        Ok(())
    }
    fn exec(&self, stmt_id: &str, params: Value) -> CoreResult<Value> {
        self.seen.lock().unwrap().push((stmt_id.to_owned(), params));
        Ok(json!({ "rows": [{ "changes": 1 }] }))
    }
}

/// A store that cannot run a statement (for example a locked or broken file).
struct BrokenStore;

impl LocalStore for BrokenStore {
    fn open(&self) -> CoreResult<()> {
        Ok(())
    }
    fn exec(&self, _stmt_id: &str, _params: Value) -> CoreResult<Value> {
        Err(CoreError::StoreUnavailable { component: "R10" })
    }
}

fn call(from: &str, store: Option<&dyn LocalStore>, args: Value) -> Result<Value, Value> {
    let c = HostComponents {
        mem: store,
        ..HostComponents::none(from, &NoBrain)
    };
    route(c, "mem_local_exec", args)
}

fn exec_args(stmt_id: &str, params: Value) -> Value {
    json!({ "stmtId": stmt_id, "params": params })
}

fn item_write(pinned: Option<bool>) -> Value {
    let mut p = json!({
        "itemId": ITEM, "mogId": MOG, "itemType": "fact", "text": MARK, "source": "you",
        "confidence": 0.9, "seenAt": 1, "now": 1
    });
    if let Some(pin) = pinned {
        p["pinned"] = json!(pin);
    }
    exec_args("mem.item.write", p)
}

fn err(code: &str) -> Result<Value, Value> {
    Err(json!({ "error": code }))
}

#[test]
fn mem_34a_issue_426_an_unknown_stmt_id_from_the_brain_is_invalid_args() {
    let store = FakeStore::default();
    for id in ["mem.nope", "DROP TABLE memory_item", ""] {
        assert_eq!(
            call(
                "brain",
                Some(&store),
                exec_args(id, json!({ "mogId": MOG }))
            ),
            err("invalid_args"),
            "{id:?}"
        );
    }
    assert!(store.seen().is_empty(), "no unknown statement reached R10");
}

#[test]
fn sec_02a_arc_4_issue_426_a_view_gets_window_not_allowed() {
    let store = FakeStore::default();
    for from in ["mog-a1", "hub", "card-a1"] {
        assert_eq!(
            call(
                from,
                Some(&store),
                exec_args(
                    "mem.exchange.get",
                    json!({ "mogId": MOG, "requestId": REQ })
                )
            ),
            err("window_not_allowed"),
            "{from}"
        );
    }
    assert!(store.seen().is_empty(), "no view reached R10");
}

#[test]
fn mem_34a_issue_426_a_rust_only_statement_is_refused() {
    let store = FakeStore::default();
    let core: Vec<&str> = catalog::STATEMENTS
        .iter()
        .filter(|s| s.access == Access::Core)
        .map(|s| s.id)
        .chain(
            catalog::GROUPS
                .iter()
                .filter(|g| g.access == Access::Core)
                .map(|g| g.id),
        )
        .chain([catalog::WIPE_MOG_LIST.stmt.id])
        .collect();
    assert!(
        core.iter().any(|id| id.starts_with("mem.wipe.")),
        "the Wipe steps are Rust-only: {core:?}"
    );
    for id in core {
        assert_eq!(
            call(
                "brain",
                Some(&store),
                exec_args(id, json!({ "mogId": MOG, "before": i64::MAX }))
            ),
            err("invalid_args"),
            "{id}"
        );
    }
    assert!(
        store.seen().is_empty(),
        "no Rust-only statement reached R10"
    );
}

#[test]
fn mem_34a_issue_426_a_brain_statement_returns_its_rows() {
    let store = FakeStore::default();
    let params = json!({ "mogId": MOG, "requestId": REQ });
    assert_eq!(
        call(
            "brain",
            Some(&store),
            exec_args("mem.exchange.get", params.clone())
        ),
        Ok(json!({ "rows": [{ "changes": 1 }] }))
    );
    assert_eq!(store.seen(), vec![("mem.exchange.get".to_owned(), params)]);
}

#[test]
fn mem_28b_sec_issue_426_a_pin_without_a_user_action_is_refused_in_rust() {
    // TODO(#574): accept a pin that a host-minted user-action ticket backs.
    let store = FakeStore::default();
    assert_eq!(
        call("brain", Some(&store), item_write(Some(true))),
        err("invalid_args")
    );
    assert!(store.seen().is_empty(), "the pin did not reach R10");
    // A write that does not pin still goes through.
    assert!(call("brain", Some(&store), item_write(Some(false))).is_ok());
    assert!(call("brain", Some(&store), item_write(None)).is_ok());
    assert_eq!(store.seen().len(), 2);
}

#[test]
fn sec_03_issue_426_no_store_or_a_store_failure_is_host_failed_with_no_content() {
    let args = item_write(None);
    assert_eq!(call("brain", None, args.clone()), err("host_failed"));
    let broken = BrokenStore;
    let got = call("brain", Some(&broken), args);
    assert_eq!(got, err("host_failed"));
    assert!(!format!("{got:?}").contains(MARK));
}

#[test]
fn nfr_issue_426_mem_local_exec_runs_off_the_main_thread() {
    // SQLite IO and the R10 connection lock must not block the IPC thread.
    assert!(runs_off_main_thread("mem_local_exec"));
}

// ---- the real store: the Ask Genie exchange persists (MEM-25a, MEM-34a) ----

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

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-426-{tag}-{}-{n}", std::process::id()));
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

#[test]
fn mem_25a_mem_34a_issue_426_an_exchange_persists_through_the_route() {
    let dir = TempDir::new("exchange");
    let store = SqlCipherLocalStore::new(dir.0.join("memory"), Arc::new(TestVault::default()));
    store.open().unwrap();
    // The caller registers the MOG row first: `genie_exchange.mog_id` references `mog` (ADR 0061).
    let mog = json!({ "mogId": MOG, "displayName": "Sales", "backendKind": "genie_one_mcp",
                      "rowHlc": 1, "deviceId": "test-device" });
    call("brain", Some(&store), exec_args("mem.mog.upsert", mog)).unwrap();
    let x = json!({
        "mogId": MOG, "exchangeId": EXCH, "requestId": REQ, "backendKind": "genie_one_mcp",
        "profileName": "test-profile", "workspaceHost": "example.cloud.databricks.com",
        "workspaceId": 1234, "homeWorkspace": true, "conversationId": "test-conv",
        "questionUser": "test question not real", "questionSentSha256": "0".repeat(64),
        "status": "completed", "answerSummary": MARK, "keyNumbers": [],
        "finalAnswerMd": MARK, "askedAt": 1, "completedAt": 2, "rowHlc": 1,
        "deviceId": "test-device"
    });
    let wrote = call("brain", Some(&store), exec_args("mem.exchange.upsert", x)).unwrap();
    assert_eq!(wrote["rows"][0]["changes"], 1, "{wrote}");
    let got = call(
        "brain",
        Some(&store),
        exec_args(
            "mem.exchange.get",
            json!({ "mogId": MOG, "requestId": REQ }),
        ),
    )
    .unwrap();
    let rows = got["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["exchangeId"], EXCH);
    assert_eq!(rows[0]["finalAnswerMd"], MARK);
}
