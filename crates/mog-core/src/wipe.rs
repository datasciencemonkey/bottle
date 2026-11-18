//! R14 WipeService (section 4.3).
//!
//! Responsibilities: Deletes user content only when the user selects Wipe in Settings (older than 30 d, older than 90 d, or everything); (v1) applies the same selection to Lakebase on the next connection. Also runs the content-free security expiries: the audio deletion deadline, log rotation and consent records (U9).

use crate::error::{CoreError, CoreResult};

const ID: &str = "R14";

/// The Wipe choice that the user selects in Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeScope {
    /// User content older than 30 days.
    OlderThan30Days,
    /// User content older than 90 days.
    OlderThan90Days,
    /// All user content.
    Everything,
}

/// Deletes user content only on a user selection, and runs the content-free expiries.
pub trait WipeService: Send + Sync {
    /// Delete user content for the scope that the user selected in Settings.
    fn wipe(&self, scope: WipeScope) -> CoreResult<()>;
    /// Run the content-free security expiries (audio deadline, log rotation, consent records).
    fn run_security_expiries(&self) -> CoreResult<()>;
}

/// The R14 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubWipeService;

impl WipeService for StubWipeService {
    fn wipe(&self, _scope: WipeScope) -> CoreResult<()> {
        // TODO(#124): WipeService and the Section 9.2 retention rules.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn run_security_expiries(&self) -> CoreResult<()> {
        // TODO(#124): audio deletion deadline, log rotation and consent records.
        Err(CoreError::NotImplemented { component: ID })
    }
}

#[cfg(test)]
mod tests;
