//! The session ledger (A4): sessions, turns, the call log and the stop rules.
//!
//! One [`State`] holds every session in memory. It keeps ids, counts, states and, for the user
//! view (`call_detail`), the question and the reply of each call. `Debug` shows none of the text.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::super::seam::{CallRecord, SessionGuard, SessionSpec, StopWhy, TurnSpec};
use super::AgentGuard;
use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::mcp_transport::{McpCall, McpTransport};
use crate::types::{
    AgentCallState, AgentTurnCall, AgentWithheld, McpCallReply, ProfileName, TagApp, TagOp, Tags,
};

const COMPONENT: &str = "agent_guard";

fn unavailable(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: COMPONENT,
        reason,
    }
}

/// One Genie call of a session.
pub(crate) struct CallRec {
    pub call_id: String,
    pub turn_id: String,
    pub alias: String,
    pub profile: ProfileName,
    pub question: String,
    pub state: AgentCallState,
    pub reply: Option<McpCallReply>,
    pub withheld: Option<AgentWithheld>,
    pub conversation: Option<String>,
    /// The ask went out and no terminal state is in yet.
    pub open: bool,
    /// Turns true when the call must stop. The poll loop and the slot wait watch it.
    pub cancel: Arc<AtomicBool>,
    pub cancel_sent: bool,
}

impl fmt::Debug for CallRec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CallRec")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// The one Genie conversation of a workspace in a turn, and whether a message is in flight.
#[derive(Default)]
pub(crate) struct Slot {
    pub busy: bool,
    pub conversation: Option<String>,
}

pub(crate) struct TurnRec {
    pub budget: u8,
    /// The user's question of the turn (memory only, never logged).
    pub user_question: String,
    pub used: u32,
    pub refusals: u32,
    pub stop: Option<StopWhy>,
    pub slots: HashMap<String, Slot>,
}

pub(crate) struct SessionRec {
    pub spec: SessionSpec,
    pub live: Option<String>,
    pub turns: HashMap<String, TurnRec>,
    pub calls: Vec<CallRec>,
    pub closed_at: Option<Duration>,
}

impl SessionRec {
    pub fn is_open(&self) -> bool {
        self.closed_at.is_none()
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub sessions: HashMap<String, SessionRec>,
}

impl fmt::Debug for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("State")
            .field("sessions", &self.sessions.len())
            .finish()
    }
}

/// The ids of a session become the `mog` tag of R8, so they are plain.
fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// A new call id in the wire form of `ipc::AgentCallId` (`c-` and 8 lowercase hex characters).
/// The `agent:turn` events, `agent_turn_call` and the brain accept only that form: a call id in
/// any other form is dropped, and the turn then has no parts, citations or links.
pub(crate) fn new_call_id() -> String {
    let mut b = [0u8; 4];
    // A failed read leaves zeros (`c-00000000`). The id must still be unique in its session:
    // `ask::unique_call_id` mints again, under the guard lock, while the id is taken.
    let _ = getrandom::fill(&mut b);
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("c-{hex}")
}

pub(crate) fn tags_of(session_id: &str) -> Tags {
    Tags {
        app: TagApp::Bottle,
        mog: session_id.to_owned(),
        op: TagOp::Agent,
    }
}

/// Send `genie_cancel_response` for `conversation`, and end its place in the ask ledger. The
/// reply is not used: a cancel is best effort.
pub(crate) fn send_cancel(
    transport: &dyn McpTransport,
    asks: &crate::mcp_transport::agent_asks::AgentAsks,
    session_id: &str,
    profile: &str,
    conversation: &str,
) {
    let tags = tags_of(session_id);
    let mut args = JsonObject::new();
    args.insert("conversation_id".into(), conversation.into());
    let _ = transport.call_tool(McpCall::new(profile, "genie_cancel_response", args, &tags));
    asks.end(conversation);
}

impl AgentGuard {
    /// Mark each call of `turn_id` that the stop `why` ends, and return the remote cancels to
    /// send: `(profile, conversation)`. A call that was not sent is always stopped. A sent call
    /// is stopped only when `why` cancels (`Timeout` and `Offline` keep it polling).
    pub(crate) fn stop_calls(
        sess: &mut SessionRec,
        turn_id: &str,
        why: StopWhy,
    ) -> Vec<(ProfileName, String)> {
        let cancels = !matches!(why, StopWhy::Timeout | StopWhy::Offline);
        let mut out = Vec::new();
        for c in sess.calls.iter_mut().filter(|c| c.turn_id == turn_id) {
            // A call that waits for its place never goes out after a stop. A call that went out
            // (or is on its way) stops only when `why` cancels.
            let waiting = matches!(c.state, AgentCallState::Queued);
            let live = matches!(c.state, AgentCallState::Asking);
            if waiting || (live && cancels) {
                c.cancel.store(true, Ordering::SeqCst);
            }
            if c.open && cancels && !c.cancel_sent {
                if let Some(conv) = &c.conversation {
                    c.cancel_sent = true;
                    out.push((c.profile.clone(), conv.clone()));
                }
            }
        }
        out
    }

    pub(crate) fn spawn_cancels(&self, session_id: &str, cancels: Vec<(ProfileName, String)>) {
        if cancels.is_empty() {
            return;
        }
        let transport = Arc::clone(&self.deps.transport);
        let asks = Arc::clone(&self.asks);
        let sid = session_id.to_owned();
        let _ = std::thread::Builder::new()
            .name("bottle-agent-cancel".into())
            .spawn(move || {
                for (profile, conv) in cancels {
                    send_cancel(transport.as_ref(), &asks, &sid, &profile, &conv);
                }
            });
    }
}

impl SessionGuard for AgentGuard {
    fn open_session(&self, spec: SessionSpec) -> CoreResult<()> {
        if !valid_id(&spec.session_id) {
            return Err(unavailable("session_id_invalid"));
        }
        let mut st = self.lock();
        if st.sessions.contains_key(&spec.session_id) {
            return Err(unavailable("session_exists"));
        }
        // One live session at a time (A5).
        if st.sessions.values().any(SessionRec::is_open) {
            return Err(unavailable("session_open"));
        }
        let id = spec.session_id.clone();
        st.sessions.insert(
            id,
            SessionRec {
                spec,
                live: None,
                turns: HashMap::new(),
                calls: Vec::new(),
                closed_at: None,
            },
        );
        Ok(())
    }

    fn begin_turn(&self, session_id: &str, turn: TurnSpec) -> CoreResult<()> {
        let mut st = self.lock();
        let sess = st
            .sessions
            .get_mut(session_id)
            .filter(|s| s.is_open())
            .ok_or_else(|| unavailable("session_unknown"))?;
        if sess.live.is_some() {
            return Err(unavailable("turn_live"));
        }
        if sess.turns.contains_key(&turn.turn_id) {
            return Err(unavailable("turn_exists"));
        }
        sess.live = Some(turn.turn_id.clone());
        sess.turns.insert(
            turn.turn_id,
            TurnRec {
                budget: turn.budget,
                user_question: turn.user_question,
                used: 0,
                refusals: 0,
                stop: None,
                slots: HashMap::new(),
            },
        );
        Ok(())
    }

    fn stop_turn(&self, turn_id: &str, why: StopWhy) {
        let mut cancels = Vec::new();
        let mut sid = None;
        {
            let mut st = self.lock();
            for (id, sess) in st.sessions.iter_mut() {
                let Some(t) = sess.turns.get_mut(turn_id) else {
                    continue;
                };
                t.stop = Some(why);
                cancels = Self::stop_calls(sess, turn_id, why);
                sid = Some(id.clone());
            }
        }
        self.cv.notify_all();
        if let Some(sid) = sid {
            self.spawn_cancels(&sid, cancels);
        }
    }

    fn end_turn(&self, turn_id: &str) {
        let mut st = self.lock();
        for sess in st.sessions.values_mut() {
            if sess.live.as_deref() == Some(turn_id) {
                sess.live = None;
                // A call that waited for a place and never went out has no turn to run in.
                for c in sess
                    .calls
                    .iter_mut()
                    .filter(|c| c.turn_id == turn_id && c.state == AgentCallState::Queued)
                {
                    c.cancel.store(true, Ordering::SeqCst);
                }
            }
        }
        drop(st);
        self.cv.notify_all();
    }

    fn calls(&self, session_id: &str) -> Vec<CallRecord> {
        self.lock()
            .sessions
            .get(session_id)
            .map(|s| {
                s.calls
                    .iter()
                    .map(|c| CallRecord {
                        call_id: c.call_id.clone(),
                        turn_id: c.turn_id.clone(),
                        alias: c.alias.clone(),
                        state: c.state,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn call_detail(&self, turn_id: &str, call_id: &str) -> Option<AgentTurnCall> {
        let st = self.lock();
        st.sessions.values().find_map(|s| {
            s.calls
                .iter()
                .find(|c| c.turn_id == turn_id && c.call_id == call_id)
                .map(|c| AgentTurnCall {
                    call_id: c.call_id.clone(),
                    turn_id: c.turn_id.clone(),
                    workspace_id: c.alias.clone(),
                    profile: c.profile.clone(),
                    question: c.question.clone(),
                    state: c.state,
                    reply: c.reply.clone(),
                    withheld: c.withheld,
                })
        })
    }

    fn wipe_all(&self) {
        let ids: Vec<String> = self.lock().sessions.keys().cloned().collect();
        for id in ids {
            self.close_session(&id, true);
        }
    }

    fn close_session(&self, session_id: &str, wipe: bool) {
        let mut cancels = Vec::new();
        {
            let mut st = self.lock();
            let now = self.deps.clock.now();
            if let Some(sess) = st.sessions.get_mut(session_id) {
                let turns: Vec<String> = sess.turns.keys().cloned().collect();
                for t in turns {
                    if let Some(rec) = sess.turns.get_mut(&t) {
                        rec.stop = Some(StopWhy::SessionEnded);
                    }
                    cancels.extend(Self::stop_calls(sess, &t, StopWhy::SessionEnded));
                }
                // Every call that is not ended stops with the session.
                for c in sess.calls.iter() {
                    if matches!(c.state, AgentCallState::Queued | AgentCallState::Asking) {
                        c.cancel.store(true, Ordering::SeqCst);
                    }
                }
                sess.live = None;
                sess.closed_at = Some(now);
            }
            if wipe {
                st.sessions.remove(session_id);
            }
        }
        self.cv.notify_all();
        self.spawn_cancels(session_id, cancels);
    }
}
