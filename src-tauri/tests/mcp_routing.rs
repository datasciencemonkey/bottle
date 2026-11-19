//! #467 (#30, ADR 0040): the Tauri host sends `mcp_call` to R8 `McpCallComponent` through the
//! hardened gateway. Only the brain may call it (SEC-02a). R8 runs here on a fake transport, so
//! no network is used. Every value is plainly fake.

use bottle_lib::ipc::{route, BrainLink, HostComponents};
use bottle_lib::mcp_host;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::rate_bucket::RateBucket;
use mog_core::ipc::ChannelRef;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mcp_transport::{McpCall, McpCallComponent, McpReply, McpTransport};
use mog_core::types::Tags;
use serde_json::{json, Value};
use std::time::Duration;

/// A bucket that never makes a call wait. The tests only check who holds it.
struct FakeBucket;

impl RateBucket for FakeBucket {
    fn reserve(&self, _profile: &str) -> Duration {
        Duration::ZERO
    }
    fn on_rate_limited(&self, _profile: &str, _retry_after: Option<Duration>) {}
}

fn fake_bucket() -> Arc<dyn RateBucket> {
    Arc::new(FakeBucket)
}
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
    fn notify_hub(&self, _notice: Value) -> bool {
        false
    }
    fn subscribe_hub(&self, _channel: &ChannelRef) -> bool {
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
}

fn call(
    from: &str,
    mcp: Option<&dyn GatewayComponents>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let components = HostComponents {
        mcp,
        ..HostComponents::none(from, &NoBrain)
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
    // #479 (ARC-F1, G-07b): an R8 `mcp_call` code in `ERROR_CODES` crosses as itself.
    // Another R8 code (for example `transport_failed`) becomes `host_failed` (ADR 0040).
    for (code, crosses) in [
        ("tool_not_allowed", "tool_not_allowed"),
        ("transport_failed", "host_failed"),
    ] {
        let r8 = Arc::new(FakeR8 {
            fail: Some(code),
            ..FakeR8::default()
        });
        let comp = McpCallComponent::new(r8.clone());
        let err = call("brain", Some(&comp), "mcp_call", ask_args("genie_ask")).unwrap_err();
        assert_eq!(err, json!({ "error": crosses }));
        let text = err.to_string();
        for needle in ["bottle-test", "revenue", MOG, "R8"] {
            assert!(!text.contains(needle), "{needle} in {text}");
        }
    }
}

#[test]
fn g_04d_w_06a_the_host_builds_r8_on_r6_and_the_r5_profile_hosts() {
    // The production R8: one R6 CliTokenBroker, the R5 profile hosts and R7 for the SCIM seam.
    let host = mcp_host::build(fake_bucket()).expect("R8 starts");
    // `build` takes the R5 test double: the call fails closed with a fixed code, before any send.
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

#[test]
fn g_09a_arc_f3_r7_and_r8_in_the_host_share_the_bucket_passed_in() {
    // G-09a, ARC-F3: R7 (the R6 SCIM seam) and R8 take the SAME bucket, so the AIMD cut and
    // Retry-After hold of a profile are shared. The test holds one Arc, R7 one, R8 one.
    let bucket = fake_bucket();
    let host = mcp_host::build(bucket.clone()).expect("R8 starts");
    assert_eq!(
        Arc::strong_count(&bucket),
        3,
        "R7 and R8 must each hold the bucket passed in, and no other"
    );
    drop(host);
    assert_eq!(
        Arc::strong_count(&bucket),
        1,
        "the host keeps no extra clone"
    );
}

#[test]
fn g_09a_arc_f3_the_app_manages_one_shared_bucket_that_mcp_host_reads() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let lib = std::fs::read_to_string(src.join("lib.rs")).unwrap();
    // One bucket per process, managed before `setup` runs any `*_host::install`.
    let managed = lib
        .find("rate_bucket_host::SharedRateBucket(")
        .expect("lib.rs manages the SharedRateBucket");
    let install = lib.find("mcp_host::install(handle)").unwrap();
    assert!(
        managed < install,
        "the bucket is managed before mcp_host installs"
    );
    let host = std::fs::read_to_string(src.join("mcp_host.rs")).unwrap();
    assert!(host.contains("try_state::<SharedRateBucket>()"));
    assert!(host.contains("ReqwestMcpTransport::new(hosts, broker, bucket)"));
}

#[test]
fn g_09a_arc_f3_src_tauri_builds_no_aimd_bucket_outside_lib_rs() {
    // #511: every R7 or R8 in the app takes the managed SharedRateBucket; none builds its own.
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut builds = Vec::new();
    for entry in std::fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "rs") {
            let text = std::fs::read_to_string(&path).unwrap();
            for _ in text.matches("AimdBucket::new(") {
                builds.push(path.file_name().unwrap().to_string_lossy().into_owned());
            }
        }
    }
    assert_eq!(
        builds,
        vec!["lib.rs".to_owned()],
        "AimdBucket::new sites: {builds:?}"
    );
}
