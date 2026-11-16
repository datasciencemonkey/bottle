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

use super::key_vault::{KeyVault, LocalKey};
use super::mog_guard::require_mog_id_param;
use super::LocalStore;
use crate::error::{CoreError, CoreResult};

const ID: &str = "R10";

/// The one database file in the memory folder (MEM-08a).
pub const DB_FILE_NAME: &str = "memory.db";

/// The `sqlite-vec` version this build links (MEM-08e).
pub const SQLITE_VEC_VERSION: &str = "v0.1.9";

/// Page cache in KiB (SP-21: it holds the 50k-item index, so a warm query decrypts nothing).
const CACHE_KIB: i64 = 65_536;

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
            // MEM-09a: the key is gone but the file is not. Do not delete the file (U9) and do
            // not make a new key; MemoryService decides.
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
    conn.execute_batch(&format!(
        "PRAGMA secure_delete = ON; PRAGMA synchronous = NORMAL; \
         PRAGMA cache_size = -{CACHE_KIB}; PRAGMA foreign_keys = ON;"
    ))
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

    fn exec(&self, _stmt_id: &str, params: serde_json::Value) -> CoreResult<serde_json::Value> {
        require_mog_id_param(&params)?;
        if self
            .conn
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_none()
        {
            return Err(CoreError::StoreUnavailable { component: ID });
        }
        // TODO(#37): fixed local statement catalog. Each catalog statement passes
        // `mog_guard::require_mog_id_filter` and binds this `mogId`.
        Err(CoreError::NotImplemented { component: ID })
    }
}
