//! The R8 in-flight poll guard (ARC-F1, G-07b; #31, ADR 0073).
//!
//! One `genie_poll_response` at a time for each conversation, whoever calls: the brain PollActor
//! (T9) through `mcp_call`, or the main agent through the loopback proxy (`super::proxy`). A second
//! poll for a conversation while one is open fails at once with [`E_POLL_INFLIGHT`]; nothing is
//! sent. The slot is freed when the first poll ends, on every path (reply, error, panic).
//!
//! The guard keeps only conversation IDs in memory, and never shows them in `Debug` output, a
//! log line or an error.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Mutex, MutexGuard};

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;

/// ARC-F1: a poll for this conversation is open already. The brain polls again later with the
/// same IDs; it never asks again.
pub const E_POLL_INFLIGHT: &str = "poll_inflight";

/// The tool that the guard serialises.
pub const POLL_TOOL: &str = "genie_poll_response";

/// The open polls, one per conversation.
#[derive(Default)]
pub struct PollInflightGuard {
    open: Mutex<HashSet<String>>,
}

impl fmt::Debug for PollInflightGuard {
    /// Shows only the count: no conversation ID.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollInflightGuard")
            .field("open", &self.open_polls())
            .finish()
    }
}

/// The open-poll slot of one conversation. Dropping it frees the slot.
pub struct PollSlot<'a> {
    guard: &'a PollInflightGuard,
    conversation: String,
}

impl fmt::Debug for PollSlot<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PollSlot").finish_non_exhaustive()
    }
}

impl Drop for PollSlot<'_> {
    fn drop(&mut self) {
        lock(&self.guard.open).remove(&self.conversation);
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl PollInflightGuard {
    /// A guard with no open poll.
    pub fn new() -> Self {
        Self::default()
    }

    /// Claim the slot of the conversation of a `genie_poll_response` call.
    ///
    /// - Another tool, or a poll with no string `conversation_id`, needs no slot: `Ok(None)`.
    ///   (The contract check refuses a poll without one before anything is sent.)
    /// - A poll on a conversation with an open poll: [`E_POLL_INFLIGHT`].
    /// - Else the slot, held until the returned [`PollSlot`] drops.
    pub fn enter(&self, tool: &str, args: &JsonObject) -> CoreResult<Option<PollSlot<'_>>> {
        if tool != POLL_TOOL {
            return Ok(None);
        }
        let Some(conversation) = args.get("conversation_id").and_then(|v| v.as_str()) else {
            return Ok(None);
        };
        let mut open = lock(&self.open);
        if !open.insert(conversation.to_owned()) {
            return Err(CoreError::Unavailable {
                component: "R8",
                reason: E_POLL_INFLIGHT,
            });
        }
        Ok(Some(PollSlot {
            guard: self,
            conversation: conversation.to_owned(),
        }))
    }

    /// The count of open polls.
    pub fn open_polls(&self) -> usize {
        lock(&self.open).len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn poll_args(cid: &str) -> JsonObject {
        let mut a = JsonObject::new();
        a.insert("conversation_id".into(), json!(cid));
        a.insert("response_id".into(), json!("r-1"));
        a
    }

    fn code<T>(r: CoreResult<T>) -> &'static str {
        match r {
            Ok(_) => "ok",
            Err(e) => e.code(),
        }
    }

    #[test]
    fn arc_f1_g_07b_second_poll_on_a_conversation_is_e_poll_inflight() {
        let g = PollInflightGuard::new();
        let first = g.enter(POLL_TOOL, &poll_args("c-1")).unwrap();
        assert!(first.is_some());
        assert_eq!(code(g.enter(POLL_TOOL, &poll_args("c-1"))), E_POLL_INFLIGHT);
        assert_eq!(g.open_polls(), 1);
    }

    #[test]
    fn arc_f1_g_07b_slot_is_freed_when_the_first_poll_ends() {
        let g = PollInflightGuard::new();
        let first = g.enter(POLL_TOOL, &poll_args("c-1")).unwrap();
        drop(first);
        assert_eq!(g.open_polls(), 0);
        assert!(g.enter(POLL_TOOL, &poll_args("c-1")).unwrap().is_some());
    }

    #[test]
    fn arc_f1_g_07b_slot_is_freed_after_a_panic() {
        let g = PollInflightGuard::new();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _slot = g.enter(POLL_TOOL, &poll_args("c-1")).unwrap();
            panic!("transport panicked");
        }));
        assert!(r.is_err());
        assert_eq!(g.open_polls(), 0);
        assert!(g.enter(POLL_TOOL, &poll_args("c-1")).unwrap().is_some());
    }

    #[test]
    fn g_07b_other_conversations_and_other_tools_are_not_blocked() {
        let g = PollInflightGuard::new();
        let _a = g.enter(POLL_TOOL, &poll_args("c-1")).unwrap();
        let b = g.enter(POLL_TOOL, &poll_args("c-2")).unwrap();
        assert!(b.is_some(), "another conversation polls at the same time");
        assert!(g.enter("genie_ask", &poll_args("c-1")).unwrap().is_none());
        assert!(g
            .enter("genie_cancel_response", &poll_args("c-1"))
            .unwrap()
            .is_none());
        assert_eq!(g.open_polls(), 2);
    }

    #[test]
    fn arc_f1_g_16c_e_poll_inflight_crosses_ipc_as_itself() {
        // The brain PollActor polls again on this code (and on `timeout`), so the host must not
        // turn it into `host_failed` (#479).
        assert_eq!(E_POLL_INFLIGHT, "poll_inflight");
        assert!(crate::ipc_gateway::ERROR_CODES.contains(&E_POLL_INFLIGHT));
        assert!(crate::ipc_gateway::ERROR_CODES.contains(&"timeout"));
    }

    #[test]
    fn sec_03_poll_guard_debug_and_error_hide_the_conversation_id() {
        let g = PollInflightGuard::new();
        let slot = g.enter(POLL_TOOL, &poll_args("conv-secret-id")).unwrap();
        let e = g
            .enter(POLL_TOOL, &poll_args("conv-secret-id"))
            .expect_err("the second poll must fail");
        for text in [format!("{g:?}"), format!("{slot:?}"), format!("{e:?} {e}")] {
            assert!(!text.contains("conv-secret-id"), "{text}");
        }
        assert_eq!(format!("{g:?}"), "PollInflightGuard { open: 1 }");
    }
}
