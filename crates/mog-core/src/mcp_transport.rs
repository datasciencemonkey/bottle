//! R8 McpTransport (section 4.3).
//!
//! Responsibilities: **Thin JSON-RPC client on `reqwest`, fresh token per request** (not the static `rmcp` header). Streamable HTTP, `Mcp-Session-Id`, protocol 2025-06-18, one re-initialise on 404 with a session, in-flight poll guard. Loopback proxy for the Genie One MCP calls of the main agent.

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::token_guard::strip_profile_keys;

const ID: &str = "R8";

/// A thin MCP JSON-RPC client that gets a fresh token from R6 for each request.
pub trait McpTransport: Send + Sync {
    /// Call one MCP tool for `profile` and return the JSON-RPC result.
    fn call_tool(
        &self,
        profile: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> CoreResult<serde_json::Value>;
    /// Start the loopback proxy for the Genie One MCP calls of the main agent. Returns the port.
    fn start_loopback_proxy(&self) -> CoreResult<u16>;
}

/// The tool arguments that R8 may forward for `mcp_call`: every `profile` key removed (4.6
/// rule 5). Every [`McpTransport`] implementation must forward only this.
pub fn forwardable_tool_args(mut args: JsonObject) -> JsonObject {
    strip_profile_keys(&mut args);
    args
}

/// The R8 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubMcpTransport;

impl McpTransport for StubMcpTransport {
    fn call_tool(
        &self,
        _profile: &str,
        _tool: &str,
        _args: serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        // TODO(#30): thin JSON-RPC client with a tool allowlist.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn start_loopback_proxy(&self) -> CoreResult<u16> {
        // TODO(#139): loopback proxy for the agent's Genie One MCP calls.
        Err(CoreError::NotImplemented { component: ID })
    }
}
