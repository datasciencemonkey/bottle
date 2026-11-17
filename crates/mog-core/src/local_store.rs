//! R10 LocalStore (section 4.3).
//!
//! Responsibilities: **The MVP memory store.** One SQLCipher 4 file with the key in the OS keychain, WAL, FTS5, `sqlite-vec`, outbox (embedding queue, v1 sync), audit log. Fixed statement catalog.

use crate::error::{CoreError, CoreResult};
use crate::ipc::MemExecArgs;

pub mod catalog;
pub mod key_vault;
mod migrate;
pub mod mog_guard;
mod store;
#[cfg(test)]
mod tests;
pub mod vec_codec;

pub use store::{SqlCipherLocalStore, DB_FILE_NAME};

const ID: &str = "R10";

/// The encrypted local memory store. Callers run fixed statements by ID, never raw SQL.
pub trait LocalStore: Send + Sync {
    /// Open the store with the key from the OS keychain.
    fn open(&self) -> CoreResult<()>;
    /// Run one statement of the fixed local catalog by its ID.
    fn exec(&self, stmt_id: &str, params: serde_json::Value) -> CoreResult<serde_json::Value>;
}

/// A store that is not there: every method returns [`CoreError::NotImplemented`]. Hosts and tests
/// use it where no memory is wired. The real store is [`SqlCipherLocalStore`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubLocalStore;

impl LocalStore for StubLocalStore {
    fn open(&self) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn exec(&self, _stmt_id: &str, _params: serde_json::Value) -> CoreResult<serde_json::Value> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The R10 side of the `mem_local_exec` IPC command (#37, MEM-34a, MEM-34b).
///
/// It parses `MemExecArgs`, refuses a `stmtId` that is not a [`catalog::Access::Brain`] entry of
/// the fixed catalog, runs the statement on `store` and answers `{ "rows": [...] }`. Every
/// refusal is a fixed IPC code with no value in it: `invalid_args` for an unknown or Rust-only
/// id and for bad params, `not_implemented` for a store that is not there, and `host_failed`
/// for a store that cannot run the statement.
/// TODO(#426): the app host routes `mem_local_exec` here (contract file `src-tauri/src/ipc.rs`).
pub fn mem_local_exec(
    store: &dyn LocalStore,
    args: serde_json::Value,
) -> CoreResult<serde_json::Value> {
    let refused = |code: &'static str| CoreError::IpcRefused { code };
    let a: MemExecArgs = serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
    match catalog::find(&a.stmt_id) {
        Some(s) if s.access == catalog::Access::Brain => {}
        _ => return Err(refused("invalid_args")),
    }
    store
        .exec(
            &a.stmt_id,
            serde_json::Value::Object(a.params.into_iter().collect()),
        )
        .map_err(|e| match e {
            CoreError::UnknownStatement { .. }
            | CoreError::InvalidStatementParams { .. }
            | CoreError::MogIdRequired { .. } => refused("invalid_args"),
            CoreError::NotImplemented { .. } => refused("not_implemented"),
            _ => refused("host_failed"),
        })
}
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod exchange_tests;
#[cfg(test)]
mod outbox_tests;
