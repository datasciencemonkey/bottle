//! AT-07 "Connect and poll Genie One" (#234), section 15: the Rust part. Pass: the URL has `?o=`;
//! polls are serial with the D31 cadence; IDs persist; the Beta URL is rejected.
//!
//! What runs here is the `mcp_call` contract part: only the brain may call Genie One MCP, the
//! server is always `genie_one`, and no argument can carry a URL, so a view or the brain cannot
//! point R8 at the Beta `/api/2.0/mcp/genie` endpoint or a per-space MCP (G-03, G-06a contract
//! part). The behaviour is not built: the endpoint config is #28, the thin JSON-RPC client R8 is
//! #30 (the SP-09 spike client in `spikes/sp-09` proves the design offline, #29), and the
//! workspace ID of R6 is #18. Those tests are `#[ignore = "pending TODO(#n)"]`. The serial poll
//! queue (G-07b) is TS and runs in `src/acceptance/at-07-genie-connect.test.ts`. The live run
//! skips without `BOTTLE_TEST_PROFILE` (docs/tests/AT-GENIE-MCP.md).

use mog_core::ipc::{self, IpcError, WindowClass};
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
fn at_07_g_06a_only_the_brain_may_call_genie_one_mcp() {
    let spec = ipc::command("mcp_call").expect("mcp_call is in the 4.5 table");
    assert_eq!(spec.windows, [WindowClass::Brain]);
    for label in ["hub", "mog-b7", "card-b7"] {
        let err = ipc::handle(label, "mcp_call", ask_args()).unwrap_err();
        assert!(
            matches!(err, IpcError::WindowNotAllowed { .. }),
            "{label}: {err:?}"
        );
    }
    // The brain passes the gateway; the transport itself is #30.
    let err = ipc::handle("brain", "mcp_call", ask_args()).unwrap_err();
    assert!(matches!(err, IpcError::NotImplemented { .. }), "{err:?}");
}

#[test]
fn at_07_g_03_mcp_call_names_only_the_genie_one_server() {
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
fn at_07_g_03_mcp_call_takes_no_url() {
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

#[test]
#[ignore = "pending TODO(#30): R8 McpTransport is a stub"]
fn at_07_g_04a_g_04d_g_05a_g_05b_g_06a_g_06b_g_07a_thin_client_rules() {
    // When #30 lands, against a scripted Genie One server: initialize names 2025-06-18 (G-04c);
    // every later call carries Mcp-Session-Id (G-04b) and a fresh token from R6 (G-04d) on one
    // reqwest transport with no rmcp (G-04a); a 404 with a session initialises again one time
    // (G-05a); a 404 without one is GENIE_ONE_OFF (G-05b); tools other than the four Genie tools
    // and tools/list are refused (G-06a); genie_ask carries _meta.warehouse_id when the MOG has
    // one (G-06b); no request carries _meta.progressToken (G-07a). The SP-09 tests in
    // spikes/sp-09/tests/transport.rs are the model.
    let err = ipc::handle("brain", "mcp_call", ask_args()).unwrap_err();
    assert!(!matches!(err, IpcError::NotImplemented { .. }), "{err:?}");
    todo!("#30: run the thin client against the scripted server")
}

#[test]
#[ignore = "pending TODO(#30): R8 McpTransport is a stub"]
fn at_07_g_06a_mcp_call_refuses_tools_outside_the_allowlist() {
    for tool in ["execute_sql", "view_ask", "genie_delete_conversation"] {
        let mut a = ask_args();
        a["tool"] = json!(tool);
        let err = ipc::handle("brain", "mcp_call", a).unwrap_err();
        assert!(
            !matches!(err, IpcError::NotImplemented { .. }),
            "{tool}: R8 must refuse the tool, not be missing: {err:?}"
        );
    }
}

#[test]
#[ignore = "pending TODO(#18): R6 TokenBroker is a stub"]
fn at_07_w_06a_w_06b_workspace_id_from_the_org_id_header_once_per_profile() {
    // When #18 lands: R6 reads workspace_id from the x-databricks-org-id header of
    // GET /api/2.0/preview/scim/v2/Me (W-06a) and caches it, so two lookups for one profile make
    // one SCIM call (W-06b). Use a scripted SCIM reply; never a live workspace here.
    todo!("#18: count SCIM calls for two workspace_id lookups")
}

#[test]
#[ignore = "pending TODO(#30): live AT-07 needs R8 and a person with a U2M profile (needs:u2m)"]
fn at_07_live_connect_list_tools_ask_and_poll() {
    let w = mog_core::live_or_skip!();
    // Steps in docs/tests/AT-GENIE-MCP.md, part AT-07: resolve the workspace ID, run tools/list,
    // ask one question, poll it to a terminal status. Never print the token or the answer.
    let _ = w.profile;
    todo!("#30: run the live AT-07 script through R8")
}
