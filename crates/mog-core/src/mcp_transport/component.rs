//! `mcp_call` (4.5) as an R12 gateway component: it parses [`McpCallArgs`] and calls R8.
//!
//! The gateway runs the window and schema checks first (SEC-02a, SEC-02b), so only the brain
//! reaches this. `tool: "tools/list"` lists the allowlisted tools; any other tool goes to
//! [`McpTransport::call_tool`], which refuses every tool not on the G-06a allowlist.
//! The Tauri host routes `mcp_call` here (#467, `src-tauri/src/mcp_host.rs`).

use std::time::Duration;

use serde_json::{json, Value};

use super::{McpCall, McpTransport};
use crate::error::{CoreError, CoreResult};
use crate::ipc::McpCallArgs;
use crate::ipc_gateway::GatewayComponents;

/// The `mcp_call` component over one R8.
#[derive(Debug, Clone)]
pub struct McpCallComponent<T> {
    transport: T,
}

impl<T: McpTransport> McpCallComponent<T> {
    /// The component over `transport`.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

impl<T: McpTransport> GatewayComponents for McpCallComponent<T> {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        if command != "mcp_call" {
            return Err(refused("not_implemented"));
        }
        let a: McpCallArgs = serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        if a.tool == "tools/list" {
            let tools = self.transport.list_tools(&a.profile, &a.tags)?;
            return Ok(json!({
                "structuredContent": {"tools": tools},
                "text": "",
                "isError": false,
                "httpStatus": 200,
            }));
        }
        let mut call = McpCall::new(&a.profile, &a.tool, a.args, &a.tags);
        if let Some(m) = &a.meta {
            call = call.meta(m);
        }
        if let Some(ms) = a.timeout_ms {
            call = call.timeout(Duration::from_millis(u64::from(ms)));
        }
        self.transport.call_tool(call).map(|r| r.to_ipc_json())
    }
}
