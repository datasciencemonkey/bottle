//! The Rust session host (R1): one `genie app-server` process per user, the turn queue, the
//! deadlines, the disposable GENIE_HOME and the wipe hook (plan Amendment 2, A3).
//!
//! - [`appserver`]: the JSON-RPC wire, the line cap, the stderr watcher and the [`Spawner`] seam.
//! - [`genie_home`]: `config.toml`, the schema file, the env allow-list and the delete.
//! - [`queue`] and [`deadlines`]: the waiting line, the rate cap and the time rules.
//! - [`genie_bin`] and [`install`]: find, pin and install the binary.
//! - [`prompt`]: the template of one turn.
//! - [`approval`]: the ⌥G latch and the Ask ticket.
//! - [`spawn`]: the real process spawner, the group kill and the stale sweep.
//! - [`component`]: the actor, the session, the turns and the commands.

pub mod approval;
pub mod appserver;
pub mod component;
pub mod deadlines;
pub mod genie_bin;
pub mod genie_home;
pub mod install;
pub mod prompt;
pub mod queue;
pub mod spawn;

#[cfg(test)]
mod tests;

pub use appserver::{ServerIo, ServerProc, SpawnSpec, Spawner};
pub use component::{
    AgentEnv, BinaryCheck, Installer, PreflightFail, SessionConfig, SessionDeps, SessionHost,
    ShimLaunch, ShimProvider, Snapshot, WorkspaceView,
};

pub use crate::agent_run::guard::D1_SUPPORTED;
