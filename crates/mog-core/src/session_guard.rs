//! R3 SessionGuard (section 4.3).
//!
//! Responsibilities: Stop-policy timers and OS signals (D27, D53, D71). Mic release as Active/Released/Unknown. Disk, sleep, lock and power watchers.

use std::sync::Arc;

use crate::clock::{MonotonicClock, SystemClock};
use crate::error::{CoreError, CoreResult};

const ID: &str = "R3";

/// The mic release state of the call app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicRelease {
    /// The call app still holds the mic.
    Active,
    /// The call app released the mic.
    Released,
    /// Bottle cannot tell.
    Unknown,
}

/// Stop-policy timers and OS watchers for a capture session.
pub trait SessionGuard: Send + Sync {
    /// Start the stop-policy timers and the OS watchers for a session.
    fn arm(&self, session_id: &str) -> CoreResult<()>;
    /// Report the mic release state of the call app.
    fn mic_release_state(&self) -> CoreResult<MicRelease>;
}

/// The R3 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Clone)]
pub struct StubSessionGuard {
    clock: Arc<dyn MonotonicClock>,
}

impl StubSessionGuard {
    /// A guard that reads time from `clock`. Tests pass a `SimClock` (#229).
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self { clock }
    }

    /// The clock of this guard.
    pub fn clock(&self) -> &dyn MonotonicClock {
        self.clock.as_ref()
    }
}

impl Default for StubSessionGuard {
    /// A guard on the [`SystemClock`].
    fn default() -> Self {
        Self::new(Arc::new(SystemClock::new()))
    }
}

impl SessionGuard for StubSessionGuard {
    fn arm(&self, _session_id: &str) -> CoreResult<()> {
        // TODO(#67): monotonic clock, stop bus and stop reasons.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn mic_release_state(&self) -> CoreResult<MicRelease> {
        // TODO(#131): Active / Released / Unknown mic release.
        Err(CoreError::NotImplemented { component: ID })
    }
}
