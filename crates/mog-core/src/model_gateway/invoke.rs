//! The `model_invoke` host component (ADR 0122, clean mode).
//!
//! The IPC command `model_invoke` runs one chat call on the profile that the brain passes (the
//! home profile). It uses [`GatewayChatClient::chat_tagged`], so the host rules, the token
//! broker, the request tags and the shared rate bucket of R7 all apply.
//!
//! - RTR-20a: a request with a tool field is refused (`invalid_args`), here as well as in the
//!   gateway, because Rust trusts no caller.
//! - The model is the user's orchestrator model, not a value from the request. A `model` key in
//!   the request is replaced.
//! - The deadline is `deadlineMs`; the `router` role defaults to [`ROUTER_DEADLINE_MS`].
//! - Only chat calls are served. The `embed` role is not wired.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::client::GatewayChatClient;
use super::responses;
use super::{ChatRequest, DEFAULT_ORCHESTRATOR_MODEL};
use crate::error::{CoreError, CoreResult};
use crate::ipc::{ModelInvokeArgs, ModelRole};
use crate::ipc_gateway::GatewayComponents;
use crate::model_request_guard::model_invoke_has_tools;

const ID: &str = "model_invoke";

/// The deadline of the `router` role when the call gives none (ADR 0122).
pub const ROUTER_DEADLINE_MS: u32 = 900;
/// The deadline of every other role when the call gives none.
pub const DEFAULT_DEADLINE_MS: u32 = 30_000;
/// The longest deadline a call may ask for.
pub const MAX_DEADLINE_MS: u32 = 120_000;

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

/// Whether a Responses `input` list holds a tool item (a call or a result): RTR-20a for the
/// Responses shape, where tools can also come in as items.
fn has_tool_items(request: &serde_json::Map<String, Value>) -> bool {
    request
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|i| {
                matches!(
                    i.get("type").and_then(Value::as_str),
                    Some("function_call" | "function_call_output")
                )
            })
        })
}

/// The client's fine-grained code as a code that already crosses IPC (`ERROR_CODES`):
/// `signed_out` is `auth_expired`; `model_not_found`, `refused` and `cancelled` are
/// `host_failed`. The rest keep their codes. The fine codes stay inside Rust.
fn ipc_code(e: CoreError) -> CoreError {
    let reason = match e.code() {
        "signed_out" => "auth_expired",
        "model_not_found" | "refused" | "cancelled" => "host_failed",
        _ => return e,
    };
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// The deadline of a call.
pub fn deadline_of(role: &ModelRole, deadline_ms: Option<u32>) -> Duration {
    let default = match role {
        ModelRole::Router => ROUTER_DEADLINE_MS,
        _ => DEFAULT_DEADLINE_MS,
    };
    Duration::from_millis(u64::from(
        deadline_ms.unwrap_or(default).clamp(1, MAX_DEADLINE_MS),
    ))
}

/// The `model_invoke` command over the gateway chat client.
pub struct ModelInvokeComponent {
    client: Arc<GatewayChatClient>,
    model: Arc<dyn Fn() -> Option<String> + Send + Sync>,
}

impl std::fmt::Debug for ModelInvokeComponent {
    /// Shows nothing: no profile or model.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelInvokeComponent")
            .finish_non_exhaustive()
    }
}

impl ModelInvokeComponent {
    /// `model` returns the user's orchestrator model, or `None` for the default.
    pub fn new(
        client: Arc<GatewayChatClient>,
        model: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> Self {
        Self { client, model }
    }
}

impl GatewayComponents for ModelInvokeComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        if command != ID {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "unknown_command",
            });
        }
        if model_invoke_has_tools(&args) {
            return Err(refused("invalid_args"));
        }
        let a: ModelInvokeArgs =
            serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        if a.role == ModelRole::Embed {
            return Err(refused("not_implemented"));
        }
        let request: serde_json::Map<String, Value> = a.request.into_iter().collect();
        if has_tool_items(&request) {
            return Err(refused("invalid_args"));
        }
        let model = (self.model)()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ORCHESTRATOR_MODEL.to_owned());
        // The brain's router request is in chat shape; the wire is the Responses API.
        let body = responses::from_chat_request(&request);
        let reply = self
            .client
            .chat_tagged(
                ChatRequest {
                    profile: &a.profile,
                    model: &model,
                    body: Value::Object(body),
                    deadline: deadline_of(&a.role, a.deadline_ms),
                    cancel: None,
                },
                a.tags,
            )
            .map_err(ipc_code)?;
        // The brain reads the chat-completions shape: one translation point (`{json: ...}`).
        Ok(json!({ "json": responses::to_chat_reply(&reply) }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording_cloud::http::ScriptedHttp;
    use serde_json::json;

    const ROUTE: &str = "mlflow.responses";

    fn comp(h: &Arc<ScriptedHttp>, model: Option<&str>) -> ModelInvokeComponent {
        let m = model.map(str::to_owned);
        ModelInvokeComponent::new(
            Arc::new(GatewayChatClient::new(h.clone(), "unused")),
            Arc::new(move || m.clone()),
        )
    }

    fn args(extra: Value) -> Value {
        let mut v = json!({
            "role": "router",
            "profile": "home",
            "request": {"messages": [{"role": "user", "content": "hello"}], "model": "evil"},
            "tags": {"app": "bottle", "mog": "mog-1", "op": "router"},
        });
        for (k, x) in extra.as_object().cloned().unwrap_or_default() {
            v[k] = x;
        }
        v
    }

    fn reply() -> Value {
        json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": "{}"}]}]})
    }

    #[test]
    fn adr_0122_model_invoke_calls_the_gateway_with_the_chosen_model_and_ipc_tags() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(ROUTE, 200, reply());
        let out = comp(&h, Some("system.ai.gpt-oss-20b"))
            .call("model_invoke", args(json!({})))
            .unwrap();
        assert_eq!(
            out,
            json!({"json": {"choices": [{"message": {
            "role": "assistant", "content": "{}"}}]}})
        );
        let seen = &h.seen()[0];
        assert_eq!(
            seen.json.as_ref().unwrap()["model"],
            "system.ai.gpt-oss-20b"
        );
        let t = seen.tags.clone().unwrap();
        assert_eq!(t.mog, "mog-1");
        assert_eq!(t.op, crate::types::TagOp::Router);
    }

    #[test]
    fn adr_0122_model_invoke_defaults_to_the_default_model() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(ROUTE, 200, reply());
        comp(&h, None)
            .call("model_invoke", args(json!({})))
            .unwrap();
        assert_eq!(
            h.seen()[0].json.as_ref().unwrap()["model"],
            DEFAULT_ORCHESTRATOR_MODEL
        );
    }

    #[test]
    fn rtr_20a_model_invoke_component_refuses_a_request_with_tools() {
        let h = Arc::new(ScriptedHttp::new());
        for tool in ["tools", "tool_choice", "functions", "function_call"] {
            let mut a = args(json!({}));
            a["request"][tool] = json!([]);
            let e = comp(&h, None).call("model_invoke", a).unwrap_err();
            assert_eq!(e.code(), "invalid_args", "{tool}");
        }
        // A Responses body: tools as fields, and tool items inside `input`.
        for request in [
            json!({"input": [], "tools": [{"type": "function", "name": "x"}]}),
            json!({"input": [], "tool_choice": "auto"}),
            json!({"input": [{"type": "function_call", "call_id": "c", "name": "n"}]}),
            json!({"input": [{"type": "function_call_output", "call_id": "c", "output": "o"}]}),
        ] {
            let e = comp(&h, None)
                .call("model_invoke", args(json!({"request": request})))
                .unwrap_err();
            assert_eq!(e.code(), "invalid_args");
        }
        assert!(h.seen().is_empty());
    }

    #[test]
    fn adr_0122_model_invoke_refuses_bad_args_and_the_embed_role() {
        let h = Arc::new(ScriptedHttp::new());
        let c = comp(&h, None);
        assert_eq!(
            c.call("model_invoke", json!({"role": "router"}))
                .unwrap_err()
                .code(),
            "invalid_args"
        );
        assert_eq!(
            c.call("model_invoke", args(json!({"role": "embed"})))
                .unwrap_err()
                .code(),
            "not_implemented"
        );
        assert!(h.seen().is_empty());
    }

    #[test]
    fn adr_0122_model_invoke_deadline_defaults_and_clamps() {
        let ms = |r, d| deadline_of(&r, d).as_millis();
        assert_eq!(ms(ModelRole::Router, None), 900);
        assert_eq!(ms(ModelRole::Router, Some(400)), 400);
        assert_eq!(ms(ModelRole::Writer, None), 30_000);
        assert_eq!(ms(ModelRole::Router, Some(0)), 1);
        assert_eq!(ms(ModelRole::Router, Some(u32::MAX)), 120_000);
    }

    #[test]
    fn adr_0122_model_invoke_passes_gateway_errors_as_content_free_codes() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(
            ROUTE,
            404,
            json!({"error_code": "NOT_FOUND", "message": "secret"}),
        );
        let e = comp(&h, None)
            .call("model_invoke", args(json!({})))
            .unwrap_err();
        assert_eq!(e.code(), "host_failed");
        h.json(ROUTE, 401, json!({}));
        let e = comp(&h, None)
            .call("model_invoke", args(json!({})))
            .unwrap_err();
        assert_eq!(e.code(), "auth_expired");
    }
}
