//! Tests of the outbox and the atomic item write (#102, MEM-08f, MEM-10a, MEM-19d, ADR 0068).
//!
//! They use the in-memory key vault double with a plainly fake fixed key and a temp folder: no OS
//! key store, no Lakebase, no profile, no token and no embedding endpoint.
//! - The kill test (AT-18) runs this test binary again as a child process. The child stops the
//!   process between the two steps of `mem.item.write`, with no rollback and no close.
//! - The power-loss test (SP-21 note on #102) cuts the WAL tail. With `synchronous = NORMAL` a
//!   power loss can drop the last commits, so every cut point must keep each item with its job.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};

use super::key_vault::{KeyVault, LocalKey, MemoryKeyVault, KEY_LEN};
use super::{mem_local_exec, LocalStore, SqlCipherLocalStore, DB_FILE_NAME};
use crate::error::CoreError;

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";
const ITEM_1: &str = "21111111-2222-4333-8444-555555555501";

/// The env names of the kill-test child. Only the child process has them.
const CHILD_DIR: &str = "BOTTLE_R10_TEST_CHILD_DIR";
const CHILD_STEP: &str = "BOTTLE_R10_EXIT_AFTER_STEP";

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!("bottle-r10o-{tag}-{}-{n}", std::process::id()));
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

/// A vault with a plainly fake fixed key, so a child process opens the same file.
fn vault() -> Arc<MemoryKeyVault> {
    let v = MemoryKeyVault::new();
    v.store(&LocalKey::from_bytes([0x5a; KEY_LEN])).unwrap();
    Arc::new(v)
}

fn open_at(dir: &Path) -> SqlCipherLocalStore {
    let s = SqlCipherLocalStore::new(dir.join("memory"), vault());
    s.open().expect("open");
    s
}

fn exec(s: &dyn LocalStore, id: &str, params: Value) -> Value {
    s.exec(id, params).unwrap_or_else(|e| panic!("{id}: {e:?}"))
}

fn rows(v: &Value) -> &Vec<Value> {
    v["rows"].as_array().expect("rows array")
}

fn seed_mog(s: &dyn LocalStore, mog: &str) {
    exec(
        s,
        "mem.mog.upsert",
        json!({"mogId": mog, "displayName": "Sales", "backendKind": "genie_one_mcp",
               "rowHlc": 1, "deviceId": "test-device"}),
    );
}

fn item_id(n: u32) -> String {
    format!("21111111-2222-4333-8444-{n:012}")
}

/// The params of `mem.item.write`: the `mem.item.upsert` params plus `now`.
fn write_params(mog: &str, id: &str, text: &str) -> Value {
    json!({"mogId": mog, "itemId": id, "itemType": "fact", "text": text, "source": "you",
           "confidence": 0.9, "seenAt": 1_000, "rowHlc": 1, "deviceId": "test-device",
           "now": 2_000})
}

fn count(s: &SqlCipherLocalStore, sql: &str, mog: &str) -> i64 {
    s.with_conn(|c| c.query_row(sql, [mog], |r| r.get(0)))
        .unwrap()
}

fn items(s: &SqlCipherLocalStore, mog: &str) -> i64 {
    count(s, "SELECT count(*) FROM memory_item WHERE mog_id = ?1", mog)
}

fn embed_jobs(s: &SqlCipherLocalStore, mog: &str) -> i64 {
    count(
        s,
        "SELECT count(*) FROM outbox WHERE mog_id = ?1 AND kind = 'embed'",
        mog,
    )
}

/// Items with no embedding and no embed job, plus embed jobs with no item. Both must be 0.
fn orphans(s: &SqlCipherLocalStore, mog: &str) -> i64 {
    count(
        s,
        "SELECT (SELECT count(*) FROM memory_item m WHERE m.mog_id = ?1 AND m.embedding IS NULL
                   AND NOT EXISTS (SELECT 1 FROM outbox o WHERE o.mog_id = ?1
                                   AND o.kind = 'embed' AND o.ref_id = m.item_id))
              + (SELECT count(*) FROM outbox o WHERE o.mog_id = ?1 AND o.kind = 'embed'
                   AND NOT EXISTS (SELECT 1 FROM memory_item m WHERE m.mog_id = ?1
                                   AND m.item_id = o.ref_id))",
        mog,
    )
}

fn due(s: &dyn LocalStore, mog: &str, kind: &str) -> Vec<Value> {
    rows(&exec(
        s,
        "mem.outbox.due",
        json!({"mogId": mog, "kind": kind, "now": 1_000_000, "limit": 100}),
    ))
    .clone()
}

// ---- MEM-10a: the item row and its outbox row commit in one transaction ----------------------

#[test]
fn mem_10a_item_and_embed_job_commit_in_one_transaction() {
    let t = TempDir::new("10a-tx");
    let s = open_at(&t.0);
    seed_mog(&s, MOG_A);
    let out = exec(
        &s,
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "Q3 target is 4.2M"),
    );
    // One answer row for each step: the item upsert, then the embed job.
    assert_eq!(out, json!({"rows": [{"changes": 1}, {"changes": 1}]}));
    assert_eq!((items(&s, MOG_A), embed_jobs(&s, MOG_A)), (1, 1));
    let jobs = due(&s, MOG_A, "embed");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["refId"], ITEM_1);
    assert_eq!(jobs[0]["kind"], "embed");
    assert_eq!(jobs[0]["op"], "item.embed");
    assert_eq!(jobs[0]["attempts"], 0);
    // The job is a pointer: it holds no item text (SEC rules).
    assert_eq!(jobs[0]["payload"], "{}");
}

#[test]
fn mem_10a_mem_local_exec_runs_the_group_and_refuses_its_rust_only_step() {
    let t = TempDir::new("10a-route");
    let s = open_at(&t.0);
    seed_mog(&s, MOG_A);
    let args = json!({"stmtId": "mem.item.write",
                      "params": write_params(MOG_A, ITEM_1, "Use the EU region.")});
    assert_eq!(
        mem_local_exec(&s, args),
        Ok(json!({"rows": [{"changes": 1}, {"changes": 1}]}))
    );
    // The enqueue step alone is Rust-only: the brain cannot add a job with no item write.
    let step = json!({"stmtId": "mem.outbox.enqueue_item_embed",
                      "params": {"mogId": MOG_A, "itemId": ITEM_1, "now": 1}});
    assert_eq!(
        mem_local_exec(&s, step),
        Err(CoreError::IpcRefused {
            code: "invalid_args"
        })
    );
    // A bad param refuses the whole group, so neither row is written.
    let mut bad = write_params(MOG_A, &item_id(9), "x");
    bad["sql"] = json!("DROP TABLE outbox");
    assert_eq!(
        mem_local_exec(&s, json!({"stmtId": "mem.item.write", "params": bad})),
        Err(CoreError::IpcRefused {
            code: "invalid_args"
        })
    );
    assert_eq!((items(&s, MOG_A), embed_jobs(&s, MOG_A)), (1, 1));
}

// ---- AT-18: a kill between the local write and the outbox row loses no item ------------------

/// The child half of the kill test. With no child env it does nothing. With it, it opens the
/// store and runs `mem.item.write`; the R10 fault hook ends the process (exit 3) right after
/// step `BOTTLE_R10_EXIT_AFTER_STEP`, inside the open transaction.
#[test]
fn at_18_child_writes_an_item_and_dies_between_steps() {
    let Ok(dir) = std::env::var(CHILD_DIR) else {
        return;
    };
    let s = open_at(Path::new(&dir));
    let r = s.exec(
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "Q3 target is 4.2M"),
    );
    // The hook did not stop the process: tell the parent with a different code.
    eprintln!("child: no fault point, result ok = {}", r.is_ok());
    std::process::exit(4);
}

fn run_child(dir: &Path, step: u32) -> Option<i32> {
    let exe = std::env::current_exe().unwrap();
    let out = Command::new(exe)
        .args([
            "--exact",
            "local_store::outbox_tests::at_18_child_writes_an_item_and_dies_between_steps",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD_DIR, dir)
        .env(CHILD_STEP, step.to_string())
        .output()
        .unwrap();
    out.status.code()
}

#[test]
fn mem_10a_at_18_kill_between_item_and_outbox_loses_nothing() {
    for step in [1, 2] {
        let t = TempDir::new(&format!("at18-kill-{step}"));
        {
            let s = open_at(&t.0);
            seed_mog(&s, MOG_A);
            s.close();
        }
        // The child dies after the item row (step 1) or after both rows but before the commit
        // (step 2), with no rollback and no close.
        assert_eq!(
            run_child(&t.0, step),
            Some(3),
            "child must die at step {step}"
        );
        let s = open_at(&t.0);
        // The open transaction rolled back: no half write, no item without its job.
        assert_eq!(
            (items(&s, MOG_A), embed_jobs(&s, MOG_A)),
            (0, 0),
            "step {step}"
        );
        assert_eq!(orphans(&s, MOG_A), 0);
        // The write never answered, so the caller writes it again: both rows are there.
        exec(
            &s,
            "mem.item.write",
            write_params(MOG_A, ITEM_1, "Q3 target is 4.2M"),
        );
        s.close();
        let s = open_at(&t.0);
        assert_eq!((items(&s, MOG_A), embed_jobs(&s, MOG_A)), (1, 1));
    }
}

// ---- SP-21 note: a power loss with synchronous = NORMAL ---------------------------------------

/// Copy the live database and its WAL, cut the WAL at `cut` bytes, open the copy and return
/// (items, embed jobs, orphans). No `-shm` is copied, so SQLite rebuilds the WAL index from the
/// frames it can check, as after a power loss.
fn reopen_with_wal_cut(src: &Path, wal: &[u8], cut: usize, tag: &str) -> (i64, i64, i64) {
    let t = TempDir::new(tag);
    let mem = t.0.join("memory");
    std::fs::create_dir_all(&mem).unwrap();
    std::fs::copy(src.join(DB_FILE_NAME), mem.join(DB_FILE_NAME)).unwrap();
    std::fs::write(mem.join(format!("{DB_FILE_NAME}-wal")), &wal[..cut]).unwrap();
    let s = open_at(&t.0);
    let r = (items(&s, MOG_A), embed_jobs(&s, MOG_A), orphans(&s, MOG_A));
    s.close();
    r
}

#[test]
fn mem_10a_sp_21_power_loss_keeps_item_and_job_together() {
    let t = TempDir::new("sp21-power");
    let s = open_at(&t.0);
    seed_mog(&s, MOG_A);
    // Make sure the schema is in the main file, so each cut point loses only item writes.
    s.with_conn(|c| c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())))
        .unwrap();
    const N: u32 = 6;
    for n in 0..N {
        exec(
            &s,
            "mem.item.write",
            write_params(MOG_A, &item_id(n), &format!("fact {n}")),
        );
    }
    let mem = t.0.join("memory");
    let wal = std::fs::read(mem.join(format!("{DB_FILE_NAME}-wal"))).unwrap();
    assert!(wal.len() > 32, "the item writes are still in the WAL");
    let page = u32::from_be_bytes(wal[8..12].try_into().unwrap()) as usize;
    let frame = 24 + page;
    let frames = (wal.len() - 32) / frame;
    let mut cuts: Vec<usize> = (0..=frames).map(|k| 32 + k * frame).collect();
    // Torn writes: a cut inside a frame header and inside a page.
    cuts.extend((0..frames).flat_map(|k| [32 + k * frame + 10, 32 + k * frame + 24 + page / 2]));
    cuts.sort_unstable();
    let mut seen = std::collections::BTreeSet::new();
    let mut last = 0;
    for (i, cut) in cuts.into_iter().enumerate() {
        let (n_items, n_jobs, n_orphans) = reopen_with_wal_cut(&mem, &wal, cut, &format!("cut{i}"));
        assert_eq!(
            n_orphans, 0,
            "cut at byte {cut}: an item lost its job, or a job its item"
        );
        assert_eq!(n_items, n_jobs, "cut at byte {cut}");
        assert!(n_items >= last, "a longer WAL never holds fewer items");
        last = n_items;
        seen.insert(n_items);
    }
    // The cuts did drop some commits (the test is not empty) and the whole WAL keeps all.
    assert_eq!(last, i64::from(N));
    assert!(
        seen.len() > 2,
        "cut points must drop a part of the commits: {seen:?}"
    );
}

// ---- MEM-08f / AT-04: a queued request survives a restart --------------------------------------

#[test]
fn mem_08f_at_04_queued_embed_job_survives_restart() {
    let t = TempDir::new("08f-restart");
    {
        let s = open_at(&t.0);
        seed_mog(&s, MOG_A);
        exec(
            &s,
            "mem.item.write",
            write_params(MOG_A, ITEM_1, "Q3 target is 4.2M"),
        );
        s.close();
    }
    let s = open_at(&t.0);
    let jobs = due(&s, MOG_A, "embed");
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["refId"], ITEM_1);
    // The item is there too, with no embedding yet.
    let got = exec(
        &s,
        "mem.item.get",
        json!({"mogId": MOG_A, "itemId": ITEM_1}),
    );
    assert_eq!(rows(&got)[0]["text"], "Q3 target is 4.2M");
}

// ---- MEM-19d / SP-21: no duplicate job ---------------------------------------------------------

#[test]
fn mem_19d_sp_21_re_mention_adds_no_duplicate_job() {
    let t = TempDir::new("19d-dup");
    let s = open_at(&t.0);
    seed_mog(&s, MOG_A);
    exec(
        &s,
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "Use the EU region."),
    );
    // A second mention while the job waits: the item counts it, the queue does not grow.
    let out = exec(
        &s,
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "Use the EU region."),
    );
    assert_eq!(out, json!({"rows": [{"changes": 1}, {"changes": 0}]}));
    assert_eq!(embed_jobs(&s, MOG_A), 1);
    // The job runs: the embedder stores a vector and the drain deletes the outbox row.
    s.with_conn(|c| {
        c.execute(
            "UPDATE memory_item SET embedding = x'00', embedding_model = 'test-model',
               embedding_dims = 256 WHERE mog_id = ?1 AND item_id = ?2",
            [MOG_A, ITEM_1],
        )
    })
    .unwrap();
    let oid = due(&s, MOG_A, "embed")[0]["oid"].as_i64().unwrap();
    exec(&s, "mem.outbox.done", json!({"mogId": MOG_A, "oid": oid}));
    // A mention of an item that has its embedding adds no job.
    let out = exec(
        &s,
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "Use the EU region."),
    );
    assert_eq!(out, json!({"rows": [{"changes": 1}, {"changes": 0}]}));
    assert_eq!(embed_jobs(&s, MOG_A), 0);
    assert_eq!(items(&s, MOG_A), 1);
}

// ---- MEM-01: the group stays inside its MOG ----------------------------------------------------

#[test]
fn mem_01_item_write_cannot_enqueue_for_another_mog() {
    let t = TempDir::new("01-group");
    let s = open_at(&t.0);
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    exec(
        &s,
        "mem.item.write",
        write_params(MOG_B, ITEM_1, "B owns this item."),
    );
    // MOG A names B's item id: no row changes and no job is queued for A.
    let out = exec(
        &s,
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "A tries to take it."),
    );
    assert_eq!(out, json!({"rows": [{"changes": 0}, {"changes": 0}]}));
    assert_eq!((items(&s, MOG_A), embed_jobs(&s, MOG_A)), (0, 0));
    let got = exec(
        &s,
        "mem.item.get",
        json!({"mogId": MOG_B, "itemId": ITEM_1}),
    );
    assert_eq!(rows(&got)[0]["text"], "B owns this item.");
    assert_eq!(embed_jobs(&s, MOG_B), 1);
    // The MOG of the call is required (MEM-01).
    let mut no_mog = write_params(MOG_A, ITEM_1, "x");
    no_mog.as_object_mut().unwrap().remove("mogId");
    assert_eq!(
        s.exec("mem.item.write", no_mog),
        Err(CoreError::MogIdRequired { component: "R10" })
    );
}

// ---- MEM-08f: the embed drain reads only embed jobs ------------------------------------------

#[test]
fn mem_08f_due_reads_only_the_named_kind() {
    let t = TempDir::new("08f-kind");
    let s = open_at(&t.0);
    seed_mog(&s, MOG_A);
    exec(
        &s,
        "mem.outbox.enqueue",
        json!({"mogId": MOG_A, "kind": "sync", "op": "item.upsert", "refId": ITEM_1, "now": 1}),
    );
    exec(
        &s,
        "mem.item.write",
        write_params(MOG_A, ITEM_1, "Use the EU region."),
    );
    let embed = due(&s, MOG_A, "embed");
    assert_eq!(embed.len(), 1);
    assert_eq!(embed[0]["kind"], "embed");
    let sync = due(&s, MOG_A, "sync");
    assert_eq!(sync.len(), 1);
    assert_eq!(sync[0]["kind"], "sync");
    // The kind is required, so no caller reads the whole queue by mistake.
    assert_eq!(
        s.exec(
            "mem.outbox.due",
            json!({"mogId": MOG_A, "now": 1_000_000, "limit": 100})
        ),
        Err(CoreError::InvalidStatementParams { component: "R10" })
    );
}

// ---- MEM-34a: the TS list names the group ------------------------------------------------------

#[test]
fn mem_34a_ts_statement_ids_name_the_item_write_group() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src/brain/components/memory-store.ts");
    let ts = std::fs::read_to_string(path).expect("memory-store.ts");
    assert!(ts.contains("'mem.item.write'"));
    assert!(!ts.contains("'mem.outbox.enqueue_item_embed'"));
}
