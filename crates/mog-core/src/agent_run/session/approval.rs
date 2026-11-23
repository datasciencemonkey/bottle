//! What approves an `agent_turn_submit`: the ⌥G latch or an Ask ticket (plan Amendment 2, A5).
//!
//! The hotkey host arms the latch when the user lifts the talk key after ⌥G. The card chips
//! `agent-more:` and `agent-resend:` mint an Ask ticket. Each approval is single-use, and a
//! queued turn keeps the approval that it got at submit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::Value;

use crate::action_ticket::{ActionTickets, Purpose};

/// How long an armed latch stays valid (the time to transcribe and submit).
pub const LATCH_TTL_MS: u64 = 60_000;

/// Approves one submit.
pub trait Approvals: Send + Sync {
    /// True if `mog_id` has an open latch, or `ticket` is a valid Ask ticket for it. The
    /// approval is used up.
    fn redeem(&self, mog_id: &str, ticket: Option<&str>) -> bool;
}

/// The ⌥G latch: one armed slot for each MOG. An empty MOG id (a hold with no open MOG) fits any
/// MOG.
#[derive(Debug, Default)]
pub struct AskLatch {
    armed: Mutex<HashMap<String, u64>>,
}

impl AskLatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// The user lifted the talk key after ⌥G.
    pub fn arm(&self, mog_id: &str, now_ms: u64) {
        self.armed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(mog_id.to_owned(), now_ms);
    }

    /// Use the latch of `mog_id` (or the wildcard latch). True once for each arm.
    pub fn take(&self, mog_id: &str, now_ms: u64) -> bool {
        let mut m = self.armed.lock().unwrap_or_else(PoisonError::into_inner);
        m.retain(|_, at| now_ms.saturating_sub(*at) < LATCH_TTL_MS);
        m.remove(mog_id).is_some() || m.remove("").is_some()
    }
}

/// The latch and the ticket store of the app.
pub struct TicketApprovals {
    pub latch: Arc<AskLatch>,
    pub tickets: Arc<ActionTickets>,
    pub now_ms: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl std::fmt::Debug for TicketApprovals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TicketApprovals").finish_non_exhaustive()
    }
}

impl Approvals for TicketApprovals {
    fn redeem(&self, mog_id: &str, ticket: Option<&str>) -> bool {
        let now = (self.now_ms)();
        match ticket {
            Some(t) => self
                .tickets
                .redeem(t, mog_id, Purpose::Ask, &Value::Null, now)
                .is_ok(),
            None => self.latch.take(mog_id, now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const MOG: &str = "0123456789abcdef0123456789abcdef";

    fn approvals() -> (TicketApprovals, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(1_000));
        let n = Arc::clone(&now);
        (
            TicketApprovals {
                latch: Arc::new(AskLatch::new()),
                tickets: Arc::new(ActionTickets::new()),
                now_ms: Arc::new(move || n.load(Ordering::SeqCst)),
            },
            now,
        )
    }

    #[test]
    fn the_latch_works_once_and_expires() {
        let (a, now) = approvals();
        assert!(!a.redeem(MOG, None));
        a.latch.arm(MOG, 1_000);
        assert!(a.redeem(MOG, None));
        assert!(!a.redeem(MOG, None));
        a.latch.arm(MOG, 1_000);
        now.store(1_000 + LATCH_TTL_MS, Ordering::SeqCst);
        assert!(!a.redeem(MOG, None));
    }

    #[test]
    fn an_empty_mog_latch_fits_any_mog_once() {
        let (a, _) = approvals();
        a.latch.arm("", 1_000);
        assert!(a.redeem(MOG, None));
        assert!(!a.redeem(MOG, None));
    }

    #[test]
    fn an_ask_ticket_works_once_for_its_mog_only() {
        let (a, _) = approvals();
        a.tickets.mint_ask("t1".into(), MOG, 1_000);
        assert!(a.redeem(MOG, Some("t1")));
        assert!(!a.redeem(MOG, Some("t1")));
        a.tickets.mint_ask("t2".into(), MOG, 1_000);
        assert!(!a.redeem("ffffffffffffffffffffffffffffffff", Some("t2")));
        // A ticket does not use the latch, and the latch does not use a ticket.
        a.latch.arm(MOG, 1_000);
        assert!(!a.redeem(MOG, Some("unknown")));
        assert!(a.redeem(MOG, None));
    }
}
