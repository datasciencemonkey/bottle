//! The session host: one live `genie app-server` per user (plan Amendment 2, A3 and A5).
//!
//! One actor thread owns all state. Commands from IPC, lines from the server, the network watch
//! and the timer all arrive as messages, so no lock order can break and every rule has one place.
//! Nothing here logs or keeps a question, an answer, a JSONL line, stderr text or a token: the
//! question goes only into the prompt, and answers reach the guard's own records, not this file.

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::appserver::{
    self, classify_line, parse_line, read_capped_line, Hint, Incoming, LineRead, ServerProc,
    Spawner, StderrWatcher, MAX_LINE_BYTES,
};
use super::deadlines::{self, TurnDue};
use super::genie_bin::BinInfo;
use super::genie_home::{self, HomeSpec, ANSWER_SCHEMA};
use super::install::InstallError;
use super::prompt::{self, PromptInput};
use super::queue::{RateCap, WaitQueue};
use super::spawn;
use crate::agent_run::pin::GENIE_PINNED;
use crate::agent_run::seam::{
    AgentEvent, CatalogEntry, NetWatch, RunEvents, RunSecret, SessionGuard, SessionSpec, StopWhy,
    TurnSpec,
};
use crate::clock::{MonotonicClock, WallClock};
use crate::error::{CoreError, CoreResult};
use crate::ipc::{
    AgentAskMode, AgentSessionEndArgs, AgentTurnCallArgs, AgentTurnResultArgs, AgentTurnStopArgs,
    AgentTurnSubmitArgs, GenieCodeInstallArgs, GenieCodeStatusArgs,
};
use crate::types::{
    AgentAnswerKind, AgentProfileState, AgentSessionEndReason, AgentSessionEvent,
    AgentSessionPhase, AgentSessionState, AgentSource, AgentTurnAccepted, AgentTurnCall,
    AgentTurnEvent, AgentTurnOutcome, AgentTurnResult, AgentUsage, AgentWorkspace, GenieCallBudget,
    GenieCodeBinary, GenieCodeStatus,
};
use crate::wipe::AgentWipeHook;

use super::approval::Approvals;

/// The most characters of the model's `answer` that Bottle keeps.
pub const ANSWER_MAX_CHARS: usize = 1200;
/// The most bytes of the final agent message.
pub const FINAL_MAX_BYTES: usize = 32 * 1024;
/// The most answers that stay in memory.
const RESULTS_MAX: usize = 64;
/// How long a caller waits for a session end that interrupts a turn.
const END_WAIT: Duration = Duration::from_secs(15);

// ---------- the seams to the app ----------

/// The settings that a session snapshots at its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub catalog: Vec<CatalogEntry>,
    /// D1 "Combine across workspaces".
    pub cross_workspace: bool,
    /// D2 "Genie calls per question": 5, 10 or 20.
    pub budget: u8,
    /// The profile that the agent runs on. `None`: no home yet.
    pub agent_profile: Option<String>,
    /// Its workspace host.
    pub agent_host: Option<String>,
}

/// The workspace store as the session host sees it.
pub trait WorkspaceView: Send + Sync {
    fn snapshot(&self) -> CoreResult<Snapshot>;
}

/// Why the agent profile cannot run a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreflightFail {
    NotU2m,
    SignedOut,
    Offline,
    NotEligible,
    Missing,
}

/// The rest of the host: R5, R6, T7 and the environment.
pub trait AgentEnv: Send + Sync {
    /// The absolute path of the R5-vetted Databricks CLI.
    fn cli_path(&self) -> Option<PathBuf>;
    /// One environment variable of Bottle.
    fn var(&self, name: &str) -> Option<String>;
    /// Probe R6 for the agent profile (10 s cap, no token kept). Blocks.
    fn preflight(&self, profile: &str) -> Result<(), PreflightFail>;
    /// The cheap state of the profile, for the status.
    fn profile_state(&self, profile: &str) -> AgentProfileState;
}

/// What the shim needs to find its session.
#[derive(Clone, PartialEq, Eq)]
pub struct ShimLaunch {
    pub command: PathBuf,
    pub args: Vec<String>,
    /// The variables of the shim, with values. They reach the server only through its
    /// environment. `config.toml` gets the names.
    pub env: Vec<(String, String)>,
}

impl std::fmt::Debug for ShimLaunch {
    /// Shows the names of the variables, never a value (one of them is the shim secret).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShimLaunch")
            .field("command", &self.command)
            .field("args", &self.args)
            .field(
                "env",
                &self.env.iter().map(|(k, _)| k.as_str()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

pub trait ShimProvider: Send + Sync {
    fn launch(&self, session_id: &str, secret: &RunSecret) -> ShimLaunch;
}

/// The binary check.
pub trait BinaryCheck: Send + Sync {
    fn check(&self, recheck: bool) -> BinInfo;
}

impl BinaryCheck for super::genie_bin::GenieBin {
    fn check(&self, recheck: bool) -> BinInfo {
        super::genie_bin::GenieBin::check(self, recheck)
    }
}

/// The install of the pinned binary.
pub trait Installer: Send + Sync {
    fn install(&self) -> Result<(), InstallError>;
}

/// The seams of the host.
pub struct SessionDeps {
    pub guard: Arc<dyn SessionGuard>,
    pub net: Arc<dyn NetWatch>,
    pub events: Arc<dyn RunEvents>,
    pub clock: Arc<dyn MonotonicClock>,
    pub wall: Arc<dyn WallClock>,
    pub spawner: Arc<dyn Spawner>,
    pub workspaces: Arc<dyn WorkspaceView>,
    pub approvals: Arc<dyn Approvals>,
    pub env: Arc<dyn AgentEnv>,
    pub shim: Arc<dyn ShimProvider>,
    pub binary: Arc<dyn BinaryCheck>,
    pub installer: Arc<dyn Installer>,
}

/// Where the host keeps things.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// GENIE_HOME.
    pub home: PathBuf,
    /// The folder for the temp cwd of the thread.
    pub tmp_root: PathBuf,
    /// The pid file of the server, outside GENIE_HOME.
    pub pid_file: PathBuf,
    /// How often the actor wakes up on its own.
    pub tick: Duration,
}

// ---------- messages ----------

type Reply<T> = Sender<T>;

struct SubmitReq {
    question: String,
    mode: AgentAskMode,
}

struct Peek {
    has_session: bool,
}

enum Msg {
    Submit(SubmitReq, Reply<Result<AgentTurnAccepted, &'static str>>),
    Stop(String, Reply<Result<(), &'static str>>),
    End(AgentSessionEndReason, Option<Reply<()>>),
    State(Reply<AgentSessionState>),
    Peek(Reply<Peek>),
    Result(String, Reply<Option<AgentTurnResult>>),
    Server(u64, Incoming),
    Oversize(u64),
    Hint(u64, Hint),
    Eof(u64),
    Net(bool),
    /// The guard saw the third refusal of a turn.
    Refusals(String),
    Tick(Option<Reply<()>>),
    Shutdown(Reply<()>),
}

// ---------- state ----------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Fingerprint {
    profile: Option<String>,
    d1: bool,
    catalog: Vec<(String, String, bool)>,
}

impl Fingerprint {
    fn of(s: &Snapshot) -> Self {
        Self {
            profile: s.agent_profile.clone(),
            d1: s.cross_workspace,
            catalog: s
                .catalog
                .iter()
                .map(|c| (c.alias.clone(), c.profile.clone(), c.is_agent_workspace))
                .collect(),
        }
    }

    fn drift(&self, now: &Fingerprint) -> Option<AgentSessionEndReason> {
        if self.profile != now.profile {
            Some(AgentSessionEndReason::ProfileChanged)
        } else if self.d1 != now.d1 {
            Some(AgentSessionEndReason::D1Changed)
        } else if self.catalog != now.catalog {
            Some(AgentSessionEndReason::CatalogChanged)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initializing,
    ThreadStarting,
    Ready,
}

#[derive(Debug, Clone)]
enum Pending {
    Initialize,
    ThreadStart,
    TurnStart,
    Interrupt,
}

struct Session {
    id: String,
    gen: u64,
    proc: Box<dyn ServerProc>,
    stdin: Box<dyn Write + Send>,
    cwd: PathBuf,
    phase: Phase,
    thread_id: Option<String>,
    next_rpc: u64,
    pending: HashMap<u64, Pending>,
    fp: Fingerprint,
    catalog: Vec<CatalogEntry>,
    cross_workspace: bool,
    started_wall_ms: i64,
    turns: u32,
    tokens: u64,
}

struct QTurn {
    id: String,
    question: String,
    mode: AgentAskMode,
    /// A new question: it must not run on a tainted session.
    fresh: bool,
}

#[derive(Default)]
struct Usage {
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
}

struct Live {
    turn: QTurn,
    began: bool,
    sent: bool,
    started_at: Duration,
    slow_sent: bool,
    server_turn_id: Option<String>,
    started_seen: bool,
    stop: Option<StopWhy>,
    interrupt_at: Option<Duration>,
    interrupt_wanted: bool,
    hint: Option<Hint>,
    /// The guard stopped the turn after 3 refused tool calls.
    refusals: bool,
    final_text: Option<String>,
    usage: Usage,
}

impl Live {
    fn new(turn: QTurn, now: Duration) -> Self {
        Self {
            turn,
            began: false,
            sent: false,
            started_at: now,
            slow_sent: false,
            server_turn_id: None,
            started_seen: false,
            stop: None,
            interrupt_at: None,
            interrupt_wanted: false,
            hint: None,
            refusals: false,
            final_text: None,
            usage: Usage::default(),
        }
    }
}

struct Ending {
    reason: AgentSessionEndReason,
    deadline: Duration,
    waiters: Vec<Reply<()>>,
}

/// State that the host handle and the actor share.
#[derive(Default)]
struct Shared {
    not_eligible: Mutex<HashSet<String>>,
    installing: AtomicBool,
}

struct Engine {
    d: SessionDeps,
    cfg: SessionConfig,
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    session: Option<Session>,
    live: Option<Live>,
    waiting: WaitQueue<QTurn>,
    ending: Option<Ending>,
    pending_end: Option<AgentSessionEndReason>,
    results: HashMap<String, AgentTurnResult>,
    result_order: VecDeque<String>,
    results_clear_at: Option<Duration>,
    rate: RateCap,
    crash_streak: u32,
    last_activity: Duration,
    last_settings_check: Duration,
    next_gen: u64,
}

fn id32() -> String {
    let mut b = [0u8; 16];
    // Without OS random bytes an id is all zero: the guard refuses a repeated one.
    let _ = getrandom::fill(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Engine {
    fn now(&self) -> Duration {
        self.d.clock.now()
    }

    fn emit_turn(&self, e: AgentTurnEvent) {
        self.d.events.emit(AgentEvent::Turn(e));
    }

    fn emit_session(&self, e: AgentSessionEvent) {
        self.d.events.emit(AgentEvent::Session(e));
    }

    fn run(mut self, rx: mpsc::Receiver<Msg>) {
        loop {
            match rx.recv_timeout(self.cfg.tick) {
                Ok(Msg::Shutdown(done)) => {
                    self.begin_end(AgentSessionEndReason::Quit, None);
                    // A quit does not wait for a graceful turn end.
                    if self.session.is_some() {
                        self.end_now(AgentSessionEndReason::Quit, false);
                    }
                    let _ = done.send(());
                    return;
                }
                Ok(m) => self.handle(m),
                Err(RecvTimeoutError::Timeout) => self.tick(),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn handle(&mut self, m: Msg) {
        match m {
            Msg::Submit(req, r) => {
                let _ = r.send(self.submit(req));
            }
            Msg::Stop(id, r) => {
                let _ = r.send(self.stop(&id));
            }
            Msg::End(reason, w) => self.begin_end(reason, w),
            Msg::State(r) => {
                let _ = r.send(self.state());
            }
            Msg::Peek(r) => {
                let _ = r.send(Peek {
                    has_session: self.session.is_some(),
                });
            }
            Msg::Result(id, r) => {
                let _ = r.send(self.results.get(&id).cloned());
            }
            Msg::Server(gen, inc) => {
                if self.is_current(gen) {
                    self.on_incoming(inc);
                }
            }
            Msg::Oversize(gen) => {
                if self.is_current(gen) {
                    self.crash();
                }
            }
            Msg::Hint(gen, h) => {
                if self.is_current(gen) {
                    self.apply_hint(h);
                }
            }
            Msg::Eof(gen) => {
                if self.is_current(gen) {
                    self.crash();
                }
            }
            Msg::Net(online) => {
                if !online {
                    self.on_offline();
                }
            }
            Msg::Refusals(turn_id) => {
                if self.live.as_ref().is_some_and(|l| l.turn.id == turn_id) {
                    if let Some(l) = self.live.as_mut() {
                        l.refusals = true;
                    }
                    self.interrupt_live(StopWhy::Failed);
                    self.pump();
                }
            }
            Msg::Tick(done) => {
                self.tick();
                if let Some(d) = done {
                    let _ = d.send(());
                }
            }
            Msg::Shutdown(_) => {}
        }
    }

    fn is_current(&self, gen: u64) -> bool {
        self.session.as_ref().is_some_and(|s| s.gen == gen)
    }

    // ----- state view -----

    fn state(&self) -> AgentSessionState {
        let profile = self
            .d
            .workspaces
            .snapshot()
            .ok()
            .and_then(|s| s.agent_profile);
        match &self.session {
            None => AgentSessionState {
                state: AgentSessionPhase::None,
                session_id: None,
                started_at: None,
                agent_profile: profile,
                turns: 0,
                queued: 0,
                tokens: 0,
            },
            Some(s) => {
                let phase = if self.ending.is_some() {
                    AgentSessionPhase::Ending
                } else if s.phase != Phase::Ready {
                    AgentSessionPhase::Starting
                } else if self.live.is_some() {
                    AgentSessionPhase::Busy
                } else {
                    AgentSessionPhase::Idle
                };
                AgentSessionState {
                    state: phase,
                    session_id: Some(s.id.clone()),
                    started_at: u64::try_from(s.started_wall_ms).ok(),
                    agent_profile: s.fp.profile.clone(),
                    turns: s.turns,
                    queued: u32::try_from(self.waiting.len()).unwrap_or(u32::MAX),
                    tokens: s.tokens,
                }
            }
        }
    }

    // ----- submit, stop -----

    fn submit(&mut self, req: SubmitReq) -> Result<AgentTurnAccepted, &'static str> {
        let now = self.now();
        if self.ending.is_some() {
            return Err("agent_session_unavailable");
        }
        if req.mode == AgentAskMode::FollowUp && self.session.is_none() {
            return Err("agent_session_unavailable");
        }
        if self.live.is_some() && self.waiting.len() >= super::queue::MAX_WAITING {
            return Err("agent_queue_full");
        }
        if !self.rate.allow(now) {
            return Err("agent_rate_limited");
        }
        let snap = self
            .d
            .workspaces
            .snapshot()
            .map_err(|_| "agent_session_unavailable")?;
        let budget =
            GenieCallBudget::try_from(snap.budget).map_err(|_| "agent_session_unavailable")?;
        let turn = QTurn {
            id: id32(),
            question: req.question,
            mode: req.mode,
            fresh: req.mode != AgentAskMode::FollowUp,
        };
        let turn_id = turn.id.clone();
        let had_session = self.session.is_some();
        let mut new_session = !had_session;
        let position;
        if self.live.is_some() || self.ending.is_some() {
            position = self
                .waiting
                .push(turn_id.clone(), turn)
                .ok_or("agent_queue_full")?;
            self.emit_turn(AgentTurnEvent::Queued {
                turn_id: turn_id.clone(),
                position,
            });
        } else {
            // Nothing runs: this turn is the live one. A tainted or drifted session first ends.
            if let Some(s) = &self.session {
                let tainted = turn.fresh && self.d.guard.tainted(&s.id);
                let drift = s.fp.drift(&Fingerprint::of(&snap));
                if let Some(r) = drift {
                    self.end_session_core(r);
                    new_session = true;
                } else if tainted {
                    self.end_session_core(AgentSessionEndReason::Tainted);
                    new_session = true;
                }
            }
            self.ensure_session(&snap)?;
            self.live = Some(Live::new(turn, now));
            position = 0;
            self.pump();
        }
        let sid = self
            .session
            .as_ref()
            .map(|s| s.id.clone())
            .ok_or("agent_session_unavailable")?;
        Ok(AgentTurnAccepted {
            turn_id,
            session_id: sid,
            position,
            new_session,
            budget,
            cross_workspace: self.session.as_ref().is_some_and(|s| s.cross_workspace),
            catalog: self
                .session
                .as_ref()
                .map(|s| {
                    s.catalog
                        .iter()
                        .map(|c| AgentWorkspace {
                            workspace_id: c.alias.clone(),
                            profile: c.profile.clone(),
                            name: c.name.clone(),
                            is_home: c.is_home,
                            is_agent_workspace: c.is_agent_workspace,
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }

    fn stop(&mut self, turn_id: &str) -> Result<(), &'static str> {
        if self.waiting.contains(turn_id) {
            self.waiting.remove(turn_id);
            self.emit_turn(AgentTurnEvent::Ended {
                turn_id: turn_id.to_owned(),
                outcome: AgentTurnOutcome::Removed,
                calls: 0,
                tokens: None,
            });
            return Ok(());
        }
        if self.live.as_ref().is_some_and(|l| l.turn.id == turn_id) {
            self.interrupt_live(StopWhy::User);
            self.pump();
            return Ok(());
        }
        Err("agent_turn_unknown")
    }

    // ----- the session -----

    /// Make sure that a session exists. A new one is spawned and the handshake starts.
    fn ensure_session(&mut self, snap: &Snapshot) -> Result<(), &'static str> {
        if self.session.is_some() {
            return Ok(());
        }
        self.results.clear();
        self.result_order.clear();
        self.results_clear_at = None;
        let (Some(profile), Some(host)) = (snap.agent_profile.clone(), snap.agent_host.clone())
        else {
            return Err("agent_profile_unsupported");
        };
        let bin = self.d.binary.check(false);
        let genie = match (bin.state, bin.path) {
            (GenieCodeBinary::Ready, Some(p)) => p,
            (GenieCodeBinary::WrongVersion, _) => return Err("genie_code_version"),
            _ => return Err("genie_code_missing"),
        };
        let cli = self.d.env.cli_path().ok_or("agent_failed")?;
        let session_id = id32();
        let spec = SessionSpec {
            session_id: session_id.clone(),
            catalog: snap.catalog.clone(),
            cross_workspace: snap.cross_workspace,
        };
        let secret = self
            .d
            .guard
            .open_session(spec)
            .map_err(|_| "agent_session_unavailable")?;
        let shim = self.d.shim.launch(&session_id, &secret);
        drop(secret);
        let fail = |this: &Self, code: &'static str| {
            this.d.guard.close_session(&session_id, true);
            let _ = genie_home::delete_home(&this.cfg.home);
            code
        };
        let home_spec = HomeSpec {
            host,
            cli_path: cli.clone(),
            profile,
            shim_command: shim.command.clone(),
            shim_args: shim.args.clone(),
            shim_env_names: shim.env.iter().map(|(k, _)| k.clone()).collect(),
        };
        if genie_home::write_home(&self.cfg.home, &home_spec).is_err() {
            return Err(fail(self, "agent_failed"));
        }
        let Ok(cwd) = genie_home::make_cwd(&self.cfg.tmp_root) else {
            return Err(fail(self, "agent_failed"));
        };
        let env_var = |n: &str| self.d.env.var(n);
        let env = genie_home::server_env(&env_var, &self.cfg.home, cli.parent(), &shim.env);
        let spawn_spec = genie_home::spawn_spec(&genie, env, Some(cwd.clone()));
        debug_assert!(!appserver::argv_forbidden(&spawn_spec.args));
        let mut proc = match self.d.spawner.spawn(&spawn_spec) {
            Ok(p) => p,
            Err(_) => {
                let _ = std::fs::remove_dir_all(&cwd);
                return Err(fail(self, "agent_failed"));
            }
        };
        let Some(io) = proc.take_io() else {
            proc.terminate();
            let _ = std::fs::remove_dir_all(&cwd);
            return Err(fail(self, "agent_failed"));
        };
        let pid = proc.pid();
        let _ = spawn::write_pid(&self.cfg.pid_file, pid);
        self.d.guard.bind_pid(&session_id, pid);
        self.next_gen += 1;
        let gen = self.next_gen;
        spawn_readers(gen, io.stdout, io.stderr, self.tx.clone());
        let now_wall = self.d.wall.now_ms();
        self.session = Some(Session {
            id: session_id,
            gen,
            proc,
            stdin: io.stdin,
            cwd,
            phase: Phase::Initializing,
            thread_id: None,
            next_rpc: 0,
            pending: HashMap::new(),
            fp: Fingerprint::of(snap),
            catalog: snap.catalog.clone(),
            cross_workspace: snap.cross_workspace && crate::agent_run::session::D1_SUPPORTED,
            started_wall_ms: now_wall,
            turns: 0,
            tokens: 0,
        });
        self.last_activity = self.now();
        self.last_settings_check = self.now();
        let ok = self.rpc(
            "initialize",
            json!({ "clientInfo": { "name": "bottle", "version": env!("CARGO_PKG_VERSION") } }),
            Pending::Initialize,
        );
        if !ok {
            self.end_session_core(AgentSessionEndReason::Crashed);
            return Err("agent_failed");
        }
        Ok(())
    }

    /// Send a request. False if the pipe is broken.
    fn rpc(&mut self, method: &str, params: Value, kind: Pending) -> bool {
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        s.next_rpc += 1;
        let id = s.next_rpc;
        s.pending.insert(id, kind);
        let bytes = appserver::request(id, method, params);
        s.stdin
            .write_all(&bytes)
            .and_then(|()| s.stdin.flush())
            .is_ok()
    }

    fn notify(&mut self, method: &str, params: Option<Value>) -> bool {
        let Some(s) = self.session.as_mut() else {
            return false;
        };
        let bytes = appserver::notification(method, params);
        s.stdin
            .write_all(&bytes)
            .and_then(|()| s.stdin.flush())
            .is_ok()
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        if let Some(s) = self.session.as_mut() {
            let _ = s.stdin.write_all(bytes).and_then(|()| s.stdin.flush());
        }
    }

    // ----- messages from the server -----

    fn on_incoming(&mut self, inc: Incoming) {
        match inc {
            Incoming::ServerRequest { id, .. } => {
                // AC-2a.23: refuse, then interrupt, then end the session.
                self.write_raw(&appserver::refusal(&id));
                self.violation();
            }
            Incoming::Response { id, ok, result } => self.on_response(id, ok, &result),
            Incoming::Notification { method, params } => self.on_notification(&method, &params),
        }
    }

    fn on_response(&mut self, id: u64, ok: bool, result: &Value) {
        let Some(kind) = self.session.as_mut().and_then(|s| s.pending.remove(&id)) else {
            return;
        };
        match kind {
            Pending::Initialize => {
                if !ok {
                    return self.crash();
                }
                if let Some(s) = self.session.as_mut() {
                    s.phase = Phase::ThreadStarting;
                }
                let cwd = self
                    .session
                    .as_ref()
                    .map(|s| s.cwd.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let sent = self.notify("initialized", None)
                    && self.rpc(
                        "thread/start",
                        json!({ "cwd": cwd, "ephemeral": true, "approvalPolicy": "never" }),
                        Pending::ThreadStart,
                    );
                if !sent {
                    self.crash();
                }
            }
            Pending::ThreadStart => {
                if !ok {
                    return self.crash();
                }
                if !thread_start_ok(result) {
                    return self.violation();
                }
                let tid = result["thread"]["id"].as_str().map(str::to_owned);
                let Some(tid) = tid else {
                    return self.violation();
                };
                let sid = {
                    let Some(s) = self.session.as_mut() else {
                        return;
                    };
                    s.thread_id = Some(tid);
                    s.phase = Phase::Ready;
                    s.id.clone()
                };
                self.emit_session(AgentSessionEvent::Started { session_id: sid });
                self.pump();
            }
            Pending::TurnStart => {
                if !ok {
                    if self.live.is_some() {
                        self.fail_live(StopWhy::Failed);
                    }
                    return;
                }
                let want_interrupt = {
                    let Some(l) = self.live.as_mut() else { return };
                    if let Some(t) = result["turn"]["id"].as_str() {
                        l.server_turn_id = Some(t.to_owned());
                    }
                    l.interrupt_wanted
                };
                if want_interrupt {
                    self.send_interrupt();
                }
            }
            Pending::Interrupt => {}
        }
    }

    fn on_notification(&mut self, method: &str, params: &Value) {
        // Tripwires on every item and on the remote-control status.
        if method == "remoteControl/status/changed" {
            if params["status"].as_str() != Some("disabled") {
                self.violation();
            }
            return;
        }
        if method.starts_with("item/") {
            if let Some(item) = params.get("item") {
                if item_violation(item) {
                    return self.violation();
                }
            }
        }
        match method {
            "turn/started" => {
                if let Some(l) = self.live.as_mut() {
                    l.started_seen = true;
                    if l.server_turn_id.is_none() {
                        l.server_turn_id = params["turn"]["id"].as_str().map(str::to_owned);
                    }
                }
                if let Some(l) = &self.live {
                    let id = l.turn.id.clone();
                    self.emit_turn(AgentTurnEvent::Thinking { turn_id: id });
                }
            }
            "item/completed" => {
                let item = &params["item"];
                if matches!(
                    item["type"].as_str(),
                    Some("agentMessage" | "agent_message")
                ) {
                    if let (Some(l), Some(t)) = (self.live.as_mut(), item["text"].as_str()) {
                        if !t.is_empty() {
                            l.final_text = Some(t.to_owned());
                        }
                    }
                }
                if item["type"].as_str() == Some("error") {
                    let msg = item["message"].as_str().or(item["text"].as_str());
                    if let Some(h) = msg.and_then(classify_line) {
                        self.apply_hint(h);
                    }
                }
            }
            "error" | "warning" => {
                let msg = params["error"]["message"]
                    .as_str()
                    .or(params["message"].as_str());
                if let Some(h) = msg.and_then(classify_line) {
                    self.apply_hint(h);
                }
            }
            "mcpServer/startupStatus/updated" => {
                if params["status"].as_str() == Some("failed") {
                    self.apply_hint(Hint::McpFailed);
                }
            }
            "thread/tokenUsage/updated" => self.on_usage(params),
            "turn/completed" => self.on_turn_completed(params),
            _ => {}
        }
    }

    /// AC-2a.29: the tokens of a turn are the sum of `last.inputTokens + last.outputTokens` over
    /// the updates between `turn/started` and `turn/completed`.
    fn on_usage(&mut self, params: &Value) {
        let Some(l) = self.live.as_mut() else { return };
        if !l.started_seen {
            return;
        }
        if let (Some(want), Some(got)) = (&l.server_turn_id, params["turnId"].as_str()) {
            if want != got {
                return;
            }
        }
        let last = &params["tokenUsage"]["last"];
        let n = |k: &str| last[k].as_u64().unwrap_or(0);
        l.usage.input += n("inputTokens");
        l.usage.cached += n("cachedInputTokens");
        l.usage.output += n("outputTokens");
        l.usage.reasoning += n("reasoningOutputTokens");
    }

    fn on_turn_completed(&mut self, params: &Value) {
        let status = params["turn"]["status"].as_str().unwrap_or("");
        let Some(l) = self.live.as_mut() else { return };
        if !l.sent {
            return;
        }
        if l.final_text.is_none() {
            // The summary of the turn also holds its items.
            if let Some(items) = params["turn"]["items"].as_array() {
                for it in items.iter().rev() {
                    if it["type"].as_str() == Some("agentMessage") {
                        l.final_text = it["text"].as_str().map(str::to_owned);
                        break;
                    }
                }
            }
        }
        let outcome = match (l.stop, status) {
            (Some(_), _) => outcome_of_live(l),
            (None, "completed") => AgentTurnOutcome::Answered,
            (None, _) => outcome_of_stop(StopWhy::Failed, l.hint),
        };
        self.finish(outcome);
    }

    // ----- turn start and stop -----

    /// Start the live turn if the session is ready.
    fn pump(&mut self) {
        let Some(sess) = self.session.as_ref() else {
            return;
        };
        if sess.phase != Phase::Ready {
            return;
        }
        let Some(l) = self.live.as_ref() else { return };
        if l.sent {
            return;
        }
        if l.stop.is_some() {
            let o = outcome_of_live(l);
            return self.finish(o);
        }
        if !self.d.net.online() {
            if let Some(l) = self.live.as_mut() {
                l.stop = Some(StopWhy::Offline);
            }
            return self.finish(AgentTurnOutcome::Offline);
        }
        let snap = self.d.workspaces.snapshot().ok();
        let (sid, thread_id, catalog, cross) = {
            let s = self.session.as_ref().expect("checked above");
            (
                s.id.clone(),
                s.thread_id.clone().unwrap_or_default(),
                s.catalog.clone(),
                s.cross_workspace,
            )
        };
        let budget = snap.as_ref().map_or(10, |s| s.budget);
        let (turn_id, question, mode) = {
            let l = self.live.as_ref().expect("checked above");
            (l.turn.id.clone(), l.turn.question.clone(), l.turn.mode)
        };
        if self
            .d
            .guard
            .begin_turn(
                &sid,
                TurnSpec {
                    turn_id: turn_id.clone(),
                    budget,
                },
            )
            .is_err()
        {
            return self.finish(AgentTurnOutcome::Failed);
        }
        let agent_alias = catalog
            .iter()
            .find(|c| c.is_agent_workspace)
            .map(|c| c.alias.clone());
        let boundary = prompt::new_boundary();
        let text = prompt::build(&PromptInput {
            catalog: &catalog,
            budget,
            cross_workspace: cross,
            agent_alias: agent_alias.as_deref(),
            skip_home: mode == AgentAskMode::AskOthers,
            question: &question,
            boundary: &boundary,
        });
        let schema: Value = serde_json::from_str(ANSWER_SCHEMA).unwrap_or(Value::Null);
        let now = self.now();
        if let Some(l) = self.live.as_mut() {
            l.began = true;
            l.sent = true;
            l.started_at = now;
        }
        let ok = self.rpc(
            "turn/start",
            json!({
                "threadId": thread_id,
                "input": [{ "type": "text", "text": text }],
                "outputSchema": schema,
            }),
            Pending::TurnStart,
        );
        self.emit_turn(AgentTurnEvent::Started { turn_id });
        if !ok {
            self.crash();
        }
    }

    /// Stop the live turn for `why`: tell the guard, and send `turn/interrupt` if it runs.
    fn interrupt_live(&mut self, why: StopWhy) {
        let now = self.now();
        let Some(l) = self.live.as_mut() else { return };
        let first = l.stop.is_none();
        if first {
            l.stop = Some(why);
        }
        let (began, turn_id, sent, already) = (l.began, l.turn.id.clone(), l.sent, l.interrupt_at);
        if first && began {
            self.d.guard.stop_turn(&turn_id, why);
        }
        if sent && already.is_none() {
            if let Some(l) = self.live.as_mut() {
                l.interrupt_at = Some(now);
            }
            self.send_interrupt();
        }
    }

    fn send_interrupt(&mut self) {
        let (thread, turn) = {
            let (Some(s), Some(l)) = (self.session.as_ref(), self.live.as_mut()) else {
                return;
            };
            match &l.server_turn_id {
                Some(t) => (s.thread_id.clone().unwrap_or_default(), t.clone()),
                None => {
                    // The id comes with the answer to `turn/start`.
                    l.interrupt_wanted = true;
                    return;
                }
            }
        };
        let _ = self.rpc(
            "turn/interrupt",
            json!({ "threadId": thread, "turnId": turn }),
            Pending::Interrupt,
        );
    }

    /// The live turn failed on our side (a refused `turn/start`).
    fn fail_live(&mut self, why: StopWhy) {
        if let Some(l) = self.live.as_mut() {
            if l.stop.is_none() {
                l.stop = Some(why);
            }
            let (began, id) = (l.began, l.turn.id.clone());
            if began {
                self.d.guard.stop_turn(&id, why);
            }
        }
        let o = self
            .live
            .as_ref()
            .map_or(AgentTurnOutcome::Failed, |l| outcome_of_stop(why, l.hint));
        self.finish(o);
    }

    fn on_offline(&mut self) {
        if self.live.is_some() {
            self.interrupt_live(StopWhy::Offline);
            self.pump();
        }
    }

    fn apply_hint(&mut self, h: Hint) {
        if self.live.is_none() {
            return;
        }
        match h {
            Hint::Offline => {
                self.interrupt_live(StopWhy::Offline);
                self.pump();
            }
            Hint::SignedOut | Hint::NotEligible | Hint::McpFailed => {
                if let Some(l) = self.live.as_mut() {
                    l.hint.get_or_insert(h);
                }
                self.interrupt_live(StopWhy::Failed);
                self.pump();
            }
        }
    }

    // ----- ending a turn -----

    /// The live turn is over. Record it, then go on.
    fn finish(&mut self, outcome: AgentTurnOutcome) {
        let Some(live) = self.live.take() else { return };
        let answered = outcome == AgentTurnOutcome::Answered;
        if answered {
            self.crash_streak = 0;
        }
        self.record_end(live, outcome);
        match self.ending.as_ref().map(|e| e.reason) {
            Some(reason) => self.end_now(reason, false),
            None => self.advance(),
        }
    }

    /// Build the result, tell the guard, send the `ended` event.
    fn record_end(&mut self, live: Live, outcome: AgentTurnOutcome) {
        let Some(sid) = self.session.as_ref().map(|s| s.id.clone()) else {
            return;
        };
        let turn_id = live.turn.id.clone();
        let calls_all = self.d.guard.calls(&sid);
        let calls = calls_all.iter().filter(|c| c.turn_id == turn_id).count();
        if outcome == AgentTurnOutcome::Answered {
            let r = build_result(&live, &calls_all);
            self.store_result(turn_id.clone(), r);
        }
        if live.began {
            self.d.guard.end_turn(&turn_id);
        }
        let tokens = live.usage.input + live.usage.output;
        if let Some(s) = self.session.as_mut() {
            if live.sent {
                s.turns += 1;
            }
            s.tokens += tokens;
        }
        self.last_activity = self.now();
        self.emit_turn(AgentTurnEvent::Ended {
            turn_id,
            outcome,
            calls: u32::try_from(calls).unwrap_or(u32::MAX),
            tokens: (tokens > 0).then_some(tokens),
        });
        if outcome == AgentTurnOutcome::NotEligible {
            if let Some(p) = self.session.as_ref().and_then(|s| s.fp.profile.clone()) {
                lock(&self.shared.not_eligible).insert(p);
            }
        }
    }

    fn store_result(&mut self, id: String, r: AgentTurnResult) {
        self.results.insert(id.clone(), r);
        self.result_order.push_back(id);
        while self.result_order.len() > RESULTS_MAX {
            if let Some(old) = self.result_order.pop_front() {
                self.results.remove(&old);
            }
        }
    }

    /// Run the next waiting turn, after a settings change or a taint if one is due.
    fn advance(&mut self) {
        loop {
            if self.live.is_some() || self.ending.is_some() {
                return;
            }
            if self.session.is_some() {
                let due = self.pending_end.take().or_else(|| self.drift());
                if let Some(r) = due {
                    self.end_session_core(r);
                    continue;
                }
            } else {
                self.pending_end = None;
            }
            let Some(next) = self.waiting.pop() else {
                return;
            };
            if let Some(s) = &self.session {
                if next.fresh && self.d.guard.tainted(&s.id) {
                    self.end_session_core(AgentSessionEndReason::Tainted);
                }
            }
            let snap = match self.d.workspaces.snapshot() {
                Ok(s) => s,
                Err(_) => {
                    self.emit_ended_unrun(&next.id, AgentTurnOutcome::Failed);
                    continue;
                }
            };
            if self.ensure_session(&snap).is_err() {
                self.emit_ended_unrun(&next.id, AgentTurnOutcome::Failed);
                continue;
            }
            let now = self.now();
            self.live = Some(Live::new(next, now));
            self.pump();
        }
    }

    fn emit_ended_unrun(&self, turn_id: &str, outcome: AgentTurnOutcome) {
        self.emit_turn(AgentTurnEvent::Ended {
            turn_id: turn_id.to_owned(),
            outcome,
            calls: 0,
            tokens: None,
        });
    }

    fn drift(&self) -> Option<AgentSessionEndReason> {
        let s = self.session.as_ref()?;
        let now = self.d.workspaces.snapshot().ok()?;
        s.fp.drift(&Fingerprint::of(&now))
    }

    // ----- ending the session -----

    /// The one routine for all causes (A3.6).
    fn begin_end(&mut self, reason: AgentSessionEndReason, waiter: Option<Reply<()>>) {
        if self.session.is_none() {
            // Nothing runs, but GENIE_HOME must be gone (Wipe, quit).
            let _ = genie_home::delete_home(&self.cfg.home);
            genie_home::sweep_cwds(&self.cfg.tmp_root);
            if reason == AgentSessionEndReason::Wipe {
                // Earlier ended sessions keep a ledger for ten minutes: Wipe drops them all.
                self.d.guard.wipe_all();
                self.results.clear();
                self.result_order.clear();
            }
            for (id, _) in self.waiting.drain() {
                self.emit_ended_unrun(&id, AgentTurnOutcome::SessionEnded);
            }
            if let Some(w) = waiter {
                let _ = w.send(());
            }
            return;
        }
        if let Some(e) = self.ending.as_mut() {
            if let Some(w) = waiter {
                e.waiters.push(w);
            }
            return;
        }
        let running = self.live.as_ref().is_some_and(|l| l.sent);
        let waiters: Vec<Reply<()>> = waiter.into_iter().collect();
        if running {
            let deadline = self.now() + deadlines::INTERRUPT_GRACE;
            self.ending = Some(Ending {
                reason,
                deadline,
                waiters,
            });
            self.interrupt_live(StopWhy::SessionEnded);
        } else {
            self.ending = Some(Ending {
                reason,
                deadline: Duration::ZERO,
                waiters,
            });
            self.end_now(reason, false);
        }
    }

    /// The server broke a lockdown rule: no waiting, the session ends now.
    fn violation(&mut self) {
        if let Some(l) = self.live.as_mut() {
            let (began, id) = (l.began, l.turn.id.clone());
            l.stop = Some(StopWhy::Lockdown);
            if began {
                self.d.guard.stop_turn(&id, StopWhy::Lockdown);
            }
            if l.sent {
                // Best effort: the process dies right after.
                self.send_interrupt();
            }
        }
        self.end_now(AgentSessionEndReason::Lockdown, false);
    }

    /// The process is gone, or the pipe is broken.
    fn crash(&mut self) {
        if self.session.is_none() {
            return;
        }
        if self.ending.is_some() {
            let r = self
                .ending
                .as_ref()
                .map_or(AgentSessionEndReason::Crashed, |e| e.reason);
            return self.end_now(r, false);
        }
        self.crash_streak += 1;
        // AC-2a.30: the next waiting turn gets one new session. A second crash in a row sends
        // the waiting questions to the direct path.
        let carry = self.crash_streak == 1 && !self.waiting.is_empty();
        self.end_now(AgentSessionEndReason::Crashed, carry);
    }

    /// End the session now: the live turn, the process, GENIE_HOME, the ledger, the waiters.
    /// With `carry`, the waiting turns go on in a new session; otherwise they end.
    fn end_now(&mut self, reason: AgentSessionEndReason, carry: bool) {
        let ending = self.ending.take();
        if let Some(live) = self.live.take() {
            let o = live_outcome(&live, reason);
            if live.began {
                // Make sure that the open calls get the right treatment.
                let why = match reason {
                    AgentSessionEndReason::Crashed => StopWhy::Failed,
                    AgentSessionEndReason::Lockdown => StopWhy::Lockdown,
                    AgentSessionEndReason::HardTimeout => live.stop.unwrap_or(StopWhy::Timeout),
                    _ => StopWhy::SessionEnded,
                };
                if live.stop.is_none() {
                    self.d.guard.stop_turn(&live.turn.id, why);
                }
            }
            self.record_end(live, o);
        }
        self.end_session_core(reason);
        if carry {
            self.advance();
        } else {
            let outcome = if reason == AgentSessionEndReason::Crashed {
                AgentTurnOutcome::Failed
            } else {
                AgentTurnOutcome::SessionEnded
            };
            for (id, _) in self.waiting.drain() {
                self.emit_ended_unrun(&id, outcome);
            }
        }
        if let Some(e) = ending {
            for w in e.waiters {
                let _ = w.send(());
            }
        }
    }

    /// Kill the process, delete the files, close the ledger, emit `ended`. The turns are the
    /// caller's job.
    fn end_session_core(&mut self, reason: AgentSessionEndReason) {
        let Some(mut s) = self.session.take() else {
            return;
        };
        s.proc.terminate();
        drop(s.stdin);
        let _ = genie_home::delete_home(&self.cfg.home);
        let _ = std::fs::remove_dir_all(&s.cwd);
        spawn::remove_pid(&self.cfg.pid_file);
        let wipe = reason == AgentSessionEndReason::Wipe;
        self.d.guard.close_session(&s.id, wipe);
        if wipe {
            self.d.guard.wipe_all();
            self.results.clear();
            self.result_order.clear();
        } else {
            self.results_clear_at = Some(self.now() + deadlines::RESULT_KEEP);
        }
        self.pending_end = None;
        self.last_activity = self.now();
        self.emit_session(AgentSessionEvent::Ended {
            session_id: s.id,
            reason,
        });
    }

    // ----- the timer -----

    fn tick(&mut self) {
        let now = self.now();
        if let Some(at) = self.results_clear_at {
            if now >= at {
                self.results.clear();
                self.result_order.clear();
                self.results_clear_at = None;
            }
        }
        if let Some(e) = &self.ending {
            if now >= e.deadline && self.session.is_some() {
                let r = e.reason;
                return self.end_now(r, false);
            }
        }
        if self.session.is_none() {
            return;
        }
        // The live turn.
        let due = self.live.as_ref().filter(|l| l.sent).map(|l| {
            (
                deadlines::turn_due(
                    now.saturating_sub(l.started_at),
                    l.slow_sent,
                    l.interrupt_at.is_some(),
                ),
                l.interrupt_at,
                l.turn.id.clone(),
            )
        });
        if let Some((due, interrupted_at, id)) = due {
            match due {
                TurnDue::Slow => {
                    if let Some(l) = self.live.as_mut() {
                        l.slow_sent = true;
                    }
                    self.emit_turn(AgentTurnEvent::Slow { turn_id: id });
                }
                TurnDue::HardStop => self.interrupt_live(StopWhy::Timeout),
                TurnDue::Nothing => {}
            }
            // An interrupt that nobody answers: kill the process group.
            if let Some(at) = interrupted_at {
                if deadlines::grace_over(now.saturating_sub(at)) {
                    return self.end_now(AgentSessionEndReason::HardTimeout, false);
                }
            }
        }
        // The settings of the session.
        if now.saturating_sub(self.last_settings_check) >= deadlines::SETTINGS_POLL {
            self.last_settings_check = now;
            if self.pending_end.is_none() {
                self.pending_end = self.drift();
            }
        }
        if self.live.is_none() && self.ending.is_none() {
            if self.pending_end.is_some() {
                return self.advance();
            }
            if self.waiting.is_empty()
                && deadlines::idle_over(now.saturating_sub(self.last_activity))
            {
                self.end_now(AgentSessionEndReason::Idle, false);
            }
        }
    }
}

// ---------- helpers ----------

fn thread_start_ok(result: &Value) -> bool {
    let t = &result["thread"];
    let provider = result["modelProvider"]
        .as_str()
        .or(t["modelProvider"].as_str());
    let sandbox_ok = match result["sandbox"]["type"].as_str() {
        Some(s) => s == "readOnly",
        None => true,
    };
    t["ephemeral"].as_bool() == Some(true)
        && t["cliVersion"].as_str() == Some(GENIE_PINNED)
        && provider == Some("databricks")
        && result["approvalPolicy"]
            .as_str()
            .is_none_or(|p| p == "never")
        && sandbox_ok
}

/// AC-2a.13: an item that must never appear.
fn item_violation(item: &Value) -> bool {
    match item["type"].as_str() {
        Some(
            "commandExecution" | "command_execution" | "fileChange" | "file_change" | "webSearch"
            | "web_search",
        ) => true,
        Some("mcpToolCall" | "mcp_tool_call") => {
            let server = item["server"].as_str().unwrap_or("");
            let tool = item["tool"].as_str().unwrap_or("");
            let builtin = server == "codex"
                && matches!(
                    tool,
                    "list_mcp_resources" | "list_mcp_resource_templates" | "read_mcp_resource"
                );
            !(server == "bottle" || builtin)
        }
        _ => false,
    }
}

fn outcome_of_live(l: &Live) -> AgentTurnOutcome {
    match (l.refusals, l.stop) {
        (true, Some(StopWhy::Failed)) => AgentTurnOutcome::Refusals,
        (_, Some(why)) => outcome_of_stop(why, l.hint),
        (_, None) => AgentTurnOutcome::Failed,
    }
}

fn outcome_of_stop(why: StopWhy, hint: Option<Hint>) -> AgentTurnOutcome {
    match why {
        StopWhy::User => AgentTurnOutcome::Stopped,
        StopWhy::Timeout => AgentTurnOutcome::Timeout,
        StopWhy::Offline => AgentTurnOutcome::Offline,
        StopWhy::Lockdown => AgentTurnOutcome::Lockdown,
        StopWhy::SessionEnded => AgentTurnOutcome::SessionEnded,
        StopWhy::Failed => match hint {
            Some(Hint::SignedOut) => AgentTurnOutcome::SignedOut,
            Some(Hint::NotEligible) => AgentTurnOutcome::NotEligible,
            Some(Hint::Offline) => AgentTurnOutcome::Offline,
            _ => AgentTurnOutcome::Failed,
        },
    }
}

fn live_outcome(live: &Live, reason: AgentSessionEndReason) -> AgentTurnOutcome {
    if live.refusals && live.stop == Some(StopWhy::Failed) {
        return AgentTurnOutcome::Refusals;
    }
    if let Some(why) = live.stop {
        // A stop that was already chosen wins, except a graceful end: a crash is a failure.
        return match (why, reason) {
            (StopWhy::SessionEnded, AgentSessionEndReason::Crashed) => AgentTurnOutcome::Failed,
            (StopWhy::SessionEnded, AgentSessionEndReason::HardTimeout) => {
                AgentTurnOutcome::Timeout
            }
            _ => outcome_of_stop(why, live.hint),
        };
    }
    match reason {
        AgentSessionEndReason::Crashed => AgentTurnOutcome::Failed,
        AgentSessionEndReason::Lockdown => AgentTurnOutcome::Lockdown,
        AgentSessionEndReason::HardTimeout => AgentTurnOutcome::Timeout,
        _ => AgentTurnOutcome::SessionEnded,
    }
}

/// The result of an answered turn: the final message, checked against the guard's call log
/// (AC-2a.12 and AC-2a.28: any call of the session counts, not only this turn).
fn build_result(live: &Live, calls: &[crate::agent_run::seam::CallRecord]) -> AgentTurnResult {
    let usage = Some(AgentUsage {
        input_tokens: live.usage.input,
        cached_input_tokens: live.usage.cached,
        output_tokens: live.usage.output,
        reasoning_tokens: live.usage.reasoning,
    });
    let hidden = AgentTurnResult {
        answer: None,
        answer_kind: AgentAnswerKind::Hidden,
        sources: Vec::new(),
        usage,
    };
    let Some(text) = live
        .final_text
        .as_deref()
        .filter(|t| t.len() <= FINAL_MAX_BYTES)
    else {
        return hidden;
    };
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return hidden;
    };
    let (Some(answer), Some(sources)) = (v["answer"].as_str(), v["sources"].as_array()) else {
        return hidden;
    };
    let mut out = Vec::new();
    for s in sources {
        let (Some(ws), Some(cid)) = (s["workspace"].as_str(), s["callId"].as_str()) else {
            return hidden;
        };
        let ok = calls.iter().find(|c| {
            c.call_id == cid && c.alias == ws && c.state == crate::types::AgentCallState::Answered
        });
        let Some(c) = ok else { return hidden };
        out.push(AgentSource {
            workspace_id: ws.to_owned(),
            call_id: cid.to_owned(),
            turn_id: c.turn_id.clone(),
        });
    }
    AgentTurnResult {
        answer: Some(answer.chars().take(ANSWER_MAX_CHARS).collect()),
        answer_kind: AgentAnswerKind::Note,
        sources: out,
        usage,
    }
}

/// The two reader threads of one server: stdout lines and stderr hints.
fn spawn_readers(
    gen: u64,
    stdout: Box<dyn Read + Send>,
    mut stderr: Box<dyn Read + Send>,
    tx: Sender<Msg>,
) {
    let tx_out = tx.clone();
    thread::spawn(move || {
        let mut r = BufReader::new(stdout);
        let mut buf = Vec::new();
        loop {
            match read_capped_line(&mut r, &mut buf, MAX_LINE_BYTES) {
                Ok(LineRead::Line) => {
                    if let Some(inc) = parse_line(&buf) {
                        if tx_out.send(Msg::Server(gen, inc)).is_err() {
                            return;
                        }
                    }
                }
                Ok(LineRead::TooLong) => {
                    let _ = tx_out.send(Msg::Oversize(gen));
                    return;
                }
                Ok(LineRead::Eof) | Err(_) => {
                    let _ = tx_out.send(Msg::Eof(gen));
                    return;
                }
            }
        }
    });
    thread::spawn(move || {
        let mut w = StderrWatcher::default();
        let mut chunk = [0u8; 4096];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => return,
                Ok(n) => {
                    for h in w.feed(&chunk[..n]) {
                        if tx.send(Msg::Hint(gen, h)).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });
}

// ---------- the public host ----------

/// The managed agent component.
pub struct SessionHost {
    tx: Sender<Msg>,
    deps_view: Arc<dyn WorkspaceView>,
    approvals: Arc<dyn Approvals>,
    env: Arc<dyn AgentEnv>,
    binary: Arc<dyn BinaryCheck>,
    installer: Arc<dyn Installer>,
    guard: Arc<dyn SessionGuard>,
    shared: Arc<Shared>,
}

impl std::fmt::Debug for SessionHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHost").finish_non_exhaustive()
    }
}

impl SessionHost {
    /// Start the host and its actor thread. It starts no server: that waits for the first turn.
    pub fn new(deps: SessionDeps, cfg: SessionConfig) -> Arc<Self> {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Shared::default());
        let host = Arc::new(Self {
            tx: tx.clone(),
            deps_view: Arc::clone(&deps.workspaces),
            approvals: Arc::clone(&deps.approvals),
            env: Arc::clone(&deps.env),
            binary: Arc::clone(&deps.binary),
            installer: Arc::clone(&deps.installer),
            guard: Arc::clone(&deps.guard),
            shared: Arc::clone(&shared),
        });
        let net_tx = tx.clone();
        deps.net.subscribe(Box::new(move |online| {
            let _ = net_tx.send(Msg::Net(online));
        }));
        let now = deps.clock.now();
        let engine = Engine {
            d: deps,
            cfg,
            tx,
            shared,
            session: None,
            live: None,
            waiting: WaitQueue::default(),
            ending: None,
            pending_end: None,
            results: HashMap::new(),
            result_order: VecDeque::new(),
            results_clear_at: None,
            rate: RateCap::default(),
            crash_streak: 0,
            last_activity: now,
            last_settings_check: now,
            next_gen: 0,
        };
        thread::spawn(move || engine.run(rx));
        host
    }

    /// At app start: kill a server that an earlier Bottle left, and delete GENIE_HOME and the
    /// temp cwd folders.
    pub fn sweep_at_start(cfg: &SessionConfig, genie: Option<&std::path::Path>) {
        if let Some(g) = genie {
            spawn::sweep_stale(&cfg.pid_file, g, &|p| spawn::exe_of(p));
        } else {
            spawn::remove_pid(&cfg.pid_file);
        }
        let _ = genie_home::delete_home(&cfg.home);
        genie_home::sweep_cwds(&cfg.tmp_root);
    }

    fn ask<T>(&self, mk: impl FnOnce(Reply<T>) -> Msg) -> Option<T> {
        let (r, rx) = mpsc::channel();
        self.tx.send(mk(r)).ok()?;
        rx.recv_timeout(END_WAIT).ok()
    }

    /// The hook for `AgentGuard::set_refusal_hook`: the third refusal of a turn interrupts it and
    /// ends it with the outcome `refusals`.
    pub fn refusal_hook(&self) -> Arc<dyn Fn(&str) + Send + Sync> {
        let tx = Mutex::new(self.tx.clone());
        Arc::new(move |turn_id: &str| {
            let _ = lock(&tx).send(Msg::Refusals(turn_id.to_owned()));
        })
    }

    /// Drive the timer once, and wait until it ran (tests, and the clock owner).
    pub fn tick(&self) {
        let _ = self.ask(|r| Msg::Tick(Some(r)));
    }

    /// End the session for `reason` and wait until the process is gone and GENIE_HOME is
    /// deleted. Used by quit and Wipe.
    pub fn end_blocking(&self, reason: AgentSessionEndReason) {
        let _ = self.ask(|r| Msg::End(reason, Some(r)));
    }

    /// Quit: end the session, delete GENIE_HOME and stop the actor.
    pub fn shutdown(&self) {
        let _ = self.ask(Msg::Shutdown);
    }

    pub fn status(&self, recheck: bool) -> GenieCodeStatus {
        if recheck {
            lock(&self.shared.not_eligible).clear();
        }
        let installing = self.shared.installing.load(Ordering::SeqCst);
        let bin = if installing {
            BinInfo {
                state: GenieCodeBinary::Installing,
                version: None,
                path: None,
            }
        } else {
            self.binary.check(recheck)
        };
        let profile = self.deps_view.snapshot().ok().and_then(|s| s.agent_profile);
        let state = match &profile {
            None => AgentProfileState::NoHome,
            Some(p) if lock(&self.shared.not_eligible).contains(p) => {
                AgentProfileState::NotEligible
            }
            Some(p) => self.env.profile_state(p),
        };
        GenieCodeStatus {
            binary: bin.state,
            version: bin.version,
            pinned: GENIE_PINNED.to_owned(),
            agent_profile: profile,
            agent_profile_state: state,
        }
    }

    fn install(&self) -> CoreResult<GenieCodeStatus> {
        self.shared.installing.store(true, Ordering::SeqCst);
        let r = self.installer.install();
        self.shared.installing.store(false, Ordering::SeqCst);
        match r {
            Ok(()) => Ok(self.status(true)),
            Err(e) => Err(CoreError::IpcRefused { code: e.code() }),
        }
    }

    pub fn submit(&self, a: &AgentTurnSubmitArgs) -> Result<AgentTurnAccepted, &'static str> {
        let mode = a.mode.unwrap_or(AgentAskMode::Ask);
        let mog: &str = &a.mog_id;
        // The binary and the profile first: the cheap refusals do not use up an approval.
        let bin = self.binary.check(false);
        match bin.state {
            GenieCodeBinary::Ready => {}
            GenieCodeBinary::WrongVersion => return Err("genie_code_version"),
            _ => return Err("genie_code_missing"),
        }
        let peek = self.ask(Msg::Peek).ok_or("agent_session_unavailable")?;
        if mode == AgentAskMode::FollowUp && !peek.has_session {
            return Err("agent_session_unavailable");
        }
        if !peek.has_session {
            let snap = self
                .deps_view
                .snapshot()
                .map_err(|_| "agent_session_unavailable")?;
            let profile = snap.agent_profile.ok_or("agent_profile_unsupported")?;
            if lock(&self.shared.not_eligible).contains(&profile) {
                return Err("agent_not_eligible");
            }
            self.env.preflight(&profile).map_err(|f| match f {
                PreflightFail::NotU2m | PreflightFail::Missing => "agent_profile_unsupported",
                PreflightFail::SignedOut => "agent_signed_out",
                PreflightFail::Offline => "agent_offline",
                PreflightFail::NotEligible => "agent_not_eligible",
            })?;
        }
        if !self
            .approvals
            .redeem(mog, a.action_ticket.as_ref().map(|t| t.as_str()))
        {
            return Err("ask_not_approved");
        }
        let req = SubmitReq {
            question: a.question.as_str().to_owned(),
            mode,
        };
        self.ask(|r| Msg::Submit(req, r))
            .ok_or("agent_session_unavailable")?
    }

    pub fn stop(&self, turn_id: &str) -> Result<(), &'static str> {
        let id = turn_id.to_owned();
        self.ask(|r| Msg::Stop(id, r))
            .ok_or("agent_session_unavailable")?
    }

    pub fn session_state(&self) -> AgentSessionState {
        self.ask(Msg::State).unwrap_or(AgentSessionState {
            state: AgentSessionPhase::None,
            session_id: None,
            started_at: None,
            agent_profile: None,
            turns: 0,
            queued: 0,
            tokens: 0,
        })
    }

    pub fn turn_result(&self, turn_id: &str) -> Option<AgentTurnResult> {
        let id = turn_id.to_owned();
        self.ask(|r| Msg::Result(id, r)).flatten()
    }

    pub fn turn_call(&self, turn_id: &str, call_id: &str) -> Option<AgentTurnCall> {
        self.guard.call_detail(turn_id, call_id)
    }
}

impl Drop for SessionHost {
    fn drop(&mut self) {
        let (r, _rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Shutdown(r));
    }
}

impl AgentWipeHook for SessionHost {
    fn end_for_wipe(&self) {
        self.end_blocking(AgentSessionEndReason::Wipe);
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

fn parse<T: serde::de::DeserializeOwned>(args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| refused("invalid_args"))
}

fn to_value<T: serde::Serialize>(v: &T) -> CoreResult<Value> {
    serde_json::to_value(v).map_err(|_| refused("host_failed"))
}

impl crate::ipc_gateway::GatewayComponents for SessionHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "genie_code_status" => {
                let a: GenieCodeStatusArgs = parse(args)?;
                to_value(&self.status(a.recheck.unwrap_or(false)))
            }
            "genie_code_install" => {
                let a: GenieCodeInstallArgs = parse(args)?;
                if !a.consent {
                    return Err(refused("invalid_args"));
                }
                to_value(&self.install()?)
            }
            "agent_turn_submit" => {
                let a: AgentTurnSubmitArgs = parse(args)?;
                to_value(&self.submit(&a).map_err(refused)?)
            }
            "agent_turn_stop" => {
                let a: AgentTurnStopArgs = parse(args)?;
                self.stop(a.turn_id.as_str()).map_err(refused)?;
                Ok(json!({}))
            }
            "agent_turn_call" => {
                let a: AgentTurnCallArgs = parse(args)?;
                let c = self
                    .turn_call(a.turn_id.as_str(), a.call_id.as_str())
                    .ok_or(refused("agent_turn_unknown"))?;
                to_value(&c)
            }
            "agent_turn_result" => {
                let a: AgentTurnResultArgs = parse(args)?;
                let r = self
                    .turn_result(a.turn_id.as_str())
                    .ok_or(refused("agent_turn_unknown"))?;
                to_value(&r)
            }
            "agent_session_state" => to_value(&self.session_state()),
            "agent_session_end" => {
                let _a: AgentSessionEndArgs = parse(args)?;
                self.end_blocking(AgentSessionEndReason::NewSession);
                Ok(json!({}))
            }
            _ => Err(refused("not_implemented")),
        }
    }
}
