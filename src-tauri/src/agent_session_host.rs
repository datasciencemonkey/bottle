//! Slice 2a (the MOG agent on one live Genie Code session): the host side of the
//! `genie_code_*`, `agent_turn_*` and `agent_session_*` commands.
//!
//! [`AgentSessionHost`] is managed from the start, so `ipc::route_from` always finds it. [`install`]
//! fills it with the one `mog_core::agent_run::session::SessionHost`: the Genie Code pin check, the
//! one app-server process, the turn queue, the deadlines and the wipe hook. Before `install` runs,
//! and when the app data folder is missing, every command answers `host_failed`.
//!
//! Two parts come from the guard lane (R2) and are taken from managed state when they exist:
//! `Arc<dyn SessionGuard>` and `Arc<dyn ShimProvider>`. Without a guard, `open_session` fails and
//! no session starts: the agent is closed, never open without its guard. There is no network
//! source in Rust yet, so the network watch is always online; a real offline shows as the
//! "waiting for network" error of the server, which interrupts the turn at once (AC-2a.10c).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use serde_json::Value;
use tauri::Manager;

use mog_core::agent_run::seam::{
    AgentEvent, CallRecord, NetWatch, RunEvents, RunSecret, SessionGuard, SessionSpec, StopWhy,
    TurnSpec, SHIM_ARG,
};
use mog_core::agent_run::session::approval::{AskLatch, TicketApprovals};
use mog_core::agent_run::session::genie_bin::GenieBin;
use mog_core::agent_run::session::install::{self, Fetcher, INSTALL_SOURCE};
use mog_core::agent_run::session::spawn::ProcessSpawner;
use mog_core::agent_run::session::{
    AgentEnv, Installer, PreflightFail, SessionConfig, SessionDeps, SessionHost, ShimLaunch,
    ShimProvider,
};
use mog_core::clock::{SystemClock, SystemWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::events;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::types::{AgentProfileState, AgentTurnCall, TokenState};
use mog_core::wipe::AgentWipeSlot;
use mog_core::workspaces::{ProfileSource, SignInSource};

use crate::cli_host::SharedCli;
use crate::ipc::BrainChannel;
use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::workspaces_host::WorkspacesHost;

/// The managed agent component.
#[derive(Debug, Default)]
pub struct AgentSessionHost {
    inner: OnceLock<Arc<SessionHost>>,
}

impl AgentSessionHost {
    /// The session host, once [`install`] has built it.
    pub fn session(&self) -> Option<&Arc<SessionHost>> {
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

/// The guard that is used when the guard lane is not wired: it never opens a session.
struct UnwiredGuard;

impl SessionGuard for UnwiredGuard {
    fn open_session(&self, _: SessionSpec) -> CoreResult<RunSecret> {
        Err(CoreError::Unavailable {
            component: "agent_session",
            reason: "guard_not_wired",
        })
    }
    fn bind_pid(&self, _: &str, _: u32) {}
    fn begin_turn(&self, _: &str, _: TurnSpec) -> CoreResult<()> {
        Err(CoreError::Unavailable {
            component: "agent_session",
            reason: "guard_not_wired",
        })
    }
    fn stop_turn(&self, _: &str, _: StopWhy) {}
    fn end_turn(&self, _: &str) {}
    fn tainted(&self, _: &str) -> bool {
        false
    }
    fn calls(&self, _: &str) -> Vec<CallRecord> {
        Vec::new()
    }
    fn call_detail(&self, _: &str, _: &str) -> Option<AgentTurnCall> {
        None
    }
    fn close_session(&self, _: &str, _: bool) {}
}

/// The shim command when the guard lane gives none: this executable with the shim argument.
struct DefaultShim;

impl ShimProvider for DefaultShim {
    fn launch(&self, session_id: &str, secret: &RunSecret) -> ShimLaunch {
        let hex: String = secret.0.iter().map(|b| format!("{b:02x}")).collect();
        ShimLaunch {
            command: std::env::current_exe().unwrap_or_default(),
            args: vec![SHIM_ARG.to_owned()],
            env: vec![
                ("BOTTLE_AGENT_SESSION".to_owned(), session_id.to_owned()),
                ("BOTTLE_AGENT_SECRET".to_owned(), hex),
                // The socket path comes from the guard (`shim::env_pairs`) in the real wiring.
            ],
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

/// R5, R6 and T7 as the session host sees them.
struct HostEnv {
    cli: SharedCli,
    profiles: Arc<dyn ProfileSource>,
    sign_in: Option<Arc<dyn SignInSource>>,
}

impl AgentEnv for HostEnv {
    fn cli_path(&self) -> Option<PathBuf> {
        use mog_core::cli_runner::CliRunner;
        self.cli.0.resolve().ok()
    }

    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    fn preflight(&self, profile: &str) -> Result<(), PreflightFail> {
        let p = self
            .profiles
            .profiles(false)
            .ok()
            .and_then(|l| l.into_iter().find(|p| p.name == profile))
            .ok_or(PreflightFail::Missing)?;
        // Only OAuth U2M through the Databricks CLI (plan A3.1).
        if !p.supported || p.auth_type != "databricks-cli" {
            return Err(PreflightFail::NotU2m);
        }
        let Some(source) = self.sign_in.clone() else {
            return Err(PreflightFail::Offline);
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let (name, host) = (profile.to_owned(), p.host.clone());
        std::thread::spawn(move || {
            let _ = tx.send(source.probe(&name, &host));
        });
        match rx.recv_timeout(std::time::Duration::from_secs(10)) {
            Ok(Ok(())) => Ok(()),
            Ok(Err(CoreError::Unavailable {
                reason: "login_required" | "auth_expired" | "token_unusable",
                ..
            })) => Err(PreflightFail::SignedOut),
            Ok(Err(_)) | Err(_) => Err(PreflightFail::Offline),
        }
    }

    fn profile_state(&self, profile: &str) -> AgentProfileState {
        let Some(p) = self
            .profiles
            .profiles(false)
            .ok()
            .and_then(|l| l.into_iter().find(|p| p.name == profile))
        else {
            return AgentProfileState::Missing;
        };
        if !p.supported || p.auth_type != "databricks-cli" {
            return AgentProfileState::NotU2m;
        }
        match self.sign_in.as_ref().map(|s| s.state(profile)) {
            Some(TokenState::LoginRequired) => AgentProfileState::SignedOut,
            _ => AgentProfileState::Ready,
        }
    }
}

/// The install is closed until the owner names the official source (`pin.rs` holds no URL).
struct PinnedInstaller {
    dir: PathBuf,
}

struct NoFetch;

impl Fetcher for NoFetch {
    fn fetch(&self, _: &str, _: usize) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("no install source"))
    }
}

impl Installer for PinnedInstaller {
    fn install(&self) -> Result<(), install::InstallError> {
        let want = mog_core::agent_run::pin::pinned_digest(
            mog_core::agent_run::session::genie_bin::current_target(),
            "genie",
        )
        .ok_or(install::InstallError::Failed)?;
        install::install(&NoFetch, INSTALL_SOURCE, want, &self.dir).map(|_| ())
    }
}

/// The session config under `dir` (the app local data dir).
pub fn config_in(dir: &std::path::Path) -> SessionConfig {
    SessionConfig {
        home: dir.join("genie-home"),
        tmp_root: std::env::temp_dir(),
        pid_file: dir.join("agent-session.pid"),
        tick: std::time::Duration::from_secs(1),
    }
}

/// Build the session host and fill the managed [`AgentSessionHost`]. [`crate::cli_host::install`],
/// [`crate::profiles_host::install`], [`crate::workspaces_host::install`] and
/// [`crate::mcp_host::install`] run first. Without them, or without an app data folder, the host
/// stays empty and the commands answer `host_failed`.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {
    let (Some(host), Some(ws), Some(cli), Some(t7), Ok(dir)) = (
        handle.try_state::<AgentSessionHost>(),
        handle.try_state::<WorkspacesHost>(),
        handle.try_state::<SharedCli>(),
        handle.try_state::<ProfilesHost>(),
        handle.path().app_local_data_dir(),
    ) else {
        return;
    };
    let cfg = config_in(&dir);
    let bin_dir = dir.join("bin");
    let genie = Arc::new(GenieBin::system(
        Some(bin_dir.clone()),
        handle.path().home_dir().ok(),
        &std::env::var("PATH").unwrap_or_default(),
    ));
    // At app start: kill a server that an earlier Bottle left, and delete GENIE_HOME.
    let known = genie.check(false).path;
    SessionHost::sweep_at_start(&cfg, known.as_deref());
    let guard: Arc<dyn SessionGuard> = handle
        .try_state::<Arc<dyn SessionGuard>>()
        .map(|g| Arc::clone(g.inner()))
        .unwrap_or_else(|| Arc::new(UnwiredGuard));
    let shim: Arc<dyn ShimProvider> = handle
        .try_state::<Arc<dyn ShimProvider>>()
        .map(|s| Arc::clone(s.inner()))
        .unwrap_or_else(|| Arc::new(DefaultShim));
    let latch = handle
        .try_state::<Arc<AskLatch>>()
        .map(|l| Arc::clone(l.inner()))
        .unwrap_or_default();
    let tickets = handle
        .try_state::<Arc<mog_core::action_ticket::ActionTickets>>()
        .map(|t| Arc::clone(t.inner()))
        .unwrap_or_default();
    let sign_in = handle.try_state::<McpHost>().map(|m| m.sign_in_source());
    let deps = SessionDeps {
        guard,
        net: Arc::new(AlwaysOnline),
        events: Arc::new(BrainEvents(handle.clone())),
        clock: Arc::new(SystemClock::new()),
        wall: Arc::new(SystemWallClock),
        spawner: Arc::new(ProcessSpawner),
        workspaces: ws.view(),
        approvals: Arc::new(TicketApprovals {
            latch,
            tickets,
            now_ms: Arc::new(crate::memory_host::now_ms),
        }),
        env: Arc::new(HostEnv {
            cli: (*cli).clone(),
            profiles: t7.0.clone(),
            sign_in,
        }),
        shim,
        binary: genie,
        installer: Arc::new(PinnedInstaller { dir: bin_dir }),
    };
    let session = SessionHost::new(deps, cfg);
    // Wipe ends the session first (plan A3.7). The slot is managed by `consent_host`.
    if let Some(slot) = handle.try_state::<Arc<AgentWipeSlot>>() {
        slot.bind(session.clone());
    }
    let _ = host.inner.set(session);
}

/// On quit: end the session, kill the process group and delete GENIE_HOME.
pub fn shutdown(app: &tauri::AppHandle<tauri::Wry>) {
    if let Some(h) = app.try_state::<AgentSessionHost>() {
        if let Some(s) = h.session() {
            s.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_host_answers_host_failed_for_each_command() {
        let h = AgentSessionHost::default();
        for c in [
            "genie_code_status",
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
    fn with_no_guard_wired_no_session_can_open() {
        let g = UnwiredGuard;
        let spec = SessionSpec {
            session_id: "s".into(),
            catalog: vec![],
            cross_workspace: false,
        };
        assert!(g.open_session(spec).is_err());
        assert!(g
            .begin_turn(
                "s",
                TurnSpec {
                    turn_id: "t".into(),
                    budget: 5
                }
            )
            .is_err());
        assert!(!g.tainted("s"));
    }

    #[test]
    fn the_default_shim_gives_the_shim_arg_and_a_hex_secret_for_the_environment_only() {
        let secret = RunSecret([0xab; 32]);
        let l = DefaultShim.launch("sess", &secret);
        assert_eq!(l.args, vec![SHIM_ARG.to_owned()]);
        let s = l
            .env
            .iter()
            .find(|(k, _)| k == "BOTTLE_AGENT_SECRET")
            .unwrap();
        assert_eq!(s.1, "ab".repeat(32));
        // `Debug` never shows a value of the environment.
        assert!(!format!("{l:?}").contains(&"ab".repeat(32)));
        assert!(format!("{l:?}").contains("BOTTLE_AGENT_SECRET"));
    }

    #[test]
    fn the_session_config_keeps_genie_home_and_the_pid_file_apart() {
        let c = config_in(std::path::Path::new("/data"));
        assert_eq!(c.home, PathBuf::from("/data/genie-home"));
        // The pid file is outside GENIE_HOME, so a delete of the home never removes the trace.
        assert!(!c.pid_file.starts_with(&c.home));
    }
}
