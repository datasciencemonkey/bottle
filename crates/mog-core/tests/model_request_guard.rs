//! #117: small-model calls never carry tools (RTR-20a, AT-17).
//!
//! The router, cleanup and writer calls go through `model_invoke`. The R12 gateway refuses a
//! `model_invoke` request with a tool field before any component runs. ASR goes through
//! `stt_transcribe`, which has no request body and refuses unknown fields.
//! No test reads a real profile or a real token.

use std::sync::{Arc, Mutex};

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc_gateway::{hardened_gateway_with, GatewayComponents, GatewayLog, IpcGateway};
use mog_core::model_request_guard::{tool_fields, TOOL_FIELDS};
use serde_json::{json, Value};

const ROLES: &[&str] = &["cleanup", "router", "writer", "judge", "embed", "merge"];

/// Counts the calls that reach the components.
#[derive(Clone, Default)]
struct Counting(Arc<Mutex<usize>>);

impl GatewayComponents for Counting {
    fn call(&self, _command: &'static str, _args: Value) -> CoreResult<Value> {
        *self.0.lock().unwrap() += 1;
        Ok(json!({"json": {}}))
    }
}

#[derive(Clone, Default)]
struct Lines(Arc<Mutex<Vec<String>>>);

impl GatewayLog for Lines {
    fn record(&self, line: &str) {
        self.0.lock().unwrap().push(line.to_owned());
    }
}

fn invoke_args(role: &str, request: Value) -> Value {
    json!({
        "role": role,
        "profile": "caller-profile",
        "request": request,
        "tags": {"app": "bottle", "mog": "a1", "op": role},
    })
}

fn refused_code(r: CoreResult<Value>) -> Option<&'static str> {
    match r {
        Err(CoreError::IpcRefused { code }) => Some(code),
        _ => None,
    }
}

#[test]
fn rtr_20a_tool_fields_names_every_chat_completions_tool_field() {
    let mut names = TOOL_FIELDS.to_vec();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "function_call",
            "functions",
            "parallel_tool_calls",
            "tool_choice",
            "tools"
        ]
    );
    let all = json!({
        "messages": [], "tools": [], "tool_choice": "auto", "functions": [],
        "function_call": "auto", "parallel_tool_calls": true,
    });
    let mut found = tool_fields(&all);
    found.sort_unstable();
    assert_eq!(found, names);
}

#[test]
fn rtr_20a_tool_fields_is_empty_for_a_router_request() {
    let req = json!({
        "messages": [{"role": "system", "content": "fixed"}, {"role": "user", "content": "x"}],
        "temperature": 0,
        "response_format": {"type": "json_schema", "json_schema": {"name": "mog_router_decision"}},
    });
    assert!(tool_fields(&req).is_empty());
    assert!(tool_fields(&json!("not an object")).is_empty());
}

#[test]
fn rtr_20a_at_17_gateway_refuses_model_invoke_with_a_tool_field_for_every_role() {
    for role in ROLES {
        for field in TOOL_FIELDS {
            let counting = Counting::default();
            let gw = hardened_gateway_with(Lines::default(), counting.clone());
            let mut request = json!({"messages": []});
            request[*field] = json!([{"type": "function"}]);
            let r = gw.dispatch("brain", "model_invoke", invoke_args(role, request));
            assert_eq!(refused_code(r), Some("invalid_args"), "{role} {field}");
            assert_eq!(
                *counting.0.lock().unwrap(),
                0,
                "{role} {field} reached a component"
            );
        }
    }
}

#[test]
fn rtr_20a_gateway_passes_a_tool_free_model_invoke() {
    let counting = Counting::default();
    let gw = hardened_gateway_with(Lines::default(), counting.clone());
    let request = json!({"messages": [{"role": "user", "content": "x"}], "temperature": 0});
    let r = gw.dispatch("brain", "model_invoke", invoke_args("router", request));
    assert!(r.is_ok());
    assert_eq!(*counting.0.lock().unwrap(), 1);
}

#[test]
fn rtr_20a_refusal_log_line_holds_no_request_content() {
    let lines = Lines::default();
    let gw = hardened_gateway_with(lines.clone(), Counting::default());
    let request = json!({
        "messages": [{"role": "user", "content": "zebra-falcon"}],
        "tools": [{"type": "function", "function": {"name": "switch_workspace"}}],
    });
    let _ = gw.dispatch("brain", "model_invoke", invoke_args("writer", request));
    let all = lines.0.lock().unwrap().join("\n");
    assert!(all.contains("outcome=invalid_args"), "{all}");
    assert!(!all.contains("zebra-falcon"));
    assert!(!all.contains("switch_workspace"));
}

#[test]
fn rtr_20a_stt_transcribe_refuses_a_tools_field() {
    let counting = Counting::default();
    let gw = hardened_gateway_with(Lines::default(), counting.clone());
    let args = json!({
        "segmentId": "s1", "provider": "dbx_whisper",
        "tags": {"app": "bottle", "mog": "a1", "op": "asr"},
        "tools": [{"type": "function"}],
    });
    let r = gw.dispatch("brain", "stt_transcribe", args);
    assert_eq!(refused_code(r), Some("invalid_args"));
    assert_eq!(*counting.0.lock().unwrap(), 0);
}
