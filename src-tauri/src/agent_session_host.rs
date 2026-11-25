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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value;
use tauri::Manager;

use mog_core::agent_run::orchestrator::{
    OrchConfig, OrchDeps, OrchEnv, OrchGuard, OrchStatus, OrchestratorHost,
};
use mog_core::agent_run::reauth::{DedupReauth, NoReauth, ReauthService};
use mog_core::agent_run::seam::{AgentEvent, NetWatch, RunEvents};
use mog_core::clock::{SystemClock, SystemWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::events;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::model_gateway::client::{CANCELLED, OFFLINE, SIGNED_OUT};
use mog_core::model_gateway::invoke::{AbandonOnWipe, ModelInvokeComponent};
use mog_core::model_gateway::models::{orchestrator_status, OrchestratorStatus};
use mog_core::model_gateway::{ChatClient, ChatRequest};
use mog_core::profile_login::LOGIN_TIMEOUT;
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

/// How often [`wait_for_running_sign_in_until`] re-checks a profile's cached sign-in state and
/// its own abandon flag.
const SETTINGS_SIGN_IN_POLL: Duration = Duration::from_millis(500);

/// The profiles whose current `reauth` is only watching a sign-in that belongs to Settings'
/// own "Sign in…" (round 5), by profile, with the abandon flag each one's wait listens to. An
/// entry here means this process did not start that sign-in and must never cancel it: Settings
/// already owns the running CLI child, and the one `profile_reauth` found running, when it
/// answered `profile_exists`. `DedupReauth`'s own "one caller at a time per profile" rule means
/// at most one entry per profile ever needs writing at once.
type Waiting = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

/// `profile_reauth`, with `profile_exists` read as "Settings already runs its own 'Sign in…' for
/// this profile right now": wait for it, in the same bound as a sign-in itself, instead of
/// failing the agent's own attempt (ADR 0122 amendment, owner decision 2026-11-24, round 4). Any
/// other error of `profile_reauth` is a plain failure, as it always was.
fn reauth_or_wait_for_settings(
    login: &dyn GatewayComponents,
    sign_in: Option<&dyn SignInSource>,
    profile: &str,
    waiting: &Waiting,
) -> CoreResult<()> {
    match login.call("profile_reauth", serde_json::json!({ "profile": profile })) {
        Ok(_) => Ok(()),
        Err(e) if e.code() == "profile_exists" => {
            wait_for_running_sign_in(sign_in, profile, waiting)
        }
        Err(e) => Err(e),
    }
}

/// Polls `sign_in.state(profile)` every [`SETTINGS_SIGN_IN_POLL`] until it is no longer
/// `LoginRequired` (Settings' own sign-in ended, one way or another), [`LOGIN_TIMEOUT`] passes,
/// or a cancel of this profile sets the abandon flag this registers in `waiting` for its own
/// run (round 5: a Stop, the turn's hard deadline or a Wipe must not block on Settings' own
/// sign-in for up to five minutes, and must never cancel it — only this wait). `sign_in` is
/// `None` only when the sign-in source is not managed yet; then `profile_exists` stays a plain
/// failure, since there is nothing here to wait on.
fn wait_for_running_sign_in(
    sign_in: Option<&dyn SignInSource>,
    profile: &str,
    waiting: &Waiting,
) -> CoreResult<()> {
    wait_for_running_sign_in_until(sign_in, profile, Instant::now() + LOGIN_TIMEOUT, waiting)
}

/// `wait_for_running_sign_in`, with its deadline passed in (a test's seam: `LOGIN_TIMEOUT` is 5
/// min, too long to wait out for real).
fn wait_for_running_sign_in_until(
    sign_in: Option<&dyn SignInSource>,
    profile: &str,
    deadline: Instant,
    waiting: &Waiting,
) -> CoreResult<()> {
    let Some(sign_in) = sign_in else {
        return Err(CoreError::IpcRefused {
            code: "profile_exists",
        });
    };
    let abandon = Arc::new(AtomicBool::new(false));
    lock(waiting).insert(profile.to_owned(), Arc::clone(&abandon));
    let result = loop {
        if abandon.load(Ordering::SeqCst) {
            break Err(CoreError::IpcRefused {
                code: "login_cancelled",
            });
        }
        if sign_in.state(profile) != TokenState::LoginRequired {
            break Ok(());
        }
        if Instant::now() >= deadline {
            break Err(CoreError::IpcRefused {
                code: "profile_exists",
            });
        }
        thread::sleep(SETTINGS_SIGN_IN_POLL);
    };
    lock(waiting).remove(profile);
    result
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Builds the "sign in, then retry" rule's `ReauthService` on `login` (R5's `profile_login` and
/// `profile_login_cancel`) and `sign_in` (R6's cached state, for the Settings-sign-in wait).
/// Standalone and unit-testable, so [`install`] stays a thin assembly of managed state.
///
/// `cancel`'s ownership rule (round 5): if this profile is only being watched (`waiting` has an
/// entry for it: `profile_reauth` found Settings' own "Sign in…" already running), abandon the
/// wait and nothing else — the running CLI child belongs to Settings, not this call, and
/// `profile_login_cancel` must never touch it. Only when this profile is not being watched (this
/// process's own `profile_reauth` is the one blocked in `databricks auth login`) does `cancel`
/// call `profile_login_cancel`, to kill that child.
fn build_reauth(
    login: Arc<dyn GatewayComponents>,
    sign_in: Option<Arc<dyn SignInSource>>,
) -> Arc<dyn ReauthService> {
    let waiting: Waiting = Arc::default();
    let run_login = Arc::clone(&login);
    let run_sign_in = sign_in.clone();
    let run_waiting = Arc::clone(&waiting);
    Arc::new(DedupReauth::new(
        move |profile: &str| {
            reauth_or_wait_for_settings(
                run_login.as_ref(),
                run_sign_in.as_deref(),
                profile,
                &run_waiting,
            )
        },
        move |profile: &str| {
            if let Some(abandon) = lock(&waiting).get(profile).cloned() {
                abandon.store(true, Ordering::SeqCst);
                return;
            }
            let _ = login.call(
                "profile_login_cancel",
                serde_json::json!({ "profile": profile }),
            );
        },
    ))
}

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
    // Home first (ADR 0122 amendment, owner decision 2026-11-24, generalized to every call and
    // hardened in round 4): "sign in, then retry" reuses the exact `profile_reauth` flow (R5's
    // `databricks auth login`, U2M OAuth) that "Sign in…" in Settings → Workspace already runs;
    // this never writes a new OAuth implementation. `DedupReauth` adds only the "one sign-in at a
    // time per profile" rule on top, and routes a Stop, the turn's hard deadline or a Wipe to the
    // same `profile_login_cancel` that the Settings "Cancel" button uses, so none of them leaves
    // the CLI child running or Settings' own "Sign in…" refused `profile_exists` afterwards. With
    // no `ProfileLoginHost` managed yet, every sign-in refuses cleanly instead of panicking.
    let reauth: Arc<dyn ReauthService> =
        match handle.try_state::<crate::profiles_host::ProfileLoginHost>() {
            Some(login) => build_reauth(
                login.0.clone() as Arc<dyn GatewayComponents>,
                sign_in.clone(),
            ),
            None => Arc::new(NoReauth),
        };
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
        reauth,
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

    /// A `SignInSource` whose `state` answers `LoginRequired` for its first `flips_after` calls,
    /// then `Valid`: a Settings "Sign in…" that is still running, then ends.
    #[derive(Default)]
    struct FlippingTokens {
        calls: std::sync::atomic::AtomicUsize,
        flips_after: usize,
    }

    impl SignInSource for FlippingTokens {
        fn state(&self, _: &str) -> TokenState {
            if self.calls.fetch_add(1, Ordering::SeqCst) < self.flips_after {
                TokenState::LoginRequired
            } else {
                TokenState::Valid
            }
        }
        fn probe(&self, _: &str, _: &str) -> CoreResult<()> {
            Ok(())
        }
    }

    /// A `GatewayComponents` of one closure, for `reauth_or_wait_for_settings`'s `login`.
    struct FakeLogin<F>(F);

    impl<F: Fn(Value) -> CoreResult<Value> + Send + Sync> GatewayComponents for FakeLogin<F> {
        fn call(&self, _command: &'static str, args: Value) -> CoreResult<Value> {
            (self.0)(args)
        }
    }

    /// A `GatewayComponents` that records how many times each command of `build_reauth`'s own
    /// `login` ran: `profile_reauth` always answers `profile_exists` (round 5's own scenario,
    /// where the wait is for Settings' own sign-in, not this process's).
    #[derive(Default)]
    struct RecordingLogin {
        reauth_calls: Mutex<usize>,
        cancel_calls: Mutex<usize>,
    }

    impl GatewayComponents for RecordingLogin {
        fn call(&self, command: &'static str, _args: Value) -> CoreResult<Value> {
            match command {
                "profile_reauth" => {
                    *self.reauth_calls.lock().unwrap() += 1;
                    fail("profile_exists")
                }
                "profile_login_cancel" => {
                    *self.cancel_calls.lock().unwrap() += 1;
                    Ok(Value::Null)
                }
                _ => Ok(Value::Null),
            }
        }
    }

    fn no_waiting() -> Waiting {
        Arc::default()
    }

    #[test]
    fn reauth_or_wait_passes_through_a_successful_reauth() {
        let login = FakeLogin(|_| Ok(Value::Null));
        assert!(reauth_or_wait_for_settings(&login, None, "p-sales", &no_waiting()).is_ok());
    }

    #[test]
    fn reauth_or_wait_passes_through_a_plain_failure_unchanged() {
        let login = FakeLogin(|_| fail("profile_invalid"));
        assert_eq!(
            reauth_or_wait_for_settings(&login, None, "p-sales", &no_waiting())
                .unwrap_err()
                .code(),
            "profile_invalid"
        );
    }

    #[test]
    fn reauth_or_wait_waits_for_a_running_settings_sign_in_then_succeeds() {
        // `profile_exists`: Settings already runs "Sign in…" for this profile. Round 4: wait
        // for it (polling the cached state, not a new network probe) instead of failing.
        let login = FakeLogin(|_| fail("profile_exists"));
        let tokens = FlippingTokens {
            calls: 0.into(),
            flips_after: 2,
        };
        assert!(
            reauth_or_wait_for_settings(&login, Some(&tokens), "p-sales", &no_waiting()).is_ok()
        );
    }

    #[test]
    fn reauth_or_wait_fails_on_profile_exists_with_no_sign_in_source_to_wait_on() {
        let login = FakeLogin(|_| fail("profile_exists"));
        assert_eq!(
            reauth_or_wait_for_settings(&login, None, "p-sales", &no_waiting())
                .unwrap_err()
                .code(),
            "profile_exists"
        );
    }

    #[test]
    fn wait_for_running_sign_in_times_out_if_it_never_ends() {
        let tokens = FlippingTokens {
            calls: 0.into(),
            flips_after: usize::MAX,
        };
        // A deadline already past: the timeout path runs at once, not after a real wait.
        assert_eq!(
            wait_for_running_sign_in_until(Some(&tokens), "p-sales", Instant::now(), &no_waiting())
                .unwrap_err()
                .code(),
            "profile_exists"
        );
    }

    #[test]
    fn cancelling_a_wait_for_settings_own_sign_in_never_touches_it() {
        // Round 5: when `profile_reauth` answers `profile_exists`, the running sign-in belongs
        // to Settings, not this process. A cancel of it (Stop, the turn's hard deadline or a
        // Wipe) must abandon the wait at once, and must never call `profile_login_cancel` on a
        // sign-in it does not own.
        let login = Arc::new(RecordingLogin::default());
        let tokens: Arc<dyn SignInSource> = Arc::new(Tokens(TokenState::LoginRequired));
        let reauth = build_reauth(login.clone() as Arc<dyn GatewayComponents>, Some(tokens));
        let r2 = Arc::clone(&reauth);
        let waiter = thread::spawn(move || r2.reauth("p-sales"));
        let deadline = Instant::now() + Duration::from_secs(5);
        while *login.reauth_calls.lock().unwrap() == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(*login.reauth_calls.lock().unwrap(), 1, "reauth never ran");
        // A moment for the wait loop to register itself before the cancel below.
        thread::sleep(Duration::from_millis(20));
        reauth.cancel("p-sales");
        let result = waiter.join().unwrap();
        assert_eq!(result.unwrap_err().code(), "login_cancelled");
        assert_eq!(
            *login.cancel_calls.lock().unwrap(),
            0,
            "Settings' own sign-in must never be cancelled"
        );
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
