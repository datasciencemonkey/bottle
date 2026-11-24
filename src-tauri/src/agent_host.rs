//! The one R16 AgentHost of the process (#307, #112, ADR 0014). `agent_start`, `agent_stop` and
//! `agent_send` reach it through the hardened gateway and [`crate::ipc::HostComponents`], so
//! only the brain can call them (ARC-4). Its `agent.status` events go to the brain Channel
//! through [`crate::ipc::BrainChannel::send`], which scrubs each payload (SEC-01).
//!
//! Tokens never reach the harness (ARC-7, RTR-28): the model proxy binds loopback and holds no
//! token broker that can give one ([`StubTokenBroker`]), and its outbound hop is
//! [`UnavailableUpstream`].
//!
//! The app has no model target resolver and no runtime program yet, so every `agent_start`
//! answers `agent_target_unavailable` before any runtime starts, and the brain routes with the
//! fallback classifier (RTR-15b).
//! TODO(#16): the real [`ModelTargets`] reads the MOG's home profile and host from R5/R6.
//! TODO(#111): SP-19 picks the runtime program, the real outbound hop and the token broker.

use std::sync::Arc;

use mog_core::agent_guard::AgentActionLog;
use mog_core::agent_host::ipc_route::{AgentComponent, BrainStatusSink};
use mog_core::agent_host::model_proxy::{ModelProxy, ModelTarget, UnavailableUpstream};
use mog_core::agent_host::runtime::{RuntimeConfig, StdRuntimeLauncher};
use mog_core::agent_host::{
    AgentError, AgentHostDeps, ModelTargets, RuntimeConfigs, SharedAgentHost,
};
use mog_core::agent_run::guard::AgentGuard;
use mog_core::agent_run::seam::{GuardDeps, RunEvents};
use mog_core::bottle_mcp::{BottleMcpDeps, EventEmit, UnavailableLocalTools, UnavailableRelay};
use mog_core::clock::SystemClock;
use mog_core::error::CoreResult;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mcp_transport::proxy::{GenieProxyDeps, UnavailableGenieTargets};
use mog_core::mcp_transport::McpTransport;
use mog_core::token_broker::StubTokenBroker;
use mog_core::workspaces::WorkspaceGate;
use serde_json::Value;
use tauri::Manager;

use crate::ipc::BrainChannel;

/// No MOG has a model target yet. TODO(#16): the R5/R6 home profile and host.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoModelTargets;

impl ModelTargets for NoModelTargets {
    fn target(&self, _mog_id: &str) -> Result<ModelTarget, AgentError> {
        Err(AgentError::TargetUnavailable)
    }
}

/// The `agent_*` component of the app: the one [`SharedAgentHost`] of the process.
#[derive(Debug, Clone)]
pub struct AgentHostApp(AgentComponent);

impl AgentHostApp {
    /// The host.
    pub fn host(&self) -> &Arc<SharedAgentHost> {
        self.0.host()
    }
}

impl GatewayComponents for AgentHostApp {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.0.call(command, args)
    }
}

/// The Genie One proxy deps of the main agent on the ONE R8 and the workspace allow-list `gate`
/// (ADR 0117, FLAG 7). The proxy fails closed: without a gate it refuses every `genie_ask`, so
/// the host always passes the `WorkspaceStore` gate (`WorkspacesHost::gate`). The Genie targets
/// stay unavailable until #563, so no ask goes out yet.
pub fn genie_proxy_deps(
    transport: Arc<dyn McpTransport>,
    gate: Arc<dyn WorkspaceGate>,
) -> GenieProxyDeps {
    GenieProxyDeps::new(transport, Arc::new(UnavailableGenieTargets)).with_gate(Some(gate))
}

/// The guard of the agent session (R2) on the ONE R8, the `WorkspaceStore` gate (a switch turned
/// off applies at once), the brain event sink and the action log. The orchestrator host calls
/// it in process.
pub fn agent_guard(
    transport: Arc<dyn McpTransport>,
    gate: Arc<dyn WorkspaceGate>,
    events: Arc<dyn RunEvents>,
    action_log: Arc<dyn AgentActionLog>,
) -> Arc<AgentGuard> {
    AgentGuard::new(GuardDeps {
        transport,
        gate,
        events,
        action_log,
        clock: Arc::new(SystemClock::new()),
    })
}

/// Build the one AgentHost. Its `agent.status` events go to `emit`. An `Err` means the model
/// proxy could not bind loopback.
pub fn build(emit: EventEmit) -> std::io::Result<AgentHostApp> {
    // TODO(#111): the token broker and the outbound hop of SP-19.
    let proxy = ModelProxy::start(Arc::new(StubTokenBroker), Arc::new(UnavailableUpstream))?;
    // TODO(#111): SP-19 names the runtime programs. With no model target no runtime starts.
    let runtimes = RuntimeConfigs {
        claude: RuntimeConfig::new("bottle-agent-runtime-claude", Vec::new()),
        codex: RuntimeConfig::new("bottle-agent-runtime-codex", Vec::new()),
    };
    // TODO(#563): the app host wires the R10 local tools, the brain tool relay, the one audit
    // log of the process and the Genie One proxy targets.
    let bottle_mcp =
        BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(UnavailableRelay));
    let host = SharedAgentHost::new(AgentHostDeps {
        launcher: Arc::new(StdRuntimeLauncher::default()),
        proxy: Arc::new(proxy),
        targets: Arc::new(NoModelTargets),
        status: Arc::new(BrainStatusSink::new(emit)),
        runtimes,
        bottle_mcp,
    });
    Ok(AgentHostApp(AgentComponent::new(host)))
}

/// Build the AgentHost and manage it, so the `agent_*` commands reach it. If the proxy cannot
/// bind loopback, the app still runs and the `agent_*` commands answer `host_failed`.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) -> std::io::Result<()> {
    let app = handle.clone();
    let emit: EventEmit = Arc::new(move |event, payload| {
        app.state::<BrainChannel>().send(event, payload);
    });
    handle.manage(build(emit)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mog_core::mcp_transport::StubMcpTransport;

    #[derive(Debug)]
    struct Closed;

    impl WorkspaceGate for Closed {
        fn allows(&self, _: &str) -> bool {
            false
        }
    }

    #[test]
    fn the_genie_proxy_deps_carry_the_workspace_gate() {
        let deps = genie_proxy_deps(Arc::new(StubMcpTransport), Arc::new(Closed));
        let gate = deps.gate.expect("the gate is wired");
        assert!(!gate.allows("any"));
    }
}
