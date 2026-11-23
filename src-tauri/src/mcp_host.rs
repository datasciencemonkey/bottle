//! The one R8 McpTransport of the process (#467, #30). `mcp_call` reaches it through the
//! hardened gateway and [`crate::ipc::HostComponents`], so only the brain can call it (SEC-02a).
//!
//! [`build`] makes one R6 [`CliTokenBroker`] on the R5 profile hosts, and one
//! [`ReqwestMcpTransport::new`] over them (G-04a, G-04d: a fresh token for each request). It
//! binds the R6 SCIM seam to one R7 [`ReqwestHttpTransport`], so R6 can read the workspace ID
//! for `?o=` (W-06a). The token stays in R6: no token goes to a window, a log or the disk.
//! R7 and R8 take the one app-managed [`SharedRateBucket`] (G-09a, ARC-F3); this module builds
//! no bucket of its own.
//!
//! The profile hosts are the one T7 ProfileService ([`crate::profiles_host`], #17): a profile that
//! T7 does not support (PAT, service principal, plaintext token storage) has no host, so R7 and
//! R8 never send for it (W-03b, W-10a). R6 runs `databricks auth token --profile <p>` on the one
//! shared R5 of the process ([`crate::cli_host::SharedCli`], #16). [`build`] keeps the R5 test
//! double (`StubCliRunner`) for host tests: there every `mcp_call` fails closed before any send.

use std::sync::{Arc, Weak};

use mog_core::cli_runner::{CliRunner, StubCliRunner};
use mog_core::clock::{SystemClock, SystemWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::rate_bucket::RateBucket;
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport, StubProfileHosts};
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mcp_transport::{McpCallComponent, ReqwestMcpTransport};
use mog_core::profile_login::LoginVerifier;
use mog_core::token_broker::{CliTokenBroker, ScimMe, TokenBroker};
use mog_core::workspaces::{SignInSource, WorkspaceGate};
use serde_json::Value;
use tauri::Manager;

use crate::cli_host::SharedCli;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;
use crate::workspaces_host::WorkspacesHost;

/// The `mcp_call` component of the app: R8 over R6, with the R7 that serves the R6 SCIM seam.
pub struct McpHost {
    component: McpCallComponent<Arc<ReqwestMcpTransport>>,
    /// The R6 of R8, for the W-06a check after a sign-in (#162, [`McpHost::login_verifier`]).
    broker: Arc<CliTokenBroker>,
    /// R6 holds R7 weakly (`bind_scim`), so the host keeps R7 alive.
    _scim: Arc<ReqwestHttpTransport>,
}

impl std::fmt::Debug for McpHost {
    /// Shows nothing: no host, profile, session or token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpHost").finish_non_exhaustive()
    }
}

impl GatewayComponents for McpHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.component.call(command, args)
    }
}

/// Build the one R8 of the process on the shared `bucket` (G-09a, ARC-F3). R7 (the R6 SCIM
/// seam) and R8 both take this Arc, so the AIMD cut and the `Retry-After` hold are shared.
/// An `Err` means the HTTP client could not start.
/// Host tests only: no profile hosts and the R5 test double.
pub fn build(bucket: Arc<dyn RateBucket>) -> CoreResult<McpHost> {
    build_with(bucket, Arc::new(StubProfileHosts), Arc::new(StubCliRunner))
}

/// [`build`] on the profile `hosts` (the app passes its one T7, #17) and R5 `cli` (the app
/// passes its one shared R5, #16).
pub fn build_with(
    bucket: Arc<dyn RateBucket>,
    hosts: Arc<dyn ProfileHosts>,
    cli: Arc<dyn CliRunner>,
) -> CoreResult<McpHost> {
    build_with_gate(bucket, hosts, cli, None)
}

/// [`build_with`] with the workspace allow-list of R8 (ADR 0117, FLAG 7). With a gate, R8
/// refuses `genie_ask` for a profile that is neither home nor available.
pub fn build_with_gate(
    bucket: Arc<dyn RateBucket>,
    hosts: Arc<dyn ProfileHosts>,
    cli: Arc<dyn CliRunner>,
    gate: Option<Arc<dyn WorkspaceGate>>,
) -> CoreResult<McpHost> {
    let mono = Arc::new(SystemClock::new());
    let broker = Arc::new(CliTokenBroker::new(
        cli,
        hosts.clone(),
        mono,
        Arc::new(SystemWallClock),
    ));
    let scim = Arc::new(ReqwestHttpTransport::new(
        hosts.clone(),
        broker.clone(),
        bucket.clone(),
    )?);
    let weak: Weak<dyn ScimMe> = Arc::downgrade(&scim) as Weak<dyn ScimMe>;
    broker.bind_scim(weak);
    let r6 = broker.clone();
    let transport = Arc::new(ReqwestMcpTransport::new(hosts, broker, bucket)?);
    Ok(McpHost {
        component: McpCallComponent::with_gate(transport, gate),
        broker: r6,
        _scim: scim,
    })
}

/// The W-06a check of a new profile (#162, W-07h): R6 reads its `workspace_id` from the
/// `x-databricks-org-id` header of SCIM `Me`. The token stays in R6.
struct WorkspaceIdCheck(Arc<CliTokenBroker>);

impl LoginVerifier for WorkspaceIdCheck {
    fn verify(&self, profile: &str) -> CoreResult<()> {
        self.0.workspace_id(profile).map(|_| ())
    }
}

impl McpHost {
    /// The one R6 of the app, for another R7 (the recording cloud host, ADR 0103). The token
    /// stays inside R6: callers get only `with_token` and `workspace_id`.
    pub fn token_broker(&self) -> Arc<dyn TokenBroker> {
        self.broker.clone()
    }

    /// R6 as the sign-in source of the workspace list (ADR 0117). It gives no token.
    pub fn sign_in_source(&self) -> Arc<dyn SignInSource> {
        self.broker.clone()
    }

    /// The W-06a check for `profile_login` (#162), on the R6 of this R8.
    pub fn login_verifier(&self) -> Arc<dyn LoginVerifier> {
        Arc::new(WorkspaceIdCheck(self.broker.clone()))
    }
}

/// Build R8 on the app-managed [`SharedRateBucket`] and manage it, so `mcp_call` reaches it.
/// Its profile hosts are the managed T7 ([`crate::profiles_host::ProfilesHost`]), so
/// [`crate::profiles_host::install`] runs first. If R8 cannot start, the app still runs and
/// `mcp_call` answers `host_failed`.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) -> CoreResult<()> {
    let bucket = handle
        .try_state::<SharedRateBucket>()
        .ok_or(CoreError::Unavailable {
            component: "mcp_host",
            reason: "rate_bucket_missing",
        })?
        .0
        .clone();
    let hosts: Arc<dyn ProfileHosts> = match handle.try_state::<ProfilesHost>() {
        Some(t7) => t7.0.clone(),
        None => Arc::new(StubProfileHosts),
    };
    // The shared R5 (#16). Without it R6 gets the R5 test double and fails closed.
    let cli: Arc<dyn CliRunner> = match handle.try_state::<SharedCli>() {
        Some(r5) => r5.0.clone(),
        None => Arc::new(StubCliRunner),
    };
    // ADR 0117: `genie_ask` only for home and available profiles. A missing workspace host is
    // not managed in the app; then the gate is empty and allows nothing (fail closed). Only the
    // test builders have no gate.
    let workspaces = handle.try_state::<WorkspacesHost>();
    let gate = match &workspaces {
        Some(w) => w.gate(),
        None => WorkspacesHost::default().gate(),
    };
    let host = build_with_gate(bucket, hosts, cli, Some(gate))?;
    if let Some(w) = &workspaces {
        w.bind_sign_in(host.sign_in_source());
    }
    handle.manage(host);
    Ok(())
}
