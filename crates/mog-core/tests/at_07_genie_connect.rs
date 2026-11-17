//! AT-07 "Connect and poll Genie One" (#234), section 15: the Rust part. Pass: the URL has `?o=`;
//! polls are serial with the D31 cadence; IDs persist; the Beta URL is rejected.
//!
//! What runs here is the `mcp_call` contract part: only the brain may call Genie One MCP
//! (SEC-02a), the server is always `genie_one`, and no argument can carry a URL, so a view or the
//! brain cannot point R8 at the Beta `/api/2.0/mcp/genie` endpoint or a per-space MCP. These two
//! IPC checks carry no G-03 tag: G-03 itself (the config loader refuses the Beta URL) is #28.
//! The thin JSON-RPC client R8 (#30) runs here on a fake Genie One server through
//! `McpCallComponent`; the Tauri host routing of `mcp_call` is TODO(#467). The workspace ID of
//! R6 (#18) runs on a scripted SCIM seam. The other tests are `#[ignore = "pending TODO(#n)"]`. The serial poll
//! queue (G-07b) is TS and runs in `src/acceptance/at-07-genie-connect.test.ts`. The live run
//! skips without `BOTTLE_TEST_PROFILE` (docs/tests/AT-GENIE-MCP.md).

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use mog_core::ipc::{self, IpcError, WindowClass};
use mog_core::ipc_gateway::{hardened_gateway_with, IpcGateway, NoLog};
use mog_core::mcp_transport::{McpCallComponent, McpLimits, McpSeams, ReqwestMcpTransport};
use serde_json::{json, Value};

const MOG: &str = "00000000-0000-4000-8000-0000000000b7";

fn ask_args() -> Value {
    json!({
        "profile": "bottle-test",
        "server": "genie_one",
        "tool": "genie_ask",
        "args": {"question": "What was revenue last quarter?"},
        "opId": "op-1",
        "tags": {"app": "bottle", "mog": MOG, "op": "genie"}
    })
}

#[test]
fn at_07_sec_02a_only_the_brain_may_call_genie_one_mcp() {
    let spec = ipc::command("mcp_call").expect("mcp_call is in the 4.5 table");
    assert_eq!(spec.windows, [WindowClass::Brain]);
    for label in ["hub", "mog-b7", "card-b7"] {
        let err = ipc::handle(label, "mcp_call", ask_args()).unwrap_err();
        assert!(
            matches!(err, IpcError::WindowNotAllowed { .. }),
            "{label}: {err:?}"
        );
    }
    // The brain passes the window check. R8 is `McpCallComponent` (#30); the Tauri host routes
    // `mcp_call` to it in TODO(#467), so `ipc::handle` alone still ends in NotImplemented.
    let err = ipc::handle("brain", "mcp_call", ask_args()).unwrap_err();
    assert!(matches!(err, IpcError::NotImplemented { .. }), "{err:?}");
}

#[test]
fn at_07_mcp_call_names_only_the_genie_one_server() {
    for server in ["genie", "genie_space", "genie_one_api", "", "GENIE_ONE"] {
        let mut a = ask_args();
        a["server"] = json!(server);
        let err = ipc::handle("brain", "mcp_call", a).unwrap_err();
        assert!(
            matches!(err, IpcError::InvalidArgs { .. }),
            "{server:?}: {err:?}"
        );
    }
}

#[test]
fn at_07_mcp_call_takes_no_url() {
    for (key, url) in [
        ("url", "https://h.example.test/api/2.0/mcp/genie"),
        (
            "endpoint",
            "https://h.example.test/api/2.0/mcp/genie/space-1",
        ),
        ("baseUrl", "https://h.example.test/api/2.0/genie/one/sse"),
    ] {
        let mut a = ask_args();
        a[key] = json!(url);
        let err = ipc::handle("brain", "mcp_call", a).unwrap_err();
        assert!(
            matches!(err, IpcError::InvalidArgs { .. }),
            "{key}: {err:?}"
        );
    }
}

#[test]
#[ignore = "pending TODO(#28): the Genie One MCP endpoint config is not built"]
fn at_07_g_02_g_03_endpoint_has_o_and_rejects_beta_urls() {
    // When #28 lands: the endpoint for host h and workspace 123 is
    // https://h/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=123 (G-02). The config loader
    // refuses /api/2.0/mcp/genie (with "end of life 2026-11-30"), the per-space Genie MCP and the
    // Genie One API SSE with a clear error (G-03). SP-09 `src/endpoint.rs` is the design.
    todo!("#28: build the endpoint and assert the G-02 URL and the three G-03 refusals")
}

/// An R8 on the fake Genie One server and fake broker of `support/fake_mcp.rs`.
fn r8_on(s: &fake_mcp::FakeMcp) -> (Arc<ReqwestMcpTransport>, Arc<fake_mcp::FakeBroker>) {
    let broker = Arc::new(fake_mcp::FakeBroker::default());
    let t = ReqwestMcpTransport::with_seams(McpSeams {
        hosts: fake_mcp::Hosts::one(&s.origin()),
        endpoints: fake_mcp::Endpoints::one(&s.url()),
        tokens: broker.clone(),
        policy: Arc::new(fake_mcp::LoopbackPolicy),
        decorator: Arc::new(mog_core::request_identity::BottleIdentity::new()),
        log: Arc::new(fake_mcp::Lines::default()),
        limits: McpLimits::default(),
    })
    .unwrap();
    (Arc::new(t), broker)
}

fn r8_ask_args(tool: &str) -> Value {
    let mut a = ask_args();
    a["profile"] = json!(fake_mcp::PROFILE);
    a["tool"] = json!(tool);
    a["tags"]["mog"] = json!(fake_mcp::MOG);
    a
}

#[test]
fn at_07_g_04a_g_04b_g_04c_g_04d_g_05a_g_06b_g_07a_thin_client_rules() {
    // Against a scripted Genie One server (support/fake_mcp.rs), through the gateway component.
    // The full matrix (G-05b, 401, caps, logs) is in tests/mcp_transport.rs.
    let first_call = std::sync::atomic::AtomicBool::new(true);
    let s = fake_mcp::genie_one(move |x| {
        (x.rpc_method() == "tools/call" && first_call.swap(false, Ordering::SeqCst))
            .then(|| fake_mcp::empty_reply(404))
    });
    let (t, broker) = r8_on(&s);
    let gw = hardened_gateway_with(NoLog, McpCallComponent::new(t));
    let mut a = r8_ask_args("genie_ask");
    a["meta"] = json!({"warehouse_id": "w1", "progressToken": 1});
    let out = gw.dispatch("brain", "mcp_call", a).unwrap();
    assert_eq!(out["isError"], false);
    let seen = s.seen();
    // G-04c: initialize names 2025-06-18.
    assert_eq!(seen[0].json()["params"]["protocolVersion"], "2025-06-18");
    // G-05a: the 404 with a session initialises again one time and resends.
    assert_eq!(
        s.methods(),
        [
            "initialize",
            "notifications/initialized",
            "tools/call",
            "initialize",
            "notifications/initialized",
            "tools/call"
        ]
    );
    // G-04b: each call after initialize carries the session ID.
    assert_eq!(seen[2].header("mcp-session-id"), Some("sess-1"));
    assert_eq!(seen[5].header("mcp-session-id"), Some("sess-2"));
    // G-04d: one fresh token per request on one transport.
    assert_eq!(broker.lend_count(), seen.len());
    // G-02: every call carries ?o=. G-06b: _meta.warehouse_id. G-07a: no progressToken.
    for x in &seen {
        assert!(x.target.ends_with("?o=1234"));
        assert!(!String::from_utf8_lossy(&x.body).contains("progressToken"));
    }
    assert_eq!(seen[5].json()["params"]["_meta"], json!({"warehouse_id": "w1"}));
}

#[test]
fn at_07_g_06a_mcp_call_refuses_tools_outside_the_allowlist() {
    let s = fake_mcp::genie_one(|_| None);
    let (t, broker) = r8_on(&s);
    let gw = hardened_gateway_with(NoLog, McpCallComponent::new(t));
    for tool in ["execute_sql", "view_ask", "genie_delete_conversation"] {
        let err = gw
            .dispatch("brain", "mcp_call", r8_ask_args(tool))
            .unwrap_err();
        assert_eq!(err.code(), "tool_not_allowed", "{tool}");
    }
    assert!(s.seen().is_empty());
    assert_eq!(broker.lend_count(), 0);
}

/// A scripted SCIM seam (never a live workspace): counts calls, answers org ID 1234.
#[derive(Default)]
struct ScriptedScim {
    calls: std::sync::atomic::AtomicUsize,
}

impl mog_core::token_broker::ScimMe for ScriptedScim {
    fn org_id_header(&self, _profile: &str) -> mog_core::error::CoreResult<Option<String>> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Some("1234".to_owned()))
    }
}

struct OneHost;

impl mog_core::http_transport::ProfileHosts for OneHost {
    fn workspace_url(&self, _profile: &str) -> mog_core::error::CoreResult<String> {
        Ok("https://h.cloud.databricks.com".to_owned())
    }
}

#[test]
fn at_07_w_06a_w_06b_workspace_id_from_the_org_id_header_once_per_profile() {
    use mog_core::token_broker::{CliTokenBroker, TokenBroker};
    use std::sync::Arc;
    // R6 reads workspace_id from the x-databricks-org-id header of
    // GET /api/2.0/preview/scim/v2/Me (W-06a) and caches it, so two lookups for one profile make
    // one SCIM call (W-06b). R7 reads the header on the wire in http_transport_reqwest.rs.
    let r6 = CliTokenBroker::new(
        Arc::new(mog_core::cli_runner::StubCliRunner),
        Arc::new(OneHost),
        Arc::new(mog_core::clock::SimClock::new()),
        Arc::new(mog_core::clock::SimWallClock::at(0)),
    );
    let scim = Arc::new(ScriptedScim::default());
    let weak = Arc::downgrade(&scim);
    r6.bind_scim(weak);
    assert_eq!(r6.workspace_id("bottle-test").unwrap(), "1234");
    assert_eq!(r6.workspace_id("bottle-test").unwrap(), "1234");
    assert_eq!(scim.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    // AT-07 pass: the URL has ?o=.
    let url = r6
        .genie_one_endpoint("bottle-test")
        .unwrap()
        .url()
        .to_owned();
    assert!(url.ends_with("/system.ai.genie_one_mcp?o=1234"), "{url}");
}

#[test]
#[ignore = "pending TODO(#29): live AT-07 needs a person with a U2M profile (needs:u2m)"]
fn at_07_live_connect_list_tools_ask_and_poll() {
    let w = mog_core::live_or_skip!();
    // Steps in docs/tests/AT-GENIE-MCP.md, part AT-07: resolve the workspace ID, run tools/list,
    // ask one question, poll it to a terminal status. Never print the token or the answer.
    let _ = w.profile;
    todo!("#29: run the live AT-07 script through R8")
}
