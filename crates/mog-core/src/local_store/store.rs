//! R10 `SqlCipherLocalStore`: the one encrypted SQLite file for all MOGs (B2).
//!
//! Configuration from SP-21 (`docs/spikes/SP-21.md`, "Recommended configuration"):
//! SQLCipher 4 with a static OpenSSL, a raw 256-bit key (`PRAGMA key = "x'<hex>'"`, no PBKDF2),
//! WAL, `secure_delete = ON`, `synchronous = NORMAL`, a 64 MiB page cache, and `sqlite-vec`
//! linked statically and registered with `sqlite3_auto_extension`.
//! TODO(#34): SP-21 passed on an M3 Max only; the M1 and Developer ID signed reruns are open.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use rusqlite::{Connection, OpenFlags};

use super::catalog;
use super::key_vault::{KeyVault, LocalKey};
use super::mog_guard::require_mog_id_param;
use super::LocalStore;
use crate::error::{CoreError, CoreResult};

const ID: &str = "R10";

/// The one database file in the memory folder (MEM-08a).
pub const DB_FILE_NAME: &str = "memory.db";

/// The `sqlite-vec` version this build links (MEM-08e).
pub const SQLITE_VEC_VERSION: &str = "v0.1.9";

/// The pragmas every connection sets after the key (MEM-08c, SP-21). `secure_delete` is set
/// explicitly even though SQLCipher defaults it on, so a build change cannot drop it silently.
/// The page cache is 64 MiB (-65536 KiB): it holds the 50k-item index, so a warm query decrypts
/// nothing. `foreign_keys` makes the MEM-33a RESTRICT and SET NULL rules of migration 0001 hold.
pub(crate) const CONNECTION_PRAGMAS: &str =
    "PRAGMA secure_delete = ON; PRAGMA synchronous = NORMAL; \
     PRAGMA cache_size = -65536; PRAGMA foreign_keys = ON;";

fn unavailable<E>(_: E) -> CoreError {
    CoreError::StoreUnavailable { component: ID }
}

static REGISTER_VEC: Once = Once::new();

/// Register the statically linked `sqlite-vec` for every new connection of this process.
fn register_vec() {
    REGISTER_VEC.call_once(|| {
        // SAFETY: `sqlite3_vec_init` has the `sqlite3_auto_extension` entry point signature
        // (db, error message, API routines); the crate exports it as an untyped symbol.
        unsafe {
            #[allow(clippy::missing_transmute_annotations)]
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

/// The real R10 store. It holds one connection; `open` is idempotent.
pub struct SqlCipherLocalStore {
    dir: PathBuf,
    vault: Arc<dyn KeyVault>,
    conn: Mutex<Option<Connection>>,
}

impl SqlCipherLocalStore {
    /// A closed store whose database lives in the memory folder `dir`.
    pub fn new(dir: impl Into<PathBuf>, vault: Arc<dyn KeyVault>) -> Self {
        SqlCipherLocalStore {
            dir: dir.into(),
            vault,
            conn: Mutex::new(None),
        }
    }

    /// The path of the database file.
    pub fn db_path(&self) -> PathBuf {
        self.dir.join(DB_FILE_NAME)
    }

    /// Close the connection. SQLite checkpoints the WAL and removes the `-wal` and `-shm` files.
    pub fn close(&self) {
        let c = self.conn.lock().unwrap_or_else(|e| e.into_inner()).take();
        drop(c);
    }

    /// Run `f` on the open connection. Crate-private: callers outside R10 use `exec` and the
    /// fixed catalog, never raw SQL (MEM-34a).
    #[allow(dead_code)]
    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> CoreResult<T> {
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let c = g
            .as_ref()
            .ok_or(CoreError::StoreUnavailable { component: ID })?;
        f(c).map_err(unavailable)
    }

    fn create_dir(&self) -> CoreResult<()> {
        std::fs::create_dir_all(&self.dir).map_err(unavailable)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&self.dir, std::fs::Permissions::from_mode(0o700))
                .map_err(unavailable)?;
        }
        Ok(())
    }

    /// The key for the file: the stored one, or a new one for a new file.
    fn key(&self) -> CoreResult<LocalKey> {
        match self.vault.load()? {
            Some(k) => Ok(k),
            // The key is gone but the file is not. R10 deletes nothing (U9) and makes no new key.
            // TODO(#190): MemoryService handles the lost key (MEM-09a "start a new empty store"
            // vs U9 and MEM-08a's one file), and the MOG tells the user (MEM-09b).
            None if self.db_path().exists() => Err(CoreError::StoreKeyLost { component: ID }),
            None => {
                let k = LocalKey::generate()?;
                self.vault.store(&k)?;
                Ok(k)
            }
        }
    }
}

/// Open the file, apply the key and the pragmas, and check the build.
fn open_connection(path: &Path, key: &LocalKey) -> CoreResult<Connection> {
    register_vec();
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_CREATE
        | OpenFlags::SQLITE_OPEN_NO_MUTEX
        | OpenFlags::SQLITE_OPEN_URI;
    let conn = Connection::open_with_flags(path, flags).map_err(unavailable)?;
    {
        let hex = key.to_hex();
        // `Zeroed` overwrites both strings with zeros when they drop.
        let pragma = super::key_vault::Zeroed(format!("PRAGMA key = \"x'{}'\";", hex.0));
        conn.execute_batch(&pragma.0).map_err(unavailable)?;
    }
    // The first read fails with SQLITE_NOTADB if the key is wrong.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
        r.get::<_, i64>(0)
    })
    .map_err(unavailable)?;
    let cipher: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .map_err(unavailable)?;
    if !cipher.starts_with("4.") {
        return Err(unavailable(()));
    }
    let mode: String = conn
        .query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))
        .map_err(unavailable)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(unavailable(()));
    }
    conn.execute_batch(CONNECTION_PRAGMAS)
        .map_err(unavailable)?;
    let vec: String = conn
        .query_row("SELECT vec_version()", [], |r| r.get(0))
        .map_err(unavailable)?;
    if vec != SQLITE_VEC_VERSION {
        return Err(unavailable(()));
    }
    let fts5: i64 = conn
        .query_row("SELECT sqlite_compileoption_used('ENABLE_FTS5')", [], |r| {
            r.get(0)
        })
        .map_err(unavailable)?;
    if fts5 != 1 {
        return Err(unavailable(()));
    }
    let mut conn = conn;
    super::migrate::run(&mut conn)?;
    super::migrate::warm_up(&conn)?;
    Ok(conn)
}

impl LocalStore for SqlCipherLocalStore {
    fn open(&self) -> CoreResult<()> {
        let mut g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        if g.is_some() {
            return Ok(());
        }
        self.create_dir()?;
        let key = self.key()?;
        *g = Some(open_connection(&self.db_path(), &key)?);
        Ok(())
    }

    fn exec(&self, stmt_id: &str, params: serde_json::Value) -> CoreResult<serde_json::Value> {
        // MEM-34a: only fixed catalog statements and groups run.
        if let Some(group) = catalog::find_group(stmt_id) {
            return self.exec_group(group, &params);
        }
        let stmt = catalog::find(stmt_id).ok_or(CoreError::UnknownStatement { component: ID })?;
        require_mog_id_param(&params)?;
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let conn = g
            .as_ref()
            .ok_or(CoreError::StoreUnavailable { component: ID })?;
        let values = catalog::validate(stmt, &params)?;
        run_statement(conn, stmt, &values).map_err(unavailable)
    }
}

impl SqlCipherLocalStore {
    /// Run the steps of an atomic group in one transaction (MEM-10a, ADR 0068). The params are
    /// checked against the group before the transaction starts. A failed step rolls back every
    /// step; a process that dies before the commit leaves only an uncommitted WAL tail, which
    /// SQLite drops on the next open.
    fn exec_group(
        &self,
        group: &catalog::Group,
        params: &serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        require_mog_id_param(params)?;
        let values = catalog::validate_params(group.params, params)?;
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let conn = g
            .as_ref()
            .ok_or(CoreError::StoreUnavailable { component: ID })?;
        let run = || -> rusqlite::Result<serde_json::Value> {
            let tx = rusqlite::Transaction::new_unchecked(
                conn,
                rusqlite::TransactionBehavior::Immediate,
            )?;
            let mut rows = Vec::with_capacity(group.steps.len());
            for (i, step) in group.steps.iter().enumerate() {
                let out = run_statement(&tx, step, &values)?;
                rows.extend(out["rows"].as_array().cloned().unwrap_or_default());
                #[cfg(test)]
                test_fault::after_step(i + 1);
                #[cfg(not(test))]
                let _ = i;
            }
            tx.commit()?;
            Ok(serde_json::json!({ "rows": rows }))
        };
        run().map_err(unavailable)
    }
}

/// One open transaction for a Rust caller of R10 (R14 WipeService, #124). It runs only fixed
/// catalog entries: [`catalog::STATEMENTS`], [`catalog::GROUPS`] and the device-wide
/// [`catalog::DeviceStmt`] entries. Nothing commits until the closure returns `Ok`.
pub(crate) struct CoreTx<'a> {
    conn: &'a Connection,
}

impl CoreTx<'_> {
    /// Run a catalog query or write with a `mogId` (MEM-01) and answer its rows.
    pub(crate) fn run(
        &self,
        stmt_id: &str,
        params: &serde_json::Value,
    ) -> CoreResult<Vec<serde_json::Value>> {
        let stmt = catalog::find(stmt_id).ok_or(CoreError::UnknownStatement { component: ID })?;
        require_mog_id_param(params)?;
        let values = catalog::validate(stmt, params)?;
        let out = run_statement(self.conn, stmt, &values).map_err(unavailable)?;
        Ok(rows_of(out))
    }

    /// Run the steps of a catalog group with a `mogId` and answer one `{changes: n}` row each.
    pub(crate) fn group(
        &self,
        group_id: &str,
        params: &serde_json::Value,
    ) -> CoreResult<Vec<serde_json::Value>> {
        let group =
            catalog::find_group(group_id).ok_or(CoreError::UnknownStatement { component: ID })?;
        require_mog_id_param(params)?;
        let values = catalog::validate_params(group.params, params)?;
        let mut rows = Vec::with_capacity(group.steps.len());
        for step in group.steps {
            let out = run_statement(self.conn, step, &values).map_err(unavailable)?;
            rows.extend(rows_of(out));
        }
        Ok(rows)
    }

    /// Run a device-wide query with no params (it crosses MOGs on purpose, with its reason).
    pub(crate) fn device_query(
        &self,
        d: &catalog::DeviceStmt,
    ) -> CoreResult<Vec<serde_json::Value>> {
        if d.stmt.kind != catalog::Kind::Query || !d.stmt.params.is_empty() {
            return Err(CoreError::UnknownStatement { component: ID });
        }
        let out = run_statement(self.conn, &d.stmt, &[]).map_err(unavailable)?;
        Ok(rows_of(out))
    }
}

fn rows_of(out: serde_json::Value) -> Vec<serde_json::Value> {
    out["rows"].as_array().cloned().unwrap_or_default()
}

impl SqlCipherLocalStore {
    /// Run `f` in one IMMEDIATE transaction (R14, #124). It commits only if `f` returns `Ok`;
    /// any error rolls back every statement that `f` ran. Crate-private: the brain reaches the
    /// store only through `mem_local_exec` and the fixed catalog (MEM-34a).
    pub(crate) fn core_transaction<T>(
        &self,
        f: impl FnOnce(&CoreTx<'_>) -> CoreResult<T>,
    ) -> CoreResult<T> {
        let g = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let conn = g
            .as_ref()
            .ok_or(CoreError::StoreUnavailable { component: ID })?;
        let tx =
            rusqlite::Transaction::new_unchecked(conn, rusqlite::TransactionBehavior::Immediate)
                .map_err(unavailable)?;
        let out = f(&CoreTx { conn: &tx })?;
        tx.commit().map_err(unavailable)?;
        Ok(out)
    }
}

/// A SQLite value as JSON. The catalog selects no BLOB column; one would answer `null`.
fn json_of(v: rusqlite::types::ValueRef<'_>) -> serde_json::Value {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Null | ValueRef::Blob(_) => serde_json::Value::Null,
        ValueRef::Integer(n) => n.into(),
        ValueRef::Real(f) => {
            serde_json::Number::from_f64(f).map_or(serde_json::Value::Null, Into::into)
        }
        ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned().into(),
    }
}

/// Bind each `:name` of the statement and run it. A query answers its rows as objects; a write
/// answers `[{ "changes": n }]`.
fn run_statement(
    conn: &Connection,
    stmt: &catalog::Stmt,
    values: &[(&'static str, rusqlite::types::Value)],
) -> rusqlite::Result<serde_json::Value> {
    let mut st = conn.prepare_cached(stmt.sql)?;
    for (name, value) in values {
        if let Some(i) = st.parameter_index(&format!(":{name}"))? {
            st.raw_bind_parameter(i, value)?;
        }
    }
    let rows = match stmt.kind {
        catalog::Kind::Write => {
            let n = st.raw_execute()?;
            vec![serde_json::json!({ "changes": n })]
        }
        catalog::Kind::Query => {
            let names: Vec<String> = st.column_names().map(str::to_owned).collect();
            let mut out = Vec::new();
            let mut rows = st.raw_query();
            while let Some(r) = rows.next()? {
                let mut obj = serde_json::Map::new();
                for (i, n) in names.iter().enumerate() {
                    obj.insert(n.clone(), json_of(r.get_ref(i)?));
                }
                out.push(serde_json::Value::Object(obj));
            }
            out
        }
    };
    Ok(serde_json::json!({ "rows": rows }))
}

/// The fault hook of the AT-18 kill test (`outbox_tests.rs`). It exists only in test builds. A
/// test runs this binary again as a child with `BOTTLE_R10_EXIT_AFTER_STEP=<n>`; the child then
/// ends the process right after step `n` of a group, inside the open transaction, with no
/// rollback and no close. With the variable unset it does nothing.
#[cfg(test)]
mod test_fault {
    pub(super) fn after_step(step: usize) {
        if std::env::var("BOTTLE_R10_EXIT_AFTER_STEP").ok() == Some(step.to_string()) {
            std::process::exit(3);
        }
    }
}
