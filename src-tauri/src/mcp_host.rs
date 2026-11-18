//! The one R8 McpTransport of the process (#467, #30). `mcp_call` reaches it through the
//! hardened gateway and [`crate::ipc::HostComponents`], so only the brain can call it (SEC-02a).
//!
//! [`build`] makes one R6 [`CliTokenBroker`] on the R5 profile hosts, and one
//! [`ReqwestMcpTransport::new`] over them (G-04a, G-04d: a fresh token for each request). It
//! binds the R6 SCIM seam to one R7 [`ReqwestHttpTransport`], so R6 can read the workspace ID
//! for `?o=` (W-06a). The token stays in R6: no token goes to a window, a log or the disk.
//!
//! R5 is not on main yet, so R6 and R8 run on the R5 stubs: every `mcp_call` fails closed with
//! `not_implemented` before any send. TODO(#17): the R5 ProfileService profile hosts.
//! TODO(#16): the R5 CliRunner that runs `databricks auth token`.

use std::sync::{Arc, Weak};

use mog_core::cli_runner::StubCliRunner;
use mog_core::clock::{SystemClock, SystemWallClock};
use mog_core::error::CoreResult;
use mog_core::http_transport::rate_bucket::AimdBucket;
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport, StubProfileHosts};
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mcp_transport::{McpCallComponent, ReqwestMcpTransport};
use mog_core::token_broker::{CliTokenBroker, ScimMe};
use serde_json::Value;
use tauri::Manager;

/// The `mcp_call` component of the app: R8 over R6, with the R7 that serves the R6 SCIM seam.
pub struct McpHost {
    component: McpCallComponent<Arc<ReqwestMcpTransport>>,
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

/// Build the one R8 of the process. An `Err` means the HTTP client could not start.
pub fn build() -> CoreResult<McpHost> {
    // TODO(#17): the R5 ProfileService profile hosts (`databricks auth profiles`).
    let hosts: Arc<dyn ProfileHosts> = Arc::new(StubProfileHosts);
    let mono = Arc::new(SystemClock::new());
    let broker = Arc::new(CliTokenBroker::new(
        // TODO(#16): the R5 CliRunner that runs `databricks auth token --profile <p>`.
        Arc::new(StubCliRunner),
        hosts.clone(),
        mono.clone(),
        Arc::new(SystemWallClock),
    ));
    let scim = Arc::new(ReqwestHttpTransport::new(
        hosts.clone(),
        broker.clone(),
        Arc::new(AimdBucket::new(mono)),
    )?);
    let weak: Weak<dyn ScimMe> = Arc::downgrade(&scim) as Weak<dyn ScimMe>;
    broker.bind_scim(weak);
    let transport = Arc::new(ReqwestMcpTransport::new(hosts, broker)?);
    Ok(McpHost {
        component: McpCallComponent::new(transport),
        _scim: scim,
    })
}

/// Build R8 and manage it, so `mcp_call` reaches it. If R8 cannot start, the app still runs
/// and `mcp_call` answers `host_failed`.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) -> CoreResult<()> {
    handle.manage(build()?);
    Ok(())
}
