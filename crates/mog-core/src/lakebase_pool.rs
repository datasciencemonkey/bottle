//! R9 LakebasePool, v1 and optional (section 4.3). Built only with the cargo feature `lakebase`,
//! which is off by default.
//!
//! Responsibilities: Direct TLS to the home Lakebase endpoint, only when Lakebase sync is on. Max 3 connections, idle 300 s, lifetime 50 min, a new credential per connection, prewarm `SELECT 1`. Fixed statement catalog.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R9";

/// A small pool of TLS connections to the home Lakebase endpoint (v1).
pub trait LakebasePool: Send + Sync {
    /// Open and prewarm a connection with a new credential.
    fn connect(&self) -> CoreResult<()>;
    /// Run one statement of the fixed remote catalog by its ID.
    fn exec(&self, stmt_id: &str, params: serde_json::Value) -> CoreResult<serde_json::Value>;
}

/// The R9 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubLakebasePool;

impl LakebasePool for StubLakebasePool {
    fn connect(&self) -> CoreResult<()> {
        // TODO(#193): short-lived credentials and cold-wake handling.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn exec(&self, _stmt_id: &str, _params: serde_json::Value) -> CoreResult<serde_json::Value> {
        // TODO(#193): fixed remote statement catalog.
        Err(CoreError::NotImplemented { component: ID })
    }
}
