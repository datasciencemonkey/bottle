//! The IPC side of the T12 consent records (#430, ADR 0060).
//!
//! [`ConsentComponent`] serves `consent_notice_record` and `consent_records_list` on the R12
//! gateway seam ([`GatewayComponents`]). The brain names only the notice version. The Rust
//! side fills the user ([`SignedInUser`]), the device and the time ([`ConsentRecords`],
//! SEC-32a). Replies are content-free: no user, no device and no MOG ID crosses IPC.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::consent::{ConsentKind, ConsentRecord, ConsentRecords};
use crate::error::{CoreError, CoreResult};
use crate::ipc::{AttestKind, ConsentNoticeRecordArgs};
use crate::ipc_gateway::GatewayComponents;
use crate::types::{ConsentRecordKind, ConsentRecordView};

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

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

/// A store read or write failed: one fixed code, never the store's reason (SEC-03).
fn storage_failed(_: CoreError) -> CoreError {
    refused("consent_storage_failed")
}

/// The content-free IPC view of one record: no user, no device and no MOG ID. `None` for a
/// session record with no attestation kind: the row is damaged, and no kind is guessed.
pub fn view_of(r: &ConsentRecord) -> Option<ConsentRecordView> {
    let kind = match (r.kind, &r.attestation_kind) {
        (ConsentKind::Notice, _) => ConsentRecordKind::Notice,
        (ConsentKind::Session, Some(AttestKind::OthersAgreed)) => ConsentRecordKind::OthersAgreed,
        (ConsentKind::Session, Some(AttestKind::OnlyMe)) => ConsentRecordKind::OnlyMe,
        (ConsentKind::Session, None) => return None,
    };
    Some(ConsentRecordView {
        record_id: r.consent_id.clone(),
        kind,
        notice_version: r.notice_version,
        session_id: r.session_id.clone(),
        accepted_at: r.accepted_at,
    })
}

impl GatewayComponents for ConsentComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "consent_notice_record" => {
                let a: ConsentNoticeRecordArgs =
                    serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
                if a.notice_version == 0 {
                    return Err(refused("invalid_args"));
                }
                let user = self.user.principal();
                let record = self
                    .records
                    .record_notice(user.as_deref(), a.notice_version)
                    .map_err(storage_failed)?;
                Ok(json!({ "recordId": record.consent_id }))
            }
            "consent_records_list" => {
                let records = self.records.records().map_err(storage_failed)?;
                let views: Vec<ConsentRecordView> = records
                    .iter()
                    .map(view_of)
                    .collect::<Option<_>>()
                    .ok_or_else(|| refused("consent_storage_failed"))?;
                Ok(json!({ "records": views }))
            }
            _ => Err(refused("not_implemented")),
        }
    }
}

#[cfg(test)]
mod tests;
