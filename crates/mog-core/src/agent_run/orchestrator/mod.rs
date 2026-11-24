//! The ask loop of ADR 0122: one orchestrator model answers the ask.
//!
//! [`OrchestratorHost`] serves the `agent_turn_*` and `agent_session_*` commands. A turn is a
//! chat tool loop on the home profile through [`crate::model_gateway::ChatClient`]. The one tool, `ask_genie`,
//! runs in process through the guard (`agent_run/guard`), so every check of the guard stays.
//!
//! - [`queue`] and [`deadlines`]: the waiting line, the rate cap and the time rules.
//! - [`prompt`]: the system and user messages of one turn.
//! - [`approval`]: the ⌥G latch and the Ask ticket.
//! - [`host`]: the actor, the sessions, the turns, the tool loop and the commands.

pub mod approval;
pub mod deadlines;
pub mod host;
pub mod prompt;
pub mod queue;

#[cfg(test)]
mod tests;

pub use host::{
    OrchConfig, OrchDeps, OrchEnv, OrchGuard, OrchStatus, OrchestratorHost, Snapshot, WorkspaceView,
};
