//! Monotonic clock (#229): one time source that SessionGuard and the tests share.
//!
//! The stop policy runs on a monotonic clock, so a wall-clock change does not move a stop
//! (CAP-74a, owned by #67). [`SystemClock`] is the production clock. [`SimClock`] is the
//! simulated clock that the stop-policy tests drive (AT-25, CAP-74b).

use std::fmt::Debug;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

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
