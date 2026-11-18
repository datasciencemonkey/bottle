//! #479: the R8 McpTransport failure codes (#30, ADR 0069) cross IPC as themselves, so the
//! brain GL-5 mapper (#32, ADR 0072) can show its own text for each one (G-05b, G-14, W-14).
//! A rejection stays `{ "error": "<code>" }`: a fixed word, never a token, a URL or reply text
//! (SEC-03). R8 internal codes that GL-5 does not name still cross as `host_failed`.
//!
//! The Tauri host routes `mcp_call` to `McpCallComponent` in #467; these tests use the
//! mog-core gateway and component, which that routing calls.

use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::{
    AUTH_EXPIRED, CLIENT_INIT_FAILED, HOST_REFUSED, NON_JSON_REPLY, OFFLINE, PROFILE_HOST_INVALID,
    RATE_LIMITED, REDIRECT_REFUSED, REPLY_TOO_LARGE, TIMEOUT, TOKEN_UNUSABLE, TRANSPORT_FAILED,
    UPSTREAM_FAILED, UPSTREAM_REJECTED,
};
use mog_core::ipc_gateway::{hardened_gateway_with, IpcGateway, NoLog, ERROR_CODES};
use mog_core::mcp_transport::{
    McpCall, McpCallComponent, McpReply, McpTransport, BAD_REPLY, GENIE_ONE_OFF, INVALID_META,
    INVALID_TOOL_ARGS, PROFILE_INVALID, PROTOCOL_UNSUPPORTED, TOOL_NOT_ALLOWED,
};
use mog_core::types::Tags;
use serde_json::{json, Value};

/// The R8 failure codes that the GL-5 mapper names (#479 step 1).
const R8_IPC_CODES: [&str; 15] = [
    GENIE_ONE_OFF,
    AUTH_EXPIRED,
    RATE_LIMITED,
    NON_JSON_REPLY,
    TIMEOUT,
    OFFLINE,
    HOST_REFUSED,
    REDIRECT_REFUSED,
    REPLY_TOO_LARGE,
    BAD_REPLY,
    PROTOCOL_UNSUPPORTED,
    TOOL_NOT_ALLOWED,
    INVALID_TOOL_ARGS,
    INVALID_META,
    PROFILE_INVALID,
];

/// The R8 code of an overlapping poll (ARC-F1, G-07b). The constant `E_POLL_INFLIGHT` is #31.
const POLL_INFLIGHT: &str = "poll_inflight";

/// An R8 double that fails every call with one code, as `ReqwestMcpTransport` does.
struct FailingR8(&'static str);

impl FailingR8 {
    fn err(&self) -> CoreError {
        CoreError::Unavailable {
            component: "R8",
            reason: self.0,
        }
    }
}

impl McpTransport for FailingR8 {
    fn call_tool(&self, _call: McpCall<'_>) -> CoreResult<McpReply> {
        Err(self.err())
    }
    fn list_tools(&self, _profile: &str, _tags: &Tags) -> CoreResult<Vec<Value>> {
        Err(self.err())
    }
    fn start_loopback_proxy(&self) -> CoreResult<u16> {
        Err(self.err())
    }
}

fn mcp_call_args(tool: &str) -> Value {
    let args = if tool == "tools/list" {
        json!({})
    } else {
        json!({"conversation_id": "c-1", "response_id": "r-1"})
    };
    json!({
        "profile": "test-profile", "server": "genie_one", "tool": tool, "args": args,
        "opId": "op-1",
        "tags": {"app": "bottle", "mog": "00000000-0000-4000-8000-0000000000a1", "op": "genie"}
    })
}

/// The code that `mcp_call` crosses with when R8 fails with `code`: the code itself when it
/// is in `ERROR_CODES`, else `host_failed` (the host `rejection`, ADR 0040).
fn crossed(code: &'static str, tool: &str) -> &'static str {
    let gw = hardened_gateway_with(NoLog, McpCallComponent::new(FailingR8(code)));
    let e = gw
        .dispatch("brain", "mcp_call", mcp_call_args(tool))
        .unwrap_err();
    let got = e.code();
    if ERROR_CODES.contains(&got) {
        got
    } else {
        "host_failed"
    }
}

#[test]
fn g_05b_g_14_w_14_every_r8_code_is_an_ipc_error_code() {
    for code in R8_IPC_CODES {
        assert!(ERROR_CODES.contains(&code), "{code} is not in ERROR_CODES");
    }
}

#[test]
fn g_05b_genie_one_off_crosses_mcp_call_as_itself() {
    assert_eq!(crossed(GENIE_ONE_OFF, "genie_ask"), "genie_one_off");
}

#[test]
fn g_14_w_14_each_r8_code_crosses_mcp_call_as_itself() {
    for code in R8_IPC_CODES {
        assert_eq!(crossed(code, "genie_poll_response"), code);
        // `tools/list` goes to the same R8 and crosses the same way.
        assert_eq!(crossed(code, "tools/list"), code);
    }
}

#[test]
fn sec_05a_non_json_reply_crosses_as_its_code() {
    // The card shows the PROXY_PAGE text, never the page (SEC-05a, GL-5).
    assert_eq!(crossed(NON_JSON_REPLY, "genie_ask"), "non_json_reply");
    assert_eq!(crossed(REDIRECT_REFUSED, "genie_ask"), "redirect_refused");
}

#[test]
fn g_07b_arc_f1_g_16c_poll_inflight_is_an_ipc_error_code() {
    assert!(ERROR_CODES.contains(&POLL_INFLIGHT));
    assert_eq!(
        crossed(POLL_INFLIGHT, "genie_poll_response"),
        "poll_inflight"
    );
}

#[test]
fn sec_03_r8_internal_codes_still_cross_as_host_failed() {
    // GL-5 names none of these; they stay inside the core.
    for code in [
        CLIENT_INIT_FAILED,
        TOKEN_UNUSABLE,
        TRANSPORT_FAILED,
        UPSTREAM_FAILED,
        UPSTREAM_REJECTED,
        PROFILE_HOST_INVALID,
    ] {
        assert!(!ERROR_CODES.contains(&code), "{code}");
        assert_eq!(crossed(code, "genie_ask"), "host_failed");
    }
}

#[test]
fn sec_03_every_r8_ipc_code_is_a_fixed_word() {
    for code in R8_IPC_CODES.iter().chain([&POLL_INFLIGHT]) {
        assert!(
            !code.is_empty() && code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
            "{code}"
        );
    }
}
