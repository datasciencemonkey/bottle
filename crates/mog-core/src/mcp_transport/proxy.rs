//! The R8 loopback proxy for the Genie One MCP calls of the main agent (#139, ADR 0083).
//!
//! RTR-27, RTR-28, ARC-7, 4.6 rule 9: the harness reaches Genie One only through the
//! `genie-one-mcp` listener of its session ([`crate::loopback::LoopbackSession`]). The listener
//! checks the `Host` header and the per-session secret before this code runs, and scrubs every
//! reply. The harness never holds a Databricks token: this proxy sends each call to the SAME
//! [`McpTransport`] that the brain uses through `mcp_call`, and R8 adds the token inside the core.
//! So each agent call gets the R8 rules of the brain:
//! - G-06a: only the allowlisted tools; the arguments match the tool contract.
//! - G-04a..G-05b: the R8 session, a fresh token for each HTTP request (only to the profile
//!   host, 4.6 rule 2), the 401 and 404 rules.
//! - G-09a..G-10b: the shared per-profile rate bucket and the 429 rules (#138).
//! - G-07b, ARC-F1: the R8 in-flight poll guard (#31).
//! - 4.6 rule 7: the reply is scrubbed.
//!
//! The proxy adds the rules that only the session can know:
//! - 4.6 rule 5, RTR-29d: the profile and the warehouse come from [`GenieTargets`] for the
//!   session's MOG, never from the agent. A `profile` argument and the agent's `_meta` are
//!   dropped (G-06b, G-07a). The request tags name the session's MOG.
//! - G-20b: `genie_get_query_result` takes only an `item_id` that matches `^pub_.+_output$`.
//! - Failures go back as content-free codes, never as upstream text.
//!
//! The proxy answers `initialize`, `ping` and notifications itself; only `tools/list` and
//! `tools/call` reach R8. The R8 call runs on its own thread, so a session stop never waits for
//! Genie: the waiting request ends within [`STOP_POLL`] with [`SESSION_CLOSED`].
//!
//! The brain-side rules of G-11..G-20a (the card, the PollActor schedule, the give-up and the
//! in-flight ask caps per MOG and per profile) are not in this proxy; ADR 0083 lists them.
//!
//! TODO(#307): the app host gives the real [`GenieTargets`] (the MOG's talk target, ADR 0058).
//! TODO(#111): SP-19 fixes the MCP wire details of the harness (streamable HTTP, SSE).
//!
//! The dispatch (local `initialize` and `ping`, tool failures as tool results, only Bottle's own
//! texts in an error) follows `dispatch`, `tool_result` and `describe` of the reference plugin.
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::{forwardable_tool_args, tools, McpCall, McpReply, McpTransport, PROTOCOL_VERSION};
use super::{INVALID_META, INVALID_TOOL_ARGS, TOOL_NOT_ALLOWED};
use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::loopback::{LoopbackRequest, LoopbackResponse};
use crate::token_guard::scrub_json;
use crate::types::{TagApp, TagOp, Tags};

/// The HTTP path of the proxy endpoint.
pub const PROXY_PATH: &str = "/mcp";
/// The MOG has no Genie target (no profile yet). Nothing is sent.
pub const TARGET_UNAVAILABLE: &str = "genie_target_unavailable";
/// The session stopped while the call waited.
pub const SESSION_CLOSED: &str = "session_closed";
/// The proxy could not start the call.
pub const PROXY_UNAVAILABLE: &str = "genie_proxy_unavailable";
/// Genie One answered with a JSON-RPC error.
pub const GENIE_ERROR: &str = "genie_error";
/// A waiting call wakes at least this often to see whether the session stopped.
pub const STOP_POLL: Duration = Duration::from_millis(50);

/// Where the Genie One calls of a MOG go: its CLI profile and its SQL warehouse.
#[derive(Clone, PartialEq, Eq)]
pub struct GenieTarget {
    /// The CLI profile. The user picked it; the proxy and the agent never pick one.
    pub profile: String,
    /// The SQL warehouse of the MOG, sent as `_meta.warehouse_id` on `genie_ask` (G-06b).
    pub warehouse_id: Option<String>,
}

impl fmt::Debug for GenieTarget {
    /// No profile name and no warehouse ID.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenieTarget").finish_non_exhaustive()
    }
}

/// Resolves the Genie target of a MOG. The proxy asks for each call, so a user-approved
/// workspace change (RTR-29d) applies to the next call with no new session.
pub trait GenieTargets: Send + Sync {
    /// The target of `mog_id`.
    fn target(&self, mog_id: &str) -> CoreResult<GenieTarget>;
}

/// No resolver is wired: every MOG has no target, so nothing is sent. TODO(#307).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableGenieTargets;

impl GenieTargets for UnavailableGenieTargets {
    fn target(&self, _mog_id: &str) -> CoreResult<GenieTarget> {
        Err(CoreError::NotImplemented { component: "R8" })
    }
}

/// What the Genie One proxy of a session uses.
#[derive(Clone)]
pub struct GenieProxyDeps {
    /// The ONE R8 of the process, the same one that serves the brain's `mcp_call`.
    pub transport: Arc<dyn McpTransport>,
    /// The Genie target of each MOG.
    pub targets: Arc<dyn GenieTargets>,
    /// The time cap of each call; `None` is the R8 default.
    pub call_timeout: Option<Duration>,
}

impl fmt::Debug for GenieProxyDeps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenieProxyDeps")
            .field("call_timeout", &self.call_timeout)
            .finish_non_exhaustive()
    }
}

impl GenieProxyDeps {
    /// Deps over `transport` and `targets`, with the R8 default time cap.
    pub fn new(transport: Arc<dyn McpTransport>, targets: Arc<dyn GenieTargets>) -> Self {
        Self {
            transport,
            targets,
            call_timeout: None,
        }
    }
}

/// G-20b: a query result item ID must match `^pub_.+_output$`.
pub fn valid_item_id(id: &str) -> bool {
    id.strip_prefix("pub_")
        .and_then(|r| r.strip_suffix("_output"))
        .is_some_and(|mid| !mid.is_empty() && !mid.contains('\n'))
}

/// The Genie One proxy of one agent session.
pub struct GenieProxy {
    mog_id: String,
    deps: GenieProxyDeps,
    closed: Arc<AtomicBool>,
}

impl fmt::Debug for GenieProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GenieProxy").finish_non_exhaustive()
    }
}

fn rpc_result(id: Value, result: Value) -> LoopbackResponse {
    LoopbackResponse {
        status: 200,
        body: json!({ "jsonrpc": "2.0", "id": id, "result": result }),
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> LoopbackResponse {
    LoopbackResponse {
        status: 200,
        body: json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    }
}

fn http_error(status: u16, code: &str) -> LoopbackResponse {
    LoopbackResponse {
        status,
        body: json!({ "error": code }),
    }
}

/// A failed call as an MCP tool result: the agent sees the code and can act on it.
fn tool_error(code: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": code }], "isError": true })
}

/// The R8 reply as an MCP tool result, scrubbed (rule 7).
fn tool_result(r: &McpReply) -> Value {
    if let Some(e) = &r.rpc_error {
        return json!({
            "content": [{ "type": "text", "text": GENIE_ERROR }],
            "structuredContent": { "httpStatus": r.http_status, "rpcError": scrub_json(e) },
            "isError": true,
        });
    }
    let mut out = Map::new();
    let content: Vec<Value> = if r.text.is_empty() {
        Vec::new()
    } else {
        vec![json!({ "type": "text", "text": r.text })]
    };
    out.insert("content".into(), Value::Array(content));
    if !r.structured_content.is_null() {
        out.insert(
            "structuredContent".into(),
            scrub_json(&r.structured_content),
        );
    }
    out.insert("isError".into(), Value::Bool(r.is_error));
    scrub_json(&Value::Object(out))
}

/// The R8 refusals that mean "the request is wrong", as JSON-RPC errors.
fn is_bad_request(code: &str) -> bool {
    matches!(code, TOOL_NOT_ALLOWED | INVALID_TOOL_ARGS | INVALID_META)
}

impl GenieProxy {
    /// The proxy of the session of `mog_id`. `closed` turns true when the session stops.
    pub fn new(mog_id: &str, deps: GenieProxyDeps, closed: Arc<AtomicBool>) -> Self {
        Self {
            mog_id: mog_id.to_owned(),
            deps,
            closed,
        }
    }

    /// Answer one authorized request of the harness.
    pub fn handle(&self, req: &LoopbackRequest) -> LoopbackResponse {
        if req.path != PROXY_PATH {
            return http_error(404, "not_found");
        }
        if req.method != "POST" {
            return http_error(405, "method_not_allowed");
        }
        self.mcp(&req.body)
    }

    fn tags(&self) -> Tags {
        Tags {
            app: TagApp::Bottle,
            mog: self.mog_id.clone(),
            op: TagOp::Genie,
        }
    }

    /// Run `f` on its own thread and wait for it, or for the session stop.
    fn off_thread<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, &'static str> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SESSION_CLOSED);
        }
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("bottle-genie-proxy-call".into())
            .spawn(move || {
                let _ = tx.send(f());
            })
            .map_err(|_| PROXY_UNAVAILABLE)?;
        loop {
            match rx.recv_timeout(STOP_POLL) {
                Ok(v) => return Ok(v),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if self.closed.load(Ordering::SeqCst) {
                        // The R8 call ends on its own time cap; its result is dropped.
                        return Err(SESSION_CLOSED);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(PROXY_UNAVAILABLE),
            }
        }
    }

    fn target(&self) -> Result<GenieTarget, &'static str> {
        self.deps
            .targets
            .target(&self.mog_id)
            .map_err(|_| TARGET_UNAVAILABLE)
    }

    fn tools_list(&self, id: Value) -> LoopbackResponse {
        let target = match self.target() {
            Ok(t) => t,
            Err(code) => return rpc_error(id, -32000, code),
        };
        let transport = Arc::clone(&self.deps.transport);
        let tags = self.tags();
        let listed = self.off_thread(move || {
            transport
                .list_tools(&target.profile, &tags)
                .map_err(|e| e.code())
        });
        match listed {
            Ok(Ok(tools)) => rpc_result(id, json!({ "tools": tools })),
            Ok(Err(code)) | Err(code) => rpc_error(id, -32000, code),
        }
    }

    fn tools_call(&self, id: Value, params: Option<&Value>) -> LoopbackResponse {
        let params = params.and_then(Value::as_object);
        let name = params.and_then(|p| p.get("name")).and_then(Value::as_str);
        // G-06a, before anything else: only the allowlisted tools exist here.
        let Some(contract) = name.and_then(tools::allowed) else {
            return rpc_error(id, -32602, "unknown tool");
        };
        let args: JsonObject = match params.and_then(|p| p.get("arguments")) {
            None | Some(Value::Null) => JsonObject::new(),
            Some(Value::Object(a)) => a.clone().into_iter().collect(),
            Some(_) => return rpc_error(id, -32602, "invalid arguments"),
        };
        // 4.6 rule 5, RTR-29d: the agent never names the profile.
        let args = forwardable_tool_args(args);
        if !tools::args_match(contract, &args) {
            return rpc_error(id, -32602, "invalid arguments");
        }
        // G-20b.
        if contract.name == "genie_get_query_result"
            && !args
                .get("item_id")
                .and_then(Value::as_str)
                .is_some_and(valid_item_id)
        {
            return rpc_error(id, -32602, "invalid arguments");
        }
        let target = match self.target() {
            Ok(t) => t,
            Err(code) => return rpc_result(id, tool_error(code)),
        };
        // G-06b, G-07a: the MOG's warehouse only; the agent's `_meta` is never read.
        let meta: Option<JsonObject> = target
            .warehouse_id
            .as_ref()
            .map(|w| JsonObject::from([("warehouse_id".to_owned(), json!(w))]));
        let transport = Arc::clone(&self.deps.transport);
        let tags = self.tags();
        let timeout = self.deps.call_timeout;
        let tool = contract.name;
        let reply = self.off_thread(move || {
            let mut call = McpCall::new(&target.profile, tool, args, &tags);
            if let Some(m) = &meta {
                call = call.meta(m);
            }
            if let Some(t) = timeout {
                call = call.timeout(t);
            }
            transport.call_tool(call).map_err(|e| e.code())
        });
        match reply {
            Ok(Ok(r)) => rpc_result(id, tool_result(&r)),
            Ok(Err(code)) if is_bad_request(code) => rpc_error(id, -32602, code),
            Ok(Err(code)) | Err(code) => rpc_result(id, tool_error(code)),
        }
    }

    fn mcp(&self, body: &[u8]) -> LoopbackResponse {
        let Ok(msg) = serde_json::from_slice::<Value>(body) else {
            return rpc_error(Value::Null, -32700, "parse error");
        };
        let Some(obj) = msg.as_object() else {
            return rpc_error(Value::Null, -32600, "invalid request");
        };
        let id = obj.get("id").cloned();
        let method = obj.get("method").and_then(Value::as_str);
        let (Some("2.0"), Some(method)) = (obj.get("jsonrpc").and_then(Value::as_str), method)
        else {
            return rpc_error(id.unwrap_or(Value::Null), -32600, "invalid request");
        };
        let Some(id) = id else {
            // A notification: accepted, not forwarded. R8 runs its own upstream session.
            return LoopbackResponse {
                status: 202,
                body: Value::Null,
            };
        };
        match method {
            "initialize" => rpc_result(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": { "listChanged": false } },
                    "serverInfo": { "name": "bottle-genie-one-proxy", "version": env!("CARGO_PKG_VERSION") },
                }),
            ),
            "ping" => rpc_result(id, json!({})),
            "tools/list" => self.tools_list(id),
            "tools/call" => self.tools_call(id, obj.get("params")),
            _ => rpc_error(id, -32601, "method not found"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_20b_item_id_pattern() {
        for ok in ["pub_1_output", "pub_a_b_output", "pub_x__output"] {
            assert!(valid_item_id(ok), "{ok}");
        }
        for bad in [
            "pub__output",
            "pub_output",
            "x_pub_1_output",
            "pub_1_output\n",
            "pub_\n_output",
            "",
        ] {
            assert!(!valid_item_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn sec_03_genie_target_debug_hides_the_profile() {
        let t = GenieTarget {
            profile: "secret-profile".into(),
            warehouse_id: Some("wh-secret".into()),
        };
        let text = format!("{t:?}");
        assert!(!text.contains("secret"), "{text}");
    }

    #[test]
    fn arc_6_tool_result_keeps_no_upstream_error_text_outside_the_scrub() {
        let r = McpReply {
            structured_content: Value::Null,
            text: String::new(),
            is_error: true,
            http_status: 400,
            rpc_error: Some(json!({"code": 1, "message": "password=not-real-pass-2"})),
        };
        let v = tool_result(&r);
        assert_eq!(v["isError"], json!(true));
        assert_eq!(v["content"][0]["text"], json!(GENIE_ERROR));
        assert!(!v.to_string().contains("not-real-pass-2"), "{v}");
    }
}
