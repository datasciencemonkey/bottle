//! Monotonic clock (#229): one time source that SessionGuard and the tests share.
//!
//! The stop policy runs on a monotonic clock, so a wall-clock change does not move a stop
//! (CAP-74a, owned by #67). [`SystemClock`] is the production clock. [`SimClock`] is the
//! simulated clock that the stop-policy tests drive (AT-25, CAP-74b).
//!
//! [`WallClock`] is the calendar time in Unix milliseconds. Consent records (#63) store it and
//! expire on it (SEC-15b). [`SystemWallClock`] is the production clock. [`SimWallClock`] is the
//! double that the 14-month test moves.

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// A clock that never goes back. `now()` is the time since an arbitrary fixed origin.
pub trait MonotonicClock: Send + Sync + Debug {
    /// The time since the origin of this clock.
    fn now(&self) -> Duration;
}

/// The production clock, on [`Instant`]. The origin is the moment of construction.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    origin: Instant,
}

impl SystemClock {
    /// A system clock whose origin is now.
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonotonicClock for SystemClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// A simulated clock for tests. It moves only on [`SimClock::advance`]. Clones share one time,
/// and it is safe to use from many threads.
#[derive(Debug, Clone, Default)]
pub struct SimClock {
    now: Arc<Mutex<Duration>>,
}

impl SimClock {
    /// A simulated clock at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// A simulated clock at `start`.
    pub fn starting_at(start: Duration) -> Self {
        Self {
            now: Arc::new(Mutex::new(start)),
        }
    }

    /// Move the clock forward by `step`. The time saturates at [`Duration::MAX`].
    pub fn advance(&self, step: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(step);
    }
}

impl MonotonicClock for SimClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The calendar time. `now_ms()` is the time since the Unix epoch in milliseconds (UTC).
pub trait WallClock: Send + Sync + Debug {
    /// Milliseconds since 1970-01-01T00:00:00Z.
    fn now_ms(&self) -> i64;
}

/// The production wall clock, on [`SystemTime`]. A time before the epoch reads as 0.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemWallClock;

impl WallClock for SystemWallClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
    }
}

/// A simulated wall clock for tests. It moves only on [`SimWallClock::set`] or
/// [`SimWallClock::advance_ms`]. Clones share one time.
#[derive(Debug, Clone, Default)]
pub struct SimWallClock {
    now_ms: Arc<Mutex<i64>>,
}

impl SimWallClock {
    /// A simulated wall clock at `now_ms`.
    pub fn at(now_ms: i64) -> Self {
        Self {
            now_ms: Arc::new(Mutex::new(now_ms)),
        }
    }

    /// Set the time to `now_ms`.
    pub fn set(&self, now_ms: i64) {
        *self.now_ms.lock().unwrap_or_else(PoisonError::into_inner) = now_ms;
    }

    /// Move the time by `step_ms`. The time saturates.
    pub fn advance_ms(&self, step_ms: i64) {
        let mut now = self.now_ms.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(step_ms);
    }
}

impl WallClock for SimWallClock {
    fn now_ms(&self) -> i64 {
        *self.now_ms.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
