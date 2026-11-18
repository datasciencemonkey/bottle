//! The blocking wait in the shared rate bucket and the busy states of the card (#138, ADR 0075;
//! G-09b, ARC-F3).
//!
//! [`acquire`] holds the calling worker thread until the [`RateBucket`] gives its call a slot,
//! with a [`Ticket`] of the call class (G-09b). [`RateLimit`] is the set of seams that R8
//! McpTransport takes for the bucket, the G-BACKOFF jitter (G-10b) and the [`BusyObserver`].
//!
//! The busy states are content-free: a MOG ID and a wait, never a profile, tool, question or
//! token. TODO(#485): the host forwards them to the brain as the `genie:busy` event, so the
//! card shows "Genie busy · queued" or "Genie busy, retrying in N s".

use std::sync::Arc;
use std::time::Duration;

use super::backoff::{Jitter, OsJitter};
use super::rate_bucket::{CallClass, RateBucket, Ticket};
use super::{Pause, ThreadPause};

/// A busy state of one MOG while R8 waits for Genie (G-09b, ARC-F3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyState {
    /// An ask waits for a slot in the bucket of its profile.
    Queued,
    /// An ask got a 429 and is sent again after `wait`.
    Retrying {
        /// The time until the ask is sent again.
        wait: Duration,
    },
    /// The wait is over: the call was sent, or R8 gave up.
    Clear,
}

impl BusyState {
    /// The card text of the state (Appendix E chip 8, ARC-F3), or `None` for [`BusyState::Clear`].
    /// The seconds round up, and the text never shows less than 1 s.
    pub fn card_text(&self) -> Option<String> {
        match self {
            Self::Queued => Some("Genie busy · queued".to_owned()),
            Self::Retrying { wait } => {
                let mut s = wait.as_secs();
                if wait.subsec_nanos() > 0 {
                    s += 1;
                }
                Some(format!("Genie busy, retrying in {} s", s.max(1)))
            }
            Self::Clear => None,
        }
    }
}

/// Where R8 reports the busy states of a MOG.
pub trait BusyObserver: Send + Sync {
    /// The MOG `mog` is now in `state`.
    fn busy(&self, mog: &str, state: BusyState);
}

/// An observer that drops every state.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoBusyObserver;

impl BusyObserver for NoBusyObserver {
    fn busy(&self, _mog: &str, _state: BusyState) {}
}

/// The rate seams of R8: the shared bucket, how to wait, the jitter and the busy observer.
pub struct RateLimit {
    /// The bucket that R7 and R8 share (one per process).
    pub bucket: Arc<dyn RateBucket>,
    /// How R8 waits.
    pub pause: Arc<dyn Pause>,
    /// The jitter of the G-BACKOFF ask retry.
    pub jitter: Arc<dyn Jitter>,
    /// Where the busy states go.
    pub observer: Arc<dyn BusyObserver>,
}

impl RateLimit {
    /// The production seams on `bucket`: [`ThreadPause`], [`OsJitter`] and no observer yet.
    /// TODO(#485): the host gives an observer that sends the `genie:busy` event.
    pub fn new(bucket: Arc<dyn RateBucket>) -> Self {
        Self {
            bucket,
            pause: Arc::new(ThreadPause),
            jitter: Arc::new(OsJitter),
            observer: Arc::new(NoBusyObserver),
        }
    }
}

/// Leaves the queue if the ticket is not spent, also on a panic.
struct Place<'a> {
    bucket: &'a dyn RateBucket,
    profile: &'a str,
    ticket: Option<Ticket>,
}

impl Drop for Place<'_> {
    fn drop(&mut self) {
        if let Some(t) = self.ticket.take() {
            self.bucket.leave(self.profile, t);
        }
    }
}

/// Wait until `bucket` gives one call of `class` on `profile` a slot, and count the call.
/// `on_first_wait` runs once, with the first wait, only if the call must wait.
pub fn acquire(
    bucket: &dyn RateBucket,
    pause: &dyn Pause,
    profile: &str,
    class: CallClass,
    on_first_wait: &mut dyn FnMut(Duration),
) {
    let mut place = Place {
        bucket,
        profile,
        ticket: Some(bucket.enqueue(profile, class)),
    };
    let mut waited = false;
    loop {
        let wait = match &place.ticket {
            Some(t) => bucket.take(profile, t),
            None => return,
        };
        if wait.is_zero() {
            // The ticket is spent: nothing to leave.
            place.ticket = None;
            return;
        }
        if !waited {
            waited = true;
            on_first_wait(wait);
        }
        pause.pause(wait);
    }
}
