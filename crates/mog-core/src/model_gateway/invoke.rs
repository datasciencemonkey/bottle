//! The `model_invoke` host component (ADR 0122, clean mode).
//!
//! The IPC command `model_invoke` runs one chat call on the home profile. It uses
//! [`GatewayChatClient::chat_tagged`], so the host rules, the token broker, the request tags and
//! the shared rate bucket of R7 all apply.
//!
//! - RTR-20a: a request with a tool field (any case) is refused (`invalid_args`), here as well as
//!   in the gateway, because Rust trusts no caller. The request is an allowlist: it must be a
//!   plain chat request (`responses::from_chat_request`), so a Responses body (`input`), a
//!   `prompt`, a `previous_response_id` and non-text parts are refused too.
//! - Home only: the profile must be the home profile, read from the workspace store. With no home
//!   the call is refused. Bottle never picks a profile.
//! - The model is the user's orchestrator model, not a value from the request.
//! - The deadline is `deadlineMs`; the `router` role defaults to [`ROUTER_DEADLINE_MS`]. Rust does
//!   not retry after a timeout.
//! - Only chat calls are served. The `embed` role is not wired.
//! - The reply is in chat-completions shape (`{json: {choices: [...]}}`), the one that the brain
//!   reads. A reply that is not one plain answer is `bad_reply`.

use std::sync::atomic::{AtomicBool, Ordering};
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
use crate::wipe::AgentWipeHook;

const ID: &str = "model_invoke";

/// The deadline of the `router` role when the call gives none. It equals the brain's
/// `DICTATION_CLEANUP_DEADLINE_MS`; the brain passes `deadlineMs` as well.
pub const ROUTER_DEADLINE_MS: u32 = 1500;
/// The deadline of every other role when the call gives none.
pub const DEFAULT_DEADLINE_MS: u32 = 30_000;
/// The longest deadline a call may ask for.
pub const MAX_DEADLINE_MS: u32 = 120_000;

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
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
    home: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// Set to abandon every call in flight (Wipe, quit). Reset it to allow calls again.
    stop: Arc<AtomicBool>,
}

impl std::fmt::Debug for ModelInvokeComponent {
    /// Shows nothing: no profile or model.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelInvokeComponent")
            .finish_non_exhaustive()
    }
}

impl ModelInvokeComponent {
    /// `model` returns the user's orchestrator model, or `None` for the default. `home` returns
    /// the home profile (read only), or `None` when there is none.
    pub fn new(
        client: Arc<GatewayChatClient>,
        model: Arc<dyn Fn() -> Option<String> + Send + Sync>,
        home: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    ) -> Self {
        Self {
            client,
            model,
            home,
            stop: Arc::new(AtomicBool::new(false)),
        }
    }

    /// The flag that abandons every call in flight and refuses new ones while it is set.
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }
}

/// The Wipe hook of the agent session, plus the stop flag of `model_invoke`: the calls in flight
/// are abandoned while the session ends, and calls are allowed again after the wipe hook returns.
pub struct AbandonOnWipe {
    inner: Arc<dyn AgentWipeHook>,
    stop: Arc<AtomicBool>,
}

impl AbandonOnWipe {
    pub fn new(inner: Arc<dyn AgentWipeHook>, stop: Arc<AtomicBool>) -> Self {
        Self { inner, stop }
    }
}

impl AgentWipeHook for AbandonOnWipe {
    fn end_for_wipe(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.inner.end_for_wipe();
        self.stop.store(false, Ordering::SeqCst);
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
        // Home only (ADR 0117): the profile is the home profile, never another one.
        if (self.home)().is_none_or(|h| h != a.profile) {
            return Err(refused("invalid_args"));
        }
        let request: serde_json::Map<String, Value> = a.request.into_iter().collect();
        // A plain chat request only. No `input`, no tools, no other field.
        let body = responses::from_chat_request(&request).ok_or_else(|| refused("invalid_args"))?;
        let model = (self.model)()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_ORCHESTRATOR_MODEL.to_owned());
        let reply = self
            .client
            .chat_tagged(
                ChatRequest {
                    profile: &a.profile,
                    model: &model,
                    body: Value::Object(body),
                    deadline: deadline_of(&a.role, a.deadline_ms),
                    cancel: Some(&self.stop),
                },
                a.tags,
            )
            .map_err(ipc_code)?;
        // The brain reads the chat-completions shape: one translation point (`{json: ...}`).
        let chat = responses::to_chat_reply(&reply).ok_or(CoreError::Unavailable {
            component: ID,
            reason: "bad_reply",
        })?;
        Ok(json!({ "json": chat }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording_cloud::http::ScriptedHttp;
    use serde_json::json;

    const ROUTE: &str = "mlflow.responses";

    fn comp_home(
        h: &Arc<ScriptedHttp>,
        model: Option<&str>,
        home: Option<&str>,
    ) -> ModelInvokeComponent {
        let m = model.map(str::to_owned);
        let home = home.map(str::to_owned);
        ModelInvokeComponent::new(
            Arc::new(GatewayChatClient::new(h.clone(), "unused")),
            Arc::new(move || m.clone()),
            Arc::new(move || home.clone()),
        )
    }

    fn comp(h: &Arc<ScriptedHttp>, model: Option<&str>) -> ModelInvokeComponent {
        comp_home(h, model, Some("home"))
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
        // A Responses body, tool fields in any case, other fields and non-text parts.
        let m = json!([{"role": "user", "content": "x"}]);
        for request in [
            json!({"input": [], "tools": [{"type": "function", "name": "x"}]}),
            json!({"input": [], "tool_choice": "auto"}),
            json!({"input": [{"type": "function_call", "call_id": "c", "name": "n"}]}),
            json!({"input": [{"type": "function_call_output", "call_id": "c", "output": "o"}]}),
            json!({"messages": m, "Tools": []}),
            json!({"messages": m, "prompt": {"id": "p"}}),
            json!({"messages": m, "previous_response_id": "r"}),
            json!({"messages": [{"role": "user", "content": [{"type": "mcp_call"}]}]}),
            json!({"messages": [{"role": "user", "content": [{"type": "input_image"}]}]}),
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
        assert_eq!(ms(ModelRole::Router, None), 1500);
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

    #[test]
    fn adr_0122_model_invoke_serves_only_the_home_profile() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(ROUTE, 200, reply());
        for home in [None, Some("other")] {
            let e = comp_home(&h, None, home)
                .call("model_invoke", args(json!({})))
                .unwrap_err();
            assert_eq!(e.code(), "invalid_args");
        }
        assert!(h.seen().is_empty());
        assert!(comp(&h, None).call("model_invoke", args(json!({}))).is_ok());
    }

    #[test]
    fn adr_0122_model_invoke_a_reply_that_is_not_one_plain_answer_is_bad_reply() {
        let msg = json!({"type": "message", "content": [{"type": "output_text", "text": "a"}]});
        let refusal = json!({"type": "message", "content": [{"type": "refusal", "refusal": "no"}]});
        for r in [
            json!({"output": [msg, msg]}),
            json!({"output": [refusal]}),
            json!({"status": "incomplete", "output": [msg]}),
        ] {
            let h = Arc::new(ScriptedHttp::new());
            h.json(ROUTE, 200, r);
            let e = comp(&h, None)
                .call("model_invoke", args(json!({})))
                .unwrap_err();
            assert_eq!(e.code(), "bad_reply");
        }
    }

    #[test]
    fn adr_0122_model_invoke_stop_flag_refuses_calls() {
        let h = Arc::new(ScriptedHttp::new());
        let c = comp(&h, None);
        c.stop_flag()
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let e = c.call("model_invoke", args(json!({}))).unwrap_err();
        assert_eq!(e.code(), "host_failed");
        assert!(h.seen().is_empty());
    }

    #[test]
    fn adr_0122_wipe_abandons_model_calls_during_the_wipe_and_allows_them_after() {
        struct Inner(Arc<AtomicBool>, std::sync::Mutex<Vec<bool>>);
        impl AgentWipeHook for Inner {
            fn end_for_wipe(&self) {
                self.1.lock().unwrap().push(self.0.load(Ordering::SeqCst));
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let inner = Arc::new(Inner(Arc::clone(&stop), std::sync::Mutex::new(Vec::new())));
        let hook = AbandonOnWipe::new(inner.clone(), Arc::clone(&stop));
        hook.end_for_wipe();
        assert_eq!(
            *inner.1.lock().unwrap(),
            vec![true],
            "set while the wipe runs"
        );
        assert!(!stop.load(Ordering::SeqCst), "clear after the wipe");
    }
}
