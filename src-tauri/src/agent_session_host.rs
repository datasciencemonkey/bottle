//! Slice 2a (the MOG agent on one live Genie Code session): the host side of the
//! `genie_code_*`, `agent_turn_*` and `agent_session_*` commands.
//!
//! This is the contract-PR component. It answers every command with `not_implemented`. The
//! session host lane (R1, `mog_core::agent_run::session`) replaces it with the real component:
//! the Genie Code binary check, the one app-server process, the turn queue and the wipe hook.

use serde_json::Value;

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc_gateway::GatewayComponents;

/// The managed agent component. Contract stub: every call is `not_implemented`.
#[derive(Debug, Default)]
pub struct AgentSessionHost;

impl GatewayComponents for AgentSessionHost {
    fn call(&self, _command: &'static str, _args: Value) -> CoreResult<Value> {
        Err(CoreError::IpcRefused {
            code: "not_implemented",
        })
    }
}
