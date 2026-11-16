//! R10 LocalStore tests (#35). They use the in-memory key vault double and a temp folder. They
//! never touch the real OS key store, a Databricks profile or a token.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::key_vault::{KeyVault, LocalKey, MemoryKeyVault, KEY_LEN};
use super::mog_guard::{require_mog_id_filter, require_mog_id_param};
use super::{LocalStore, SqlCipherLocalStore, DB_FILE_NAME};
use crate::error::CoreError;

const MOG_A: &str = "0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01";
const MOG_B: &str = "6a7f0c2e-91d4-4b8e-8f1e-2c4b5d6e7f02";

/// A unique temp folder that is removed on drop (no extra crate).
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir().join(format!(
            "bottle-r10-{tag}-{}-{n}",
            std::process::id()
        ));
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

fn open_store(dir: &Path, vault: Arc<MemoryKeyVault>) -> SqlCipherLocalStore {
    let s = SqlCipherLocalStore::new(dir.join("memory"), vault);
    s.open().expect("open");
    s
}

fn files_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn pragma_text(s: &SqlCipherLocalStore, sql: &str) -> String {
    s.with_conn(|c| c.query_row(sql, [], |r| r.get::<_, String>(0)))
        .unwrap()
}

// ---- MEM-08a: one database file -----------------------------------------------------------

#[test]
fn mem_08a_memory_dir_holds_one_database_file() {
    let t = TempDir::new("08a");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    s.with_conn(|c| c.execute_batch("CREATE TABLE probe(mog_id TEXT NOT NULL); INSERT INTO probe VALUES ('x');"))
        .unwrap();
    // While open, WAL adds only the companion files of the same database.
    for f in files_in(&t.path().join("memory")) {
        assert!(
            f == DB_FILE_NAME || f == format!("{DB_FILE_NAME}-wal") || f == format!("{DB_FILE_NAME}-shm"),
            "unexpected file {f}"
        );
    }
    s.close();
    assert_eq!(files_in(&t.path().join("memory")), vec![DB_FILE_NAME.to_string()]);
}

#[test]
fn mem_08a_all_mogs_share_one_file() {
    let t = TempDir::new("08a-mogs");
    let vault = Arc::new(MemoryKeyVault::new());
    let s = open_store(t.path(), vault.clone());
    s.with_conn(|c| {
        c.execute_batch("CREATE TABLE probe(mog_id TEXT NOT NULL, v TEXT)")?;
        c.execute("INSERT INTO probe VALUES (?1, 'a')", [MOG_A])?;
        c.execute("INSERT INTO probe VALUES (?1, 'b')", [MOG_B])?;
        Ok(())
    })
    .unwrap();
    s.close();
    assert_eq!(files_in(&t.path().join("memory")), vec![DB_FILE_NAME.to_string()]);
    let s = open_store(t.path(), vault);
    let n: i64 = s
        .with_conn(|c| c.query_row("SELECT count(DISTINCT mog_id) FROM probe", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n, 2);
}

// ---- MEM-08b / AT-17: SQLCipher 4 and a random 256-bit key in the key store ---------------

#[test]
fn mem_08b_file_unreadable_without_key() {
    let t = TempDir::new("08b");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    s.with_conn(|c| {
        c.execute_batch(
            "CREATE TABLE probe(mog_id TEXT NOT NULL, v TEXT);
             INSERT INTO probe VALUES ('m', 'PLAINTEXT-MARKER-0123456789');",
        )
    })
    .unwrap();
    s.close();
    let path = t.path().join("memory").join(DB_FILE_NAME);
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.starts_with(b"SQLite format 3\0"), "plain SQLite header");
    let marker = b"PLAINTEXT-MARKER-0123456789";
    assert!(!bytes.windows(marker.len()).any(|w| w == marker), "plaintext in file");
    // Without a key, the same SQLCipher build cannot read the file.
    let c = rusqlite::Connection::open(&path).unwrap();
    let r = c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0));
    assert!(r.is_err(), "readable without a key");
}

#[test]
fn mem_08b_wrong_key_fails() {
    let t = TempDir::new("08b-wrong");
    open_store(t.path(), Arc::new(MemoryKeyVault::new())).close();
    let other = MemoryKeyVault::new();
    other.store(&LocalKey::generate().unwrap()).unwrap();
    let s = SqlCipherLocalStore::new(t.path().join("memory"), Arc::new(other));
    assert_eq!(
        s.open(),
        Err(CoreError::StoreUnavailable { component: "R10" })
    );
}

#[test]
fn mem_08b_key_is_random_256_bit_and_reused() {
    let t = TempDir::new("08b-key");
    let vault = Arc::new(MemoryKeyVault::new());
    assert!(vault.load().unwrap().is_none());
    open_store(t.path(), vault.clone()).close();
    let k1 = vault.load().unwrap().expect("key stored on first open");
    assert_eq!(k1.as_bytes().len(), KEY_LEN);
    assert_ne!(k1.as_bytes(), &[0u8; KEY_LEN]);
    // Reopen uses the same key; it does not make a new one.
    open_store(t.path(), vault.clone()).close();
    assert_eq!(vault.load().unwrap().unwrap().as_bytes(), k1.as_bytes());
    // A second store gets a different random key.
    let t2 = TempDir::new("08b-key2");
    let vault2 = Arc::new(MemoryKeyVault::new());
    open_store(t2.path(), vault2.clone()).close();
    assert_ne!(vault2.load().unwrap().unwrap().as_bytes(), k1.as_bytes());
}

#[test]
fn mem_08b_key_debug_hides_bytes() {
    let k = LocalKey::from_bytes([0xab; KEY_LEN]);
    let d = format!("{k:?}");
    assert!(!d.contains("ab") && !d.contains("171"), "{d}");
}

#[test]
fn mem_08b_cipher_is_sqlcipher_4_openssl() {
    let t = TempDir::new("08b-cipher");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    assert!(pragma_text(&s, "PRAGMA cipher_version").starts_with("4."));
    assert_eq!(pragma_text(&s, "PRAGMA cipher_provider"), "openssl");
}

#[cfg(target_os = "macos")]
#[test]
fn mem_08b_macos_key_uses_after_first_unlock_this_device_only() {
    use super::key_vault::macos::{Accessibility, KeychainVault};
    // Builds the item description only. It never calls the real Keychain.
    let spec = KeychainVault::bottle().spec();
    assert_eq!(spec.accessibility, Accessibility::AfterFirstUnlockThisDeviceOnly);
    assert!(spec.data_protection_keychain, "accessibility needs the data protection keychain");
    assert!(!spec.synchronizable, "the key must not sync to iCloud");
}

#[cfg(windows)]
#[test]
fn mem_08b_windows_key_is_local_machine_generic_credential() {
    use super::key_vault::windows::CredentialVault;
    let spec = CredentialVault::bottle().spec();
    assert!(spec.persist_local_machine, "the key must not roam");
    assert!(spec.generic);
}

#[test]
fn mem_09a_lost_key_does_not_delete_the_file() {
    let t = TempDir::new("09a");
    let vault = Arc::new(MemoryKeyVault::new());
    open_store(t.path(), vault.clone()).close();
    vault.forget();
    let s = SqlCipherLocalStore::new(t.path().join("memory"), vault.clone());
    assert_eq!(s.open(), Err(CoreError::StoreKeyLost { component: "R10" }));
    // U9: R10 deletes nothing and makes no new key on its own.
    assert_eq!(files_in(&t.path().join("memory")), vec![DB_FILE_NAME.to_string()]);
    assert!(vault.load().unwrap().is_none());
}

// ---- MEM-08c: WAL and secure_delete -------------------------------------------------------

#[test]
fn mem_08c_journal_mode_is_wal() {
    let t = TempDir::new("08c");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    assert_eq!(pragma_text(&s, "PRAGMA journal_mode"), "wal");
}

#[test]
fn mem_08c_secure_delete_is_on() {
    let t = TempDir::new("08c-sd");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let v: i64 = s
        .with_conn(|c| c.query_row("PRAGMA secure_delete", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(v, 1);
    let sync: i64 = s
        .with_conn(|c| c.query_row("PRAGMA synchronous", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(sync, 1, "synchronous=NORMAL");
    let cache: i64 = s
        .with_conn(|c| c.query_row("PRAGMA cache_size", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(cache, -65_536, "64 MiB page cache (SP-21)");
}

// ---- MEM-08d: FTS5 ------------------------------------------------------------------------

#[test]
fn mem_08d_fts5_is_available() {
    let t = TempDir::new("08d");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let hits: i64 = s
        .with_conn(|c| {
            c.execute_batch(
                "CREATE VIRTUAL TABLE probe_fts USING fts5(text, mog_id UNINDEXED);
                 INSERT INTO probe_fts VALUES ('quarterly revenue by region', 'm');",
            )?;
            c.query_row(
                "SELECT count(*) FROM probe_fts WHERE probe_fts MATCH 'revenue' AND mog_id = 'm'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(hits, 1);
}

// ---- MEM-08e: sqlite-vec ------------------------------------------------------------------

#[test]
fn mem_08e_sqlite_vec_is_registered() {
    let t = TempDir::new("08e");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    assert_eq!(pragma_text(&s, "SELECT vec_version()"), "v0.1.9");
}

#[test]
fn mem_08e_knn_is_partitioned_by_mog_and_model() {
    let t = TempDir::new("08e-knn");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let near = |x: i8| -> Vec<u8> { (0..256).map(|i| if i == 0 { x as u8 } else { 0 }).collect() };
    let rows: Vec<i64> = s
        .with_conn(|c| {
            c.execute_batch(
                "CREATE VIRTUAL TABLE probe_vec USING vec0(
                   mog_id text partition key, embedding_model text partition key,
                   embedding int8[256] distance_metric=l2)",
            )?;
            let mut ins = c.prepare(
                "INSERT INTO probe_vec(rowid, mog_id, embedding_model, embedding) VALUES (?1, ?2, ?3, vec_int8(?4))",
            )?;
            ins.execute(rusqlite::params![1, MOG_A, "qwen3@256", near(100)])?;
            ins.execute(rusqlite::params![2, MOG_B, "qwen3@256", near(100)])?;
            ins.execute(rusqlite::params![3, MOG_A, "gte@256", near(100)])?;
            ins.execute(rusqlite::params![4, MOG_A, "qwen3@256", near(-100)])?;
            let mut q = c.prepare(
                "SELECT rowid FROM probe_vec WHERE embedding MATCH vec_int8(?1) AND k = 10
                 AND mog_id = ?2 AND embedding_model = ?3 ORDER BY distance",
            )?;
            let ids = q
                .query_map(rusqlite::params![near(100), MOG_A, "qwen3@256"], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            Ok(ids)
        })
        .unwrap();
    assert_eq!(rows, vec![1, 4], "other MOGs and models never meet");
}

// ---- MEM-01: every statement filters by mog_id --------------------------------------------

#[test]
fn mem_01_guard_accepts_statements_with_mog_id_filter() {
    for sql in [
        "SELECT item_id, text FROM memory_item WHERE mog_id = ?1 AND status = 'active'",
        "SELECT rowid FROM memory_item_fts WHERE memory_item_fts MATCH :q AND mog_id = :mog_id",
        "UPDATE memory_item SET pinned = 1 WHERE mog_id = ?1 AND item_id = ?2",
        "DELETE FROM kv WHERE mog_id = $mog AND k = $k",
        "INSERT INTO kv (mog_id, k, v) VALUES (?1, ?2, ?3)",
        "INSERT INTO memory_item (item_id, mog_id, text) VALUES (?1, ?2, ?3) \
         ON CONFLICT (item_id) DO UPDATE SET text = excluded.text WHERE memory_item.row_hlc < excluded.row_hlc",
        "SELECT m.item_id FROM memory_item m WHERE m.mog_id = ?1 AND m.rid IN \
         (SELECT rowid FROM memory_item_fts WHERE memory_item_fts MATCH ?2 AND mog_id = ?1)",
    ] {
        assert_eq!(require_mog_id_filter(sql), Ok(()), "{sql}");
    }
}

#[test]
fn mem_01_guard_rejects_statement_without_mog_id() {
    let refused = Err(CoreError::MogIdRequired { component: "R10" });
    for sql in [
        "SELECT item_id FROM memory_item",
        "SELECT item_id FROM memory_item WHERE status = 'active'",
        "SELECT item_id FROM memory_item WHERE status = 'mog_id = ?1'",
        "SELECT item_id FROM memory_item WHERE item_id = ?1 -- mog_id = ?2",
        "SELECT item_id FROM memory_item WHERE mog_id = 'fixed-literal'",
        "SELECT item_id FROM memory_item WHERE mog_id = ?1 OR 1 = 1",
        "SELECT m.item_id FROM memory_item m WHERE m.mog_id = ?1 AND m.rid IN \
         (SELECT rowid FROM memory_item_fts WHERE memory_item_fts MATCH ?2)",
        "UPDATE memory_item SET pinned = 1",
        "DELETE FROM kv",
        "INSERT INTO kv (k, v) VALUES (?1, ?2)",
        "PRAGMA key = 'x'",
        "DROP TABLE memory_item",
        "",
    ] {
        assert_eq!(require_mog_id_filter(sql), refused, "{sql}");
    }
}

#[test]
fn mem_01_exec_rejects_params_without_mog_id() {
    let refused = Err(CoreError::MogIdRequired { component: "R10" });
    assert_eq!(require_mog_id_param(&json!({})).map(|_| ()), refused);
    assert_eq!(require_mog_id_param(&json!({"mogId": ""})).map(|_| ()), refused);
    assert_eq!(require_mog_id_param(&json!({"mogId": 7})).map(|_| ()), refused);
    assert_eq!(require_mog_id_param(&json!({"mogId": "not-a-uuid"})).map(|_| ()), refused);
    assert_eq!(require_mog_id_param(&json!({"mogId": MOG_A})), Ok(MOG_A));

    let t = TempDir::new("01-exec");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    assert_eq!(s.exec("mem.item.get", json!({"itemId": "x"})).map(|_| ()), refused);
}

#[test]
fn mem_01_exec_needs_an_open_store() {
    let t = TempDir::new("01-closed");
    let s = SqlCipherLocalStore::new(t.path().join("memory"), Arc::new(MemoryKeyVault::new()));
    assert_eq!(
        s.exec("mem.item.get", json!({"mogId": MOG_A})),
        Err(CoreError::StoreUnavailable { component: "R10" })
    );
}
