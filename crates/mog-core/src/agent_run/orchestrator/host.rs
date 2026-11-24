//! The orchestrator host: the ask loop of ADR 0122 behind the public API of the old session host.
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
    AgentTurnSubmitArgs,
};
use crate::model_gateway::{ChatClient, ChatRequest};
use crate::types::{
    AgentAnswerKind, AgentCallState, AgentSessionEndReason, AgentSessionEvent, AgentSessionPhase,
    AgentSessionState, AgentSource, AgentTurnAccepted, AgentTurnCall, AgentTurnEvent,
    AgentTurnOutcome, AgentTurnResult, AgentUsage, AgentWorkspace, GenieCallBudget,
};
use crate::wipe::AgentWipeHook;

/// The most characters of the model's `answer` that Bottle keeps.
pub const ANSWER_MAX_CHARS: usize = 1200;
/// The most bytes of the final model message.
pub const FINAL_MAX_BYTES: usize = 32 * 1024;
/// The most model steps (chat calls) of one turn.
pub const MAX_STEPS: usize = 12;
/// The most earlier turns of a thread that go back to the model.
pub const HISTORY_TURNS: usize = 8;
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
}

/// The guard as the host sees it: the seam of the session, plus the in-process `ask_genie`.
pub trait OrchGuard: SessionGuard {
    /// Run one `ask_genie` call with the same checks as the shim path. Blocks until it ends.
    fn ask_genie(&self, session_id: &str, args: &Value) -> ToolOut;
}

impl OrchGuard for AgentGuard {
    fn ask_genie(&self, session_id: &str, args: &Value) -> ToolOut {
        AgentGuard::ask_genie(self, session_id, args)
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
    mode: AgentAskMode,
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
            mode: req.mode,
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
        // The run secret is for the shim only. There is no shim now: drop it at once.
        drop(
            self.d
                .guard
                .open_session(spec)
                .map_err(|_| "agent_session_unavailable")?,
        );
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
        let (turn_id, question, mode, cancel) = {
            let l = self.live.as_ref().expect("checked above");
            (
                l.turn.id.clone(),
                l.turn.question.clone(),
                l.turn.mode,
                Arc::clone(&l.cancel),
            )
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
        let system = prompt::system(&PromptInput {
            catalog: &catalog,
            budget,
            cross_workspace: cross,
            agent_alias: agent_alias.as_deref(),
            skip_home: mode == AgentAskMode::AskOthers,
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
        let ctx = LoopCtx {
            chat: Arc::clone(&self.d.chat),
            guard: Arc::clone(&self.d.guard),
            net: Arc::clone(&self.d.net),
            clock: Arc::clone(&self.d.clock),
            session_id: sid,
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
            let out = run_loop(ctx);
            let _ = tx.send(Msg::Loop(tid, out));
        });
        self.emit_turn(AgentTurnEvent::Started {
            turn_id: turn_id.clone(),
        });
        self.emit_turn(AgentTurnEvent::Thinking { turn_id });
    }

    /// Stop the live turn for `why`: cancel the worker, tell the guard (it cancels the open
    /// Genie polls for `User`, `Lockdown`, `Failed` and `SessionEnded`), and end the turn.
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
            LoopStop::Cancelled => {}
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
            self.store_result(turn_id.clone(), r);
            if let Some(s) = self.session.as_mut() {
                s.history.push_back(std::mem::take(&mut live.new_messages));
                while s.history.len() > HISTORY_TURNS {
                    s.history.pop_front();
                }
            }
        }
        if live.began {
            self.d.guard.end_turn(&turn_id);
        }
        let tokens = live.usage.input + live.usage.output;
        if let Some(s) = self.session.as_mut() {
            if live.began {
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

/// The one tool, as a Responses-API function tool.
fn tools() -> Value {
    let s = guard::tool_schema();
    json!([{
        "type": "function",
        "name": guard::TOOL_NAME,
        "description": s["description"],
        "parameters": s["inputSchema"],
    }])
}

fn text_format() -> Value {
    let schema: Value = serde_json::from_str(ANSWER_SCHEMA).unwrap_or(Value::Null);
    json!({
        "format": {
            "type": "json_schema",
            "name": "bottle_answer",
            "strict": true,
            "schema": schema,
        }
    })
}

/// The most output tokens of one model step.
const MAX_OUTPUT_TOKENS: u32 = 2048;

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
    /// The `function_call` items, in order. An item with no `call_id` gets a made-up one.
    pub(super) calls: Vec<FnCall>,
    /// The text of the `message` items, joined.
    pub(super) text: Option<String>,
}

/// Read a Responses reply. `None` when the reply is not a finished answer: a failed or
/// incomplete reply, or one with no `output` array.
pub(super) fn parse_reply(resp: &Value) -> Option<Reply1> {
    if resp["error"].is_object() || matches!(resp["status"].as_str(), Some("failed" | "incomplete"))
    {
        return None;
    }
    let items = resp["output"].as_array()?;
    let mut calls = Vec::new();
    let mut text = String::new();
    let mut has_text = false;
    for it in items {
        match it["type"].as_str() {
            Some("function_call") => calls.push(FnCall {
                call_id: it["call_id"]
                    .as_str()
                    .map_or_else(|| format!("call_{}", calls.len()), str::to_owned),
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

/// The `function_call_output` item for a call. A call that is not `ask_genie` with a JSON object
/// as arguments is never run: `Value::Null` goes to the guard, which refuses it as `invalid_args`
/// and counts the refusal (the third one stops the turn).
fn run_tool_call(ctx: &LoopCtx, call: &FnCall) -> Value {
    let args = serde_json::from_str::<Value>(&call.arguments)
        .ok()
        .filter(Value::is_object);
    let run = match (call.name == guard::TOOL_NAME, args) {
        (true, Some(a)) => a,
        _ => Value::Null,
    };
    let out = ctx.guard.ask_genie(&ctx.session_id, &run);
    json!({ "type": "function_call_output", "call_id": call.call_id, "output": out.text })
}

fn run_loop(mut ctx: LoopCtx) -> LoopOut {
    let mut usage = Usage::default();
    let tools = tools();
    let format = text_format();
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
                "text": format,
                "max_output_tokens": MAX_OUTPUT_TOKENS,
            }),
            deadline: left,
        });
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                let o = match e.code() {
                    "login_required" | "auth_expired" | "token_unusable" => {
                        AgentTurnOutcome::SignedOut
                    }
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
        // The items of the reply go back as they came, so the next step sees its own calls.
        if let Some(items) = resp["output"].as_array() {
            ctx.input.extend(items.iter().cloned());
        }
        if reply.calls.is_empty() {
            return match reply.text {
                Some(t) => end(&ctx, LoopStop::Final(t), usage),
                None => end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage),
            };
        }
        // The step cap: a last step that still asks for tools is a failure. Nothing runs.
        if step + 1 == MAX_STEPS {
            return end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage);
        }
        let results: Vec<Value> = thread::scope(|sc| {
            let hs: Vec<_> = reply
                .calls
                .iter()
                .map(|c| sc.spawn(|| run_tool_call(&ctx, c)))
                .collect();
            hs.into_iter()
                .zip(&reply.calls)
                .map(|(h, c)| {
                    h.join().unwrap_or_else(|_| {
                        json!({
                            "type": "function_call_output",
                            "call_id": c.call_id,
                            "output": "run_stopped",
                        })
                    })
                })
                .collect()
        });
        ctx.input.extend(results);
    }
    end(&ctx, LoopStop::Failed(AgentTurnOutcome::Failed), usage)
}

// ---------- helpers ----------

fn outcome_of_stop(why: StopWhy) -> AgentTurnOutcome {
    match why {
        StopWhy::User => AgentTurnOutcome::Stopped,
        StopWhy::Timeout => AgentTurnOutcome::Timeout,
        StopWhy::Offline => AgentTurnOutcome::Offline,
        StopWhy::Lockdown => AgentTurnOutcome::Lockdown,
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
    AgentTurnResult {
        answer: Some(answer.chars().take(ANSWER_MAX_CHARS).collect()),
        answer_kind: if d1 && cites_many_workspaces(&out) {
            AgentAnswerKind::Combined
        } else {
            AgentAnswerKind::Note
        },
        sources: out,
        usage,
    }
}

/// More than one workspace among the sources.
fn cites_many_workspaces(sources: &[AgentSource]) -> bool {
    sources
        .iter()
        .any(|s| s.workspace_id != sources[0].workspace_id)
}

// ---------- the public host ----------

/// The orchestrator component. The API matches the old `SessionHost`, minus the process parts.
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
    pub fn status(&self, _recheck: bool) -> OrchStatus {
        let Ok(snap) = self.workspaces.snapshot() else {
            return OrchStatus::NoHome;
        };
        match snap.home_profile {
            None => OrchStatus::NoHome,
            Some(p) => self.env.probe(&p, &snap.model),
        }
    }

    pub fn submit(&self, a: &AgentTurnSubmitArgs) -> Result<AgentTurnAccepted, &'static str> {
        let mode = a.mode.unwrap_or(AgentAskMode::Ask);
        let mog: &str = &a.mog_id;
        let peek = self.ask(Msg::Peek).ok_or("agent_session_unavailable")?;
        if mode == AgentAskMode::FollowUp && !peek.has_session {
            return Err("agent_session_unavailable");
        }
        if !peek.has_session {
            // The cheap refusals come first: they do not use up an approval.
            match self.status(false) {
                OrchStatus::Ready => {}
                OrchStatus::NoHome => return Err("agent_profile_unsupported"),
                OrchStatus::SignedOut => return Err("agent_signed_out"),
                OrchStatus::NoModel => return Err("agent_not_eligible"),
                OrchStatus::Unreachable => return Err("agent_offline"),
            }
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
            "agent_session_end" => {
                let _a: AgentSessionEndArgs = parse(args)?;
                self.end_blocking(AgentSessionEndReason::NewSession);
                Ok(json!({}))
            }
            _ => Err(refused("not_implemented")),
        }
    }
}
