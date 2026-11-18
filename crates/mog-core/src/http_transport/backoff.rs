//! Table G-BACKOFF (G-10b, #138, ADR 0075): the retry wait of an ask after a 429 with no retry
//! header. Full jitter · start 2 s · factor 2 · maximum 60 s · 5 tries or less.
//!
//! "5 tries" counts every send of the ask, the first one too: so at most 4 waits. A poll or a
//! cancel never uses this table; R8 does not send it again (only its interval gets longer).

use std::time::Duration;

/// The cap of the first retry wait.
pub const BACKOFF_START: Duration = Duration::from_secs(2);
/// Each retry doubles the cap.
pub const BACKOFF_FACTOR: u32 = 2;
/// The largest cap.
pub const BACKOFF_MAX: Duration = Duration::from_secs(60);
/// The most sends of one ask after 429 replies, the first send included.
pub const MAX_ASK_TRIES: u32 = 5;

/// A source of uniform numbers in `[0, 1)` for full jitter.
pub trait Jitter: Send + Sync {
    /// A uniform number in `[0, 1)`.
    fn unit(&self) -> f64;
}

/// [`Jitter`] from the OS random source (`getrandom`). If the source fails, the wait is the
/// full cap, never zero, so a fault does not send the ask again at once.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsJitter;

impl Jitter for OsJitter {
    fn unit(&self) -> f64 {
        let mut b = [0u8; 8];
        if getrandom::fill(&mut b).is_err() {
            return 1.0 - f64::EPSILON;
        }
        // The top 53 bits give a uniform f64 in [0, 1).
        (u64::from_le_bytes(b) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// The cap of retry `retry` (1 for the first retry): 2, 4, 8, 16, 32 s, then 60 s.
pub fn backoff_cap(retry: u32) -> Duration {
    let doublings = retry.saturating_sub(1).min(16);
    BACKOFF_START
        .saturating_mul(BACKOFF_FACTOR.saturating_pow(doublings))
        .min(BACKOFF_MAX)
}

/// The full-jitter wait of retry `retry`: a uniform part of [`backoff_cap`]. A jitter value out
/// of `[0, 1]` (or not a number) is held inside it.
pub fn ask_backoff(retry: u32, jitter: &dyn Jitter) -> Duration {
    let u = jitter.unit();
    let u = if u.is_nan() { 0.0 } else { u.clamp(0.0, 1.0) };
    backoff_cap(retry).mul_f64(u)
}
