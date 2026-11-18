//! #467 (#30, ADR 0040): the Tauri host sends `mcp_call` to R8 `McpCallComponent` through the
//! hardened gateway. Only the brain may call it (SEC-02a). R8 runs here on a fake transport, so
//! no network is used. Every value is plainly fake.

use bottle_lib::ipc::{route, BrainLink, HostComponents};
use bottle_lib::mcp_host;
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::ChannelRef;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mcp_transport::{McpCall, McpCallComponent, McpReply, McpTransport};
use mog_core::types::Tags;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const MOG: &str = "00000000-0000-4000-8000-0000000000b7";

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

/// One call that reached the fake R8.
#[derive(Debug, Clone, PartialEq)]
enum Seen {
    Call {
        profile: String,
        tool: String,
        args: Value,
        mog: String,
    },
    List {
        profile: String,
    },
}

/// A fake R8: it records each call and answers with a fixed reply, or with `fail`.
#[derive(Default)]
struct FakeR8 {
    seen: Mutex<Vec<Seen>>,
    fail: Option<&'static str>,
}

impl FakeR8 {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    fn answer<T>(&self, ok: T) -> CoreResult<T> {
        match self.fail {
            Some(reason) => Err(CoreError::Unavailable {
                component: "R8",
                reason,
            }),
            None => Ok(ok),
        }
    }
}

impl McpTransport for FakeR8 {
    fn call_tool(&self, call: McpCall<'_>) -> CoreResult<McpReply> {
        self.seen.lock().unwrap().push(Seen::Call {
            profile: call.profile.to_owned(),
            tool: call.tool.to_owned(),
            args: json!(call.args),
            mog: call.tags.mog.clone(),
        });
        self.answer(McpReply {
            structured_content: json!({"conversation_id": "conv-fake-1"}),
            text: "fake answer".to_owned(),
            is_error: false,
            http_status: 200,
            rpc_error: None,
        })
    }
    fn list_tools(&self, profile: &str, _tags: &Tags) -> CoreResult<Vec<Value>> {
        self.seen.lock().unwrap().push(Seen::List {
            profile: profile.to_owned(),
        });
        self.answer(vec![json!({"name": "genie_ask"})])
    }
    fn start_loopback_proxy(&self) -> CoreResult<u16> {
        Err(CoreError::NotImplemented { component: "R8" })
    }
}

fn call(
    from: &str,
    mcp: Option<&dyn GatewayComponents>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let components = HostComponents {
        from,
        brain: &NoBrain,
        windows: None,
        hotkeys: None,
        permissions: None,
        consent: None,
        config: None,
        mcp,
        capture: None,
        capture_ticket: None,
        app_settings: None,
    };
    route(components, command, args)
}

fn ask_args(tool: &str) -> Value {
    json!({
        "profile": "bottle-test",
        "server": "genie_one",
        "tool": tool,
        "args": {"question": "What was revenue last quarter in the fake region?"},
        "opId": "op-1",
        "tags": {"app": "bottle", "mog": MOG, "op": "genie"}
    })
}

#[test]
fn at_07_g_04a_brain_mcp_call_reaches_mcp_call_component() {
    let r8 = Arc::new(FakeR8::default());
    let comp = McpCallComponent::new(r8.clone());
    let out = call("brain", Some(&comp), "mcp_call", ask_args("genie_ask")).unwrap();
    assert_eq!(
        out,
        json!({
            "structuredContent": {"conversation_id": "conv-fake-1"},
            "text": "fake answer",
            "isError": false,
            "httpStatus": 200,
        })
    );
    assert_eq!(
        r8.seen(),
        vec![Seen::Call {
            profile: "bottle-test".into(),
            tool: "genie_ask".into(),
            args: json!({"question": "What was revenue last quarter in the fake region?"}),
            mog: MOG.into(),
        }]
    );
}

#[test]
fn at_07_g_06a_tools_list_reaches_r8_list_tools() {
    let r8 = Arc::new(FakeR8::default());
    let comp = McpCallComponent::new(r8.clone());
    let out = call("brain", Some(&comp), "mcp_call", ask_args("tools/list")).unwrap();
    assert_eq!(
        out["structuredContent"]["tools"],
        json!([{"name": "genie_ask"}])
    );
    assert_eq!(
        r8.seen(),
        vec![Seen::List {
            profile: "bottle-test".into()
        }]
    );
}

#[test]
fn sec_02a_at_17_a_view_window_cannot_call_mcp_call() {
    let r8 = Arc::new(FakeR8::default());
    let comp = McpCallComponent::new(r8.clone());
    for from in ["hub", "mog-b7", "card-b7"] {
        assert_eq!(
            call(from, Some(&comp), "mcp_call", ask_args("genie_ask")),
            Err(json!({"error": "window_not_allowed"})),
            "{from}"
        );
    }
    assert!(r8.seen().is_empty(), "R8 got a call from a view window");
}

#[test]
fn sec_02a_the_schema_check_runs_before_r8() {
    let r8 = Arc::new(FakeR8::default());
    let comp = McpCallComponent::new(r8.clone());
    let mut a = ask_args("genie_ask");
    a["url"] = json!("https://example.invalid/api/2.0/mcp/genie");
    assert_eq!(
        call("brain", Some(&comp), "mcp_call", a),
        Err(json!({"error": "invalid_args"}))
    );
    assert!(r8.seen().is_empty());
}

#[test]
fn sec_02a_no_r8_answers_host_failed() {
    assert_eq!(
        call("brain", None, "mcp_call", ask_args("genie_ask")),
        Err(json!({"error": "host_failed"}))
    );
    // The window check still runs first.
    assert_eq!(
        call("hub", None, "mcp_call", ask_args("genie_ask")),
        Err(json!({"error": "window_not_allowed"}))
    );
}

#[test]
fn sec_03_an_r8_error_crosses_as_a_content_free_code() {
    let r8 = Arc::new(FakeR8 {
        fail: Some("tool_not_allowed"),
        ..FakeR8::default()
    });
    let comp = McpCallComponent::new(r8.clone());
    let err = call("brain", Some(&comp), "mcp_call", ask_args("genie_ask")).unwrap_err();
    // A component code outside `ERROR_CODES` becomes `host_failed` (ADR 0040).
    assert_eq!(err, json!({"error": "host_failed"}));
    let text = err.to_string();
    for needle in ["bottle-test", "revenue", MOG, "R8"] {
        assert!(!text.contains(needle), "{needle} in {text}");
    }
}

#[test]
fn g_04d_w_06a_the_host_builds_r8_on_r6_and_the_r5_profile_hosts() {
    // The production R8: one R6 CliTokenBroker, the R5 profile hosts and R7 for the SCIM seam.
    let host = mcp_host::build().expect("R8 starts");
    // R5 (#16, #17) is not on main: the call fails closed with a fixed code, before any send.
    let err = call("brain", Some(&host), "mcp_call", ask_args("genie_ask")).unwrap_err();
    assert_eq!(err, json!({"error": "not_implemented"}));
    // The window check holds on the production component too.
    assert_eq!(
        call("card-b7", Some(&host), "mcp_call", ask_args("genie_ask")),
        Err(json!({"error": "window_not_allowed"}))
    );
}

#[test]
fn g_04a_the_app_installs_r8_and_reads_it_with_try_state() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let ipc = std::fs::read_to_string(src.join("ipc.rs")).unwrap();
    assert!(ipc.contains("try_state::<crate::mcp_host::McpHost>()"));
    let lib = std::fs::read_to_string(src.join("lib.rs")).unwrap();
    assert!(lib.contains("mcp_host::install(handle)"));
    let host = std::fs::read_to_string(src.join("mcp_host.rs")).unwrap();
    assert!(host.contains("ReqwestMcpTransport::new("));
    assert!(host.contains("CliTokenBroker::new("));
}
