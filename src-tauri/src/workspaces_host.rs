//! ADR 0117 (one MOG with workspaces): the host component of the `workspace_*` commands.
//!
//! TODO(one-mog slice 1, S1-A): this is the contract-PR stub. It answers every
//! `workspace_*` command with `not_implemented`. S1-A replaces it with the `WorkspaceStore`
//! over `workspaces.json` (the migration from the MOG configs, the switches, the home, the
//! agent settings and the sign-in probe), and passes its `WorkspaceGate` to R8.

use serde_json::Value;

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc_gateway::GatewayComponents;

/// The managed `workspace_*` component. Stub: every call is `not_implemented`.
#[derive(Debug, Default)]
pub struct WorkspacesHost;

impl GatewayComponents for WorkspacesHost {
    fn call(&self, _command: &'static str, _args: Value) -> CoreResult<Value> {
        Err(CoreError::IpcRefused {
            code: "not_implemented",
        })
    }
}
