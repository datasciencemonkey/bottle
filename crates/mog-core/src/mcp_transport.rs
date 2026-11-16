//! R8 McpTransport (section 4.3).
//!
//! Responsibilities: **Thin JSON-RPC client on `reqwest`, fresh token per request** (not the static `rmcp` header). Streamable HTTP, `Mcp-Session-Id`, protocol 2025-06-18, one re-initialise on 404 with a session, in-flight poll guard. Loopback proxy for the Genie One MCP calls of the main agent.

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::request_identity::{BottleIdentity, CallKind, IdentityError, RequestDecorator};
use crate::token_guard::strip_profile_keys;
use crate::types::Tags;

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

/// The identity headers that R8 must send on every MCP request: the Bottle User-Agent
/// (SEC-06b) and the request tags from the IPC `tags` argument of `mcp_call` (SEC-06a, SP-18).
/// Every [`McpTransport`] implementation must send these (#197).
/// TODO(#136): SP-18 confirms if `MCP_SERVICE` usage rows keep the tags; else attribution
/// relies on the User-Agent.
pub fn outbound_identity_headers(
    _tags: &Tags,
) -> Result<Vec<(&'static str, String)>, IdentityError> {
    let _ = (BottleIdentity::default, CallKind::Mcp);
    todo!("#197 green")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request_identity::{user_agent, TAGS_HEADER, USER_AGENT_HEADER};
    use crate::types::{TagApp, TagOp};

    #[test]
    fn sp_18_sec_06a_sec_06b_mcp_requests_carry_identity_headers() {
        let tags = Tags {
            app: TagApp::Bottle,
            mog: "mog-1".into(),
            op: TagOp::Genie,
        };
        let h = outbound_identity_headers(&tags).unwrap();
        assert!(h.contains(&(USER_AGENT_HEADER, user_agent())));
        assert!(h.contains(&(
            TAGS_HEADER,
            r#"{"app":"bottle","mog":"mog-1","op":"genie"}"#.to_owned()
        )));
    }

    #[test]
    fn sec_06a_mcp_bad_tags_fail_closed() {
        let tags = Tags {
            app: TagApp::Bottle,
            mog: "https://x".into(),
            op: TagOp::Genie,
        };
        assert_eq!(
            outbound_identity_headers(&tags),
            Err(IdentityError::MogIdChars)
        );
    }
}
