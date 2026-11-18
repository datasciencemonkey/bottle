//! R8 McpTransport (section 4.3).
//!
//! Responsibilities: **Thin JSON-RPC client on `reqwest`, fresh token per request** (not the static `rmcp` header). Streamable HTTP, `Mcp-Session-Id`, protocol 2025-06-18, one re-initialise on 404 with a session, in-flight poll guard. Loopback proxy for the Genie One MCP calls of the main agent.
//!
//! The real R8 is [`ReqwestMcpTransport`] (#30, ADR 0069):
//! - G-04a: JSON-RPC 2.0 over MCP streamable HTTP on blocking `reqwest`; no `rmcp`.
//!   `initialize`, `notifications/initialized`, `tools/list` and `tools/call`; JSON and SSE replies.
//! - G-04b, G-04c: one `Mcp-Session-Id` per (profile, MOG); protocol [`PROTOCOL_VERSION`].
//! - G-04d: a token from R6 for each HTTP request, only for the profile host (4.6 rule 2).
//!   A 401 drops that token once and resends once (W-05d).
//! - G-05a, G-05b: a 404 with a session initialises again one time; without one it is
//!   [`GENIE_ONE_OFF`].
//! - G-06a: only the tools in [`ALLOWED_TOOLS`]; anything else is refused before anything is
//!   sent, and `tools/list` drops every other tool. G-06b, G-07a: `_meta` holds only
//!   `warehouse_id`, only on `genie_ask`.
//! - Caps on reply size and time ([`McpLimits`]). No token, request content or session ID in a
//!   log, `Debug` output or an error.
//!
//! - G-07b, ARC-F1 (#31, ADR 0073): one `genie_poll_response` at a time for each conversation;
//!   a second one fails at once with [`E_POLL_INFLIGHT`] ([`poll`]).
//! - RTR-27, RTR-28 (#139, ADR 0083): the loopback proxy for the Genie One MCP calls of the main
//!   agent is [`proxy`]. Each agent session has its own proxy listener and secret; the proxy sends
//!   every call to the same R8, so the agent's calls get every rule above, the poll guard too.

use std::fmt;
use std::time::Duration;

use serde_json::{json, Value};

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::request_identity::{BottleIdentity, CallKind, IdentityError, RequestDecorator};
use crate::token_guard::strip_profile_keys;
use crate::types::Tags;

pub mod agent_asks;
mod client;
mod component;
pub mod poll;
pub mod proxy;
pub mod sse;
pub mod tools;

pub use client::*;
pub use component::McpCallComponent;
pub use poll::{PollInflightGuard, E_POLL_INFLIGHT};
pub use tools::{ToolContract, ALLOWED_TOOLS};

const ID: &str = "R8";

/// The MCP protocol version that R8 speaks (G-04c).
pub const PROTOCOL_VERSION: &str = "2025-06-18";

/// The tool is not on the G-06a allowlist.
pub const TOOL_NOT_ALLOWED: &str = "tool_not_allowed";
/// The tool arguments do not match the tool contract (`genie_tools.json`).
pub const INVALID_TOOL_ARGS: &str = "invalid_tool_args";
/// `meta.warehouse_id` is not a warehouse ID.
pub const INVALID_META: &str = "invalid_meta";
/// The profile name is empty. R8 never picks a profile.
pub const PROFILE_INVALID: &str = "profile_invalid";
/// G-05b: Genie One is not on in the workspace (a 404 without a session).
pub const GENIE_ONE_OFF: &str = "genie_one_off";
/// The server named another protocol version than [`PROTOCOL_VERSION`].
pub const PROTOCOL_UNSUPPORTED: &str = "protocol_unsupported";
/// The reply holds no JSON-RPC response for the request.
pub const BAD_REPLY: &str = "bad_reply";

/// The text the card shows for [`GENIE_ONE_OFF`] (G-05b). The brain GL-5 mapper
/// (`src/brain/components/genie-errors.ts`, #32, ADR 0072) gives the same text for the
/// `genie_one_off` code. The code is in `ipc_gateway::ERROR_CODES` (#479), so the IPC
/// rejection of `mcp_call` is `{ "error": "genie_one_off" }`.
pub fn genie_one_off_message(host: &str) -> String {
    format!("Genie One isn't available in {host}.")
}

fn err(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// One `tools/call` for R8. The session is the one of (`profile`, `tags.mog`).
pub struct McpCall<'a> {
    /// The CLI profile. The caller passes it; R8 never picks one.
    pub profile: &'a str,
    /// The tool name. Only [`ALLOWED_TOOLS`] are sent.
    pub tool: &'a str,
    /// The tool arguments. `profile` keys are removed before they go out (4.6 rule 5).
    pub args: JsonObject,
    /// The `meta` of `mcp_call`. Only `warehouse_id` is used, and only for `genie_ask`.
    pub meta: Option<&'a JsonObject>,
    /// The request tags (SEC-06a). `tags.mog` names the MCP session.
    pub tags: &'a Tags,
    /// The per-call time cap, at most [`McpLimits::max_call_timeout`].
    pub timeout: Option<Duration>,
}

impl<'a> McpCall<'a> {
    /// A call with no `meta` and the default time cap.
    pub fn new(profile: &'a str, tool: &'a str, args: JsonObject, tags: &'a Tags) -> Self {
        Self {
            profile,
            tool,
            args,
            meta: None,
            tags,
            timeout: None,
        }
    }

    /// Set the `meta` of `mcp_call`.
    pub fn meta(mut self, meta: &'a JsonObject) -> Self {
        self.meta = Some(meta);
        self
    }

    /// Set the per-call time cap.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

impl fmt::Debug for McpCall<'_> {
    /// Shows only the argument count: no profile, tool name, argument or MOG ID.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpCall")
            .field("args", &self.args.len())
            .finish_non_exhaustive()
    }
}

/// The reply of one `tools/call`: the `mcp_call` result of the IPC table (4.5).
#[derive(Clone, PartialEq)]
pub struct McpReply {
    /// `result.structuredContent`, or `null`. Scrubbed (rule 7).
    pub structured_content: Value,
    /// The `text` items of `result.content`, joined with a new line. Scrubbed (rule 7).
    pub text: String,
    /// `result.isError`, or true for a JSON-RPC error or an HTTP error.
    pub is_error: bool,
    /// The HTTP status of the reply.
    pub http_status: u16,
    /// The JSON-RPC `error` (or the JSON body of an HTTP error). Scrubbed (rule 7).
    pub rpc_error: Option<Value>,
}

impl McpReply {
    /// The `mcp_call` reply JSON: `{structuredContent, text, isError, httpStatus, rpcError?}`.
    pub fn to_ipc_json(&self) -> Value {
        let mut v = json!({
            "structuredContent": self.structured_content,
            "text": self.text,
            "isError": self.is_error,
            "httpStatus": self.http_status,
        });
        if let Some(e) = &self.rpc_error {
            v["rpcError"] = e.clone();
        }
        v
    }
}

impl fmt::Debug for McpReply {
    /// Shows only the status and the error flag: no server text.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpReply")
            .field("http_status", &self.http_status)
            .field("is_error", &self.is_error)
            .finish_non_exhaustive()
    }
}

/// A thin MCP JSON-RPC client that gets a fresh token from R6 for each request.
pub trait McpTransport: Send + Sync {
    /// Call one allowlisted MCP tool and return its reply.
    fn call_tool(&self, call: McpCall<'_>) -> CoreResult<McpReply>;
    /// `tools/list` for `profile` in the session of `tags.mog`: only the allowlisted tools.
    fn list_tools(&self, profile: &str, tags: &Tags) -> CoreResult<Vec<Value>>;
}

impl<T: McpTransport + ?Sized> McpTransport for std::sync::Arc<T> {
    fn call_tool(&self, call: McpCall<'_>) -> CoreResult<McpReply> {
        (**self).call_tool(call)
    }
    fn list_tools(&self, profile: &str, tags: &Tags) -> CoreResult<Vec<Value>> {
        (**self).list_tools(profile, tags)
    }
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
    tags: &Tags,
) -> Result<Vec<(&'static str, String)>, IdentityError> {
    BottleIdentity::new().headers(CallKind::Mcp, Some(tags))
}

/// The R8 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubMcpTransport;

impl McpTransport for StubMcpTransport {
    fn call_tool(&self, _call: McpCall<'_>) -> CoreResult<McpReply> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn list_tools(&self, _profile: &str, _tags: &Tags) -> CoreResult<Vec<Value>> {
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

    #[test]
    fn sec_03_mcp_call_debug_hides_profile_tool_args_and_mog() {
        let tags = Tags {
            app: TagApp::Bottle,
            mog: "mog-secret-id".into(),
            op: TagOp::Genie,
        };
        let mut args = JsonObject::new();
        args.insert("question".into(), json!("secret question text"));
        let meta = JsonObject::from([("warehouse_id".to_owned(), json!("wh-secret"))]);
        let call = McpCall::new("secret-profile", "secret_tool_name", args, &tags).meta(&meta);
        let text = format!("{call:?}");
        for needle in [
            "secret-profile",
            "secret_tool_name",
            "secret question",
            "mog-secret-id",
            "wh-secret",
        ] {
            assert!(!text.contains(needle), "{needle} in {text}");
        }
        assert_eq!(text, "McpCall { args: 1, .. }");
    }

    #[test]
    fn g_04a_ipc_reply_has_rpc_error_only_when_set() {
        let mut r = McpReply {
            structured_content: Value::Null,
            text: String::new(),
            is_error: true,
            http_status: 200,
            rpc_error: None,
        };
        assert!(r.to_ipc_json().get("rpcError").is_none());
        r.rpc_error = Some(json!({"code": -1}));
        assert_eq!(r.to_ipc_json()["rpcError"], json!({"code": -1}));
    }
}
