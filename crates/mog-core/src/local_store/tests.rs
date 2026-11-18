//! R10 LocalStore tests (#35). They use the in-memory key vault double and a temp folder. They
//! never touch the real OS key store, a Databricks profile or a token.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::key_vault::{KeyVault, LocalKey, MemoryKeyVault, KEY_LEN};
use super::mog_guard::{require_mog_id_filter, require_mog_id_param};
use super::vec_codec::{quantize_int8, INT8_MAX};
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
        let p = std::env::temp_dir().join(format!("bottle-r10-{tag}-{}-{n}", std::process::id()));
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
    s.with_conn(|c| {
        c.execute_batch("CREATE TABLE probe(mog_id TEXT NOT NULL); INSERT INTO probe VALUES ('x');")
    })
    .unwrap();
    // While open, WAL adds only the companion files of the same database.
    for f in files_in(&t.path().join("memory")) {
        assert!(
            f == DB_FILE_NAME
                || f == format!("{DB_FILE_NAME}-wal")
                || f == format!("{DB_FILE_NAME}-shm"),
            "unexpected file {f}"
        );
    }
    s.close();
    assert_eq!(
        files_in(&t.path().join("memory")),
        vec![DB_FILE_NAME.to_string()]
    );
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
    assert_eq!(
        files_in(&t.path().join("memory")),
        vec![DB_FILE_NAME.to_string()]
    );
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
    assert!(
        !bytes.starts_with(b"SQLite format 3\0"),
        "plain SQLite header"
    );
    let marker = b"PLAINTEXT-MARKER-0123456789";
    assert!(
        !bytes.windows(marker.len()).any(|w| w == marker),
        "plaintext in file"
    );
    // Without a key, the same SQLCipher build cannot read the file.
    let c = rusqlite::Connection::open(&path).unwrap();
    let r = c.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    });
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
    assert_eq!(
        spec.accessibility,
        Accessibility::AfterFirstUnlockThisDeviceOnly
    );
    assert!(
        spec.data_protection_keychain,
        "accessibility needs the data protection keychain"
    );
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
fn u9_lost_key_keeps_the_file() {
    // MEM-09a (a new empty store) is MemoryService's job: TODO(#190).
    let t = TempDir::new("09a");
    let vault = Arc::new(MemoryKeyVault::new());
    open_store(t.path(), vault.clone()).close();
    vault.forget();
    let s = SqlCipherLocalStore::new(t.path().join("memory"), vault.clone());
    assert_eq!(s.open(), Err(CoreError::StoreKeyLost { component: "R10" }));
    // U9: R10 deletes nothing and makes no new key on its own.
    assert_eq!(
        files_in(&t.path().join("memory")),
        vec![DB_FILE_NAME.to_string()]
    );
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
    let near = |x: f32| -> Vec<u8> {
        quantize_int8(
            &(0..256)
                .map(|i| if i == 0 { x } else { 0.0 })
                .collect::<Vec<f32>>(),
        )
    };
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
            ins.execute(rusqlite::params![1, MOG_A, "qwen3@256", near(1.0)])?;
            ins.execute(rusqlite::params![2, MOG_B, "qwen3@256", near(1.0)])?;
            ins.execute(rusqlite::params![3, MOG_A, "gte@256", near(1.0)])?;
            ins.execute(rusqlite::params![4, MOG_A, "qwen3@256", near(-1.0)])?;
            let mut q = c.prepare(
                "SELECT rowid FROM probe_vec WHERE embedding MATCH vec_int8(?1) AND k = 10
                 AND mog_id = ?2 AND embedding_model = ?3 ORDER BY distance",
            )?;
            let ids = q
                .query_map(rusqlite::params![near(1.0), MOG_A, "qwen3@256"], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            Ok(ids)
        })
        .unwrap();
    assert_eq!(rows, vec![1, 4], "other MOGs and models never meet");
}

#[test]
fn mem_08e_int8_quantizer_keeps_every_difference_in_range() {
    // sqlite-vec 0.1.9 NEON int8 L2 squares an i16 difference with vmulq_s16, which wraps at
    // |diff| >= 182 and gives a NULL distance. The quantizer keeps values in [-90, 90], so a
    // difference is at most 180 (see mem_08e_neon_l2_is_valid_up_to_a_difference_of_180).
    assert_eq!(INT8_MAX, 90);
    let q = quantize_int8(&[1.0, -1.0, 2.0, -2.0, 0.5, 0.0, f32::NAN]);
    let v: Vec<i8> = q.iter().map(|b| *b as i8).collect();
    assert_eq!(v, vec![INT8_MAX, -INT8_MAX, INT8_MAX, -INT8_MAX, 45, 0, 0]);

    let t = TempDir::new("08e-range");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let full: Vec<f32> = (0..256)
        .map(|i| if i % 2 == 0 { 1.0 } else { -1.0 })
        .collect();
    let flip: Vec<f32> = full.iter().map(|x| -x).collect();
    let d: Option<f64> = s
        .with_conn(|c| {
            c.query_row(
                "SELECT vec_distance_l2(vec_int8(?1), vec_int8(?2))",
                rusqlite::params![quantize_int8(&full), quantize_int8(&flip)],
                |r| r.get(0),
            )
        })
        .unwrap();
    let d = d.expect("distance is not NULL");
    assert!((d - (256.0f64 * 180.0 * 180.0).sqrt()).abs() < 1e-6, "{d}");
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
        "SELECT item_id FROM memory_item WHERE 1 + mog_id = ?1",
        "SELECT item_id FROM memory_item WHERE mog_id = ?1 + 1",
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
    assert_eq!(
        require_mog_id_param(&json!({"mogId": ""})).map(|_| ()),
        refused
    );
    assert_eq!(
        require_mog_id_param(&json!({"mogId": 7})).map(|_| ()),
        refused
    );
    assert_eq!(
        require_mog_id_param(&json!({"mogId": "not-a-uuid"})).map(|_| ()),
        refused
    );
    assert_eq!(require_mog_id_param(&json!({"mogId": MOG_A})), Ok(MOG_A));

    let t = TempDir::new("01-exec");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    assert_eq!(
        s.exec("mem.item.get", json!({"itemId": "x"})).map(|_| ()),
        refused
    );
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

// ---- Migrations and schema (MEM-35a, MEM-08d, MEM-08e, MEM-01, U9) -------------------------

/// (type, name, tbl_name, sql) of every schema object, sorted.
fn schema(s: &SqlCipherLocalStore) -> Vec<(String, String, String, Option<String>)> {
    s.with_conn(|c| {
        let mut st =
            c.prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    })
    .unwrap()
}

fn virtual_tables(s: &SqlCipherLocalStore) -> Vec<String> {
    schema(s)
        .into_iter()
        .filter(|(t, _, _, sql)| {
            t == "table"
                && sql
                    .as_deref()
                    .is_some_and(|q| q.starts_with("CREATE VIRTUAL TABLE"))
        })
        .map(|(_, n, _, _)| n)
        .collect()
}

/// A shadow table of a virtual table (FTS5 `_data`, vec0 `_chunks`, ...) or an SQLite table.
fn is_internal(name: &str, vtabs: &[String]) -> bool {
    name.starts_with("sqlite_")
        || name == "schema_migrations"
        || vtabs.iter().any(|v| name.starts_with(&format!("{v}_")))
}

fn columns(s: &SqlCipherLocalStore, table: &str) -> Vec<(String, String)> {
    s.with_conn(|c| {
        let mut st = c.prepare(&format!("PRAGMA table_xinfo(\"{table}\")"))?;
        let v = st
            .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(v)
    })
    .unwrap()
}

fn user_tables(s: &SqlCipherLocalStore) -> Vec<String> {
    let vtabs = virtual_tables(s);
    schema(s)
        .into_iter()
        .filter(|(t, n, _, _)| t == "table" && !is_internal(n, &vtabs))
        .map(|(_, n, _, _)| n)
        .collect()
}

const MIRROR_TABLES: [&str; 8] = [
    "mog",
    "session",
    "utterance",
    "genie_exchange",
    "memory_item",
    "consent_record",
    "sync_state",
    "forget_log",
];

fn seed_mog(s: &SqlCipherLocalStore, mog: &str) {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO mog (mog_id, display_name, backend_kind, row_hlc, origin_device_id)
             VALUES (?1, 'Sales', 'genie_one_mcp', 1, ?1)",
            [mog],
        )
    })
    .unwrap();
}

fn seed_item(s: &SqlCipherLocalStore, mog: &str, item: &str, text: &str) -> i64 {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO memory_item (item_id, mog_id, item_type, text, source, confidence,
               first_seen_at, last_seen_at, row_hlc, origin_device_id)
             VALUES (?1, ?2, 'fact', ?3, 'you', 0.9, 1, 1, 1, ?2)",
            [item, mog, text],
        )?;
        Ok(c.last_insert_rowid())
    })
    .unwrap()
}

#[test]
fn mem_35a_migrations_apply_on_empty_db() {
    let t = TempDir::new("35a-apply");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let rows: Vec<(i64, String)> = s
        .with_conn(|c| {
            let mut st =
                c.prepare("SELECT version, app_version FROM schema_migrations ORDER BY version")?;
            let v = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(v)
        })
        .unwrap();
    let want: Vec<(i64, String)> = (1..=super::migrate::latest_version())
        .map(|v| (v, env!("CARGO_PKG_VERSION").to_string()))
        .collect();
    assert!(!want.is_empty(), "there is a migration 0001");
    assert_eq!(rows, want);
    let tables = user_tables(&s);
    for t in MIRROR_TABLES.iter().chain(&["outbox", "kv"]) {
        assert!(
            tables.iter().any(|x| x == t),
            "table {t} is missing: {tables:?}"
        );
    }
    assert!(columns(&s, "genie_exchange")
        .iter()
        .any(|(n, _)| n == "final_answer_md"));
}

#[test]
fn mem_35a_reopen_is_idempotent() {
    let t = TempDir::new("35a-reopen");
    let vault = Arc::new(MemoryKeyVault::new());
    let s = open_store(t.path(), vault.clone());
    seed_mog(&s, MOG_A);
    seed_item(&s, MOG_A, "item-1", "revenue grew in EMEA");
    let before = schema(&s);
    s.close();
    let s = open_store(t.path(), vault);
    assert_eq!(schema(&s), before);
    let (n, items): (i64, i64) = s
        .with_conn(|c| {
            c.query_row(
                "SELECT (SELECT count(*) FROM schema_migrations), (SELECT count(*) FROM memory_item)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap();
    assert_eq!(n, super::migrate::latest_version());
    assert_eq!(items, 1);
}

#[test]
fn mem_35a_refuses_newer_schema() {
    let t = TempDir::new("35a-newer");
    let vault = Arc::new(MemoryKeyVault::new());
    let s = open_store(t.path(), vault.clone());
    let future = super::migrate::latest_version() + 1;
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO schema_migrations (version, applied_at, app_version) VALUES (?1, 0, '99.0.0')",
            [future],
        )
    })
    .unwrap();
    s.close();
    let s = SqlCipherLocalStore::new(t.path().join("memory"), vault.clone());
    assert_eq!(
        s.open(),
        Err(CoreError::StoreSchemaNewer { component: "R10" })
    );
    // The newer DB is left as it is: no downgrade, no delete.
    assert_eq!(
        files_in(&t.path().join("memory")),
        vec![DB_FILE_NAME.to_string()]
    );
}

#[test]
fn mem_35a_no_down_migration_exists() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/local_store/migrations");
    let names = files_in(&dir);
    let listed: Vec<String> = super::migrate::MIGRATIONS
        .iter()
        .map(|m| format!("{}.sql", m.name))
        .collect();
    assert_eq!(names, listed, "every file is a listed forward migration");
    for (i, m) in super::migrate::MIGRATIONS.iter().enumerate() {
        assert_eq!(
            m.version,
            i as i64 + 1,
            "versions run 1, 2, 3 ... with no gap"
        );
        assert!(
            m.name.starts_with(&format!("{:04}_", m.version)),
            "{}",
            m.name
        );
        let lower = m.name.to_ascii_lowercase();
        for bad in ["down", "rollback", "revert", "undo"] {
            assert!(
                !lower.contains(bad),
                "{} looks like a down migration",
                m.name
            );
        }
        assert!(
            !m.sql.to_ascii_lowercase().contains("-- down"),
            "{}",
            m.name
        );
    }
}

#[test]
fn mem_08d_schema_has_fts5_tables() {
    let t = TempDir::new("08d-schema");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let fts: Vec<String> = schema(&s)
        .into_iter()
        .filter(|(_, _, _, sql)| sql.as_deref().is_some_and(|q| q.contains("USING fts5")))
        .map(|(_, n, _, _)| n)
        .collect();
    assert_eq!(
        fts,
        vec!["memory_item_fts".to_string(), "utterance_fts".to_string()]
    );
}

#[test]
fn mem_08d_fts5_finds_inserted_item() {
    let t = TempDir::new("08d-find");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    seed_item(&s, MOG_A, "item-a", "quarterly revenue by region");
    seed_item(&s, MOG_B, "item-b", "revenue forecast for next year");
    let hits = |mog: &str| -> Vec<String> {
        s.with_conn(|c| {
            let mut st = c.prepare(
                "SELECT m.item_id FROM memory_item m WHERE m.mog_id = ?1 AND m.rid IN
                 (SELECT rowid FROM memory_item_fts WHERE memory_item_fts MATCH 'revenue' AND mog_id = ?1)",
            )?;
            let v = st
                .query_map([mog], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(v)
        })
        .unwrap()
    };
    assert_eq!(hits(MOG_A), vec!["item-a".to_string()]);
    assert_eq!(hits(MOG_B), vec!["item-b".to_string()]);
    // A text update moves the index entry with it.
    s.with_conn(|c| {
        c.execute(
            "UPDATE memory_item SET text = 'churn by segment' WHERE mog_id = ?1",
            [MOG_A],
        )
    })
    .unwrap();
    assert!(hits(MOG_A).is_empty());
}

#[test]
fn mem_08e_schema_has_vec0_tables() {
    let t = TempDir::new("08e-schema");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let vec: Vec<(String, String)> = schema(&s)
        .into_iter()
        .filter(|(_, _, _, sql)| sql.as_deref().is_some_and(|q| q.contains("USING vec0")))
        .map(|(_, n, _, sql)| (n, sql.unwrap()))
        .collect();
    let names: Vec<&str> = vec.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, vec!["memory_item_vec_1024", "memory_item_vec_256"]);
    for (n, sql) in &vec {
        let dim = n.rsplit('_').next().unwrap();
        assert!(sql.contains(&format!("int8[{dim}]")), "{sql}");
        assert!(sql.contains("mog_id text partition key"), "{sql}");
        assert!(sql.contains("embedding_model text partition key"), "{sql}");
    }
    // A vector of a deleted item leaves the index too (forget, Wipe).
    seed_mog(&s, MOG_A);
    let rid = seed_item(&s, MOG_A, "item-v", "margin by product");
    let v: Vec<f32> = (0..256).map(|i| if i == 3 { 1.0 } else { 0.0 }).collect();
    let knn = || -> Vec<i64> {
        s.with_conn(|c| {
            let mut st = c.prepare(
                "SELECT rowid FROM memory_item_vec_256 WHERE embedding MATCH vec_int8(?1) AND k = 5
                 AND mog_id = ?2 AND embedding_model = 'qwen3@256'",
            )?;
            let ids = st
                .query_map(rusqlite::params![quantize_int8(&v), MOG_A], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<i64>>>()?;
            Ok(ids)
        })
        .unwrap()
    };
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO memory_item_vec_256 (rowid, mog_id, embedding_model, embedding)
             VALUES (?1, ?2, 'qwen3@256', vec_int8(?3))",
            rusqlite::params![rid, MOG_A, quantize_int8(&v)],
        )
    })
    .unwrap();
    assert_eq!(knn(), vec![rid]);
    s.with_conn(|c| {
        c.execute(
            "DELETE FROM memory_item WHERE mog_id = ?1 AND rid = ?2",
            rusqlite::params![MOG_A, rid],
        )
    })
    .unwrap();
    assert!(knn().is_empty());
}

#[test]
fn mem_01_every_table_has_mog_id() {
    let t = TempDir::new("01-tables");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let tables = user_tables(&s);
    assert!(tables.len() >= MIRROR_TABLES.len() + 2, "{tables:?}");
    for t in tables {
        assert!(
            columns(&s, &t).iter().any(|(n, _)| n == "mog_id"),
            "table {t} has no mog_id"
        );
    }
}

#[test]
fn mem_01_every_index_has_mog_id() {
    let t = TempDir::new("01-index");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let vtabs = virtual_tables(&s);
    let indexes: Vec<(String, String, Option<String>)> = schema(&s)
        .into_iter()
        .filter(|(ty, _, tbl, _)| ty == "index" && !is_internal(tbl, &vtabs))
        .map(|(_, n, tbl, sql)| (n, tbl, sql))
        .collect();
    assert!(
        indexes.iter().any(|(_, _, sql)| sql.is_some()),
        "explicit indexes exist"
    );
    for (name, tbl, sql) in indexes {
        let cols: Vec<String> = s
            .with_conn(|c| {
                let mut st = c.prepare(&format!("PRAGMA index_info(\"{name}\")"))?;
                let v = st
                    .query_map([], |r| r.get::<_, String>(2))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(v)
            })
            .unwrap();
        if sql.is_some() {
            assert_eq!(
                cols.first().map(String::as_str),
                Some("mog_id"),
                "index {name} on {tbl}"
            );
        } else {
            // A PRIMARY KEY or UNIQUE autoindex: it has mog_id, or it is one globally unique UUID.
            assert!(
                cols.iter().any(|c| c == "mog_id") || cols.len() == 1,
                "autoindex {name} on {tbl}: {cols:?}"
            );
        }
    }
}

#[test]
fn u9_no_trigger_deletes_user_content() {
    let t = TempDir::new("u9-trig");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let triggers: Vec<(String, String)> = schema(&s)
        .into_iter()
        .filter(|(ty, _, _, _)| ty == "trigger")
        .map(|(_, n, _, sql)| (n, sql.unwrap_or_default().to_ascii_lowercase()))
        .collect();
    assert!(!triggers.is_empty());
    let derived = |name: &str| name.ends_with("_fts") || name.starts_with("memory_item_vec_");
    for (name, sql) in triggers {
        let body = sql.split_once("begin").map(|(_, b)| b).unwrap_or("");
        let words: Vec<&str> = body
            .split(|ch: char| ch.is_whitespace() || ch == '(' || ch == ';')
            .filter(|w| !w.is_empty())
            .collect();
        for (i, w) in words.iter().enumerate() {
            let target = match *w {
                "into" | "from" => words.get(i + 1),
                "update" => words.get(i + 1),
                _ => None,
            };
            if let Some(target) = target {
                assert!(derived(target), "trigger {name} writes to {target}");
            }
        }
    }
    // The schema has no expiry index either: nothing is built to purge by age.
    assert!(!schema(&s)
        .iter()
        .any(|(ty, n, _, _)| ty == "index" && n.contains("expir") && n != "consent_expiry"));
}

#[test]
fn u9_schema_has_no_token_or_audio_column() {
    let t = TempDir::new("u9-cols");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    for table in user_tables(&s) {
        for (col, ty) in columns(&s, &table) {
            let c = col.to_ascii_lowercase();
            for bad in [
                "token",
                "password",
                "secret",
                "credential",
                "audio",
                "pcm",
                "wav",
            ] {
                assert!(!c.contains(bad), "{table}.{col}");
            }
            if ty.eq_ignore_ascii_case("blob") {
                assert_eq!(
                    (table.as_str(), c.as_str()),
                    ("memory_item", "embedding"),
                    "only the f16 cache is a BLOB"
                );
            }
        }
    }
    // kv refuses a key that names a secret.
    let r = s.with_conn(|c| {
        c.execute(
            "INSERT INTO kv (mog_id, k, v) VALUES (?1, 'databricks_token', 'x')",
            [MOG_A],
        )
    });
    assert!(r.is_err());
}

#[test]
fn mem_08f_outbox_request_survives_restart() {
    let t = TempDir::new("08f-outbox");
    let vault = Arc::new(MemoryKeyVault::new());
    let s = open_store(t.path(), vault.clone());
    seed_mog(&s, MOG_A);
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO outbox (mog_id, kind, op, ref_id, payload_json, created_at, next_attempt_at)
             VALUES (?1, 'embed', 'memory_item.embed', 'item-1', '{}', 1, 1)",
            [MOG_A],
        )
    })
    .unwrap();
    s.close();
    let s = open_store(t.path(), vault);
    let (n, kind): (i64, String) = s
        .with_conn(|c| {
            c.query_row(
                "SELECT count(*), max(kind) FROM outbox WHERE mog_id = ?1",
                [MOG_A],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        })
        .unwrap();
    assert_eq!((n, kind.as_str()), (1, "embed"));
    // Only the two queue kinds exist.
    let bad = s.with_conn(|c| {
        c.execute(
            "INSERT INTO outbox (mog_id, kind, op, ref_id, payload_json, created_at, next_attempt_at)
             VALUES (?1, 'upload_audio', 'x', 'y', '{}', 1, 1)",
            [MOG_A],
        )
    });
    assert!(bad.is_err());
}

// ---- MEM-33a: an age-based Wipe deletes only the matching rows (review of #365) ----------

fn seed_exchange(
    s: &SqlCipherLocalStore,
    mog: &str,
    id: &str,
    session: Option<&str>,
    asked_at: i64,
) {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO genie_exchange (exchange_id, request_id, mog_id, session_id, backend_kind,
               profile_name, workspace_host, is_home_workspace, question_sent_sha256, status,
               asked_at, row_hlc, origin_device_id)
             VALUES (?1, ?1, ?2, ?3, 'genie_one_mcp', 'p', 'h', 1, 'x', 'completed', ?4, 1, ?2)",
            rusqlite::params![id, mog, session, asked_at],
        )
    })
    .unwrap();
}

fn seed_session(s: &SqlCipherLocalStore, mog: &str, id: &str, started_at: i64) {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO session (session_id, mog_id, kind, started_at, row_hlc, origin_device_id)
             VALUES (?1, ?2, 'meeting', ?3, 1, ?2)",
            rusqlite::params![id, mog, started_at],
        )
    })
    .unwrap();
}

fn count(s: &SqlCipherLocalStore, sql: &str) -> i64 {
    s.with_conn(|c| c.query_row(sql, [], |r| r.get(0))).unwrap()
}

#[test]
fn mem_33a_wiping_an_old_exchange_keeps_a_recent_item_and_its_index_rows() {
    let t = TempDir::new("33a-exchange");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_exchange(&s, MOG_A, "ex-old", None, 1);
    let rid = seed_item(&s, MOG_A, "item-new", "gross margin by region");
    let v: Vec<f32> = (0..256).map(|i| if i == 7 { 1.0 } else { 0.0 }).collect();
    s.with_conn(|c| {
        c.execute(
            "UPDATE memory_item SET source_exchange_id = 'ex-old', last_seen_at = 9999999999999
             WHERE mog_id = ?1 AND rid = ?2",
            rusqlite::params![MOG_A, rid],
        )?;
        c.execute(
            "INSERT INTO memory_item_vec_256 (rowid, mog_id, embedding_model, embedding)
             VALUES (?1, ?2, 'qwen3@256', vec_int8(?3))",
            rusqlite::params![rid, MOG_A, quantize_int8(&v)],
        )
    })
    .unwrap();
    // A "Wipe older than 30 d" deletes the old exchange by its own age column only.
    s.with_conn(|c| {
        c.execute(
            "DELETE FROM genie_exchange WHERE mog_id = ?1 AND asked_at < 1000",
            [MOG_A],
        )
    })
    .unwrap();
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_item WHERE item_id = 'item-new'"
        ),
        1
    );
    assert_eq!(
        count(&s, "SELECT count(*) FROM memory_item WHERE item_id = 'item-new' AND source_exchange_id IS NULL"),
        1,
        "the pointer is cleared, the item stays"
    );
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_item_fts WHERE memory_item_fts MATCH 'margin'"
        ),
        1
    );
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item_vec_256"), 1);
}

#[test]
fn mem_33a_wiping_an_old_session_keeps_a_recent_exchange() {
    let t = TempDir::new("33a-session");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_session(&s, MOG_A, "sess-old", 1);
    seed_exchange(&s, MOG_A, "ex-new", Some("sess-old"), 9_999_999_999_999);
    s.with_conn(|c| {
        c.execute(
            "DELETE FROM session WHERE mog_id = ?1 AND started_at < 1000",
            [MOG_A],
        )
    })
    .unwrap();
    assert_eq!(
        count(&s, "SELECT count(*) FROM genie_exchange WHERE exchange_id = 'ex-new' AND session_id IS NULL"),
        1
    );
}

#[test]
fn mem_33a_no_delete_cascades_from_a_mog_or_a_session() {
    let t = TempDir::new("33a-restrict");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_item(&s, MOG_A, "item-1", "churn by segment");
    seed_session(&s, MOG_A, "sess-1", 1);
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO utterance (utterance_id, session_id, mog_id, seq, speaker_track, started_at,
               duration_ms, text_clean, route, stt_provider, row_hlc, origin_device_id)
             VALUES ('u-1', 'sess-1', ?1, 1, 'you', 9999999999999, 10, 'hello', 'noise', 'local_parakeet', 1, ?1)",
            [MOG_A],
        )
    })
    .unwrap();
    // The WipeService deletes each table by its own rule; the schema never deletes a child for it.
    let mog = s.with_conn(|c| c.execute("DELETE FROM mog WHERE mog_id = ?1", [MOG_A]));
    assert!(mog.is_err(), "deleting a MOG with content is refused");
    let sess = s.with_conn(|c| {
        c.execute(
            "DELETE FROM session WHERE mog_id = ?1 AND session_id = 'sess-1'",
            [MOG_A],
        )
    });
    assert!(
        sess.is_err(),
        "deleting a session with a remaining utterance is refused"
    );
    assert_eq!(count(&s, "SELECT count(*) FROM memory_item"), 1);
    assert_eq!(count(&s, "SELECT count(*) FROM utterance"), 1);
}

// ---- MEM-08e: the NEON int8 L2 boundary in sqlite-vec 0.1.9 ------------------------------

fn l2_int8(s: &SqlCipherLocalStore, a: i8, b: i8) -> Option<f64> {
    let va: Vec<u8> = vec![a as u8; 256];
    let vb: Vec<u8> = vec![b as u8; 256];
    s.with_conn(|c| {
        c.query_row(
            "SELECT vec_distance_l2(vec_int8(?1), vec_int8(?2))",
            rusqlite::params![va, vb],
            |r| r.get(0),
        )
    })
    .unwrap()
}

#[test]
fn mem_08e_neon_l2_is_valid_up_to_a_difference_of_180() {
    // l2_sqr_int8_neon widens to i16, then vmulq_s16(diff, diff) wraps at |diff| >= 182.
    let t = TempDir::new("08e-neon");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let d = l2_int8(&s, 90, -90).expect("a difference of 180 has a distance");
    assert!((d - (256.0f64 * 180.0 * 180.0).sqrt()).abs() < 1e-6, "{d}");
    // If an upgrade fixes the overflow, this flips. Then revisit INT8_MAX (vec_codec).
    if cfg!(target_arch = "aarch64") {
        assert_eq!(
            l2_int8(&s, 91, -91),
            None,
            "NEON overflow at a difference of 182"
        );
    }
    assert!(
        2 * i16::from(INT8_MAX) <= 180,
        "the quantizer never makes a difference above 180"
    );
}

// ---- MEM-08d: English stemming, as in Appendix A to_tsvector('english', ...) -------------

#[test]
fn mem_08d_fts5_stems_english_words() {
    let t = TempDir::new("08d-stem");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_item(&s, MOG_A, "item-s", "revenues grew in EMEA");
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM memory_item_fts WHERE memory_item_fts MATCH 'revenue'"
        ),
        1
    );
}

// ---- MEM-08c: the secure_delete line itself (SQLCipher already defaults it to 1) ---------

#[test]
fn mem_08c_secure_delete_is_set_explicitly_on_every_connection() {
    // SQLCipher reports secure_delete = 1 by default, so a PRAGMA read cannot detect a removed
    // line. This test guards the line in the connection pragmas.
    let p = super::store::CONNECTION_PRAGMAS.to_ascii_lowercase();
    assert!(p.contains("pragma secure_delete = on"), "{p}");
}

// ---- Appendix A: the local mirror keeps every column (B2), less the U9 removals -----------

#[test]
fn mem_08a_local_mirror_has_every_appendix_a_column() {
    let t = TempDir::new("appx-a");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    let want: [(&str, &[&str]); 8] = [
        (
            "mog",
            &[
                "mog_id",
                "owner_principal",
                "display_name",
                "backend_kind",
                "default_profile",
                "default_host",
                "genie_target",
                "memory_enabled",
                "transcript_sync_optin",
                "answer_freshness_hours",
                "created_at",
                "updated_at",
                "deleted_at",
                "row_hlc",
                "origin_device_id",
                "server_seq",
            ],
        ),
        (
            "session",
            &[
                "session_id",
                "mog_id",
                "owner_principal",
                "kind",
                "title",
                "started_at",
                "ended_at",
                "end_reason",
                "attestation_kind",
                "others_track",
                "workspace_host",
                "workspace_id",
                "profile_name",
                "utterance_count",
                "question_count",
                "summary",
                "transcript_synced",
                "writer_watermark_seq",
                "expires_at",
                "created_at",
                "updated_at",
                "deleted_at",
                "row_hlc",
                "origin_device_id",
                "server_seq",
            ],
        ),
        (
            "utterance",
            &[
                "rid",
                "utterance_id",
                "session_id",
                "mog_id",
                "owner_principal",
                "seq",
                "speaker_track",
                "started_at",
                "duration_ms",
                "text_clean",
                "text_raw",
                "lang",
                "route",
                "stt_provider",
                "stt_model",
                "stt_runtime",
                "expires_at",
                "created_at",
                "updated_at",
                "deleted_at",
                "row_hlc",
                "origin_device_id",
                "server_seq",
            ],
        ),
        (
            "genie_exchange",
            &[
                "exchange_id",
                "request_id",
                "mog_id",
                "session_id",
                "owner_principal",
                "backend_kind",
                "profile_name",
                "workspace_host",
                "workspace_id",
                "is_home_workspace",
                "space_id",
                "conversation_id",
                "response_id",
                "question_user",
                "question_sent_sha256",
                "preamble_item_ids",
                "status",
                "answer_summary",
                "key_numbers",
                "final_answer_md",
                "topic_label",
                "deep_link",
                "query_item_ids",
                "split_group_id",
                "feedback",
                "feedback_at",
                "error_code",
                "latency_ms",
                "asked_at",
                "completed_at",
                "expires_at",
                "created_at",
                "updated_at",
                "deleted_at",
                "row_hlc",
                "origin_device_id",
                "server_seq",
            ],
        ),
        (
            "memory_item",
            &[
                "rid",
                "item_id",
                "mog_id",
                "owner_principal",
                "item_type",
                "text",
                "source",
                "entities",
                "entity_keys",
                "attributes",
                "confidence",
                "status",
                "superseded_by",
                "resolved_by",
                "pinned",
                "sensitivity",
                "source_session_ids",
                "source_utterance_ids",
                "source_exchange_id",
                "workspace_host",
                "workspace_id",
                "space_id",
                "mention_count",
                "first_seen_at",
                "last_seen_at",
                "valid_until",
                "embedding",
                "embedding_model",
                "embedding_dims",
                "embedded_at",
                "expires_at",
                "created_at",
                "updated_at",
                "deleted_at",
                "row_hlc",
                "origin_device_id",
                "server_seq",
            ],
        ),
        (
            "consent_record",
            &[
                "consent_id",
                "mog_id",
                "owner_principal",
                "device_id",
                "notice_version",
                "kind",
                "session_id",
                "attestation_kind",
                "disclosure_copied",
                "recorded_at",
                "expires_at",
                "server_seq",
                "updated_at",
            ],
        ),
        (
            "sync_state",
            &[
                "device_id",
                "mog_id",
                "owner_principal",
                "device_name",
                "app_version",
                "last_pull_seq",
                "last_push_at",
                "local_schema_version",
                "updated_at",
            ],
        ),
        (
            "forget_log",
            &[
                "forget_id",
                "mog_id",
                "owner_principal",
                "scope",
                "target_ids",
                "requested_at",
                "device_id",
                "server_seq",
                "updated_at",
            ],
        ),
    ];
    for (table, cols) in want {
        let got: Vec<String> = columns(&s, table).into_iter().map(|(n, _)| n).collect();
        assert_eq!(got, cols, "columns of {table}");
    }
}

// ---- MEM-01: an utterance belongs to the MOG of its session (#410, migration 0002) --------

fn insert_utterance(
    s: &SqlCipherLocalStore,
    id: &str,
    session: &str,
    mog: &str,
    seq: i64,
) -> Result<usize, CoreError> {
    s.with_conn(|c| {
        c.execute(
            "INSERT INTO utterance (utterance_id, session_id, mog_id, seq, speaker_track, started_at,
               duration_ms, text_clean, route, stt_provider, row_hlc, origin_device_id)
             VALUES (?1, ?2, ?3, ?4, 'you', 1, 10, 'hello', 'noise', 'local_parakeet', 1, ?3)",
            rusqlite::params![id, session, mog, seq],
        )
    })
}

#[test]
fn mem_01_an_utterance_cannot_point_at_a_session_of_another_mog() {
    let t = TempDir::new("01-utt-tie");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    seed_session(&s, MOG_A, "sess-a", 1);
    assert!(insert_utterance(&s, "u-ok", "sess-a", MOG_A, 1).is_ok());
    assert!(
        insert_utterance(&s, "u-bad", "sess-a", MOG_B, 2).is_err(),
        "MOG B may not write into a session of MOG A"
    );
    // An update cannot move an utterance to another MOG, or to a session of another MOG.
    seed_session(&s, MOG_B, "sess-b", 1);
    let moved = s.with_conn(|c| {
        c.execute(
            "UPDATE utterance SET mog_id = ?1 WHERE utterance_id = 'u-ok'",
            [MOG_B],
        )
    });
    assert!(moved.is_err());
    let moved = s.with_conn(|c| {
        c.execute(
            "UPDATE utterance SET session_id = 'sess-b' WHERE utterance_id = 'u-ok'",
            [],
        )
    });
    assert!(moved.is_err());
    // A session with utterances cannot change its MOG.
    let moved = s.with_conn(|c| {
        c.execute(
            "UPDATE session SET mog_id = ?1 WHERE session_id = 'sess-a'",
            [MOG_B],
        )
    });
    assert!(moved.is_err());
    assert_eq!(
        count(&s, "SELECT count(*) FROM utterance WHERE mog_id = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01' AND session_id = 'sess-a'"),
        1,
        "nothing was deleted or moved (U9)"
    );
}

#[test]
fn mem_01_session_id_and_seq_are_unique_across_mogs() {
    let t = TempDir::new("01-utt-seq");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    seed_session(&s, MOG_A, "sess-a", 1);
    assert!(insert_utterance(&s, "u-1", "sess-a", MOG_A, 1).is_ok());
    // Appendix A: UNIQUE (session_id, seq). The same MOG hits the unique index; another MOG
    // hits the session tie, so no (session_id, seq) pair can repeat.
    assert!(insert_utterance(&s, "u-2", "sess-a", MOG_A, 1).is_err());
    assert!(insert_utterance(&s, "u-3", "sess-a", MOG_B, 1).is_err());
    assert!(insert_utterance(&s, "u-4", "sess-a", MOG_A, 2).is_ok());
}

#[test]
fn mem_35a_migration_0002_is_forward_only_and_deletes_nothing() {
    let sql = super::migrate::MIGRATIONS
        .iter()
        .find(|m| m.version == 2)
        .expect("migration 0002 exists")
        .sql
        .to_ascii_lowercase();
    for bad in ["drop table", "delete from", "drop index", "alter table"] {
        assert!(!sql.contains(bad), "0002 has `{bad}`");
    }
    assert!(sql.contains("raise(abort"));
}

#[test]
fn mem_01_a_session_replace_cannot_move_its_utterances_to_another_mog() {
    let t = TempDir::new("01-sess-replace");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    seed_session(&s, MOG_A, "sess-a", 1);
    assert!(insert_utterance(&s, "u-1", "sess-a", MOG_A, 1).is_ok());
    // A REPLACE deletes and inserts the session row, so no UPDATE trigger runs. Deferred
    // foreign keys would let the delete through; the INSERT trigger still refuses.
    let moved = s.with_conn(|c| {
        c.execute_batch("PRAGMA defer_foreign_keys = ON")?;
        c.execute(
            "INSERT OR REPLACE INTO session (session_id, mog_id, kind, started_at, row_hlc, origin_device_id)
             VALUES ('sess-a', ?1, 'meeting', 1, 1, ?1)",
            [MOG_B],
        )
    });
    assert!(moved.is_err());
    assert_eq!(
        count(&s, "SELECT count(*) FROM session WHERE session_id = 'sess-a' AND mog_id = '0f8b1a52-6c1e-4d0a-9a51-3a3d2f6f0a01'"),
        1
    );
    assert!(insert_utterance(&s, "u-2", "sess-a", MOG_B, 1).is_err());
}

#[test]
fn mem_35a_migration_0002_applies_to_a_store_that_has_0001_and_data() {
    let t = TempDir::new("35a-upgrade");
    let vault = Arc::new(MemoryKeyVault::new());
    let s = open_store(t.path(), vault.clone());
    seed_mog(&s, MOG_A);
    seed_session(&s, MOG_A, "sess-a", 1);
    assert!(insert_utterance(&s, "u-1", "sess-a", MOG_A, 1).is_ok());
    // Make the file look like a 0001-only store with data (test setup only).
    s.with_conn(|c| {
        c.execute_batch(
            "DROP TRIGGER utterance_session_mog_bi;
             DROP TRIGGER utterance_session_mog_bu;
             DROP TRIGGER session_mog_with_utterances_bu;
             DROP TRIGGER session_mog_with_utterances_bi;
             DELETE FROM schema_migrations WHERE version = 2;",
        )
    })
    .unwrap();
    s.close();
    let s = open_store(t.path(), vault.clone());
    assert_eq!(
        count(&s, "SELECT max(version) FROM schema_migrations"),
        super::migrate::latest_version()
    );
    assert_eq!(
        count(&s, "SELECT count(*) FROM utterance"),
        1,
        "the data stays"
    );
    let triggers = "SELECT count(*) FROM sqlite_master WHERE type = 'trigger' AND name IN \
         ('utterance_session_mog_bi', 'utterance_session_mog_bu', \
          'session_mog_with_utterances_bu', 'session_mog_with_utterances_bi')";
    assert_eq!(count(&s, triggers), 4, "all four 0002 triggers exist");
    seed_mog(&s, MOG_B);
    assert!(insert_utterance(&s, "u-2", "sess-a", MOG_B, 2).is_err());
    // A second open changes nothing: 0002 runs once.
    s.close();
    let s = open_store(t.path(), vault);
    assert_eq!(count(&s, triggers), 4);
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM schema_migrations WHERE version = 2"
        ),
        1
    );
    assert_eq!(count(&s, "SELECT count(*) FROM utterance"), 1);
}

#[test]
fn mem_01_a_session_with_utterances_cannot_be_renamed() {
    let t = TempDir::new("01-sess-rename");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_session(&s, MOG_A, "sess-a", 1);
    assert!(insert_utterance(&s, "u-1", "sess-a", MOG_A, 1).is_ok());
    let renamed = s.with_conn(|c| {
        c.execute_batch("PRAGMA foreign_keys = OFF")?;
        c.execute(
            "UPDATE session SET session_id = 'sess-z' WHERE session_id = 'sess-a'",
            [],
        )
    });
    assert!(renamed.is_err());
    assert_eq!(
        count(
            &s,
            "SELECT count(*) FROM session WHERE session_id = 'sess-a'"
        ),
        1
    );
}

#[test]
fn mem_01_an_update_or_replace_cannot_take_the_id_of_another_mogs_session() {
    let t = TempDir::new("01-sess-update-replace");
    let s = open_store(t.path(), Arc::new(MemoryKeyVault::new()));
    seed_mog(&s, MOG_A);
    seed_mog(&s, MOG_B);
    seed_session(&s, MOG_A, "sess-a", 1);
    seed_session(&s, MOG_B, "sess-b", 2);
    assert!(insert_utterance(&s, "u-1", "sess-a", MOG_A, 1).is_ok());
    // sess-b (MOG B, no utterances) takes the ID sess-a. REPLACE would delete the MOG A row,
    // and deferred foreign keys would let it through; the UPDATE trigger refuses.
    let taken = s.with_conn(|c| {
        c.execute_batch("PRAGMA defer_foreign_keys = ON")?;
        c.execute(
            "UPDATE OR REPLACE session SET session_id = 'sess-a' WHERE session_id = 'sess-b'",
            [],
        )
    });
    assert!(taken.is_err());
    assert_eq!(count(&s, "SELECT count(*) FROM session"), 2);
    assert!(insert_utterance(&s, "u-2", "sess-a", MOG_B, 1).is_err());
}

// #541: an unsigned or ad-hoc macOS build has no keychain access group entitlement. Then the data
// protection keychain refuses a store with errSecMissingEntitlement (-34018), and it answers a get
// with errSecItemNotFound (-25300), as seen on a live `pnpm tauri dev` run. A store falls back to
// the login keychain only on -34018. A load falls back only on -34018 or -25300, so a key that an
// earlier run stored in the login keychain is found again. These tests drive the pure fallback
// helper with a recording double; they never call the real Keychain.
mod keychain_fallback {
    use super::super::key_vault::{
        with_login_fallback, KeychainTarget, ERR_SEC_ITEM_NOT_FOUND, ERR_SEC_MISSING_ENTITLEMENT,
        LOAD_FALLBACK, STORE_FALLBACK,
    };

    /// Runs `with_login_fallback` with a double that answers `answers` in order and records
    /// each target it is called with.
    fn run(on: &[i32], answers: Vec<Result<u8, i32>>) -> (Result<u8, i32>, Vec<KeychainTarget>) {
        let mut calls = Vec::new();
        let mut it = answers.into_iter();
        let out = with_login_fallback(on, |t| {
            calls.push(t);
            it.next().expect("the double got an extra call")
        });
        (out, calls)
    }

    #[test]
    fn mem_08b_fallback_codes_are_exact() {
        assert_eq!(ERR_SEC_MISSING_ENTITLEMENT, -34018);
        assert_eq!(ERR_SEC_ITEM_NOT_FOUND, -25300);
        assert_eq!(STORE_FALLBACK, &[-34018]);
        assert_eq!(LOAD_FALLBACK, &[-34018, -25300]);
    }

    #[test]
    fn mem_08b_store_missing_entitlement_falls_back_to_login_keychain() {
        let (out, calls) = run(
            STORE_FALLBACK,
            vec![Err(ERR_SEC_MISSING_ENTITLEMENT), Ok(7)],
        );
        assert_eq!(out, Ok(7));
        assert_eq!(
            calls,
            vec![KeychainTarget::DataProtection, KeychainTarget::Login]
        );
    }

    #[test]
    fn mem_08b_load_item_not_found_reads_login_keychain() {
        // The live failure of #541: the second start found no key in the data protection
        // keychain and reported store_key_lost, though the key was in the login keychain.
        let (out, calls) = run(LOAD_FALLBACK, vec![Err(ERR_SEC_ITEM_NOT_FOUND), Ok(9)]);
        assert_eq!(out, Ok(9));
        assert_eq!(
            calls,
            vec![KeychainTarget::DataProtection, KeychainTarget::Login]
        );
        let (out, _) = run(LOAD_FALLBACK, vec![Err(ERR_SEC_MISSING_ENTITLEMENT), Ok(3)]);
        assert_eq!(out, Ok(3));
    }

    #[test]
    fn mem_08b_store_item_not_found_does_not_fall_back() {
        let (out, calls) = run(STORE_FALLBACK, vec![Err(ERR_SEC_ITEM_NOT_FOUND)]);
        assert_eq!(out, Err(ERR_SEC_ITEM_NOT_FOUND));
        assert_eq!(calls, vec![KeychainTarget::DataProtection]);
    }

    #[test]
    fn mem_08b_data_protection_success_does_not_touch_login_keychain() {
        for on in [LOAD_FALLBACK, STORE_FALLBACK] {
            let (out, calls) = run(on, vec![Ok(1)]);
            assert_eq!(out, Ok(1));
            assert_eq!(calls, vec![KeychainTarget::DataProtection]);
        }
    }

    #[test]
    fn mem_08b_other_errors_do_not_fall_back() {
        // errSecAuthFailed, errSecUserCanceled, errSecInteractionNotAllowed, errSecParam,
        // errSecDuplicateItem, errSecDecode.
        for on in [LOAD_FALLBACK, STORE_FALLBACK] {
            for code in [-25293, -128, -25308, -50, -25299, -26275] {
                let (out, calls) = run(on, vec![Err(code)]);
                assert_eq!(out, Err(code), "code {code}");
                assert_eq!(calls, vec![KeychainTarget::DataProtection], "code {code}");
            }
        }
    }

    #[test]
    fn mem_08b_login_keychain_error_is_returned_as_is() {
        let (out, calls) = run(
            LOAD_FALLBACK,
            vec![
                Err(ERR_SEC_MISSING_ENTITLEMENT),
                Err(ERR_SEC_ITEM_NOT_FOUND),
            ],
        );
        assert_eq!(out, Err(ERR_SEC_ITEM_NOT_FOUND));
        assert_eq!(calls.len(), 2, "no third try");
    }
}
