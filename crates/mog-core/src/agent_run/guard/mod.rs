//! The `ask_genie` guard (R2): the Rust guardrail behind the single MCP tool that the one Genie
//! Code session may call, the stdio shim, and the channel between them.
//!
//! - [`ledger`]: the session ledger and [`SessionGuard`] (`seam.rs`): sessions, turns, the
//!   per-turn budget, the session call log, the session taint, the stop rules.
//! - [`ask`]: the ten `ask_genie` checks, in order, fail closed.
//! - [`genie_poll`]: the Rust poll loop (G-POLL delays, 30 s cap).
//! - [`receipt`]: receipts and text blocks (quarantine, D1 off).
//! - [`session_log`]: the session id as a UUID, for the real audit log.
//! - [`channel`], [`peer`] (unix): the app socket and its authentication.
//! - [`shim`] (unix): the stdio MCP shim that Genie Code starts once per server process.
//!
//! One shim per server process. `hello{sessionId, secret}` binds it to the session. The parent of
//! the peer process must be the app-server PID of that session (ADR in
//! `docs/adr/*-bottle-mcp-shim-and-guardrail.md`).
//!
//! No type here puts a question, an answer, a secret or a token into `Debug` output, a log line
//! or an error. The module writes no log line.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{json, Value};

use super::seam::GuardDeps;
use crate::mcp_transport::agent_asks::AgentAsks;

pub mod ask;
#[cfg(unix)]
pub mod channel;
pub mod genie_poll;
mod ledger;
#[cfg(unix)]
pub mod peer;
pub mod receipt;
pub mod session_log;
#[cfg(unix)]
pub mod shim;
#[cfg(test)]
mod tests;

pub use receipt::ToolOut;

/// D1 (combine across workspaces) is not supported in 2a. With this off, a workspace that is not
/// the agent workspace always gets a receipt, whatever `SessionSpec::cross_workspace` says.
pub const D1_SUPPORTED: bool = false;

/// The one tool that the model may call.
pub const TOOL_NAME: &str = "ask_genie";
/// The longest `workspace` argument.
pub const MAX_WORKSPACE_CHARS: usize = 32;
/// The longest `question` argument.
pub const MAX_QUESTION_CHARS: usize = 1_000;
/// The most refusals of one turn. The third one stops the turn (`refusals`).
pub const MAX_REFUSALS: u32 = 3;
/// The ledger of an ended session stays this long, unless Wipe drops it at once.
pub const LEDGER_KEEP: Duration = Duration::from_secs(10 * 60);

/// Model-facing code: no live turn. Nothing was sent.
pub const NO_ACTIVE_TURN: &str = "no_active_turn";
/// Model-facing code: the session or the turn stopped.
pub const RUN_STOPPED: &str = "run_stopped";

/// The `ask_genie` tool as the shim lists it. It has no enum of aliases: the catalog is in each
/// turn's prompt, and Rust checks the alias.
pub fn tool_schema() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Ask the Genie space of ONE connected Databricks workspace a \
            natural-language question. `workspace` is one of the workspace aliases that the \
            prompt lists. The reply holds a callId and the answer, or a receipt when the user \
            sees the answer and you do not.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "workspace": {"type": "string", "maxLength": MAX_WORKSPACE_CHARS},
                "question": {"type": "string", "minLength": 1, "maxLength": MAX_QUESTION_CHARS},
            },
            "required": ["workspace", "question"],
            "additionalProperties": false,
        },
    })
}

/// How the guard waits between polls. The production sleeper sleeps in short slices and wakes at
/// once when `cancel` turns true. Tests give one that moves a simulated clock.
pub trait Sleeper: Send + Sync {
    fn sleep(&self, d: Duration, cancel: &AtomicBool);
}

/// The production [`Sleeper`].
#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&self, d: Duration, cancel: &AtomicBool) {
        let slice = Duration::from_millis(50);
        let end = std::time::Instant::now() + d;
        loop {
            let left = end.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() || cancel.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(left.min(slice));
        }
    }
}

/// A number in [0, 1) for the poll jitter.
fn system_random() -> f64 {
    let mut b = [0u8; 4];
    if getrandom::fill(&mut b).is_err() {
        return 0.5;
    }
    f64::from(u32::from_le_bytes(b)) / (f64::from(u32::MAX) + 1.0)
}

/// The seams of the guard beyond [`GuardDeps`]. The defaults are the production ones.
pub struct GuardOptions {
    pub sleeper: Arc<dyn Sleeper>,
    pub random: Arc<dyn Fn() -> f64 + Send + Sync>,
    /// The in-flight agent asks. `None` makes one on the clock of the deps.
    pub asks: Option<Arc<AgentAsks>>,
}

impl Default for GuardOptions {
    fn default() -> Self {
        Self {
            sleeper: Arc::new(ThreadSleeper),
            random: Arc::new(system_random),
            asks: None,
        }
    }
}

/// Called with the turn id when the third refusal of a turn stops it. The session host (R1)
/// interrupts the turn and ends it with the outcome `refusals`.
pub type RefusalHook = Arc<dyn Fn(&str) + Send + Sync>;

/// The guard of the one session (R2). It implements [`super::seam::SessionGuard`] for the session
/// host and serves `ask_genie` for the shim channel.
pub struct AgentGuard {
    pub(crate) deps: GuardDeps,
    pub(crate) opts: GuardOptions,
    pub(crate) asks: Arc<AgentAsks>,
    state: Mutex<ledger::State>,
    pub(crate) cv: Condvar,
    hook: Mutex<Option<RefusalHook>>,
    #[cfg(unix)]
    pub(crate) server: Mutex<Option<channel::Server>>,
}

impl fmt::Debug for AgentGuard {
    /// Shows nothing of any session.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentGuard").finish_non_exhaustive()
    }
}

impl AgentGuard {
    /// A guard with the production seams.
    pub fn new(deps: GuardDeps) -> Arc<Self> {
        Self::with_options(deps, GuardOptions::default())
    }

    /// A guard with the given seams.
    pub fn with_options(deps: GuardDeps, opts: GuardOptions) -> Arc<Self> {
        let asks = opts
            .asks
            .clone()
            .unwrap_or_else(|| Arc::new(AgentAsks::new(deps.clock.clone())));
        Arc::new(Self {
            deps,
            opts,
            asks,
            state: Mutex::new(ledger::State::default()),
            cv: Condvar::new(),
            hook: Mutex::new(None),
            #[cfg(unix)]
            server: Mutex::new(None),
        })
    }

    /// Set the function that runs when the third refusal of a turn stops it.
    pub fn set_refusal_hook(&self, hook: RefusalHook) {
        *self.hook.lock().unwrap_or_else(|p| p.into_inner()) = Some(hook);
    }

    pub(crate) fn fire_refusal_hook(&self, turn_id: &str) {
        let hook = self.hook.lock().unwrap_or_else(|p| p.into_inner()).clone();
        if let Some(h) = hook {
            h(turn_id);
        }
    }

    /// The refusals of a turn, for the host.
    pub fn refusals(&self, turn_id: &str) -> u32 {
        let st = self.lock();
        st.sessions
            .values()
            .find_map(|s| s.turns.get(turn_id))
            .map_or(0, |t| t.refusals)
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, ledger::State> {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        // An ended session leaves 10 min after it ended.
        let now = self.deps.clock.now();
        st.sessions.retain(|_, s| {
            s.closed_at
                .is_none_or(|at| now.saturating_sub(at) < LEDGER_KEEP)
        });
        st
    }

    pub(crate) fn emit(&self, events: Vec<super::seam::AgentEvent>) {
        for e in events {
            self.deps.events.emit(e);
        }
    }
}

impl Drop for AgentGuard {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(s) = self
            .server
            .get_mut()
            .unwrap_or_else(|p| p.into_inner())
            .take()
        {
            s.stop();
        }
    }
}
