//! R2 CaptureService (section 4.3).
//!
//! Responsibilities: The only owner of the mic and system audio. Leases, You and Others tracks, Silero VAD, segments (15-20 s target, 30 s hard), level stream, encrypted journal (60 s rotation, crypto-shred), raw-audio deletion.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R2";

/// The only owner of the mic and system audio. Callers hold a lease while they capture.
pub trait CaptureService: Send + Sync {
    /// Take a capture lease for one MOG. Returns the lease ID.
    fn acquire_lease(&self, mog_id: &str) -> CoreResult<u64>;
    /// Give back a lease. The mic closes when no lease is left.
    fn release_lease(&self, lease_id: u64) -> CoreResult<()>;
}

/// The R2 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubCaptureService;

impl CaptureService for StubCaptureService {
    fn acquire_lease(&self, _mog_id: &str) -> CoreResult<u64> {
        // TODO(#4): single mic owner and leases.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn release_lease(&self, _lease_id: u64) -> CoreResult<()> {
        // TODO(#4): release the lease and close the mic when idle.
        Err(CoreError::NotImplemented { component: ID })
    }
}
