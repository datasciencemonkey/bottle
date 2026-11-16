//! R16 AgentHost (section 4.3).
//!
//! Responsibilities: Starts and stops the harness child process of each active MOG (Claude or Codex), gives it the loopback endpoints and a per-session secret, gives it no token, writes the content-free agent action log.
//!
//! Binding override (U11, ARC-7, RTR-21a, `AGENTS.md`): there is one shared harness runtime with
//! one session per active MOG, not one child process per MOG. [`SharedAgentHost`] starts one
//! runtime process for each harness kind in use (ADR 0014) and opens one session in it for each
//! MOG. Sessions share no conversation state: each has its own ID, its own secret on the model
//! proxy and its own turns. A failed session restarts once (ARC-F14) without touching the others;
//! after the second failure it reports `Failed`, and the brain routes with the fallback classifier
//! (RTR-15b).
//!
//! Tokens never reach the harness (ARC-7, RTR-28): the runtime gets a clean environment with the
//! loopback base URL only ([`env`]), the session secret goes on stdin, and only the
//! [`model_proxy`] adds the token on the outbound hop.
//!
//! The host never starts capture, approves context, publishes a Page or switches the workspace:
//! it has no handle to any of those components. It writes no log line; its status reports carry
//! the MOG ID, the session ID and the state only.
//!
//! Locking: turns, close frames, process stops and status reports run after the state lock is
//! released, so a slow runtime or a slow sink never stops the other MOGs. Open frames are queued
//! under the lock; [`runtime::RuntimeProcess::send_frame`] must not block.
//!
//! TODO(#111): SP-19 picks the runtime program, the sidecar protocol and the real outbound hop.
//! TODO(#111): the AT-42 tool-contract run with each live harness, the first-tool-call latency
//! (NFR-03d, 2.0 s or less at p95) and "usage rows show the user" (RTR-23) need SP-19 and a live
//! workspace; the test doubles here do not measure them.
//! TODO(#307): `mog_core::ipc::handle` routes `agent_start`/`agent_stop`/`agent_send` here.

pub mod adapter;
pub mod env;
pub mod model_proxy;
pub mod runtime;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde_json::{json, Value};

use self::adapter::{adapter, launch_spec};
use self::model_proxy::{ModelProxy, ModelRoute, ModelTarget, SessionSecret};
use self::runtime::{RuntimeConfig, RuntimeEvent, RuntimeEvents, RuntimeLauncher, RuntimeProcess};

/// The agent harness kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Harness {
    /// The Claude harness.
    Claude,
    /// The Codex harness.
    Codex,
}

impl Harness {
    /// The wire name (`claude` or `codex`), as in the `agent_start` arguments.
    pub fn as_str(self) -> &'static str {
        match self {
            Harness::Claude => "claude",
            Harness::Codex => "codex",
        }
    }
}

impl From<crate::ipc::Harness> for Harness {
    fn from(h: crate::ipc::Harness) -> Self {
        match h {
            crate::ipc::Harness::Claude => Harness::Claude,
            crate::ipc::Harness::Codex => Harness::Codex,
        }
    }
}

/// An AgentHost error. Only [`AgentError::code`] crosses IPC; no variant carries content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentError {
    /// The MOG has no agent session.
    UnknownMog,
    /// The session failed after its one restart, or the runtime is gone. Use the fallback.
    Unavailable,
    /// The runtime process did not start.
    LaunchFailed,
    /// The MOG has no model target (no home profile or host yet).
    TargetUnavailable,
    /// The model target is not allowed (not a bare host, or a local model off loopback).
    InvalidTarget,
    /// The MOG has a live session of the other harness kind. Stop it first.
    HarnessMismatch,
}

impl AgentError {
    /// The stable, content-free error code.
    pub fn code(self) -> &'static str {
        match self {
            AgentError::UnknownMog => "agent_unknown_mog",
            AgentError::Unavailable => "agent_unavailable",
            AgentError::LaunchFailed => "agent_launch_failed",
            AgentError::TargetUnavailable => "agent_target_unavailable",
            AgentError::InvalidTarget => "agent_invalid_target",
            AgentError::HarnessMismatch => "agent_harness_mismatch",
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for AgentError {}

/// Starts one agent session per active MOG. The harness gets loopback endpoints, never a token.
pub trait AgentHost: Send + Sync {
    /// Start the agent session of `mog_id`. Returns the agent session ID. A second call for a
    /// MOG with a live session of the same kind returns the same ID; a call with the other kind
    /// fails with [`AgentError::HarnessMismatch`].
    fn start_session(&self, mog_id: &str, harness: Harness) -> Result<String, AgentError>;
    /// Stop the agent session of `mog_id`.
    fn stop_session(&self, mog_id: &str) -> Result<(), AgentError>;
    /// Send one turn (the 4.7 `AgentTurn`) to the session of `mog_id`.
    fn send(&self, mog_id: &str, turn: &Value) -> Result<(), AgentError>;
}

/// The state of one agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Open and never restarted.
    Running,
    /// Restarted once after a failure (ARC-F14).
    Restarted,
    /// Failed again after the restart; the fallback classifier routes (RTR-15b).
    Failed,
}

/// A content-free status report: IDs and the state only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatus {
    /// The MOG.
    pub mog_id: String,
    /// The agent session.
    pub agent_session_id: String,
    /// The new state.
    pub state: SessionState,
}

/// Gets the session state changes (for the `agent.status` event).
pub trait AgentStatusSink: Send + Sync {
    /// One state change. The host calls it without holding its state lock.
    fn status(&self, s: &AgentStatus);
}

/// Resolves the model target of a MOG: the AI Gateway of its home profile, or a local model.
/// TODO(#16, #18): the real resolver reads the MOG's home profile and host from R5/R6.
pub trait ModelTargets: Send + Sync {
    /// The target of `mog_id`.
    fn target(&self, mog_id: &str) -> Result<ModelTarget, AgentError>;
}

/// The runtime program of each harness kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfigs {
    /// The Claude runtime.
    pub claude: RuntimeConfig,
    /// The Codex runtime.
    pub codex: RuntimeConfig,
}

impl RuntimeConfigs {
    fn of(&self, h: Harness) -> &RuntimeConfig {
        match h {
            Harness::Claude => &self.claude,
            Harness::Codex => &self.codex,
        }
    }
}

/// The parts that [`SharedAgentHost`] uses.
pub struct AgentHostDeps {
    /// Starts runtime processes.
    pub launcher: Arc<dyn RuntimeLauncher>,
    /// The loopback model proxy.
    pub proxy: Arc<ModelProxy>,
    /// The model target of each MOG.
    pub targets: Arc<dyn ModelTargets>,
    /// Gets the session state changes.
    pub status: Arc<dyn AgentStatusSink>,
    /// The runtime programs.
    pub runtimes: RuntimeConfigs,
}

struct Session {
    id: String,
    harness: Harness,
    model: Option<String>,
    secret: SessionSecret,
    target: ModelTarget,
    state: SessionState,
}

struct Runtime {
    generation: u64,
    process: Arc<dyn RuntimeProcess>,
    restarted: bool,
}

#[derive(Default)]
struct State {
    sessions: HashMap<String, Session>,
    runtimes: HashMap<Harness, Runtime>,
    next_generation: u64,
}

/// The R16 AgentHost: one shared runtime per harness kind in use, one session per MOG.
pub struct SharedAgentHost {
    deps: AgentHostDeps,
    state: Mutex<State>,
    me: Weak<SharedAgentHost>,
}

fn new_session_id() -> String {
    let full = SessionSecret::generate();
    format!("as-{}", &full.as_str()[..16])
}

impl SharedAgentHost {
    /// A host with no runtime and no session.
    pub fn new(deps: AgentHostDeps) -> Arc<Self> {
        Arc::new_cyclic(|me| Self {
            deps,
            state: Mutex::new(State::default()),
            me: me.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The number of runtime processes.
    pub fn runtime_count(&self) -> usize {
        self.lock().runtimes.len()
    }

    /// The number of agent sessions, failed ones included.
    pub fn session_count(&self) -> usize {
        self.lock().sessions.len()
    }

    /// The state of the session of `mog_id`.
    pub fn session_state(&self, mog_id: &str) -> Option<SessionState> {
        self.lock().sessions.get(mog_id).map(|s| s.state)
    }

    /// [`AgentHost::start_session`] with an optional model name (the `agent_start` `model`).
    pub fn start_session_with_model(
        &self,
        mog_id: &str,
        harness: Harness,
        model: Option<&str>,
    ) -> Result<String, AgentError> {
        // The target lookup may run the CLI (#16, #18): do it before the lock, so it never
        // stalls the other MOGs.
        let target = self.deps.targets.target(mog_id);
        let mut st = self.lock();
        if let Some(s) = st.sessions.get(mog_id) {
            if s.state != SessionState::Failed && s.harness != harness {
                return Err(AgentError::HarnessMismatch);
            }
            if s.state != SessionState::Failed {
                return Ok(s.id.clone());
            }
            // A new start after a failure is a user action: replace the failed session.
            st.sessions.remove(mog_id);
        }
        let target = target?;
        let session = Session {
            id: new_session_id(),
            harness,
            model: model.map(str::to_owned),
            secret: SessionSecret::generate(),
            target,
            state: SessionState::Running,
        };
        self.register(mog_id, &session)?;
        let opened = self
            .ensure_runtime(&mut st, harness)
            .and_then(|()| self.open(&st, &session));
        if let Err(e) = opened {
            self.deps.proxy.unregister(&session.secret);
            let idle = Self::take_idle_runtime(&mut st, harness);
            drop(st);
            if let Some(p) = idle {
                p.stop();
            }
            return Err(e);
        }
        let id = session.id.clone();
        st.sessions.insert(mog_id.to_owned(), session);
        Ok(id)
    }

    fn register(&self, mog_id: &str, s: &Session) -> Result<(), AgentError> {
        self.deps.proxy.register(
            &s.secret,
            ModelRoute {
                mog_id: mog_id.to_owned(),
                harness: s.harness,
                target: s.target.clone(),
            },
        )
    }

    fn ensure_runtime(&self, st: &mut State, h: Harness) -> Result<(), AgentError> {
        if st.runtimes.contains_key(&h) {
            return Ok(());
        }
        let rt = self.launch(st, h)?;
        st.runtimes.insert(h, rt);
        Ok(())
    }

    fn launch(&self, st: &mut State, h: Harness) -> Result<Runtime, AgentError> {
        st.next_generation += 1;
        let generation = st.next_generation;
        let spec = launch_spec(h, self.deps.runtimes.of(h), &self.deps.proxy.base_url());
        let me = self.me.clone();
        let events: RuntimeEvents = Arc::new(move |e| {
            if let Some(host) = me.upgrade() {
                host.on_runtime_event(h, generation, e);
            }
        });
        let process: Arc<dyn RuntimeProcess> = Arc::from(self.deps.launcher.launch(&spec, events)?);
        Ok(Runtime {
            generation,
            process,
            restarted: false,
        })
    }

    fn open(&self, st: &State, s: &Session) -> Result<(), AgentError> {
        let rt = st.runtimes.get(&s.harness).ok_or(AgentError::Unavailable)?;
        let a = adapter(s.harness);
        let base = a.harness_base_url(&self.deps.proxy.base_url());
        let frame = a.open_frame(&s.id, &base, s.secret.as_str(), s.model.as_deref());
        rt.process.send_frame(&frame)
    }

    /// Restart one session once (ARC-F14), with a new secret, or mark it failed. A session
    /// whose runtime is gone for good fails at once. The status report goes to `out`; the caller
    /// sends it after it releases the lock.
    fn restart_or_fail(
        &self,
        st: &State,
        mog_id: &str,
        s: &mut Session,
        out: &mut Vec<AgentStatus>,
    ) {
        self.deps.proxy.unregister(&s.secret);
        let runtime_up = st.runtimes.contains_key(&s.harness);
        if s.state == SessionState::Running && runtime_up {
            s.secret = SessionSecret::generate();
            s.state = SessionState::Restarted;
            if self.register(mog_id, s).is_err() || self.open(st, s).is_err() {
                self.deps.proxy.unregister(&s.secret);
                s.state = SessionState::Failed;
            }
        } else {
            s.state = SessionState::Failed;
        }
        out.push(AgentStatus {
            mog_id: mog_id.to_owned(),
            agent_session_id: s.id.clone(),
            state: s.state,
        });
    }

    /// Handle one event of the runtime of kind `h`, generation `generation`. Events of an older
    /// process of the same kind are ignored.
    fn on_runtime_event(&self, h: Harness, generation: u64, e: RuntimeEvent) {
        let mut out = Vec::new();
        let old = self.handle_event(h, generation, e, &mut out);
        // Outside the lock: stop the dead process and report the state changes.
        if let Some(p) = old {
            p.stop();
        }
        for s in &out {
            self.deps.status.status(s);
        }
    }

    /// The locked part of [`Self::on_runtime_event`]. Returns the exited process, if any.
    fn handle_event(
        &self,
        h: Harness,
        generation: u64,
        e: RuntimeEvent,
        out: &mut Vec<AgentStatus>,
    ) -> Option<Arc<dyn RuntimeProcess>> {
        let mut guard = self.lock();
        let st = &mut *guard;
        let current = st
            .runtimes
            .get(&h)
            .is_some_and(|rt| rt.generation == generation);
        if !current {
            return None;
        }
        match e {
            RuntimeEvent::SessionFailed { session_id } => {
                let mog_id = st
                    .sessions
                    .iter()
                    .find(|(_, s)| s.id == session_id && s.harness == h)
                    .map(|(m, _)| m.clone())?;
                if st.sessions[&mog_id].state == SessionState::Failed {
                    return None;
                }
                let mut s = st.sessions.remove(&mog_id).expect("found above");
                self.restart_or_fail(st, &mog_id, &mut s, out);
                st.sessions.insert(mog_id, s);
                None
            }
            RuntimeEvent::Exited => {
                let old = st.runtimes.remove(&h).expect("current above");
                let relaunched = if old.restarted {
                    None
                } else {
                    self.launch(st, h).ok().map(|mut rt| {
                        rt.restarted = true;
                        rt
                    })
                };
                if let Some(rt) = relaunched {
                    st.runtimes.insert(h, rt);
                }
                let mogs: Vec<String> = st
                    .sessions
                    .iter()
                    .filter(|(_, s)| s.harness == h && s.state != SessionState::Failed)
                    .map(|(m, _)| m.clone())
                    .collect();
                for mog_id in mogs {
                    // Without a relaunched runtime (no second restart) every session fails.
                    let mut s = st.sessions.remove(&mog_id).expect("listed above");
                    self.restart_or_fail(st, &mog_id, &mut s, out);
                    st.sessions.insert(mog_id, s);
                }
                Some(old.process)
            }
        }
    }

    /// Remove the runtime of `h` if no live session uses it. The caller stops it after it
    /// releases the lock.
    fn take_idle_runtime(st: &mut State, h: Harness) -> Option<Arc<dyn RuntimeProcess>> {
        let in_use = st
            .sessions
            .values()
            .any(|s| s.harness == h && s.state != SessionState::Failed);
        if in_use {
            return None;
        }
        st.runtimes.remove(&h).map(|rt| rt.process)
    }
}

impl AgentHost for SharedAgentHost {
    fn start_session(&self, mog_id: &str, harness: Harness) -> Result<String, AgentError> {
        self.start_session_with_model(mog_id, harness, None)
    }

    fn stop_session(&self, mog_id: &str) -> Result<(), AgentError> {
        let (close, idle) = {
            let mut st = self.lock();
            let s = st.sessions.remove(mog_id).ok_or(AgentError::UnknownMog)?;
            self.deps.proxy.unregister(&s.secret);
            let close = (s.state != SessionState::Failed)
                .then(|| st.runtimes.get(&s.harness).map(|rt| rt.process.clone()))
                .flatten()
                .map(|p| (p, s.id.clone()));
            (close, Self::take_idle_runtime(&mut st, s.harness))
        };
        if let Some((p, id)) = close {
            let _ = p.send_frame(&json!({ "type": "close_session", "sessionId": id }));
        }
        if let Some(p) = idle {
            p.stop();
        }
        Ok(())
    }

    fn send(&self, mog_id: &str, turn: &Value) -> Result<(), AgentError> {
        let (process, id) = {
            let st = self.lock();
            let s = st.sessions.get(mog_id).ok_or(AgentError::UnknownMog)?;
            if s.state == SessionState::Failed {
                return Err(AgentError::Unavailable);
            }
            let rt = st.runtimes.get(&s.harness).ok_or(AgentError::Unavailable)?;
            (rt.process.clone(), s.id.clone())
        };
        // Outside the lock: a large or slow turn never stops the other MOGs.
        process.send_frame(&json!({ "type": "turn", "sessionId": id, "turn": turn }))
    }
}
