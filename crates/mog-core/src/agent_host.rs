//! R16 AgentHost (section 4.3).
//!
//! Responsibilities: Starts and stops the harness child process of each active MOG (Claude or Codex), gives it the loopback endpoints and a per-session secret, gives it no token, writes the content-free agent action log.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R16";

/// The agent harness kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Harness {
    /// The Claude harness.
    Claude,
    /// The Codex harness.
    Codex,
}

/// Starts one agent session per active MOG. The harness gets loopback endpoints, never a token.
pub trait AgentHost: Send + Sync {
    /// Start the agent session of `mog_id`. Returns the agent session ID.
    fn start_session(&self, mog_id: &str, harness: Harness) -> CoreResult<String>;
    /// Stop the agent session of `mog_id`.
    fn stop_session(&self, mog_id: &str) -> CoreResult<()>;
}

/// The R16 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubAgentHost;

impl AgentHost for StubAgentHost {
    fn start_session(&self, _mog_id: &str, _harness: Harness) -> CoreResult<String> {
        // TODO(#112): one shared agent runtime with a session per MOG.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn stop_session(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#112): stop the session of the MOG.
        Err(CoreError::NotImplemented { component: ID })
    }
}
