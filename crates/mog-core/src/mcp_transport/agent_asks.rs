//! The in-flight ask ledger of the main agent's Genie One calls (#139, ADR 0083).
//!
//! The brain counts its own asks (GenieBridge, PollActor). This ledger applies the same rules to
//! the asks that the main agent sends through the loopback proxy ([`super::proxy`]):
//! - G-11a: 2 in-flight asks or less for each MOG.
//! - G-11b: 4 in-flight asks or less for each profile.
//! - G-11c: 1 in-flight ask for each conversation.
//! - G-12b: an ask gives up 10 min after it started; after that no poll of its conversation is
//!   sent ([`ASK_GIVEN_UP`]).
//!
//! An ask is in flight from its `genie_ask` until a poll gives a terminal status (`completed`,
//! `incomplete`, `failed`), until a `genie_cancel_response` of its conversation, or until the
//! give-up. An ask whose send timed out (G-16a, uncertain) stays in flight until its give-up, so
//! the agent cannot ask again at once. A refused ask sends nothing.
//!
//! TODO(#518): brain asks and agent asks are counted apart, so a MOG can have 2 brain asks and
//! 2 agent asks. #518 counts them in one cap, adds the late re-polls (G-13a) and the re-ask rules.
//!
//! The ledger keeps only conversation IDs, MOG IDs and profile names, in memory, and never shows
//! them in `Debug` output, a log line or an error.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::Value;

use crate::clock::{MonotonicClock, SystemClock};

/// G-11a: the most in-flight agent asks of one MOG.
pub const MAX_ASKS_PER_MOG: usize = 2;
/// G-11b: the most in-flight agent asks of one profile.
pub const MAX_ASKS_PER_PROFILE: usize = 4;
/// G-12b: an ask gives up this long after it started.
pub const GIVE_UP_AFTER: Duration = Duration::from_secs(10 * 60);
/// The most given-up conversations the ledger remembers.
const MAX_GIVEN_UP: usize = 256;

/// G-11a: the MOG has [`MAX_ASKS_PER_MOG`] asks in flight.
pub const ASK_LIMIT_MOG: &str = "ask_limit_mog";
/// G-11b: the profile has [`MAX_ASKS_PER_PROFILE`] asks in flight.
pub const ASK_LIMIT_PROFILE: &str = "ask_limit_profile";
/// G-11c: the conversation has an ask in flight.
pub const ASK_INFLIGHT_CONVERSATION: &str = "ask_inflight_conversation";
/// G-12b: the ask of this conversation gave up; no poll is sent.
pub const ASK_GIVEN_UP: &str = "ask_given_up";

/// The Genie statuses that end an ask.
pub const TERMINAL_STATUSES: [&str; 3] = ["completed", "incomplete", "failed"];

struct Entry {
    id: u64,
    mog: String,
    profile: String,
    conversation: Option<String>,
    started: Duration,
}

#[derive(Default)]
struct State {
    next_id: u64,
    open: Vec<Entry>,
    given_up: VecDeque<String>,
}

/// The in-flight asks of the main agent, for every session of the process.
pub struct AgentAsks {
    clock: Arc<dyn MonotonicClock>,
    state: Mutex<State>,
}

impl fmt::Debug for AgentAsks {
    /// Shows only the count: no ID, MOG or profile.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentAsks")
            .field("open", &self.open_asks())
            .finish()
    }
}

impl Default for AgentAsks {
    fn default() -> Self {
        Self::new(Arc::new(SystemClock::new()))
    }
}

/// One ask that took its place. Pass it to [`AgentAsks::sent`].
#[derive(Debug)]
pub struct AskPlace(u64);

impl AgentAsks {
    /// A ledger on `clock`.
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            clock,
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // G-12b: every ask past its give-up time leaves; its conversation is given up.
        let now = self.clock.now();
        let (old, keep): (Vec<Entry>, Vec<Entry>) = std::mem::take(&mut st.open)
            .into_iter()
            .partition(|e| now.saturating_sub(e.started) >= GIVE_UP_AFTER);
        st.open = keep;
        for e in old {
            if let Some(c) = e.conversation {
                st.given_up.push_back(c);
                if st.given_up.len() > MAX_GIVEN_UP {
                    st.given_up.pop_front();
                }
            }
        }
        st
    }

    /// The number of in-flight agent asks.
    pub fn open_asks(&self) -> usize {
        self.lock().open.len()
    }

    /// Take a place for a `genie_ask` of `mog` on `profile`, on `conversation` for a follow-up.
    /// `Err` is the content-free code of the cap that refuses it; then nothing may be sent.
    pub fn begin(
        &self,
        mog: &str,
        profile: &str,
        conversation: Option<&str>,
    ) -> Result<AskPlace, &'static str> {
        let mut st = self.lock();
        if let Some(c) = conversation {
            if st.given_up.iter().any(|g| g == c) {
                return Err(ASK_GIVEN_UP);
            }
            if st.open.iter().any(|e| e.conversation.as_deref() == Some(c)) {
                return Err(ASK_INFLIGHT_CONVERSATION);
            }
        }
        if st.open.iter().filter(|e| e.mog == mog).count() >= MAX_ASKS_PER_MOG {
            return Err(ASK_LIMIT_MOG);
        }
        if st.open.iter().filter(|e| e.profile == profile).count() >= MAX_ASKS_PER_PROFILE {
            return Err(ASK_LIMIT_PROFILE);
        }
        st.next_id += 1;
        let id = st.next_id;
        let started = self.clock.now();
        st.open.push(Entry {
            id,
            mog: mog.to_owned(),
            profile: profile.to_owned(),
            conversation: conversation.map(str::to_owned),
            started,
        });
        Ok(AskPlace(id))
    }

    /// The ask of `place` went out. `outcome` is the tool result (`structuredContent`) or the
    /// failure code. A failure before the send ends the ask; a time-out keeps it (G-16a). A reply
    /// names the conversation, and a terminal status ends the ask.
    pub fn sent(&self, place: AskPlace, outcome: Result<&Value, &str>) {
        let mut st = self.lock();
        let Some(i) = st.open.iter().position(|e| e.id == place.0) else {
            return;
        };
        match outcome {
            Err("timeout") => {}
            Err(_) => {
                st.open.remove(i);
            }
            Ok(sc) => {
                if is_terminal(sc) {
                    st.open.remove(i);
                } else if let Some(c) = sc.get("conversation_id").and_then(Value::as_str) {
                    st.open[i].conversation = Some(c.to_owned());
                } else if st.open[i].conversation.is_none() {
                    // No conversation to poll: nothing keeps this ask in flight.
                    st.open.remove(i);
                }
            }
        }
    }

    /// G-12b: `Err` if the ask of `conversation` gave up, so no poll may be sent.
    pub fn may_poll(&self, conversation: &str) -> Result<(), &'static str> {
        if self.lock().given_up.iter().any(|g| g == conversation) {
            return Err(ASK_GIVEN_UP);
        }
        Ok(())
    }

    /// A poll of `conversation` answered `sc`: a terminal status ends its ask.
    pub fn polled(&self, conversation: &str, sc: &Value) {
        if is_terminal(sc) {
            self.end(conversation);
        }
    }

    /// The ask of `conversation` ended (a terminal status or a cancel, G-25c).
    pub fn end(&self, conversation: &str) {
        self.lock()
            .open
            .retain(|e| e.conversation.as_deref() != Some(conversation));
    }
}

fn is_terminal(sc: &Value) -> bool {
    sc.get("status")
        .and_then(Value::as_str)
        .is_some_and(|s| TERMINAL_STATUSES.contains(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SimClock;
    use serde_json::json;

    fn ledger() -> (AgentAsks, SimClock) {
        let c = SimClock::default();
        (AgentAsks::new(Arc::new(c.clone())), c)
    }

    fn running(cid: &str) -> Value {
        json!({"status": "in_progress", "conversation_id": cid})
    }

    #[test]
    fn g_11a_two_asks_per_mog() {
        let (l, _) = ledger();
        let a = l.begin("m", "p", None).unwrap();
        l.sent(a, Ok(&running("c-1")));
        let b = l.begin("m", "p", None).unwrap();
        l.sent(b, Ok(&running("c-2")));
        assert_eq!(l.begin("m", "p", None).unwrap_err(), ASK_LIMIT_MOG);
        assert!(
            l.begin("m2", "p", None).is_ok(),
            "another MOG is not blocked"
        );
    }

    #[test]
    fn g_11b_four_asks_per_profile() {
        let (l, _) = ledger();
        for m in ["a", "b", "c", "d"] {
            let p = l.begin(m, "p", None).unwrap();
            l.sent(p, Ok(&running(&format!("c-{m}"))));
        }
        assert_eq!(l.begin("e", "p", None).unwrap_err(), ASK_LIMIT_PROFILE);
        assert!(
            l.begin("e", "q", None).is_ok(),
            "another profile is not blocked"
        );
    }

    #[test]
    fn g_11c_one_ask_per_conversation_until_a_terminal_poll() {
        let (l, _) = ledger();
        let a = l.begin("m", "p", Some("c-1")).unwrap();
        l.sent(a, Ok(&running("c-1")));
        assert_eq!(
            l.begin("m", "p", Some("c-1")).unwrap_err(),
            ASK_INFLIGHT_CONVERSATION
        );
        l.polled("c-1", &json!({"status": "in_progress"}));
        assert!(l.begin("m", "p", Some("c-1")).is_err(), "not terminal");
        l.polled("c-1", &json!({"status": "completed"}));
        assert!(l.begin("m", "p", Some("c-1")).is_ok());
    }

    #[test]
    fn g_25c_a_cancel_ends_the_ask() {
        let (l, _) = ledger();
        let a = l.begin("m", "p", None).unwrap();
        l.sent(a, Ok(&running("c-1")));
        l.end("c-1");
        assert_eq!(l.open_asks(), 0);
    }

    #[test]
    fn g_12b_give_up_after_10_min_stops_polls() {
        let (l, c) = ledger();
        let a = l.begin("m", "p", None).unwrap();
        l.sent(a, Ok(&running("c-1")));
        c.advance(GIVE_UP_AFTER - Duration::from_secs(1));
        assert!(l.may_poll("c-1").is_ok());
        c.advance(Duration::from_secs(1));
        assert_eq!(l.may_poll("c-1").unwrap_err(), ASK_GIVEN_UP);
        assert_eq!(l.open_asks(), 0, "the ask left the caps");
        assert_eq!(l.begin("m", "p", Some("c-1")).unwrap_err(), ASK_GIVEN_UP);
    }

    #[test]
    fn g_16a_a_timed_out_ask_stays_in_flight_a_failed_one_does_not() {
        let (l, _) = ledger();
        let a = l.begin("m", "p", None).unwrap();
        l.sent(a, Err("timeout"));
        assert_eq!(l.open_asks(), 1, "uncertain: it may run upstream");
        let b = l.begin("m", "p", None).unwrap();
        l.sent(b, Err("rate_limited"));
        assert_eq!(l.open_asks(), 1, "a refused send ends the ask");
    }

    #[test]
    fn sec_03_ledger_debug_hides_ids() {
        let (l, _) = ledger();
        let a = l
            .begin("mog-secret", "profile-secret", Some("conv-secret"))
            .unwrap();
        let text = format!("{l:?} {a:?}");
        assert!(!text.contains("secret"), "{text}");
    }
}
