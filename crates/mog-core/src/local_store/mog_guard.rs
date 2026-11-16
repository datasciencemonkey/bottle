//! MEM-01 guard. Red skeleton for #35.

use crate::error::{CoreError, CoreResult};

/// Reject a statement without a `mog_id` filter.
pub fn require_mog_id_filter(_sql: &str) -> CoreResult<()> {
    Err(CoreError::NotImplemented { component: "R10" })
}

/// Reject params without a `mogId`.
pub fn require_mog_id_param(_params: &serde_json::Value) -> CoreResult<&str> {
    Err(CoreError::NotImplemented { component: "R10" })
}
