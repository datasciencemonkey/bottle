//! The frozen internal seam of slice 2a: what the session host (R1) and the guard (R2) agree on.
//!
//! This is not a contract file, but only the orchestrator changes it. A lane that needs a
//! change here must stop and report. The seam holds counts, ids and states only: no question,
//! no answer and no token.

use std::fmt;
use std::sync::Arc;

use crate::agent_guard::AgentActionLog;
use crate::clock::MonotonicClock;
use crate::error::CoreResult;
use crate::mcp_transport::McpTransport;
use crate::types::{AgentCallState, AgentSessionEvent, AgentTurnCall, AgentTurnEvent, ProfileName};
use crate::workspaces::WorkspaceGate;

/// The argument of the shim command that Bottle puts in `config.toml`.
pub const SHIM_ARG: &str = "--agent-mcp-shim";

/// One workspace in the catalog of a session. Rust checks the alias, the model never sees a
/// profile name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    pub alias: String,
    pub profile: ProfileName,
    pub name: String,
    pub is_home: bool,
    pub is_agent_workspace: bool,
}

/// One live turn: its id and its D2 budget (a snapshot at turn start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnSpec {
    pub turn_id: String,
    pub budget: u8,
}

/// One session: the D1, catalog and agent profile snapshot at session start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSpec {
    pub session_id: String,
    pub catalog: Vec<CatalogEntry>,
    pub cross_workspace: bool,
}

/// The secret of the shim `hello`. `Debug` shows nothing of it, and it has no `PartialEq`: use
/// [`RunSecret::matches`]. It is zeroed when dropped (a volatile write per byte, so the
/// compiler keeps it).
#[derive(Clone)]
pub struct RunSecret(pub [u8; 32]);

impl RunSecret {
    /// A constant-time compare with `other`. False for a length other than 32.
    pub fn matches(&self, other: &[u8]) -> bool {
        if other.len() != self.0.len() {
            return false;
        }
        let diff = self
            .0
            .iter()
            .zip(other)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b));
        std::hint::black_box(diff) == 0
    }
}

impl Drop for RunSecret {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY: `b` is a valid, aligned, exclusive reference to one byte.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

impl fmt::Debug for RunSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RunSecret(<redacted>)")
    }
}

/// One Genie call of a session. Content-free.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallRecord {
    pub call_id: String,
    pub turn_id: String,
    pub alias: String,
    pub state: AgentCallState,
}

/// Why a turn stops. `Offline` keeps the open Genie calls polling; `User`, `Lockdown`, `Failed`
/// and `SessionEnded` cancel them; `Timeout` keeps them polling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopWhy {
    User,
    Timeout,
    Offline,
    Lockdown,
    Failed,
    SessionEnded,
}

/// The guard of the one session (R2), as the session host (R1) sees it.
pub trait SessionGuard: Send + Sync {
    fn open_session(&self, spec: SessionSpec) -> CoreResult<RunSecret>;
    fn bind_pid(&self, session_id: &str, server_pid: u32);
    fn begin_turn(&self, session_id: &str, turn: TurnSpec) -> CoreResult<()>;
    fn stop_turn(&self, turn_id: &str, why: StopWhy);
    fn end_turn(&self, turn_id: &str);
    /// True once a D1-off session got text from the agent workspace (a taint lasts the session).
    fn tainted(&self, session_id: &str) -> bool;
    /// The whole session, all turns.
    fn calls(&self, session_id: &str) -> Vec<CallRecord>;
    fn call_detail(&self, turn_id: &str, call_id: &str) -> Option<AgentTurnCall>;
    /// `wipe` drops the records at once; otherwise they stay for 10 minutes.
    fn close_session(&self, session_id: &str, wipe: bool);
}

/// The network state from ARC-F7 `NetStatus`.
pub trait NetWatch: Send + Sync {
    fn online(&self) -> bool;
    /// Call once, at build. There is no way to unsubscribe.
    fn subscribe(&self, f: Box<dyn Fn(bool) + Send + Sync>);
}

/// One content-free event that the guard and the session host send to the brain Channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    Session(AgentSessionEvent),
    Turn(AgentTurnEvent),
}

/// Where events go: the host sends them to the brain Channel as `agent:session` or `agent:turn`.
pub trait RunEvents: Send + Sync {
    fn emit(&self, e: AgentEvent);
}

/// What the guard needs from the host. It does not change in slice 2a.
pub struct GuardDeps {
    pub transport: Arc<dyn McpTransport>,
    pub gate: Arc<dyn WorkspaceGate>,
    pub events: Arc<dyn RunEvents>,
    pub action_log: Arc<dyn AgentActionLog>,
    pub clock: Arc<dyn MonotonicClock>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_run_secret_debug_shows_no_byte() {
        let s = RunSecret([0xab; 32]);
        let shown = format!("{s:?}");
        assert!(!shown.contains("ab") && !shown.contains("171"), "{shown}");
    }

    #[test]
    fn the_run_secret_compares_in_full_and_only_to_32_bytes() {
        let s = RunSecret([7; 32]);
        assert!(s.matches(&[7; 32]));
        let mut other = [7u8; 32];
        other[31] = 8;
        assert!(!s.matches(&other));
        assert!(!s.matches(&[7; 31]));
        assert!(!s.matches(&[7; 33]));
    }

    #[test]
    fn the_seam_types_are_object_safe() {
        fn _guard(_: &dyn SessionGuard) {}
        fn _net(_: &dyn NetWatch) {}
        fn _events(_: &dyn RunEvents) {}
    }
}
