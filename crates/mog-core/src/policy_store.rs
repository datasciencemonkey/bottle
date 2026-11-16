//! R13 PolicyStore (section 4.3).
//!
//! Responsibilities: MDM and admin keys (Appendix C) and the telemetry gate (D42).

use crate::error::{CoreError, CoreResult};

const ID: &str = "R13";

/// The effective MDM and admin policy values.
pub trait PolicyStore: Send + Sync {
    /// The effective value of one MDM or admin key, or `None` if it is not set.
    fn get(&self, key: &str) -> CoreResult<Option<serde_json::Value>>;
    /// The telemetry gate: true only if policy allows content-free telemetry.
    fn telemetry_allowed(&self) -> CoreResult<bool>;
}

/// The R13 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubPolicyStore;

impl PolicyStore for StubPolicyStore {
    fn get(&self, _key: &str) -> CoreResult<Option<serde_json::Value>> {
        // TODO(#186): MDM keys of Appendix C.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn telemetry_allowed(&self) -> CoreResult<bool> {
        // TODO(#184): telemetry gate (D42).
        Err(CoreError::NotImplemented { component: ID })
    }
}
