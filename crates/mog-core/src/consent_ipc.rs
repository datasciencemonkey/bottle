//! The IPC side of the T12 consent records (#430, ADR 0060).
//!
//! [`ConsentComponent`] serves `consent_notice_record` and `consent_records_list` on the R12
//! gateway seam ([`GatewayComponents`]). The brain names only the notice version. The Rust
//! side fills the user ([`SignedInUser`]), the device and the time ([`ConsentRecords`],
//! SEC-32a). Replies are content-free: no user, no device and no MOG ID crosses IPC.

use std::sync::Arc;

use serde_json::Value;

use crate::consent::ConsentRecords;
use crate::error::{CoreError, CoreResult};
use crate::ipc_gateway::GatewayComponents;

/// Where the Rust side gets the signed-in user for a notice record (SEC-32a `user`).
pub trait SignedInUser: Send + Sync {
    /// The signed-in principal, or `None` before the first sign-in.
    fn principal(&self) -> Option<String>;
}

/// No user is signed in.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoSignedInUser;

impl SignedInUser for NoSignedInUser {
    fn principal(&self) -> Option<String> {
        None
    }
}

/// The `consent_*` component over the device's consent records.
pub struct ConsentComponent {
    records: Arc<ConsentRecords>,
    user: Arc<dyn SignedInUser>,
}

impl ConsentComponent {
    pub fn new(records: Arc<ConsentRecords>, user: Arc<dyn SignedInUser>) -> Self {
        Self { records, user }
    }
}

impl GatewayComponents for ConsentComponent {
    fn call(&self, _command: &'static str, _args: Value) -> CoreResult<Value> {
        let _ = (&self.records, &self.user);
        Err(CoreError::IpcRefused {
            code: "not_implemented",
        })
    }
}

#[cfg(test)]
mod tests;
