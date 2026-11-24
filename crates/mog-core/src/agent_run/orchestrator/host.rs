//! The orchestrator host: the ask loop of ADR 0122, with a session, a turn queue and a tool loop.
//!
//! One actor thread owns all state. Commands from IPC, the network watch, the timer and the tool
//! loop worker all arrive as messages, so no lock order can break and every rule has one place.
//! The live turn runs its blocking tool loop on a worker thread. A stop, a timeout or a session
//! end finishes the turn at once; the worker then stops at its next step, and its late result is
//! dropped. Nothing here logs or keeps a question, an answer or a token outside the in-memory
//! thread history, which is cleared at session end and on Wipe.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use super::approval::Approvals;
use super::deadlines::{self, TurnDue};
use super::prompt::{self, PromptInput, ANSWER_SCHEMA};
use super::queue::{RateCap, WaitQueue, MAX_WAITING};
use crate::agent_run::guard::{self, AgentGuard, ToolOut};
use crate::agent_run::seam::{
    AgentEvent, CallRecord, CatalogEntry, NetWatch, RunEvents, SessionGuard, SessionSpec, StopWhy,
    TurnSpec,
};
use crate::clock::{MonotonicClock, WallClock};
use crate::error::{CoreError, CoreResult};
use crate::ipc::{
    AgentAskMode, AgentSessionEndArgs, AgentTurnCallArgs, AgentTurnResultArgs, AgentTurnStopArgs,
    AgentTurnSubmitArgs, OrchestratorStatusArgs,
};
use crate::model_gateway::client;
use crate::model_gateway::models::list_orchestrator_models;
use crate::model_gateway::{ChatClient, ChatRequest, DEFAULT_ORCHESTRATOR_MODEL};
use crate::types::{
    AgentAnswerKind, AgentCallState, AgentSessionEndReason, AgentSessionEvent, AgentSessionPhase,
    AgentSessionState, AgentSource, AgentTurnAccepted, AgentTurnCall, AgentTurnEvent,
    AgentTurnOutcome, AgentTurnResult, AgentUsage, AgentWorkspace, GenieCallBudget,
    OrchestratorModel, OrchestratorModelInfo, OrchestratorModels, OrchestratorState,
    OrchestratorStatus,
};
use crate::wipe::AgentWipeHook;

/// The most characters of the model's `answer` that Bottle keeps.
pub const ANSWER_MAX_CHARS: usize = 8000;
/// The most bytes of the final model message.
pub const FINAL_MAX_BYTES: usize = 32 * 1024;
/// The most model steps (chat calls) of one turn.
pub const MAX_STEPS: usize = 12;
/// The most earlier turns of a thread that go back to the model.
pub const HISTORY_TURNS: usize = 8;
/// The most bytes of history that go back to the model. The oldest turns go first.
pub const HISTORY_BYTES: usize = 256 * 1024;
/// The most `ask_genie` calls that run in one step.
const MAX_CALLS_PER_STEP: usize = 20;
/// The most answers that stay in memory.
const RESULTS_MAX: usize = 64;
/// How long a caller waits for the actor.
const ACTOR_WAIT: Duration = Duration::from_secs(15);

// ---------- the seams to the app ----------

/// The settings that a session snapshots at its start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub catalog: Vec<CatalogEntry>,
    /// D1 "Combine across workspaces".
    pub cross_workspace: bool,
    /// D2 "Genie calls per question": 5, 10 or 20.
    pub budget: u8,
    /// The home profile: its AI Gateway runs the model. `None`: no home yet.
    pub home_profile: Option<String>,
    /// The orchestrator model, read at the start of each turn.
    pub model: String,
}

/// The workspace store and the settings as the host sees them.
pub trait WorkspaceView: Send + Sync {
    fn snapshot(&self) -> CoreResult<Snapshot>;
}

/// Whether an ask can run now, or why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrchStatus {
    Ready,
    NoHome,
    SignedOut,
    NoModel,
    Unreachable,
}

impl OrchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            OrchStatus::Ready => "ready",
            OrchStatus::NoHome => "no_home",
            OrchStatus::SignedOut => "signed_out",
            OrchStatus::NoModel => "no_model",
            OrchStatus::Unreachable => "unreachable",
        }
    }
}

/// The check of the home profile and the model. A check-only function: the gateway lane
/// implements the real probe. It may block.
pub trait OrchEnv: Send + Sync {
    fn probe(&self, profile: &str, model: &str) -> OrchStatus;
    /// Forget the cached result of earlier failed calls, so that the next `probe` starts clean
    /// (`orchestrator_status` with `recheck`).
    fn clear(&self) {}
}

/// The guard as the host sees it: the seam of the session, plus the in-process `ask_genie`.
pub trait OrchGuard: SessionGuard {
    /// Run one `ask_genie` call of `turn_id` with the checks of the guard. The call is
    /// refused (`run_stopped`) unless that turn is the live one. Blocks until it ends.
    fn ask_genie(&self, session_id: &str, turn_id: &str, args: &Value) -> ToolOut;
    /// Set the function that runs when the third refusal of a turn stops it.
    fn set_refusal_hook(&self, hook: Arc<dyn Fn(&str) + Send + Sync>);
}

impl OrchGuard for AgentGuard {
    fn ask_genie(&self, session_id: &str, turn_id: &str, args: &Value) -> ToolOut {
        AgentGuard::ask_genie_for(self, session_id, turn_id, args)
    }
    fn set_refusal_hook(&self, hook: Arc<dyn Fn(&str) + Send + Sync>) {
        AgentGuard::set_refusal_hook(self, hook);
    }
}

/// The seams of the host.
pub struct OrchDeps {
    pub guard: Arc<dyn OrchGuard>,
    pub chat: Arc<dyn ChatClient>,
    pub net: Arc<dyn NetWatch>,
    pub events: Arc<dyn RunEvents>,
    pub clock: Arc<dyn MonotonicClock>,
    pub wall: Arc<dyn WallClock>,
    pub workspaces: Arc<dyn WorkspaceView>,
    pub approvals: Arc<dyn Approvals>,
    pub env: Arc<dyn OrchEnv>,
}

/// How the host runs.
#[derive(Debug, Clone)]
pub struct OrchConfig {
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

#[derive(Default, Clone, Copy)]
struct Usage {
    input: u64,
    cached: u64,
    output: u64,
    reasoning: u64,
}

enum LoopStop {
    /// The final message of the model.
    Final(String),
    /// The turn failed with this outcome.
    Failed(AgentTurnOutcome),
    /// The 10 min turn deadline passed.
    Timeout,
    /// The turn was stopped from outside.
    Cancelled,
}

struct LoopOut {
    stop: LoopStop,
    usage: Usage,
    /// The messages of this turn (the question, the tool calls and the final message).
    new_messages: Vec<Value>,
}

enum Msg {
    Submit(SubmitReq, Reply<Result<AgentTurnAccepted, &'static str>>),
    Stop(String, Reply<Result<(), &'static str>>),
    End(AgentSessionEndReason, Option<Reply<()>>),
    State(Reply<AgentSessionState>),
    Peek(Reply<Peek>),
    Result(String, Reply<Option<AgentTurnResult>>),
    Loop(String, LoopOut),
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
            profile: s.home_profile.clone(),
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

/// One session: the D1, catalog and profile snapshot, and the thread history (memory only).
struct Session {
    id: String,
    fp: Fingerprint,
    catalog: Vec<CatalogEntry>,
    cross_workspace: bool,
    started_wall_ms: i64,
    turns: u32,
    tokens: u64,
    history: VecDeque<Vec<Value>>,
}

struct QTurn {
    id: String,
    question: String,
    /// A new question: it must not run on a tainted session.
    fresh: bool,
}

struct Live {
    turn: QTurn,
    began: bool,
    started_at: Duration,
    slow_sent: bool,
    /// Tells the worker to stop at its next step.
    cancel: Arc<AtomicBool>,
    /// The guard stopped the turn after 3 refused tool calls.
    refusals: bool,
    final_text: Option<String>,
    new_messages: Vec<Value>,
    usage: Usage,
}

impl Live {
    fn new(turn: QTurn, now: Duration) -> Self {
        Self {
            turn,
            began: false,
            started_at: now,
            slow_sent: false,
            cancel: Arc::new(AtomicBool::new(false)),
            refusals: false,
            final_text: None,
            new_messages: Vec::new(),
            usage: Usage::default(),
        }
    }
}

struct Engine {
    d: OrchDeps,
    cfg: OrchConfig,
    tx: Sender<Msg>,
    session: Option<Session>,
    live: Option<Live>,
    waiting: WaitQueue<QTurn>,
    pending_end: Option<AgentSessionEndReason>,
    results: HashMap<String, AgentTurnResult>,
    result_order: VecDeque<String>,
    results_clear_at: Option<Duration>,
    rate: RateCap,
    last_activity: Duration,
    last_settings_check: Duration,
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
            Msg::Loop(turn_id, out) => self.on_loop(&turn_id, out),
            Msg::Net(online) => {
                if !online && self.live.is_some() {
                    self.stop_live(StopWhy::Offline);
                }
            }
            Msg::Refusals(turn_id) => {
                if self.live.as_ref().is_some_and(|l| l.turn.id == turn_id) {
                    if let Some(l) = self.live.as_mut() {
                        l.refusals = true;
                    }
                    self.stop_live(StopWhy::Failed);
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

    // ----- state view -----

    fn state(&self) -> AgentSessionState {
        let profile = self
            .d
            .workspaces
            .snapshot()
            .ok()
            .and_then(|s| s.home_profile);
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
            Some(s) => AgentSessionState {
                state: if self.live.is_some() {
                    AgentSessionPhase::Busy
                } else {
                    AgentSessionPhase::Idle
                },
                session_id: Some(s.id.clone()),
                started_at: u64::try_from(s.started_wall_ms).ok(),
                agent_profile: s.fp.profile.clone(),
                turns: s.turns,
                queued: u32::try_from(self.waiting.len()).unwrap_or(u32::MAX),
                tokens: s.tokens,
            },
        }
    }

    // ----- submit, stop -----

    fn submit(&mut self, req: SubmitReq) -> Result<AgentTurnAccepted, &'static str> {
        let now = self.now();
        if req.mode == AgentAskMode::FollowUp && self.session.is_none() {
            return Err("agent_session_unavailable");
        }
        if self.live.is_some() && self.waiting.len() >= MAX_WAITING {
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
            fresh: req.mode != AgentAskMode::FollowUp,
        };
        let turn_id = turn.id.clone();
        let mut new_session = self.session.is_none();
        let position;
        if self.live.is_some() {
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
        let s = self.session.as_ref().ok_or("agent_session_unavailable")?;
        Ok(AgentTurnAccepted {
            turn_id,
            session_id: s.id.clone(),
            position,
            new_session,
            budget,
            cross_workspace: s.cross_workspace,
            catalog: s
                .catalog
                .iter()
                .map(|c| AgentWorkspace {
                    workspace_id: c.alias.clone(),
                    profile: c.profile.clone(),
                    name: c.name.clone(),
                    is_home: c.is_home,
                    is_agent_workspace: c.is_agent_workspace,
                })
                .collect(),
        })
    }

    fn stop(&mut self, turn_id: &str) -> Result<(), &'static str> {
        if self.waiting.contains(turn_id) {
            self.waiting.remove(turn_id);
            self.emit_ended_unrun(turn_id, AgentTurnOutcome::Removed);
            return Ok(());
        }
        if self.live.as_ref().is_some_and(|l| l.turn.id == turn_id) {
            self.stop_live(StopWhy::User);
            return Ok(());
        }
        Err("agent_turn_unknown")
    }

    // ----- the session -----

    /// Make sure that a session exists. A session is a guard ledger and a thread history.
    fn ensure_session(&mut self, snap: &Snapshot) -> Result<(), &'static str> {
        if self.session.is_some() {
            return Ok(());
        }
        self.results.clear();
        self.result_order.clear();
        self.results_clear_at = None;
        if snap.home_profile.is_none() {
            return Err("agent_profile_unsupported");
        }
        let session_id = id32();
        let spec = SessionSpec {
            session_id: session_id.clone(),
            catalog: snap.catalog.clone(),
            cross_workspace: snap.cross_workspace,
        };
        self.d
            .guard
            .open_session(spec)
            .map_err(|_| "agent_session_unavailable")?;
        self.session = Some(Session {
            id: session_id.clone(),
            fp: Fingerprint::of(snap),
            catalog: snap.catalog.clone(),
            cross_workspace: snap.cross_workspace,
            started_wall_ms: self.d.wall.now_ms(),
            turns: 0,
            tokens: 0,
            history: VecDeque::new(),
        });
        self.last_activity = self.now();
        self.last_settings_check = self.now();
        self.emit_session(AgentSessionEvent::Started { session_id });
        Ok(())
    }

    // ----- turn start and stop -----

    /// Start the live turn: tell the guard, build the messages, run the tool loop on a worker.
    fn pump(&mut self) {
        let Some(l) = self.live.as_ref() else { return };
        if l.began {
            return;
        }
        if !self.d.net.online() {
            return self.finish(AgentTurnOutcome::Offline);
        }
        let Ok(snap) = self.d.workspaces.snapshot() else {
            return self.finish(AgentTurnOutcome::Failed);
        };
        let Some(profile) = snap.home_profile.clone() else {
            return self.finish(AgentTurnOutcome::Failed);
        };
        let Some(s) = self.session.as_ref() else {
            return;
        };
        let (sid, catalog, cross) = (s.id.clone(), s.catalog.clone(), s.cross_workspace);
        let budget = snap.budget;
        let (turn_id, question, cancel, fresh) = {
            let l = self.live.as_ref().expect("checked above");
            (
                l.turn.id.clone(),
                l.turn.question.clone(),
                Arc::clone(&l.cancel),
                l.turn.fresh,
            )
        };
        // A new ask starts a new thread: the earlier threads do not go to the model. Their
        // answers (up to `HISTORY_BYTES`) would make every step of this turn slower, and the
        // model would answer them again. A follow-up keeps the history of its thread.
        if fresh {
            if let Some(s) = self.session.as_mut() {
                s.history.clear();
            }
        }
        if self
            .d
            .guard
            .begin_turn(
                &sid,
                TurnSpec {
                    turn_id: turn_id.clone(),
                    user_question: question.clone(),
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
        let system = prompt::system(&PromptInput {
            catalog: &catalog,
            budget,
            cross_workspace: cross,
            agent_alias: agent_alias.as_deref(),
        });
        let mut input: Vec<Value> = Vec::new();
        if let Some(s) = self.session.as_ref() {
            for turn in &s.history {
                input.extend(turn.iter().cloned());
            }
        }
        let base = input.len();
        input.push(user_item(&prompt::user(&question, &prompt::new_boundary())));
        let now = self.now();
        if let Some(l) = self.live.as_mut() {
            l.began = true;
            l.started_at = now;
        }
        // The question counts when it starts, so the thread header never says "0 questions"
        // while its first answer shows.
        if let Some(s) = self.session.as_mut() {
            s.turns += 1;
        }
        let ctx = LoopCtx {
            chat: Arc::clone(&self.d.chat),
            guard: Arc::clone(&self.d.guard),
            net: Arc::clone(&self.d.net),
            clock: Arc::clone(&self.d.clock),
            session_id: sid,
            turn_id: turn_id.clone(),
            budget,
            asked: 0,
            profile,
            model: snap.model,
            started_at: now,
            cancel,
            system,
            input,
            base,
        };
        let tx = self.tx.clone();
        let tid = turn_id.clone();
        thread::spawn(move || {
            // A panic of the loop must still end the turn.
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_loop(ctx)))
                .unwrap_or_else(|_| LoopOut {
                    stop: LoopStop::Failed(AgentTurnOutcome::Failed),
                    usage: Usage::default(),
                    new_messages: Vec::new(),
                });
            let _ = tx.send(Msg::Loop(tid, out));
        });
        self.emit_turn(AgentTurnEvent::Started {
            turn_id: turn_id.clone(),
        });
        self.emit_turn(AgentTurnEvent::Thinking { turn_id });
    }

    /// Stop the live turn for `why`: cancel the worker, tell the guard (it cancels the open
    /// Genie polls for `User`, `Failed` and `SessionEnded`), and end the turn.
    fn stop_live(&mut self, why: StopWhy) {
        let Some(l) = self.live.as_ref() else { return };
        l.cancel.store(true, Ordering::SeqCst);
        if l.began {
            self.d.guard.stop_turn(&l.turn.id, why);
        }
        let outcome = match (l.refusals, why) {
            (true, StopWhy::Failed) => AgentTurnOutcome::Refusals,
            _ => outcome_of_stop(why),
        };
        self.finish(outcome);
    }

    /// The worker is done.
    fn on_loop(&mut self, turn_id: &str, out: LoopOut) {
        let Some(l) = self.live.as_mut().filter(|l| l.turn.id == turn_id) else {
            // A late result of a turn that already ended.
            return;
        };
        l.usage = out.usage;
        match out.stop {
            LoopStop::Final(text) => {
                l.final_text = Some(text);
                l.new_messages = out.new_messages;
                self.finish(AgentTurnOutcome::Answered);
            }
            LoopStop::Failed(o) => self.finish_stopping(o),
            LoopStop::Timeout => self.stop_live(StopWhy::Timeout),
            LoopStop::Cancelled => {
                // A stop from outside has already ended the turn (or set the flag). A worker
                // that gave up on its own, for example after a panic of a tool thread, failed.
                if !l.cancel.load(Ordering::SeqCst) {
                    self.finish_stopping(AgentTurnOutcome::Failed);
                }
            }
        }
    }

    /// End the live turn with `outcome`; the guard stops the turn first, so open calls end.
    fn finish_stopping(&mut self, outcome: AgentTurnOutcome) {
        if let Some(l) = self.live.as_ref() {
            l.cancel.store(true, Ordering::SeqCst);
            if l.began {
                self.d.guard.stop_turn(&l.turn.id, StopWhy::Failed);
            }
        }
        self.finish(outcome);
    }

    // ----- ending a turn -----

    /// The live turn is over. Record it, then go on.
    fn finish(&mut self, outcome: AgentTurnOutcome) {
        let Some(live) = self.live.take() else { return };
        self.record_end(live, outcome);
        self.advance();
    }

    /// Build the result, tell the guard, send the `ended` event.
    fn record_end(&mut self, mut live: Live, outcome: AgentTurnOutcome) {
        let Some(sid) = self.session.as_ref().map(|s| s.id.clone()) else {
            return;
        };
        let turn_id = live.turn.id.clone();
        let calls_all = self.d.guard.calls(&sid);
        let calls = calls_all.iter().filter(|c| c.turn_id == turn_id).count();
        if outcome == AgentTurnOutcome::Answered {
            let d1 = self.session.as_ref().is_some_and(|s| s.cross_workspace);
            let r = build_result(
                live.final_text.as_deref(),
                live.usage,
                &calls_all,
                d1,
                self.d.guard.as_ref(),
            );
            let hidden = r.answer_kind == AgentAnswerKind::Hidden;
            self.store_result(turn_id.clone(), r);
            // A hidden answer (a bad source, a withheld call) leaves no history.
            if !hidden {
                if let Some(s) = self.session.as_mut() {
                    s.history.push_back(std::mem::take(&mut live.new_messages));
                    let bytes = |h: &VecDeque<Vec<Value>>| -> usize {
                        h.iter().flatten().map(|v| v.to_string().len()).sum()
                    };
                    while s.history.len() > HISTORY_TURNS
                        || (s.history.len() > 1 && bytes(&s.history) > HISTORY_BYTES)
                    {
                        s.history.pop_front();
                    }
                    if bytes(&s.history) > HISTORY_BYTES {
                        s.history.clear();
                    }
                }
            }
        }
        if live.began {
            self.d.guard.end_turn(&turn_id);
        }
        let tokens = live.usage.input + live.usage.output;
        if let Some(s) = self.session.as_mut() {
            // A turn that began was counted at its start (`pump`).
            s.tokens += tokens;
        }
        self.last_activity = self.now();
        self.emit_turn(AgentTurnEvent::Ended {
            turn_id,
            outcome,
            calls: u32::try_from(calls).unwrap_or(u32::MAX),
            tokens: (tokens > 0).then_some(tokens),
        });
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
            if self.live.is_some() {
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
            let Ok(snap) = self.d.workspaces.snapshot() else {
                self.emit_ended_unrun(&next.id, AgentTurnOutcome::Failed);
                continue;
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

    /// The one routine for all causes: the live turn, the waiting turns, the history, the ledger.
    fn begin_end(&mut self, reason: AgentSessionEndReason, waiter: Option<Reply<()>>) {
        if self.session.is_none() {
            if reason == AgentSessionEndReason::Wipe {
                // Earlier ended sessions keep a ledger for ten minutes: Wipe drops them all.
                self.d.guard.wipe_all();
                self.results.clear();
                self.result_order.clear();
            }
        } else {
            if let Some(live) = self.live.take() {
                live.cancel.store(true, Ordering::SeqCst);
                if live.began {
                    self.d.guard.stop_turn(&live.turn.id, StopWhy::SessionEnded);
                }
                self.record_end(live, AgentTurnOutcome::SessionEnded);
            }
            self.end_session_core(reason);
        }
        for (id, _) in self.waiting.drain() {
            self.emit_ended_unrun(&id, AgentTurnOutcome::SessionEnded);
        }
        if let Some(w) = waiter {
            let _ = w.send(());
        }
    }

    /// Drop the thread history, close the ledger, emit `ended`. The turns are the caller's job.
    fn end_session_core(&mut self, reason: AgentSessionEndReason) {
        let Some(s) = self.session.take() else {
            return;
        };
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
        if self.session.is_none() {
            return;
        }
        let due = self.live.as_ref().filter(|l| l.began).map(|l| {
            (
                deadlines::turn_due(now.saturating_sub(l.started_at), l.slow_sent),
                l.turn.id.clone(),
            )
        });
        if let Some((due, id)) = due {
            match due {
                TurnDue::Slow => {
                    if let Some(l) = self.live.as_mut() {
                        l.slow_sent = true;
                    }
                    self.emit_turn(AgentTurnEvent::Slow { turn_id: id });
                }
                TurnDue::HardStop => self.stop_live(StopWhy::Timeout),
                TurnDue::Nothing => {}
            }
        }
        // The settings of the session.
        if now.saturating_sub(self.last_settings_check) >= deadlines::SETTINGS_POLL {
            self.last_settings_check = now;
            if self.pending_end.is_none() {
                self.pending_end = self.drift();
            }
        }
        if self.live.is_none() {
            if self.pending_end.is_some() {
                return self.advance();
            }
            if self.session.is_some()
                && self.waiting.is_empty()
                && deadlines::idle_over(now.saturating_sub(self.last_activity))
            {
                self.end_session_core(AgentSessionEndReason::Idle);
            }
        }
    }
}

// ---------- the tool loop ----------

/// What the worker thread of one turn owns.
struct LoopCtx {
    chat: Arc<dyn ChatClient>,
    guard: Arc<dyn OrchGuard>,
    net: Arc<dyn NetWatch>,
    clock: Arc<dyn MonotonicClock>,
    session_id: String,
    turn_id: String,
    /// The D2 budget of this turn.
    budget: u8,
    /// The `ask_genie` calls that went to the guard so far.
    asked: usize,
    profile: String,
    model: String,
    started_at: Duration,
    cancel: Arc<AtomicBool>,
    /// The system instructions of this turn.
    system: String,
    /// The input items: the earlier turns, then the question.
    input: Vec<Value>,
    /// The index of the question in `input`.
    base: usize,
}

/// The tool that ends a turn: its arguments are the final answer. It has no side effect.
pub const SUBMIT_TOOL: &str = "submit_answer";

/// The two tools, as Responses-API function tools. Only `ask_genie` acts, through the guard.
fn tools() -> Value {
    let s = guard::tool_schema();
    let answer: Value = serde_json::from_str(ANSWER_SCHEMA).unwrap_or(Value::Null);
    json!([{
        "type": "function",
        "name": guard::TOOL_NAME,
        "description": s["description"],
        "parameters": s["inputSchema"],
    }, {
        "type": "function",
        "name": SUBMIT_TOOL,
        "description": "Give the final answer to the user. Call it once, when you have the \
            answer. `answer` cites sources inline as [n], where n is the 1-based position in \
            `sources`.",
        "parameters": answer,
    }])
}

/// The most output tokens of one model step. The final `submit_answer` holds the whole answer
/// and its sources, so the cap leaves room for a long one.
const MAX_OUTPUT_TOKENS: u32 = 4096;

fn user_item(text: &str) -> Value {
    json!({ "role": "user", "content": [{ "type": "input_text", "text": text }] })
}

fn add_usage(u: &mut Usage, resp: &Value) {
    let n = |v: &Value| v.as_u64().unwrap_or(0);
    let us = &resp["usage"];
    u.input += n(&us["input_tokens"]);
    u.cached += n(&us["input_tokens_details"]["cached_tokens"]);
    u.output += n(&us["output_tokens"]);
    u.reasoning += n(&us["output_tokens_details"]["reasoning_tokens"]);
}

/// One `function_call` item of a Responses reply.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct FnCall {
    pub(super) call_id: String,
    pub(super) name: String,
    pub(super) arguments: String,
}

/// What one Responses reply holds.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Reply1 {
    /// The `function_call` items, in order.
    pub(super) calls: Vec<FnCall>,
    /// The text of the `message` items, joined.
    pub(super) text: Option<String>,
}

/// Read a Responses reply. `None` when the reply is not a finished answer: a failed reply, one
/// with no `output` array, or one with a call that has no `call_id`. An `incomplete` reply (the
/// token cap cut it off) counts only when its one call is a whole `submit_answer`.
pub(super) fn parse_reply(resp: &Value) -> Option<Reply1> {
    if resp["error"].is_object() || resp["status"].as_str() == Some("failed") {
        return None;
    }
    let items = resp["output"].as_array()?;
    if resp["status"].as_str() == Some("incomplete") {
        let mut calls = items.iter().filter(|i| i["type"] == "function_call");
        let (Some(c), None) = (calls.next(), calls.next()) else {
            return None;
        };
        let args = c["arguments"].as_str().unwrap_or("");
        if c["name"].as_str() != Some(SUBMIT_TOOL)
            || !serde_json::from_str::<Value>(args).is_ok_and(|v| v.is_object())
        {
            return None;
        }
    }
    let mut calls = Vec::new();
    let mut text = String::new();
    let mut has_text = false;
    for it in items {
        match it["type"].as_str() {
            Some("function_call") => calls.push(FnCall {
                // No id is made up: a call with no id refuses the whole reply.
                call_id: it["call_id"]
                    .as_str()
                    .filter(|c| !c.is_empty())
                    .map(str::to_owned)?,
                name: it["name"].as_str().unwrap_or("").to_owned(),
                arguments: it["arguments"].as_str().unwrap_or("").to_owned(),
            }),
            Some("message") => {
                for part in it["content"].as_array().into_iter().flatten() {
                    if part["type"].as_str() == Some("output_text") {
                        if let Some(t) = part["text"].as_str() {
                            text.push_str(t);
                            has_text = true;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Some(Reply1 {
        calls,
        text: has_text.then_some(text),
    })
}

/// The items of a reply that may go back into `input` and the history: `function_call` items
/// with a string `call_id`, assistant `message` items and `reasoning` items. Nothing else.
pub(super) fn echo_items(resp: &Value) -> Vec<Value> {
    resp["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|it| match it["type"].as_str() {
            Some("function_call") => it["call_id"].is_string(),
            Some("message") => it["role"].as_str() == Some("assistant"),
            Some("reasoning") => true,
            _ => false,
        })
        .cloned()
        .collect()
}

fn output_item(call_id: &str, text: &str) -> Value {
    json!({ "type": "function_call_output", "call_id": call_id, "output": text })
}

/// The `function_call_output` item for a call. A call that is not `ask_genie` with a JSON object
/// as arguments is never run: `Value::Null` goes to the guard, which refuses it as `invalid_args`
/// and counts the refusal (the third one stops the turn). A stopped turn runs nothing.
/// The second value is true when the guard accepted the call (it used budget), whatever came
/// back; a refusal is a bare code, not JSON.
fn run_tool_call(ctx: &LoopCtx, call: &FnCall) -> (Value, bool) {
    if ctx.cancel.load(Ordering::SeqCst) {
        return (output_item(&call.call_id, guard::RUN_STOPPED), false);
    }
    let args = serde_json::from_str::<Value>(&call.arguments)
        .ok()
        .filter(Value::is_object);
    let run = match (call.name == guard::TOOL_NAME, args) {
        (true, Some(a)) => a,
        _ => Value::Null,
    };
    let out = ctx.guard.ask_genie(&ctx.session_id, &ctx.turn_id, &run);
    let accepted = !(out.is_error && !out.text.starts_with('{'));
    (output_item(&call.call_id, &out.text), accepted)
}

/// Run the calls of one step. At most `min(budget left, 20)` `ask_genie` calls get a thread; the
/// others run one at a time on this thread, so the guard still checks them all.
fn run_step(ctx: &mut LoopCtx, calls: &[FnCall]) -> Vec<Value> {
    // `store: false` is in the request body. A live probe (claude-haiku-5-5, default profile)
    // showed that the two-step loop works with it: the echoed `function_call` item and its
    // output are accepted, and the model reads the callId from the `call` attribute of the
    // `<genie_answer>` block. No `include` field is sent.
    let cap = usize::from(ctx.budget)
        .saturating_sub(ctx.asked)
        .min(MAX_CALLS_PER_STEP);
    let mut go = Vec::new();
    for (i, c) in calls.iter().enumerate() {
        if c.name == guard::TOOL_NAME && go.len() < cap {
            go.push(i);
        }
    }
    let done: &LoopCtx = ctx;
    let (out, accepted) = thread::scope(|sc| {
        let hs: Vec<(usize, _)> = go
            .iter()
            .map(|&i| (i, sc.spawn(move || run_tool_call(done, &calls[i]))))
            .collect();
        let mut out: Vec<Option<Value>> = vec![None; calls.len()];
        let mut accepted = 0;
        for (i, c) in calls.iter().enumerate() {
            if !go.contains(&i) {
                // No thread: a call past the cap, or a call that is not `ask_genie`, runs here,
                // one at a time. The guard refuses it (and counts it) or runs it.
                let (o, ok) = run_tool_call(done, c);
                accepted += usize::from(ok && c.name == guard::TOOL_NAME);
                out[i] = Some(o);
            }
        }
        for (i, h) in hs {
            let (o, ok) = h
                .join()
                .unwrap_or_else(|_| (output_item(&calls[i].call_id, guard::RUN_STOPPED), false));
            accepted += usize::from(ok);
            out[i] = Some(o);
        }
        (out.into_iter().flatten().collect::<Vec<_>>(), accepted)
    });
    // Only the calls that the guard accepted use up the budget of the cap.
    ctx.asked += accepted;
    out
}

fn run_loop(mut ctx: LoopCtx) -> LoopOut {
    let mut usage = Usage::default();
    let tools = tools();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let end = |ctx: &LoopCtx, stop: LoopStop, usage: Usage| LoopOut {
        stop,
        usage,
        new_messages: ctx.input.get(ctx.base..).unwrap_or_default().to_vec(),
    };
    for step in 0..MAX_STEPS {
        if ctx.cancel.load(Ordering::SeqCst) {
            return end(&ctx, LoopStop::Cancelled, usage);
        }
        let left =
            deadlines::HARD_AFTER.saturating_sub(ctx.clock.now().saturating_sub(ctx.started_at));
        if left.is_zero() {
            return end(&ctx, LoopStop::Timeout, usage);
        }
        let resp = ctx.chat.chat(ChatRequest {
            profile: &ctx.profile,
            model: &ctx.model,
            body: json!({
                "instructions": ctx.system,
                "input": ctx.input,
                "tools": tools,
                "tool_choice": "auto",
                "max_output_tokens": MAX_OUTPUT_TOKENS,
                "store": false,
            }),
            deadline: left,
            cancel: Some(ctx.cancel.as_ref()),
        });
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                // The codes of the real chat client (`model_gateway::client`). `login_required`
                // and the like never reach this point: the client maps them to `signed_out`.
                let o = match e.code() {
                    client::CANCELLED => return end(&ctx, LoopStop::Cancelled, usage),
                    client::SIGNED_OUT => AgentTurnOutcome::SignedOut,
                    client::OFFLINE => AgentTurnOutcome::Offline,
                    client::TIMEOUT => AgentTurnOutcome::Timeout,
                    _ if !ctx.net.online() => AgentTurnOutcome::Offline,
                    _ => AgentTurnOutcome::Failed,
                };
                return end(&ctx, LoopStop::Failed(o), usage);
            }
        };
        add_usage(&mut usage, &resp);
        if ctx.cancel.load(Ordering::SeqCst) {
            return end(&ctx, LoopStop::Cancelled, usage);
        }
        let Some(reply) = parse_reply(&resp) else {
            return end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage);
        };
        // A call id that repeats (in this reply or an earlier one) refuses the reply.
        if !reply.calls.iter().all(|c| seen.insert(c.call_id.clone())) {
            return end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage);
        }
        // Only known item types go back, so the next step sees its own calls.
        ctx.input.extend(echo_items(&resp));
        // `submit_answer` ends the turn. Nothing else of the step runs. A submit with arguments
        // that are no JSON object is a refusal, like any unknown call (below).
        if let Some(c) = reply.calls.iter().find(|c| {
            c.name == SUBMIT_TOOL
                && serde_json::from_str::<Value>(&c.arguments).is_ok_and(|v| v.is_object())
        }) {
            // Every call gets an output, so the history stays valid for the next turn.
            for k in &reply.calls {
                let o = if k.call_id == c.call_id {
                    "ok"
                } else {
                    guard::RUN_STOPPED
                };
                ctx.input.push(output_item(&k.call_id, o));
            }
            return end(&ctx, LoopStop::Final(c.arguments.clone()), usage);
        }
        if reply.calls.is_empty() {
            // Plain text is the answer, with no sources.
            return match reply.text {
                Some(t) => {
                    let v = json!({ "answer": t, "sources": [] });
                    end(&ctx, LoopStop::Final(v.to_string()), usage)
                }
                None => end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage),
            };
        }
        // The step cap: a last step that still asks for tools is a failure. Nothing runs.
        if step + 1 == MAX_STEPS {
            return end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage);
        }
        let results = run_step(&mut ctx, &reply.calls);
        // The guard says `run_stopped` once it stopped the turn (the third refusal, a stop):
        // the actor ends the turn, so the worker stops here and asks the model nothing more.
        let stopped = results
            .iter()
            .any(|r| r["output"].as_str() == Some(guard::RUN_STOPPED));
        ctx.input.extend(results);
        if stopped {
            return end(&ctx, LoopStop::Cancelled, usage);
        }
    }
    end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage)
}

// ---------- helpers ----------

fn outcome_of_stop(why: StopWhy) -> AgentTurnOutcome {
    match why {
        StopWhy::User => AgentTurnOutcome::Stopped,
        StopWhy::Timeout => AgentTurnOutcome::Timeout,
        StopWhy::Offline => AgentTurnOutcome::Offline,
        StopWhy::SessionEnded => AgentTurnOutcome::SessionEnded,
        StopWhy::Failed => AgentTurnOutcome::Failed,
    }
}

/// The result of an answered turn: the final message, checked against the guard's call log (any
/// call of the session counts, not only this turn). A withheld call is never a source.
fn build_result(
    final_text: Option<&str>,
    usage: Usage,
    calls: &[CallRecord],
    d1: bool,
    guard: &dyn OrchGuard,
) -> AgentTurnResult {
    let usage = Some(AgentUsage {
        input_tokens: usage.input,
        cached_input_tokens: usage.cached,
        output_tokens: usage.output,
        reasoning_tokens: usage.reasoning,
    });
    let hidden = AgentTurnResult {
        answer: None,
        answer_kind: AgentAnswerKind::Hidden,
        sources: Vec::new(),
        usage,
    };
    let Some(text) = final_text.filter(|t| t.len() <= FINAL_MAX_BYTES) else {
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
        let ok = calls
            .iter()
            .find(|c| c.call_id == cid && c.alias == ws && c.state == AgentCallState::Answered);
        let Some(c) = ok else { return hidden };
        // A withheld call keeps the state Answered, so ask the guard. No detail: not a source.
        if guard
            .call_detail(&c.turn_id, &c.call_id)
            .is_none_or(|d| d.withheld.is_some())
        {
            return hidden;
        }
        out.push(AgentSource {
            workspace_id: ws.to_owned(),
            call_id: cid.to_owned(),
            turn_id: c.turn_id.clone(),
        });
    }
    // An empty answer means "show the Genie answer as it is" (the prompt's one-workspace rule):
    // it needs a validated source to show.
    if answer.trim().is_empty() && out.is_empty() {
        return hidden;
    }
    let ids: Vec<&str> = calls.iter().map(|c| c.call_id.as_str()).collect();
    let aliases: Vec<&str> = out.iter().map(|s| s.workspace_id.as_str()).collect();
    let answer = strip_bad_markers(&strip_source_footer(answer, &ids, &aliases), out.len());
    AgentTurnResult {
        answer: Some(cut_answer(&answer)),
        answer_kind: if d1 && cites_many_workspaces(&out) {
            AgentAnswerKind::Combined
        } else {
            AgentAnswerKind::Note
        },
        sources: out,
        usage,
    }
}

/// Cut an answer that is too long back to its last line break before the limit, so a link or a
/// table row is not split. A text with no line break is cut at the limit.
pub(super) fn cut_answer(answer: &str) -> String {
    if answer.chars().count() <= ANSWER_MAX_CHARS {
        return answer.to_owned();
    }
    let cut: String = answer.chars().take(ANSWER_MAX_CHARS).collect();
    match cut.rfind('\n') {
        // Only a break past the halfway point counts: an early break (one leading newline,
        // then a long paragraph) must not throw away almost the whole answer.
        Some(i) if i > ANSWER_MAX_CHARS / 2 => cut[..i].trim_end().to_owned(),
        _ => cut,
    }
}

/// Remove each `[n]` marker where n is not in `1..=n_sources`. With no source, all go. A `[n](url)`
/// link is not a marker and stays.
pub(super) fn strip_bad_markers(answer: &str, n_sources: usize) -> String {
    static MARK: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = MARK.get_or_init(|| regex::Regex::new(r"( *)\[(\d+)\]( *)").expect("a fixed pattern"));
    let mut out = String::with_capacity(answer.len());
    let mut last = 0;
    for c in re.captures_iter(answer) {
        let m = c.get(0).expect("group 0");
        out.push_str(&answer[last..m.start()]);
        last = m.end();
        let next = answer[m.end()..].chars().next();
        let link = next == Some('(') && c[3].is_empty();
        if link || matches!(c[2].parse::<usize>(), Ok(n) if (1..=n_sources).contains(&n)) {
            out.push_str(m.as_str());
            continue;
        }
        // A removed marker leaves one space between words, and none before punctuation or the end.
        let tight = next.is_none_or(|ch| matches!(ch, ',' | '.' | ';' | ':' | '!' | '?' | ')'));
        if !tight && !c[1].is_empty() {
            out.push(' ');
        }
    }
    out.push_str(&answer[last..]);
    out
}

/// True when `alias` stands in `text` as a word of its own. A dot, `_` or `-` next to it makes it
/// part of a longer name (`main.sales.orders` does not name `sales`).
fn names_alias(text: &str, alias: &str) -> bool {
    if alias.is_empty() {
        return false;
    }
    let (t, a) = (text.to_lowercase(), alias.to_lowercase());
    let part =
        |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-'));
    t.match_indices(&a)
        .any(|(i, m)| !part(t[..i].chars().next_back()) && !part(t[i + m.len()..].chars().next()))
}

/// Bottle draws the sources of an answer itself (ADR 0122), so a sources footer that the model
/// wrote goes. A footer line is a "Source: …" line (also as a list item or after `[n]`) that
/// starts with a `[n]` marker or names a call id of `call_ids` or a cited workspace of
/// `aliases`, or a list item that names a call id of `call_ids`. The footer is the last lines of
/// the answer, from the topmost footer line, when every line under it is a footer line, a "[n] …"
/// item, a "Sources" heading or blank. A "Sources" heading, a rule (`---`) and blank lines right
/// above it go too. Any other line stays, so a "Source: main.sales.orders" that Genie wrote, or a
/// list of ids that look like call ids, is kept. Then every call id of `call_ids` that is still in
/// the text goes (with brackets that hold only the id): a call id is internal.
pub(super) fn strip_source_footer(answer: &str, call_ids: &[&str], aliases: &[&str]) -> String {
    static SRC: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static HEAD: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RULE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static ITEM: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let item = ITEM.get_or_init(|| {
        regex::Regex::new(r"^\s*(?:[-*+]\s+)?\[\d{1,3}\](?:[^(]|$)").expect("a fixed pattern")
    });
    let src = SRC.get_or_init(|| {
        regex::Regex::new(
            r"(?i)^\s*(?:[-*+]\s+)?(\[\d{1,3}\]\s*)?[*_]{0,2}\s*sources?[*_]{0,2}\s*:(.*)$",
        )
        .expect("a fixed pattern")
    });
    let head = HEAD.get_or_init(|| {
        regex::Regex::new(r"(?i)^\s*[*_#\s]*sources?[*_]{0,2}\s*:?\s*[*_]{0,2}\s*$")
            .expect("a fixed pattern")
    });
    let rule = RULE.get_or_init(|| {
        regex::Regex::new(r"^\s*(?:-{3,}|\*{3,}|_{3,})\s*$").expect("a fixed pattern")
    });
    let ids: Vec<&str> = call_ids
        .iter()
        .copied()
        .filter(|id| !id.is_empty())
        .collect();
    let has_id = |l: &str| ids.iter().any(|id| l.contains(id));
    let footer_line = |l: &str| {
        if let Some(c) = src.captures(l) {
            let rest = c.get(2).map_or("", |m| m.as_str());
            if c.get(1).is_some() || has_id(rest) || aliases.iter().any(|a| names_alias(rest, a)) {
                return true;
            }
        }
        let listed = item.is_match(l) || l.trim_start().starts_with(['-', '*', '+']);
        listed && has_id(l)
    };
    let lines: Vec<&str> = answer.lines().collect();
    // From the end: the run of footer-like lines, and the topmost footer line in it.
    let mut top: Option<usize> = None;
    for (i, l) in lines.iter().enumerate().rev() {
        if footer_line(l) {
            top = Some(i);
        } else if !(l.trim().is_empty() || item.is_match(l) || head.is_match(l)) {
            break;
        }
    }
    let mut text = match top {
        Some(mut end) => {
            while end > 0 && {
                let l = lines[end - 1];
                l.trim().is_empty() || rule.is_match(l) || head.is_match(l)
            } {
                end -= 1;
            }
            lines[..end].join("\n").trim_end().to_owned()
        }
        None => answer.to_owned(),
    };
    for id in ids {
        let re = regex::Regex::new(&format!(
            r"\s*[(\[]\s*(?:callId:?\s*)?{0}\s*[)\]]|\s*\b(?:callId:?\s*)?{0}\b",
            regex::escape(id)
        ))
        .expect("an escaped id");
        text = re.replace_all(&text, "").into_owned();
    }
    text
}

/// More than one workspace among the sources.
fn cites_many_workspaces(sources: &[AgentSource]) -> bool {
    sources
        .iter()
        .any(|s| s.workspace_id != sources[0].workspace_id)
}

// ---------- the public host ----------

/// The orchestrator component. One actor, one live session.
pub struct OrchestratorHost {
    tx: Sender<Msg>,
    workspaces: Arc<dyn WorkspaceView>,
    approvals: Arc<dyn Approvals>,
    env: Arc<dyn OrchEnv>,
    guard: Arc<dyn OrchGuard>,
}

impl std::fmt::Debug for OrchestratorHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrchestratorHost").finish_non_exhaustive()
    }
}

impl OrchestratorHost {
    /// Start the host and its actor thread. A session starts with the first turn.
    pub fn new(deps: OrchDeps, cfg: OrchConfig) -> Arc<Self> {
        let (tx, rx) = mpsc::channel();
        let host = Arc::new(Self {
            tx: tx.clone(),
            workspaces: Arc::clone(&deps.workspaces),
            approvals: Arc::clone(&deps.approvals),
            env: Arc::clone(&deps.env),
            guard: Arc::clone(&deps.guard),
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
            session: None,
            live: None,
            waiting: WaitQueue::default(),
            pending_end: None,
            results: HashMap::new(),
            result_order: VecDeque::new(),
            results_clear_at: None,
            rate: RateCap::default(),
            last_activity: now,
            last_settings_check: now,
        };
        thread::spawn(move || engine.run(rx));
        host.guard.set_refusal_hook(host.refusal_hook());
        host
    }

    fn ask<T>(&self, mk: impl FnOnce(Reply<T>) -> Msg) -> Option<T> {
        let (r, rx) = mpsc::channel();
        self.tx.send(mk(r)).ok()?;
        rx.recv_timeout(ACTOR_WAIT).ok()
    }

    /// The hook for `AgentGuard::set_refusal_hook`: the third refusal of a turn ends it with the
    /// outcome `refusals`.
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

    /// End the session for `reason` and wait until it is gone. Used by quit and Wipe.
    pub fn end_blocking(&self, reason: AgentSessionEndReason) {
        let _ = self.ask(|r| Msg::End(reason, Some(r)));
    }

    /// Quit: end the session and stop the actor.
    pub fn shutdown(&self) {
        let _ = self.ask(Msg::Shutdown);
    }

    /// Whether an ask can run now: the home profile, its sign-in, the model and the gateway.
    pub fn status(&self, recheck: bool) -> OrchStatus {
        if recheck {
            self.env.clear();
        }
        // A settings file that cannot be read is a fault, not a missing home.
        let Ok(snap) = self.workspaces.snapshot() else {
            return OrchStatus::Unreachable;
        };
        match snap.home_profile {
            None => OrchStatus::NoHome,
            Some(p) => self.env.probe(&p, &snap.model),
        }
    }

    /// The `orchestrator_status` reply: the state and the model that a turn would use.
    pub fn orchestrator_status(&self, recheck: bool) -> OrchestratorStatus {
        OrchestratorStatus {
            state: match self.status(recheck) {
                OrchStatus::Ready => OrchestratorState::Ready,
                OrchStatus::NoHome => OrchestratorState::NoHome,
                OrchStatus::SignedOut => OrchestratorState::SignedOut,
                OrchStatus::NoModel => OrchestratorState::NoModel,
                OrchStatus::Unreachable => OrchestratorState::Unreachable,
            },
            model: self.selected_model(),
        }
    }

    /// The `orchestrator_models` reply: the picker list, the chosen model and the default.
    pub fn orchestrator_models(&self) -> OrchestratorModels {
        let home = self
            .workspaces
            .snapshot()
            .ok()
            .and_then(|s| s.home_profile)
            .unwrap_or_default();
        let models = list_orchestrator_models(&home)
            .into_iter()
            .filter_map(|m| {
                Some(OrchestratorModelInfo {
                    id: OrchestratorModel::try_from(m.id).ok()?,
                    label: m.label,
                    tested: m.tested,
                    tools: m.tools,
                })
            })
            .collect();
        OrchestratorModels {
            models,
            selected: self.selected_model(),
            default_model: default_model(),
        }
    }

    /// The model of the settings, or the default when the settings cannot be read.
    fn selected_model(&self) -> OrchestratorModel {
        self.workspaces
            .snapshot()
            .ok()
            .and_then(|s| OrchestratorModel::try_from(s.model).ok())
            .unwrap_or_else(default_model)
    }

    pub fn submit(&self, a: &AgentTurnSubmitArgs) -> Result<AgentTurnAccepted, &'static str> {
        let mode = a.mode.unwrap_or(AgentAskMode::Ask);
        let mog: &str = &a.mog_id;
        let peek = self.ask(Msg::Peek).ok_or("agent_session_unavailable")?;
        if mode == AgentAskMode::FollowUp && !peek.has_session {
            return Err("agent_session_unavailable");
        }
        // The cheap refusals come first: they do not use up an approval. The check makes no model
        // call, so it runs with an open session too.
        match self.status(false) {
            OrchStatus::Ready => {}
            OrchStatus::NoHome => return Err("agent_profile_unsupported"),
            OrchStatus::SignedOut => return Err("agent_signed_out"),
            OrchStatus::NoModel => return Err("agent_not_eligible"),
            OrchStatus::Unreachable => return Err("agent_offline"),
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

impl Drop for OrchestratorHost {
    fn drop(&mut self) {
        let (r, _rx) = mpsc::channel();
        let _ = self.tx.send(Msg::Shutdown(r));
    }
}

impl AgentWipeHook for OrchestratorHost {
    fn end_for_wipe(&self) {
        self.end_blocking(AgentSessionEndReason::Wipe);
    }
}

fn default_model() -> OrchestratorModel {
    OrchestratorModel::try_from(DEFAULT_ORCHESTRATOR_MODEL.to_owned())
        .expect("the default model id is valid")
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

impl crate::ipc_gateway::GatewayComponents for OrchestratorHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
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
            "orchestrator_status" => {
                let a: OrchestratorStatusArgs = parse(args)?;
                to_value(&self.orchestrator_status(a.recheck.unwrap_or(false)))
            }
            "orchestrator_models" => {
                let _: crate::ipc::NoArgs = parse(args)?;
                to_value(&self.orchestrator_models())
            }
            "agent_session_end" => {
                let _a: AgentSessionEndArgs = parse(args)?;
                self.end_blocking(AgentSessionEndReason::NewSession);
                Ok(json!({}))
            }
            _ => Err(refused("not_implemented")),
        }
    }
}
