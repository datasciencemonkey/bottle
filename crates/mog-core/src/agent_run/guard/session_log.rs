//! The action log as the guard needs it. The audit log (#140) takes a UUID as the MOG id, and the
//! session id of the host is 32 hex digits. [`SessionUuidLog`] writes the session id as a UUID
//! (the same digits, in 8-4-4-4-12 groups), so the real audit log accepts the entry. It adds no
//! other data: an entry stays content-free.

use std::sync::Arc;

use crate::agent_guard::AgentActionLog;
use crate::audit::AuditError;

/// An [`AgentActionLog`] that gives the session id to `inner` in UUID form.
pub struct SessionUuidLog(pub Arc<dyn AgentActionLog>);

impl std::fmt::Debug for SessionUuidLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionUuidLog").finish_non_exhaustive()
    }
}

/// `id` as a UUID if it is 32 lower-case hex digits, else `id` as it is.
fn as_uuid(id: &str) -> String {
    let b = id.as_bytes();
    let hex = b.len() == 32 && b.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'));
    if !hex {
        return id.to_owned();
    }
    format!(
        "{}-{}-{}-{}-{}",
        &id[..8],
        &id[8..12],
        &id[12..16],
        &id[16..20],
        &id[20..]
    )
}

impl AgentActionLog for SessionUuidLog {
    fn record(&self, mog_id: &str, tool: &str, args_bytes: u64) -> Result<(), AuditError> {
        self.0.record(&as_uuid(mog_id), tool, args_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_32_hex_session_id_becomes_a_uuid_and_other_text_stays() {
        assert_eq!(
            as_uuid("0123456789abcdef0123456789abcdef"),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
        assert_eq!(as_uuid("s-1"), "s-1");
        assert_eq!(as_uuid("0123456789ABCDEF0123456789ABCDEF").len(), 32);
    }
}
