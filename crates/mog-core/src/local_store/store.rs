//! R10 SqlCipherLocalStore. Red skeleton for #35.

use std::path::PathBuf;
use std::sync::Arc;

use super::key_vault::KeyVault;
use super::LocalStore;
use crate::error::{CoreError, CoreResult};

/// The one database file name in the memory folder.
pub const DB_FILE_NAME: &str = "memory.db";

/// The real R10 store.
pub struct SqlCipherLocalStore {
    _dir: PathBuf,
    _vault: Arc<dyn KeyVault>,
}

impl SqlCipherLocalStore {
    /// A closed store in `dir`.
    pub fn new(dir: impl Into<PathBuf>, vault: Arc<dyn KeyVault>) -> Self {
        SqlCipherLocalStore { _dir: dir.into(), _vault: vault }
    }
    /// Close the connection.
    pub fn close(&self) {}
    pub(crate) fn with_conn<T>(
        &self,
        _f: impl FnOnce(&rusqlite::Connection) -> rusqlite::Result<T>,
    ) -> CoreResult<T> {
        Err(CoreError::NotImplemented { component: "R10" })
    }
}

impl LocalStore for SqlCipherLocalStore {
    fn open(&self) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: "R10" })
    }
    fn exec(&self, _stmt_id: &str, _params: serde_json::Value) -> CoreResult<serde_json::Value> {
        Err(CoreError::NotImplemented { component: "R10" })
    }
}
