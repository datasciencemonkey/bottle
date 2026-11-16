//! R10 LocalStore (section 4.3).
//!
//! Responsibilities: **The MVP memory store.** One SQLCipher 4 file with the key in the OS keychain, WAL, FTS5, `sqlite-vec`, outbox (embedding queue, v1 sync), audit log. Fixed statement catalog.

use crate::error::{CoreError, CoreResult};

pub mod key_vault;
pub mod mog_guard;
mod store;
#[cfg(test)]
mod tests;

pub use store::{SqlCipherLocalStore, DB_FILE_NAME};

const ID: &str = "R10";

/// The encrypted local memory store. Callers run fixed statements by ID, never raw SQL.
pub trait LocalStore: Send + Sync {
    /// Open the store with the key from the OS keychain.
    fn open(&self) -> CoreResult<()>;
    /// Run one statement of the fixed local catalog by its ID.
    fn exec(&self, stmt_id: &str, params: serde_json::Value) -> CoreResult<serde_json::Value>;
}

/// The R10 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubLocalStore;

impl LocalStore for StubLocalStore {
    fn open(&self) -> CoreResult<()> {
        // TODO(#35): SQLCipher, WAL, FTS5 and sqlite-vec.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn exec(&self, _stmt_id: &str, _params: serde_json::Value) -> CoreResult<serde_json::Value> {
        // TODO(#37): fixed local statement catalog.
        Err(CoreError::NotImplemented { component: ID })
    }
}
