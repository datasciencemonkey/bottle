//! The ten `ask_genie` checks (Section 2.3 of the slice 2 plan, with Amendment 2 A4), in order,
//! fail closed. A refusal is a tool result with `isError: true` and one fixed code. A refused ask
//! sends nothing and does not use the budget.
//!
//! 1. The session is open and its live turn runs. Else `run_stopped`, or `no_active_turn`.
//! 2. The arguments are valid. Else `invalid_args`.
//! 3. The alias is in the catalog of the session. Else `workspace_not_available`.
//! 4. `used < budget` (D2, per turn). The reservation counts. Else `budget_reached`.
//! 5. The live [`crate::workspaces::WorkspaceGate`] allows the profile. Else
//!    `workspace_not_available`, and the reservation goes back.
//! 6. One conversation per workspace per turn, one message in flight. A second call to the same
//!    workspace waits as Queued.
//! 7. `genie_ask` goes through the one R8 transport, then the Rust poll loop runs.
//! 8. The result is a text block after the quarantine check ([`super::receipt`]). Cross-workspace
//!    data is always on (ADR 0122 amendment, owner decision 2026-11-24: D1 removed), so every
//!    answered call gets text, never a receipt.
//! 9. One content-free `AgentActionLog` entry. It is written before the send: no entry, no call.
//!
//! The third refusal of a turn stops it (`refusals`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use regex::Regex;
use serde_json::Value;

use super::genie_poll::{self, is_terminal, PollEnd, PollEnv};
use super::ledger::{new_call_id, send_cancel, tags_of, CallRec, SessionRec, State};
use super::receipt::{self, Outcome, ToolOut};
use super::{
    AgentGuard, MAX_QUESTION_CHARS, MAX_REFUSALS, MAX_WORKSPACE_CHARS, NO_ACTIVE_TURN, RUN_STOPPED,
};
use crate::agent_guard::AUDIT_UNAVAILABLE;
use crate::agent_run::seam::{AgentEvent, StopWhy};
use crate::error::CoreError;
use crate::ipc::JsonObject;
use crate::local_store::quarantine::quarantine_reasons;
use crate::mcp_transport::agent_asks::ASK_GIVEN_UP;
use crate::mcp_transport::McpCall;
use crate::types::{
    AgentCallState, AgentRefusal, AgentTurnEvent, AgentWithheld, McpCallReply, ProfileName,
};

/// How often a waiting call looks again.
const WAIT_SLICE: Duration = Duration::from_millis(100);

fn refusal_code(c: AgentRefusal) -> &'static str {
    match c {
        AgentRefusal::WorkspaceNotAvailable => "workspace_not_available",
        AgentRefusal::BudgetReached => "budget_reached",
        AgentRefusal::InvalidArgs => "invalid_args",
    }
}

/// Check 2: `workspace` is 1 to 32 chars, `question` is 1 to 1,000 chars with no control char
/// other than a new line, and there is no other key.
fn parse_args(args: &Value) -> Option<(String, String)> {
    let o = args.as_object()?;
    if o.len() != 2 {
        return None;
    }
    let w = o.get("workspace")?.as_str()?;
    let q = o.get("question")?.as_str()?;
    let w_len = w.chars().count();
    let q_len = q.chars().count();
    if w_len == 0 || w_len > MAX_WORKSPACE_CHARS || w.chars().any(char::is_control) {
        return None;
    }
    if q_len == 0
        || q_len > MAX_QUESTION_CHARS
        || q.chars().any(|c| c.is_control() && c != '\n')
        || q.trim().is_empty()
    {
        return None;
    }
    Some((w.to_owned(), q.to_owned()))
}

/// The links of a text: `http(s)://…`, `www.…` and a bare domain with a path, lower case,
/// trailing punctuation cut.
fn links_of(text: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    // The pattern is a constant; a test runs it.
    let re = RE.get_or_init(|| {
        // A scheme or `www.` link, or a bare domain with a path (`evil.example/x`).
        Regex::new(
            r#"(?i)\b(?:https?://|www\.)[^\s<>"'`)\]]+|\b[a-z0-9-]+(?:\.[a-z0-9-]+)+/[^\s<>"'`)\]]*"#,
        )
        .expect("link pattern")
    });
    re.find_iter(text)
        .map(|m| {
            m.as_str()
                .trim_end_matches(['.', ',', ';', ':', '!', '?'])
                .to_lowercase()
        })
        .collect()
}

/// Check 2b (defence in depth): a model-written `ask_genie` question that has a link the user's
/// question does not have, or an order to send data elsewhere that the user did not give.
fn question_reaches_out(question: &str, user_question: &str) -> bool {
    let user = user_question.to_lowercase();
    if links_of(question)
        .iter()
        .any(|l| !user.contains(l.as_str()))
    {
        return true;
    }
    let sends = |t: &str| quarantine_reasons(t).contains(&"send_elsewhere");
    sends(question) && !sends(user_question)
}

fn state_of_code(code: &str) -> AgentCallState {
    match code {
        "timeout" => AgentCallState::Timeout,
        "login_required" | "auth_expired" | "token_unusable" => AgentCallState::SignedOut,
        _ => AgentCallState::Failed,
    }
}

/// A call that passed checks 1 to 6.
struct Admitted {
    turn_id: String,
    call_id: String,
    alias: String,
    profile: ProfileName,
    question: String,
    cancel: Arc<AtomicBool>,
}

/// Check 1 on a locked state: the live turn of the open session, if it may still ask.
fn live_turn(st: &State, sid: &str) -> Result<String, &'static str> {
    let Some(sess) = st.sessions.get(sid).filter(|s| s.is_open()) else {
        return Err(RUN_STOPPED);
    };
    let Some(turn_id) = sess.live.clone() else {
        return Err(NO_ACTIVE_TURN);
    };
    match sess.turns.get(&turn_id) {
        Some(t) if t.stop.is_none() => Ok(turn_id),
        _ => Err(RUN_STOPPED),
    }
}

/// A call id that no call of the session has. The session looks calls up by id alone, so a repeat
/// would mix two calls. A few fresh ids are tried; if random bytes fail (every id is
/// `c-00000000`), a counter gives the next free id.
pub(super) fn unique_call_id(sess: &SessionRec) -> String {
    unique_call_id_with(sess, new_call_id)
}

pub(super) fn unique_call_id_with(sess: &SessionRec, mut mint: impl FnMut() -> String) -> String {
    let taken = |id: &str| sess.calls.iter().any(|c| c.call_id == id);
    let mut id = mint();
    let mut k: u32 = 0;
    while taken(&id) {
        k += 1;
        id = if k <= 4 { mint() } else { format!("c-{k:08x}") };
    }
    id
}

fn call_mut<'a>(sess: &'a mut SessionRec, call_id: &str) -> Option<&'a mut CallRec> {
    sess.calls.iter_mut().find(|c| c.call_id == call_id)
}

impl AgentGuard {
    /// Serve one `ask_genie` call of the model, bound to `turn_id`. Check 1 needs that turn to be
    /// the live one. A worker of a stopped turn that arrives late is refused as `run_stopped`
    /// and cannot spend the budget of the next turn. It blocks until the call ends.
    pub fn ask_genie_for(&self, session_id: &str, turn_id: &str, args: &Value) -> ToolOut {
        let a = match self.admit(session_id, turn_id, args) {
            Ok(a) => a,
            Err(out) => return out,
        };
        self.run_call(session_id, &a)
    }

    /// A refusal of a turn: count it, tell the brain, and stop the turn at the third one.
    fn refuse(&self, sid: &str, turn_id: &str, code: AgentRefusal) -> ToolOut {
        let mut trip = false;
        let mut events = Vec::new();
        {
            let mut st = self.lock();
            if let Some(t) = st
                .sessions
                .get_mut(sid)
                .and_then(|s| s.turns.get_mut(turn_id))
            {
                t.refusals += 1;
                events.push(AgentEvent::Turn(AgentTurnEvent::Refused {
                    turn_id: turn_id.to_owned(),
                    code,
                    used: t.used,
                }));
                if t.refusals >= MAX_REFUSALS && t.stop.is_none() {
                    t.stop = Some(StopWhy::Failed);
                    trip = true;
                }
            }
        }
        self.emit(events);
        if trip {
            self.cv.notify_all();
            self.fire_refusal_hook(turn_id);
            // The refusal that stops the turn reads as a stop, so the caller stops at once.
            return ToolOut::refusal(RUN_STOPPED);
        }
        ToolOut::refusal(refusal_code(code))
    }

    /// Checks 1 to 6, then the call record in the state Queued.
    fn admit(&self, sid: &str, bound: &str, args: &Value) -> Result<Admitted, ToolOut> {
        // 1
        let turn_id = live_turn(&self.lock(), sid).map_err(ToolOut::refusal)?;
        if bound != turn_id {
            return Err(ToolOut::refusal(RUN_STOPPED));
        }
        // 2
        let Some((alias, question)) = parse_args(args) else {
            return Err(self.refuse(sid, &turn_id, AgentRefusal::InvalidArgs));
        };
        // 2b: the model wrote this question. It may not reach out further than the user did.
        let user_q = self
            .lock()
            .sessions
            .get(sid)
            .and_then(|s| s.turns.get(&turn_id))
            .map(|t| t.user_question.clone())
            .unwrap_or_default();
        if question_reaches_out(&question, &user_q) {
            return Err(self.refuse(sid, &turn_id, AgentRefusal::InvalidArgs));
        }
        // 3, 4 on one lock: the reservation is atomic with the checks.
        enum Verdict {
            Stopped(&'static str),
            Refuse(AgentRefusal),
            Go(ProfileName),
        }
        let verdict = {
            let mut st = self.lock();
            match live_turn(&st, sid) {
                Ok(t) if t == turn_id => {}
                Ok(_) => return Err(ToolOut::refusal(RUN_STOPPED)),
                Err(c) => return Err(ToolOut::refusal(c)),
            }
            let Some(sess) = st.sessions.get_mut(sid) else {
                return Err(ToolOut::refusal(RUN_STOPPED));
            };
            let entry = sess.spec.catalog.iter().find(|c| c.alias == alias).cloned();
            match entry {
                None => Verdict::Refuse(AgentRefusal::WorkspaceNotAvailable),
                Some(e) => {
                    let turn = sess.turns.get_mut(&turn_id);
                    match turn {
                        Some(t) if t.used < u32::from(t.budget) => {
                            t.used += 1;
                            Verdict::Go(e.profile)
                        }
                        Some(_) => Verdict::Refuse(AgentRefusal::BudgetReached),
                        None => Verdict::Stopped(RUN_STOPPED),
                    }
                }
            }
        };
        let profile = match verdict {
            Verdict::Stopped(c) => return Err(ToolOut::refusal(c)),
            Verdict::Refuse(c) => return Err(self.refuse(sid, &turn_id, c)),
            Verdict::Go(p) => p,
        };
        // 5: the live gate, so a workspace switched off applies at once. The reservation goes back.
        if !self.deps.gate.allows(&profile) {
            if let Some(t) = self
                .lock()
                .sessions
                .get_mut(sid)
                .and_then(|s| s.turns.get_mut(&turn_id))
            {
                t.used = t.used.saturating_sub(1);
            }
            return Err(self.refuse(sid, &turn_id, AgentRefusal::WorkspaceNotAvailable));
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (event, call_id) = {
            let mut st = self.lock();
            let Some(sess) = st.sessions.get_mut(sid).filter(|s| s.is_open()) else {
                return Err(ToolOut::refusal(RUN_STOPPED));
            };
            let call_id = unique_call_id(sess);
            let (used, budget) = sess
                .turns
                .get(&turn_id)
                .map_or((0, 0), |t| (t.used, u32::from(t.budget)));
            sess.calls.push(CallRec {
                call_id: call_id.clone(),
                turn_id: turn_id.clone(),
                alias: alias.clone(),
                profile: profile.clone(),
                question: question.clone(),
                state: AgentCallState::Queued,
                reply: None,
                withheld: None,
                conversation: None,
                open: false,
                cancel: Arc::clone(&cancel),
                cancel_sent: false,
            });
            let event = call_event(
                &turn_id,
                &call_id,
                &alias,
                AgentCallState::Queued,
                used,
                budget,
            );
            (event, call_id)
        };
        self.emit(vec![event]);
        Ok(Admitted {
            turn_id,
            call_id,
            alias,
            profile,
            question,
            cancel,
        })
    }

    /// Set the state of a call and tell the brain.
    fn set_state(&self, sid: &str, a: &Admitted, state: AgentCallState) {
        let event = {
            let mut st = self.lock();
            let Some(sess) = st.sessions.get_mut(sid) else {
                return;
            };
            let (used, budget) = sess
                .turns
                .get(&a.turn_id)
                .map_or((0, 0), |t| (t.used, u32::from(t.budget)));
            if let Some(c) = call_mut(sess, &a.call_id) {
                c.state = state;
            }
            call_event(&a.turn_id, &a.call_id, &a.alias, state, used, budget)
        };
        self.emit(vec![event]);
    }

    /// Check 6: wait for the place of the workspace in this turn. `Err` is the result of a call
    /// that was stopped before it was sent.
    fn take_slot(&self, sid: &str, a: &Admitted) -> Result<Option<String>, ToolOut> {
        let mut st = self.lock();
        loop {
            let Some(sess) = st.sessions.get_mut(sid).filter(|s| s.is_open()) else {
                return Err(ToolOut::refusal(RUN_STOPPED));
            };
            let turn_stopped = sess.turns.get(&a.turn_id).is_none_or(|t| t.stop.is_some());
            if a.cancel.load(Ordering::SeqCst) || turn_stopped {
                if let Some(c) = call_mut(sess, &a.call_id) {
                    c.state = AgentCallState::Stopped;
                }
                drop(st);
                self.set_state(sid, a, AgentCallState::Stopped);
                return Err(ToolOut::refusal(RUN_STOPPED));
            }
            let slot = sess
                .turns
                .get_mut(&a.turn_id)
                .map(|t| t.slots.entry(a.alias.clone()).or_default());
            if let Some(slot) = slot {
                if !slot.busy {
                    slot.busy = true;
                    return Ok(slot.conversation.clone());
                }
            }
            st = self
                .cv
                .wait_timeout(st, WAIT_SLICE)
                .unwrap_or_else(|p| p.into_inner())
                .0;
        }
    }

    /// Checks 7 to 9 for one admitted call.
    fn run_call(&self, sid: &str, a: &Admitted) -> ToolOut {
        let conversation = match self.take_slot(sid, a) {
            Ok(c) => c,
            Err(out) => return out,
        };
        // 9: the entry is written before the send. No entry, no call.
        let args_bytes = (a.alias.len() + a.question.len()) as u64;
        if self
            .deps
            .action_log
            .record(sid, "genie_one", args_bytes)
            .is_err()
        {
            return self.finish(
                sid,
                a,
                conversation,
                Ended {
                    refuse_code: Some(AUDIT_UNAVAILABLE),
                    ..Ended::state(AgentCallState::Failed)
                },
            );
        }
        self.set_state(sid, a, AgentCallState::Asking);
        // G-11a..c: a cap makes the call wait, it does not refuse it.
        let place = loop {
            if a.cancel.load(Ordering::SeqCst) {
                return self.finish(sid, a, conversation, Ended::state(AgentCallState::Stopped));
            }
            match self.asks.begin(sid, &a.profile, conversation.as_deref()) {
                Ok(p) => break p,
                Err(ASK_GIVEN_UP) => {
                    return self.finish(sid, a, conversation, Ended::state(AgentCallState::Failed))
                }
                Err(_) => self.opts.sleeper.sleep(WAIT_SLICE * 2, &a.cancel),
            }
        };
        // 7: the send, through the one R8.
        let mut args = JsonObject::new();
        args.insert("question".into(), a.question.clone().into());
        if let Some(c) = &conversation {
            args.insert("conversation_id".into(), c.clone().into());
        }
        let tags = tags_of(sid);
        let asked_at = self.deps.clock.now();
        let sent =
            self.deps
                .transport
                .call_tool(McpCall::new(&a.profile, "genie_ask", args, &tags));
        let reply = match sent {
            Err(e) => {
                self.asks.sent(place, Err(e.code()));
                return self.finish(sid, a, conversation, Ended::code(&e));
            }
            Ok(r) if r.is_error => {
                self.asks.sent(place, Err("genie_error"));
                return self.finish(sid, a, conversation, Ended::state(AgentCallState::Failed));
            }
            Ok(r) => {
                self.asks.sent(place, Ok(&r.structured_content));
                r
            }
        };
        let sc = &reply.structured_content;
        let conv = sc
            .get("conversation_id")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if is_terminal(sc) {
            // The reply holds the final answer: no poll.
            return self.finish(
                sid,
                a,
                conv.or(conversation),
                Ended::outcome(receipt::outcome_of_terminal(reply)),
            );
        }
        let (Some(conv), Some(response)) = (
            conv,
            sc.get("response_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ) else {
            return self.finish(sid, a, conversation, Ended::state(AgentCallState::Failed));
        };
        // The call is open now: a stop can cancel it. The poll loop reads the cancel flag.
        if let Some(c) = self
            .lock()
            .sessions
            .get_mut(sid)
            .and_then(|s| call_mut(s, &a.call_id))
        {
            c.conversation = Some(conv.clone());
            c.open = true;
        }
        let end = {
            let inflight = || self.inflight_on(&a.profile);
            let env = PollEnv {
                transport: self.deps.transport.as_ref(),
                clock: self.deps.clock.as_ref(),
                sleeper: self.opts.sleeper.as_ref(),
                random: self.opts.random.as_ref(),
                asks: &self.asks,
                tags: &tags,
                profile: &a.profile,
                inflight: &inflight,
                cancel: &a.cancel,
            };
            genie_poll::run(&env, &conv, &response, asked_at)
        };
        let ended = match end {
            PollEnd::Terminal(r) => Ended::outcome(receipt::outcome_of_terminal(r)),
            PollEnd::GaveUp => Ended::state(AgentCallState::Timeout),
            PollEnd::Cancelled => {
                self.cancel_remote_once(sid, a, &conv);
                Ended::state(AgentCallState::Stopped)
            }
            PollEnd::Failed(code) => Ended::state(state_of_code(code)),
        };
        self.finish(sid, a, Some(conv), ended)
    }

    /// The calls of the profile that are sent and not ended, for the G-08 floor.
    fn inflight_on(&self, profile: &str) -> usize {
        self.lock()
            .sessions
            .values()
            .flat_map(|s| s.calls.iter())
            .filter(|c| c.open && c.profile == profile)
            .count()
    }

    /// Send `genie_cancel_response` for a call unless a stop did it already.
    fn cancel_remote_once(&self, sid: &str, a: &Admitted, conversation: &str) {
        let send = {
            let mut st = self.lock();
            st.sessions
                .get_mut(sid)
                .and_then(|s| call_mut(s, &a.call_id))
                .is_some_and(|c| !std::mem::replace(&mut c.cancel_sent, true))
        };
        if send {
            send_cancel(
                self.deps.transport.as_ref(),
                &self.asks,
                sid,
                &a.profile,
                conversation,
            );
        }
    }

    /// Check 9: end the call, free the place, and build the result for the model.
    fn finish(
        &self,
        sid: &str,
        a: &Admitted,
        conversation: Option<String>,
        ended: Ended,
    ) -> ToolOut {
        let mut out = ToolOut::refusal(RUN_STOPPED);
        let ws_url = self.deps.gate.workspace_url(&a.profile);
        let event = {
            let mut st = self.lock();
            if let Some(sess) = st.sessions.get_mut(sid) {
                let (used, budget) = sess
                    .turns
                    .get(&a.turn_id)
                    .map_or((0, 0), |t| (t.used, u32::from(t.budget)));
                let mut state = ended.state;
                let mut reply: Option<McpCallReply> = None;
                let mut withheld: Option<AgentWithheld> = None;
                match ended.outcome {
                    Some(o) => {
                        reply = o.reply.as_ref().map(McpCallReply::from);
                        // Cross-workspace data is always on (ADR 0122 amendment, owner decision
                        // 2026-11-24: D1 removed), so every answered call gets text, never a
                        // receipt.
                        let (text, w) =
                            receipt::text_block(&a.call_id, &a.alias, &o, ws_url.as_deref());
                        if w.is_some() {
                            // The user view hides a withheld reply.
                            reply = None;
                        }
                        withheld = w;
                        state = o.state;
                        out = text;
                    }
                    None => out = receipt::unanswered(&a.call_id, &a.alias, state),
                }
                if let Some(code) = ended.refuse_code {
                    out = ToolOut::refusal(code);
                }
                if let Some(c) = call_mut(sess, &a.call_id) {
                    c.open = false;
                    c.state = state;
                    c.reply = reply;
                    c.withheld = withheld;
                    if conversation.is_some() {
                        c.conversation.clone_from(&conversation);
                    }
                }
                if let Some(slot) = sess
                    .turns
                    .get_mut(&a.turn_id)
                    .and_then(|t| t.slots.get_mut(&a.alias))
                {
                    slot.busy = false;
                    if conversation.is_some() {
                        slot.conversation = conversation;
                    }
                }
                Some(call_event(
                    &a.turn_id, &a.call_id, &a.alias, state, used, budget,
                ))
            } else {
                None
            }
        };
        self.cv.notify_all();
        if let Some(e) = event {
            self.emit(vec![e]);
        }
        out
    }
}

/// How a call ended, before the result is built.
struct Ended {
    state: AgentCallState,
    outcome: Option<Outcome>,
    refuse_code: Option<&'static str>,
}

impl Ended {
    fn state(state: AgentCallState) -> Self {
        Self {
            state,
            outcome: None,
            refuse_code: None,
        }
    }

    fn code(e: &CoreError) -> Self {
        Self::state(state_of_code(e.code()))
    }

    fn outcome(o: Outcome) -> Self {
        Self {
            state: o.state,
            outcome: Some(o),
            refuse_code: None,
        }
    }
}

fn call_event(
    turn_id: &str,
    call_id: &str,
    alias: &str,
    state: AgentCallState,
    used: u32,
    budget: u32,
) -> AgentEvent {
    AgentEvent::Turn(AgentTurnEvent::Call {
        turn_id: turn_id.to_owned(),
        call_id: call_id.to_owned(),
        workspace_id: alias.to_owned(),
        state,
        used,
        budget,
    })
}
