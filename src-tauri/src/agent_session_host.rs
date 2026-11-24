//! ADR 0122: the host side of the `orchestrator_*`, `agent_turn_*` and `agent_session_*`
//! commands.
//!
//! [`AgentSessionHost`] is managed from the start, so `ipc::route_from` always finds it. [`install`]
//! fills it with the one `mog_core::agent_run::orchestrator::OrchestratorHost`: the ask loop on
//! the chat client of the home AI Gateway, the turn queue, the deadlines and the wipe hook. Before
//! `install` runs, and when the guard is missing, every command answers `host_failed`.
//!
//! Two parts come from earlier steps and are taken from managed state: the `ask_genie` guard
//! ([`install_guard`]) and the ask loop's own `GatewayChatClient` of [`crate::model_host`]. Without the guard no
//! host is built: the agent is closed, never open without its guard. There is no network source in
//! Rust yet, so the network watch is always online; a real offline shows as the `offline` error
//! of the chat client.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::Manager;

use mog_core::agent_run::orchestrator::{
    OrchConfig, OrchDeps, OrchEnv, OrchGuard, OrchStatus, OrchestratorHost,
};
use mog_core::agent_run::seam::{AgentEvent, NetWatch, RunEvents};
use mog_core::clock::{SystemClock, SystemWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::events;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::model_gateway::client::{CANCELLED, OFFLINE, SIGNED_OUT};
use mog_core::model_gateway::invoke::{AbandonOnWipe, ModelInvokeComponent};
use mog_core::model_gateway::models::{orchestrator_status, OrchestratorStatus};
use mog_core::model_gateway::{ChatClient, ChatRequest};
use mog_core::types::TokenState;
use mog_core::wipe::AgentWipeSlot;
use mog_core::workspaces::SignInSource;

use crate::ipc::BrainChannel;
use crate::mcp_host::McpHost;
use crate::model_host::{AskChatClient, InstallFailure};
use crate::workspaces_host::WorkspacesHost;

/// How long the code of a failed chat call counts for the status. After this, the next ask tries
/// the model again.
const ERROR_TTL: Duration = Duration::from_secs(30);

/// The managed agent component.
#[derive(Debug, Default)]
pub struct AgentSessionHost {
    inner: OnceLock<Arc<OrchestratorHost>>,
}

impl AgentSessionHost {
    /// The orchestrator host, once [`install`] has built it.
    pub fn session(&self) -> Option<&Arc<OrchestratorHost>> {
        self.inner.get()
    }
}

impl GatewayComponents for AgentSessionHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match self.inner.get() {
            Some(h) => h.call(command, args),
            None => Err(CoreError::IpcRefused {
                code: "host_failed",
            }),
        }
    }
}

/// No network source exists in Rust yet (the brain owns `net:status`): always online.
struct AlwaysOnline;

impl NetWatch for AlwaysOnline {
    fn online(&self) -> bool {
        true
    }
    fn subscribe(&self, _: Box<dyn Fn(bool) + Send + Sync>) {}
}

/// The events go to the brain Channel, content-free (`agent:session`, `agent:turn`).
struct BrainEvents(tauri::AppHandle<tauri::Wry>);

impl RunEvents for BrainEvents {
    fn emit(&self, e: AgentEvent) {
        let Some(ch) = self.0.try_state::<BrainChannel>() else {
            return;
        };
        let (name, payload) = match &e {
            AgentEvent::Session(s) => (events::AGENT_SESSION, serde_json::to_value(s)),
            AgentEvent::Turn(t) => (events::AGENT_TURN, serde_json::to_value(t)),
        };
        if let Ok(p) = payload {
            ch.send(name, p);
        }
    }
}

/// The code of the last failed chat call, with the model and the time, for the status. It holds
/// no prompt, no reply and no profile.
#[derive(Debug, Default)]
struct LastError(Mutex<Option<(String, &'static str, Instant)>>);

impl LastError {
    fn record(&self, model: &str, result: &CoreResult<Value>, now: Instant) {
        let next = match result {
            Ok(_) => None,
            // A stop or a wipe is not a failure of the gateway.
            Err(e) if e.code() == CANCELLED => return,
            Err(e) => Some((model.to_owned(), e.code(), now)),
        };
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = next;
    }

    /// Forget the note (the user asked for a recheck).
    fn clear(&self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// The code of the last failure of `model`, if it is newer than [`ERROR_TTL`].
    fn get(&self, model: &str, now: Instant) -> Option<&'static str> {
        let g = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let (m, code, at) = g.as_ref()?;
        (m == model && now.saturating_duration_since(*at) < ERROR_TTL).then_some(*code)
    }
}

/// The chat client of the ask loop, and a note of how each call ended (for the status).
struct NotingChat {
    inner: Arc<dyn ChatClient>,
    last: Arc<LastError>,
}

impl ChatClient for NotingChat {
    fn chat(&self, req: ChatRequest<'_>) -> CoreResult<Value> {
        let model = req.model.to_owned();
        let out = self.inner.chat(req);
        self.last.record(&model, &out, Instant::now());
        out
    }
}

/// The chat client when [`crate::model_host::install`] failed: every call is `offline`.
struct NoChat;

impl ChatClient for NoChat {
    fn chat(&self, _: ChatRequest<'_>) -> CoreResult<Value> {
        Err(CoreError::Unavailable {
            component: "agent_session",
            reason: OFFLINE,
        })
    }
}

/// The status of the orchestrator: the sign-in state of the home profile and the code of the last
/// chat call. It makes no model call.
struct HostEnv {
    sign_in: Option<Arc<dyn SignInSource>>,
    last: Arc<LastError>,
    /// The chat client was not built, so no call can work.
    no_client: bool,
}

impl HostEnv {
    fn check(&self, profile: &str, model: &str, now: Instant) -> OrchStatus {
        let last = if self.no_client {
            Some(OFFLINE)
        } else if self.sign_in.as_ref().map(|s| s.state(profile)) == Some(TokenState::LoginRequired)
        {
            Some(SIGNED_OUT)
        } else {
            self.last.get(model, now)
        };
        match orchestrator_status(Some(profile), Some(model), last) {
            OrchestratorStatus::Ready => OrchStatus::Ready,
            OrchestratorStatus::NoHome => OrchStatus::NoHome,
            OrchestratorStatus::SignedOut => OrchStatus::SignedOut,
            OrchestratorStatus::NoModel => OrchStatus::NoModel,
            OrchestratorStatus::Unreachable => OrchStatus::Unreachable,
        }
    }
}

impl OrchEnv for HostEnv {
    fn probe(&self, profile: &str, model: &str) -> OrchStatus {
        self.check(profile, model, Instant::now())
    }

    fn clear(&self) {
        self.last.clear();
    }
}

/// Build the `ask_genie` guard (R2) on the one R8, the workspace gate and the brain events, and
/// manage it. [`install`] takes it from managed state. If R8 or the workspace host is missing,
/// nothing is managed: the host then has no guard and is not built (the agent is closed, not
/// open).
pub fn install_guard(handle: &tauri::AppHandle<tauri::Wry>) {
    use mog_core::agent_guard::AuditActionLog;
    use mog_core::agent_run::guard::session_log::SessionUuidLog;
    use mog_core::audit::{AuditLog, AuditRetention, MemoryAuditStore};

    let (Some(mcp), Some(ws)) = (
        handle.try_state::<McpHost>(),
        handle.try_state::<WorkspacesHost>(),
    ) else {
        return;
    };
    // TODO(#442): the SQLCipher audit store. Until then the entries of one run stay in memory.
    let log = AuditActionLog::new(
        AuditLog::new(MemoryAuditStore::default(), AuditRetention::default()),
        Arc::new(SystemWallClock),
    );
    let guard = crate::agent_host::agent_guard(
        mcp.transport(),
        ws.gate(),
        Arc::new(BrainEvents(handle.clone())),
        Arc::new(SessionUuidLog(Arc::new(log))),
    );
    let orch_guard: Arc<dyn OrchGuard> = guard.clone();
    handle.manage(guard);
    handle.manage(orch_guard);
}

/// Build the orchestrator host and fill the managed [`AgentSessionHost`].
/// [`crate::workspaces_host::install`], [`crate::mcp_host::install`] and
/// [`crate::model_host::install`] run first. Without the workspace host or the guard the host
/// stays empty and the commands answer `host_failed`.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {
    let (Some(host), Some(ws), Some(guard)) = (
        handle.try_state::<AgentSessionHost>(),
        handle.try_state::<WorkspacesHost>(),
        handle.try_state::<Arc<dyn OrchGuard>>(),
    ) else {
        return;
    };
    let last = Arc::new(LastError::default());
    // The ask loop has its own client (its own in-flight cap), so dictation cleanup calls cannot
    // use up its slots.
    let client: Option<Arc<dyn ChatClient>> = handle
        .try_state::<AskChatClient>()
        .map(|c| Arc::clone(&c.0) as Arc<dyn ChatClient>);
    let no_client = client.is_none() || handle.try_state::<InstallFailure>().is_some();
    let chat: Arc<dyn ChatClient> = Arc::new(NotingChat {
        inner: client.unwrap_or_else(|| Arc::new(NoChat)),
        last: Arc::clone(&last),
    });
    let latch = handle
        .try_state::<Arc<mog_core::agent_run::orchestrator::approval::AskLatch>>()
        .map(|l| Arc::clone(l.inner()))
        .unwrap_or_default();
    let tickets = handle
        .try_state::<Arc<mog_core::action_ticket::ActionTickets>>()
        .map(|t| Arc::clone(t.inner()))
        .unwrap_or_default();
    let sign_in = handle.try_state::<McpHost>().map(|m| m.sign_in_source());
    let deps = OrchDeps {
        guard: Arc::clone(guard.inner()),
        chat,
        net: Arc::new(AlwaysOnline),
        events: Arc::new(BrainEvents(handle.clone())),
        clock: Arc::new(SystemClock::new()),
        wall: Arc::new(SystemWallClock),
        workspaces: ws.view(),
        approvals: Arc::new(
            mog_core::agent_run::orchestrator::approval::TicketApprovals {
                latch,
                tickets,
                now_ms: Arc::new(crate::memory_host::now_ms),
            },
        ),
        env: Arc::new(HostEnv {
            sign_in,
            last,
            no_client,
        }),
    };
    // `OrchestratorHost::new` sets the refusal hook of the guard: the third refusal of a turn
    // interrupts it (ADR 0119).
    let session = OrchestratorHost::new(
        deps,
        OrchConfig {
            tick: Duration::from_secs(1),
        },
    );
    // Wipe ends the session first, and it abandons the `model_invoke` calls in flight while it
    // runs. The slot is managed by `consent_host`.
    if let Some(slot) = handle.try_state::<Arc<AgentWipeSlot>>() {
        match handle.try_state::<Arc<ModelInvokeComponent>>() {
            Some(model) => slot.bind(Arc::new(AbandonOnWipe::new(
                session.clone(),
                model.stop_flag(),
            ))),
            None => slot.bind(session.clone()),
        }
    }
    let _ = host.inner.set(session);
}

/// On quit: abandon the `model_invoke` calls in flight, and end the session.
pub fn shutdown(app: &tauri::AppHandle<tauri::Wry>) {
    if let Some(model) = app.try_state::<Arc<ModelInvokeComponent>>() {
        model.stop_flag().store(true, Ordering::SeqCst);
    }
    if let Some(h) = app.try_state::<AgentSessionHost>() {
        if let Some(s) = h.session() {
            s.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mog_core::model_gateway::client::{MODEL_NOT_FOUND, TIMEOUT};

    struct Tokens(TokenState);

    impl SignInSource for Tokens {
        fn state(&self, _: &str) -> TokenState {
            self.0
        }
        fn probe(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
    }

    fn env(state: TokenState, last: &Arc<LastError>) -> HostEnv {
        HostEnv {
            sign_in: Some(Arc::new(Tokens(state))),
            last: Arc::clone(last),
            no_client: false,
        }
    }

    fn fail(code: &'static str) -> CoreResult<Value> {
        Err(CoreError::Unavailable {
            component: "t",
            reason: code,
        })
    }

    #[test]
    fn an_empty_host_answers_host_failed_for_each_command() {
        let h = AgentSessionHost::default();
        for c in [
            "orchestrator_status",
            "orchestrator_models",
            "agent_turn_submit",
            "agent_session_state",
        ] {
            let e = h.call(c, serde_json::json!({})).unwrap_err();
            assert!(matches!(
                e,
                CoreError::IpcRefused {
                    code: "host_failed"
                }
            ));
        }
        assert!(h.session().is_none());
    }

    #[test]
    fn the_status_is_ready_with_no_failure_and_signed_out_from_the_token_state() {
        let last = Arc::new(LastError::default());
        let now = Instant::now();
        let m = "system.ai.claude-haiku-5-5";
        assert_eq!(
            env(TokenState::Valid, &last).check("home", m, now),
            OrchStatus::Ready
        );
        assert_eq!(
            env(TokenState::Unknown, &last).check("home", m, now),
            OrchStatus::Ready
        );
        assert_eq!(
            env(TokenState::LoginRequired, &last).check("home", m, now),
            OrchStatus::SignedOut
        );
    }

    #[test]
    fn the_status_follows_the_last_failure_of_that_model_and_it_expires() {
        let last = Arc::new(LastError::default());
        let e = env(TokenState::Valid, &last);
        let (m, other) = ("system.ai.a", "system.ai.b");
        let t0 = Instant::now();
        last.record(m, &fail(MODEL_NOT_FOUND), t0);
        assert_eq!(e.check("home", m, t0), OrchStatus::NoModel);
        // A model change does not inherit the failure of the old model.
        assert_eq!(e.check("home", other, t0), OrchStatus::Ready);
        // After the time to live, the next ask tries the model again.
        assert_eq!(
            e.check("home", m, t0 + ERROR_TTL + Duration::from_secs(1)),
            OrchStatus::Ready
        );
        last.record(m, &fail(TIMEOUT), t0);
        assert_eq!(e.check("home", m, t0), OrchStatus::Unreachable);
        // A stop is not a failure and keeps the note. A good call clears it.
        last.record(m, &fail(CANCELLED), t0);
        assert_eq!(e.check("home", m, t0), OrchStatus::Unreachable);
        last.record(m, &Ok(Value::Null), t0);
        assert_eq!(e.check("home", m, t0), OrchStatus::Ready);
    }

    #[test]
    fn a_bad_reply_keeps_the_status_ready_and_a_recheck_clears_a_failure() {
        use mog_core::model_gateway::client::BAD_REPLY;
        let last = Arc::new(LastError::default());
        let e = env(TokenState::Valid, &last);
        let (m, t0) = ("system.ai.a", Instant::now());
        // The gateway answered, so a reply that is not usable is no fault of the link.
        last.record(m, &fail(BAD_REPLY), t0);
        assert_eq!(e.check("home", m, t0), OrchStatus::Ready);
        last.record(m, &fail(OFFLINE), t0);
        assert_eq!(e.check("home", m, t0), OrchStatus::Unreachable);
        // `orchestrator_status` with `recheck` clears the note through `OrchEnv::clear`.
        OrchEnv::clear(&e);
        assert_eq!(e.check("home", m, t0), OrchStatus::Ready);
    }

    #[test]
    fn with_no_chat_client_the_status_is_unreachable() {
        let e = HostEnv {
            sign_in: None,
            last: Arc::new(LastError::default()),
            no_client: true,
        };
        assert_eq!(
            e.check("home", "system.ai.a", Instant::now()),
            OrchStatus::Unreachable
        );
    }
}
