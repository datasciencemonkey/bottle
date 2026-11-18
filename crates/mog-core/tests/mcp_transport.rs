//! R8 McpTransport (#30): the thin JSON-RPC 2.0 client on `reqwest` (no `rmcp`), against a fake
//! Genie One MCP server on loopback and a fake R6 broker. No live workspace.
//!
//! Timing tests never use a tight wall-clock window: a held reply waits on a channel that the
//! test releases after the client gave up, so the result does not depend on machine load.

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fake_mcp::*;
use mog_core::egress_policy::TokenEgressPolicy;
use mog_core::error::CoreResult;
use mog_core::http_transport::host_policy::AllowlistEgressPolicy;
use mog_core::ipc_gateway::{hardened_gateway_with, IpcGateway, NoLog};
use mog_core::mcp_transport::{
    genie_one_off_message, McpCall, McpCallComponent, McpLimits, McpReply, McpSeams, McpTransport,
    ReqwestMcpTransport, ALLOWED_TOOLS, PROTOCOL_VERSION,
};
use mog_core::request_identity::{user_agent, BottleIdentity, TAGS_HEADER};
use mog_core::token_broker::TokenLease;
use serde_json::{json, Value};

struct Rig {
    t: Arc<ReqwestMcpTransport>,
    broker: Arc<FakeBroker>,
    hosts: Arc<Hosts>,
    log: Arc<Lines>,
}

fn rig_full(
    origin: &str,
    url: &str,
    policy: Arc<dyn TokenEgressPolicy + Send + Sync>,
    limits: McpLimits,
) -> Rig {
    let broker = Arc::new(FakeBroker::default());
    let hosts = Hosts::one(origin);
    let log = Arc::new(Lines::default());
    let t = ReqwestMcpTransport::with_seams(McpSeams {
        hosts: hosts.clone(),
        endpoints: Endpoints::one(url),
        tokens: broker.clone(),
        policy,
        decorator: Arc::new(BottleIdentity::new()),
        log: log.clone(),
        limits,
    })
    .unwrap();
    Rig {
        t: Arc::new(t),
        broker,
        hosts,
        log,
    }
}

fn rig(s: &FakeMcp) -> Rig {
    rig_full(
        &s.origin(),
        &s.url(),
        Arc::new(LoopbackPolicy),
        McpLimits::default(),
    )
}

fn ask_args() -> mog_core::ipc::JsonObject {
    obj(json!({"question": QUESTION}))
}

fn poll_args() -> mog_core::ipc::JsonObject {
    obj(json!({"conversation_id": "c-1", "response_id": "r-1"}))
}

fn ask(r: &Rig) -> CoreResult<McpReply> {
    let t = tags(MOG);
    r.t.call_tool(McpCall::new(PROFILE, "genie_ask", ask_args(), &t))
}

fn poll(r: &Rig, mog: &str) -> CoreResult<McpReply> {
    let t = tags(mog);
    r.t.call_tool(McpCall::new(
        PROFILE,
        "genie_poll_response",
        poll_args(),
        &t,
    ))
}

fn code<T: std::fmt::Debug>(r: CoreResult<T>) -> &'static str {
    r.expect_err("the call must fail").code()
}

fn tool_calls(s: &FakeMcp) -> Vec<Seen> {
    s.seen()
        .into_iter()
        .filter(|x| x.rpc_method() == "tools/call")
        .collect()
}

// ---------- G-04a: no rmcp ----------

#[test]
fn g_04a_build_has_no_rmcp_transport() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let lock = std::fs::read_to_string(format!("{root}/Cargo.lock")).unwrap();
    assert!(
        !lock.lines().any(|l| l.trim() == "name = \"rmcp\""),
        "rmcp must not be in the build"
    );
    let manifest =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    assert!(!manifest.contains("rmcp"));
}

// ---------- G-04c, G-04b: initialize and the session ----------

#[test]
fn g_04c_initialize_names_protocol_2025_06_18() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    ask(&r).unwrap();
    let seen = s.seen();
    assert_eq!(PROTOCOL_VERSION, "2025-06-18");
    assert_eq!(seen[0].rpc_method(), "initialize");
    assert_eq!(seen[0].json()["jsonrpc"], "2.0");
    assert_eq!(seen[0].json()["params"]["protocolVersion"], "2025-06-18");
    assert_eq!(seen[0].header("mcp-session-id"), None);
    for x in &seen {
        assert_eq!(x.method, "POST");
        assert!(x.target.starts_with(GENIE_ONE_PATH), "{}", x.target);
        assert!(x.target.ends_with("?o=1234"), "G-02: {}", x.target);
        let accept = x.header("accept").unwrap();
        assert!(accept.contains("application/json") && accept.contains("text/event-stream"));
    }
    // Every request after initialize names the protocol in the header.
    for x in &seen[1..] {
        assert_eq!(x.header("mcp-protocol-version"), Some("2025-06-18"));
    }
}

#[test]
fn g_04c_other_server_protocol_version_is_refused() {
    let s = genie_one(|x| {
        (x.rpc_method() == "initialize").then(|| {
            let mut v = init_result(&x.rpc_id());
            v["result"]["protocolVersion"] = json!("2024-11-05");
            json_reply(200, v).with_header("Mcp-Session-Id", "sess-old")
        })
    });
    let r = rig(&s);
    assert_eq!(code(ask(&r)), "protocol_unsupported");
    assert_eq!(s.count("tools/call"), 0);
}

#[test]
fn g_04b_session_id_on_every_call_after_initialize() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    ask(&r).unwrap();
    poll(&r, MOG).unwrap();
    poll(&r, MOG).unwrap();
    assert_eq!(
        s.methods(),
        [
            "initialize",
            "notifications/initialized",
            "tools/call",
            "tools/call",
            "tools/call"
        ]
    );
    for x in &s.seen()[1..] {
        assert_eq!(
            x.header("mcp-session-id"),
            Some("sess-1"),
            "{}",
            x.rpc_method()
        );
    }
    // The notification has no id; each request has its own id.
    assert!(s.seen()[1].json().get("id").is_none());
    let ids: Vec<Value> = tool_calls(&s).iter().map(Seen::rpc_id).collect();
    assert_ne!(ids[0], ids[1]);
    assert_ne!(ids[1], ids[2]);
}

#[test]
fn g_04b_sessions_are_kept_per_profile_and_mog() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    poll(&r, MOG).unwrap();
    poll(&r, MOG_2).unwrap();
    poll(&r, MOG).unwrap();
    poll(&r, MOG_2).unwrap();
    assert_eq!(s.count("initialize"), 2);
    let calls = tool_calls(&s);
    let sids: Vec<_> = calls
        .iter()
        .map(|x| x.header("mcp-session-id").unwrap().to_owned())
        .collect();
    assert_eq!(sids, ["sess-1", "sess-2", "sess-1", "sess-2"]);
}

#[test]
fn g_04b_concurrent_first_calls_share_one_initialize() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    // One conversation per thread: two open polls on one conversation are E_POLL_INFLIGHT
    // (ARC-F1, #31), and this test is about the shared `initialize` only.
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let t = Arc::clone(&r.t);
            std::thread::spawn(move || {
                let tg = tags(MOG);
                let args = obj(json!({"conversation_id": format!("c-{i}"), "response_id": "r-1"}));
                t.call_tool(McpCall::new(PROFILE, "genie_poll_response", args, &tg))
                    .map(|_| ())
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap().unwrap();
    }
    assert_eq!(s.count("initialize"), 1);
    assert_eq!(s.count("tools/call"), 4);
}

// ---------- G-04d, W-05d: a fresh token per request, one retry on 401 ----------

#[test]
fn g_04d_fresh_token_per_request_without_new_transport() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    for _ in 0..3 {
        poll(&r, MOG).unwrap();
    }
    let seen = s.seen();
    // One lend per HTTP request, and each request carries the token of its own lend.
    assert_eq!(r.broker.lend_count(), seen.len());
    let lent = r.broker.lent();
    for (x, (profile, host, tok)) in seen.iter().zip(&lent) {
        assert_eq!(x.bearer().as_deref(), Some(tok.as_str()));
        assert_eq!(profile, PROFILE);
        assert_eq!(host, "127.0.0.1", "4.6 rule 2: only the profile host");
    }
    // The tokens rotate, yet the session stays: one initialize for all calls.
    assert_eq!(s.count("initialize"), 1);
    let tokens: std::collections::BTreeSet<_> = seen.iter().filter_map(Seen::bearer).collect();
    assert_eq!(tokens.len(), seen.len());
}

#[test]
fn w_05d_sp_09_401_invalidates_once_and_retries_once_on_the_same_session() {
    let calls = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && calls.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| json_reply(401, json!({"error_code": "UNAUTHENTICATED"})))
    });
    let r = rig(&s);
    let reply = poll(&r, MOG).unwrap();
    assert!(!reply.is_error);
    let calls = tool_calls(&s);
    assert_eq!(calls.len(), 2);
    // The 401 was the 3rd lend (initialize, notification, call): only that lease is dropped.
    assert_eq!(r.broker.invalidations(), [TokenLease::new(3)]);
    assert_ne!(calls[0].bearer(), calls[1].bearer());
    assert_eq!(calls[1].header("mcp-session-id"), Some("sess-1"));
    assert_eq!(s.count("initialize"), 1, "no new transport or session");
}

#[test]
fn w_05d_second_401_is_auth_expired_without_a_loop() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| empty_reply(401)));
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "auth_expired");
    assert_eq!(tool_calls(&s).len(), 2, "one retry only");
    assert_eq!(r.broker.invalidations().len(), 1, "one refresh only");
}

// ---------- G-05a, G-05b: 404 ----------

#[test]
fn g_05a_404_with_a_session_initialises_again_once_and_resends() {
    let calls = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && calls.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| empty_reply(404))
    });
    let r = rig(&s);
    poll(&r, MOG).unwrap();
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
    let seen = s.seen();
    assert_eq!(seen[3].header("mcp-session-id"), None);
    assert_eq!(seen[5].header("mcp-session-id"), Some("sess-2"));
    // The next call keeps the new session.
    poll(&r, MOG).unwrap();
    assert_eq!(
        s.seen().last().unwrap().header("mcp-session-id"),
        Some("sess-2")
    );
    assert_eq!(s.count("initialize"), 2);
}

#[test]
fn g_05a_second_404_after_initialise_is_genie_one_off_without_a_loop() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| empty_reply(404)));
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "genie_one_off");
    assert_eq!(s.count("tools/call"), 2);
    assert_eq!(s.count("initialize"), 2);
}

#[test]
fn g_05b_404_without_a_session_is_genie_one_off() {
    // The server gives no Mcp-Session-Id, so a 404 on a call is not a lost session.
    let s = genie_one(|x| match x.rpc_method().as_str() {
        "initialize" => Some(json_reply(200, init_result(&x.rpc_id()))),
        "tools/call" => Some(empty_reply(404)),
        _ => None,
    });
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "genie_one_off");
    assert_eq!(s.count("tools/call"), 1);
    assert_eq!(s.count("initialize"), 1);
    for x in &s.seen() {
        assert_eq!(x.header("mcp-session-id"), None);
    }
}

#[test]
fn g_05b_404_on_initialize_is_genie_one_off() {
    let s = genie_one(|x| (x.rpc_method() == "initialize").then(|| empty_reply(404)));
    let r = rig(&s);
    assert_eq!(code(ask(&r)), "genie_one_off");
    assert_eq!(s.methods(), ["initialize"]);
}

#[test]
fn g_05b_genie_one_off_card_text_names_the_host() {
    assert_eq!(
        genie_one_off_message("dbc-fake.cloud.databricks.com"),
        "Genie One isn't available in dbc-fake.cloud.databricks.com."
    );
}

// ---------- G-06a: the strict tool allowlist ----------

#[test]
fn g_06a_allowlist_is_exactly_the_four_genie_tools() {
    let names: Vec<&str> = ALLOWED_TOOLS.iter().map(|t| t.name).collect();
    assert_eq!(
        names,
        [
            "genie_ask",
            "genie_poll_response",
            "genie_get_query_result",
            "genie_cancel_response"
        ]
    );
}

#[test]
fn g_06a_unlisted_tool_is_refused_before_anything_is_sent() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags(MOG);
    for tool in [
        "execute_sql",
        "view_ask",
        "genie_delete_conversation",
        "GENIE_ASK",
        "genie_ask ",
        "",
        "tools/call",
        "initialize",
    ] {
        let got = r.t.call_tool(McpCall::new(PROFILE, tool, ask_args(), &t));
        assert_eq!(code(got), "tool_not_allowed", "{tool:?}");
    }
    assert!(s.seen().is_empty(), "nothing reaches the server");
    assert_eq!(r.broker.lend_count(), 0, "no token is lent");
    assert!(r.hosts.asked.lock().unwrap().is_empty());
}

#[test]
fn g_06a_tools_list_drops_tools_not_on_the_allowlist() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/list").then(|| {
            let mut v = tools_result(
                &x.rpc_id(),
                &[
                    "view_ask",
                    "genie_ask",
                    "execute_sql",
                    "genie_poll_response",
                    "genie_get_query_result",
                    "genie_cancel_response",
                    "genie_ask_v2",
                ],
            );
            v["result"]["tools"]
                .as_array_mut()
                .unwrap()
                .push(json!({"description": "no name"}));
            json_reply(200, v)
        })
    });
    let r = rig(&s);
    let tools = r.t.list_tools(PROFILE, &tags(MOG)).unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(
        names,
        [
            "genie_ask",
            "genie_poll_response",
            "genie_get_query_result",
            "genie_cancel_response"
        ]
    );
    assert_eq!(tools.len(), 4);
}

#[test]
fn g_06a_tools_list_follows_next_cursor() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/list").then(|| {
            let id = x.rpc_id();
            if x.json()["params"]["cursor"] == "page-2" {
                json_reply(200, tools_result(&id, &["genie_cancel_response"]))
            } else {
                let mut v = tools_result(&id, &["genie_ask", "genie_poll_response"]);
                v["result"]["nextCursor"] = json!("page-2");
                json_reply(200, v)
            }
        })
    });
    let r = rig(&s);
    let tools = r.t.list_tools(PROFILE, &tags(MOG)).unwrap();
    assert_eq!(tools.len(), 3);
    assert_eq!(s.count("tools/list"), 2);
}

#[test]
fn g_06a_contract_matches_genie_tools_json() {
    let fixture: Value =
        serde_json::from_str(include_str!("../../../src/test-support/genie-tools.json")).unwrap();
    let listed = fixture.as_array().unwrap();
    assert_eq!(listed.len(), ALLOWED_TOOLS.len());
    for contract in ALLOWED_TOOLS {
        let tool = listed
            .iter()
            .find(|t| t["name"] == contract.name)
            .unwrap_or_else(|| panic!("{} is not in genie_tools.json", contract.name));
        let schema = &tool["inputSchema"];
        let mut required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        required.sort_unstable();
        let mut want_required = contract.required.to_vec();
        want_required.sort_unstable();
        assert_eq!(required, want_required, "{}", contract.name);
        let props = schema["properties"].as_object().unwrap();
        let mut names: Vec<&str> = props.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut want: Vec<&str> = contract
            .required
            .iter()
            .chain(contract.optional)
            .copied()
            .collect();
        want.sort_unstable();
        assert_eq!(names, want, "{}", contract.name);
        for p in props.values() {
            assert_eq!(p["type"], "string", "{}", contract.name);
        }
    }
}

#[test]
fn g_06a_args_outside_the_contract_are_refused_before_send() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags(MOG);
    for (tool, args) in [
        ("genie_ask", json!({})),
        ("genie_ask", json!({"question": 7})),
        ("genie_ask", json!({"question": "q", "sql": "select 1"})),
        ("genie_poll_response", json!({"conversation_id": "c-1"})),
        (
            "genie_get_query_result",
            json!({"conversation_id": "c", "response_id": "r"}),
        ),
        ("genie_cancel_response", json!({"conversation_id": null})),
    ] {
        let got =
            r.t.call_tool(McpCall::new(PROFILE, tool, obj(args.clone()), &t));
        assert_eq!(code(got), "invalid_tool_args", "{tool} {args}");
    }
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lend_count(), 0);
}

#[test]
fn arc_6_rule_5_profile_keys_are_stripped_from_tool_args() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags(MOG);
    let args = obj(json!({"question": QUESTION, "profile": "other-profile"}));
    r.t.call_tool(McpCall::new(PROFILE, "genie_ask", args, &t))
        .unwrap();
    let call = &tool_calls(&s)[0];
    assert_eq!(
        call.json()["params"]["arguments"],
        json!({"question": QUESTION})
    );
    assert_eq!(call.tool(), "genie_ask");
}

// ---------- G-06b, G-07a: _meta ----------

#[test]
fn g_06b_genie_ask_carries_meta_warehouse_id() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags(MOG);
    let meta = obj(json!({"warehouse_id": "abc123def456"}));
    r.t.call_tool(McpCall::new(PROFILE, "genie_ask", ask_args(), &t).meta(&meta))
        .unwrap();
    // Only genie_ask carries it.
    r.t.call_tool(McpCall::new(PROFILE, "genie_poll_response", poll_args(), &t).meta(&meta))
        .unwrap();
    // No warehouse: no _meta at all.
    r.t.call_tool(McpCall::new(PROFILE, "genie_ask", ask_args(), &t))
        .unwrap();
    let calls = tool_calls(&s);
    assert_eq!(
        calls[0].json()["params"]["_meta"],
        json!({"warehouse_id": "abc123def456"})
    );
    assert!(calls[1].json()["params"].get("_meta").is_none());
    assert!(calls[2].json()["params"].get("_meta").is_none());
}

#[test]
fn g_06b_bad_warehouse_id_is_refused_before_send() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags(MOG);
    for w in [
        json!(""),
        json!(5),
        json!("a b"),
        json!("../x"),
        json!("x".repeat(65)),
    ] {
        let meta = obj(json!({ "warehouse_id": w }));
        let got =
            r.t.call_tool(McpCall::new(PROFILE, "genie_ask", ask_args(), &t).meta(&meta));
        assert_eq!(code(got), "invalid_meta", "{w}");
    }
    assert!(s.seen().is_empty());
}

#[test]
fn g_07a_no_request_carries_progress_token() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags(MOG);
    let meta = obj(json!({"warehouse_id": "w1", "progressToken": 7, "other": "x"}));
    r.t.call_tool(McpCall::new(PROFILE, "genie_ask", ask_args(), &t).meta(&meta))
        .unwrap();
    r.t.call_tool(McpCall::new(PROFILE, "genie_poll_response", poll_args(), &t).meta(&meta))
        .unwrap();
    for x in s.seen() {
        let body = String::from_utf8_lossy(&x.body).to_string();
        assert!(!body.contains("progressToken"), "{body}");
        assert!(!body.contains("\"other\""), "{body}");
    }
}

// ---------- SEC-06a/b: identity headers ----------

#[test]
fn sec_06a_sec_06b_every_mcp_request_carries_user_agent_and_tags() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    ask(&r).unwrap();
    for x in s.seen() {
        assert_eq!(x.header("user-agent"), Some(user_agent().as_str()));
        assert_eq!(
            x.header(TAGS_HEADER),
            Some(format!(r#"{{"app":"bottle","mog":"{MOG}","op":"genie"}}"#).as_str())
        );
    }
}

#[test]
fn sec_06a_bad_tags_send_nothing() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let t = tags("https://x");
    let got =
        r.t.call_tool(McpCall::new(PROFILE, "genie_ask", ask_args(), &t));
    assert_eq!(code(got), "tags_mog_chars");
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lend_count(), 0);
}

// ---------- 4.6 rules 2, 3, 6, 7 ----------

#[test]
fn arc_6_rule_2_token_goes_only_to_the_profile_host() {
    let s = genie_one(|_| None);
    // The endpoint names another host than the profile host: refused before any lend.
    let other = s.url().replace("127.0.0.1", "localhost");
    let r = rig_full(
        &s.origin(),
        &other,
        Arc::new(LoopbackPolicy),
        McpLimits::default(),
    );
    assert_eq!(code(ask(&r)), "host_refused");
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lend_count(), 0);
}

#[test]
fn arc_6_rule_2_production_policy_refuses_a_loopback_endpoint() {
    let s = genie_one(|_| None);
    let r = rig_full(
        &s.origin(),
        &s.url(),
        Arc::new(AllowlistEgressPolicy),
        McpLimits::default(),
    );
    assert_eq!(code(ask(&r)), "host_refused");
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lend_count(), 0);
}

#[test]
fn arc_6_rule_3_redirect_is_not_followed() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/call")
            .then(|| empty_reply(307).with_header("Location", "http://127.0.0.1:1/elsewhere"))
    });
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "redirect_refused");
    assert_eq!(s.count("tools/call"), 1);
}

#[test]
fn arc_6_rule_6_non_json_page_is_not_echoed_or_retried() {
    for status in [200, 502] {
        let s = genie_one(move |x| (x.rpc_method() == "tools/call").then(|| page_reply(status)));
        let r = rig(&s);
        let err = poll(&r, MOG).unwrap_err();
        assert_eq!(err.code(), "non_json_reply");
        assert!(!format!("{err} {err:?}").contains("secret-page-text"));
        assert_eq!(s.count("tools/call"), 1, "a page is never retried");
    }
}

#[test]
fn arc_6_rule_6_page_on_401_is_not_retried_with_a_token() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| page_reply(401)));
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "non_json_reply");
    assert_eq!(s.count("tools/call"), 1, "no resend with a fresh token");
    assert!(r.broker.invalidations().is_empty());
}

#[test]
fn arc_6_rule_6_page_on_404_with_a_session_is_not_retried() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| page_reply(404)));
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "non_json_reply");
    assert_eq!(s.count("tools/call"), 1);
    assert_eq!(s.count("initialize"), 1, "no new session for a page");
}

#[test]
fn arc_6_rule_7_reply_text_is_scrubbed() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/call").then(|| {
            json_reply(
                200,
                json!({"jsonrpc": "2.0", "id": x.rpc_id(), "result": {
                    "content": [{"type": "text", "text": "see password=not-real-value"}],
                    "structuredContent": {"final_answer": "password=not-real-value"}
                }}),
            )
        })
    });
    let r = rig(&s);
    let reply = poll(&r, MOG).unwrap();
    let all = format!("{} {}", reply.text, reply.structured_content);
    assert!(!all.contains("not-real-value"), "{all}");
}

// ---------- replies: JSON, SSE, envelopes ----------

#[test]
fn g_04a_reply_maps_text_structured_content_and_status() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let reply = poll(&r, MOG).unwrap();
    assert_eq!(reply.text, "Revenue was **42**.");
    assert_eq!(reply.structured_content["status"], "completed");
    assert!(!reply.is_error);
    assert_eq!(reply.http_status, 200);
    assert!(reply.rpc_error.is_none());
    assert_eq!(
        reply.to_ipc_json(),
        json!({
            "structuredContent": reply.structured_content,
            "text": "Revenue was **42**.",
            "isError": false,
            "httpStatus": 200
        })
    );
}

#[test]
fn sp_09_sse_reply_with_a_notification_before_the_response() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/call").then(|| {
            sse_reply(&[
                json!({"jsonrpc": "2.0", "method": "notifications/progress", "params": {"progress": 1}}),
                json!({"jsonrpc": "2.0", "id": "other", "result": {"content": []}}),
                poll_result(&x.rpc_id()),
            ])
        })
    });
    let r = rig(&s);
    let reply = poll(&r, MOG).unwrap();
    assert_eq!(reply.text, "Revenue was **42**.");
}

#[test]
fn sp_09_sse_stream_with_no_response_is_a_bad_reply() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/call")
            .then(|| sse_reply(&[json!({"jsonrpc": "2.0", "method": "notifications/progress"})]))
    });
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "bad_reply");
    assert_eq!(s.count("tools/call"), 1);
}

#[test]
fn g_15b_sp_09_busy_envelopes_reach_the_caller_and_are_not_retried() {
    let busy = "asking Genie failed: OneChat stream closed before response.created event.";
    for kind in ["rpc", "is_error"] {
        let s = genie_one(move |x| {
            (x.rpc_method() == "tools/call").then(|| {
                let id = x.rpc_id();
                json_reply(
                    200,
                    if kind == "rpc" {
                        json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": busy}})
                    } else {
                        json!({"jsonrpc": "2.0", "id": id, "result": {
                            "content": [{"type": "text", "text": busy}], "isError": true}})
                    },
                )
            })
        });
        let r = rig(&s);
        let reply = ask(&r).unwrap();
        assert!(reply.is_error, "{kind}");
        assert_eq!(reply.http_status, 200);
        if kind == "rpc" {
            assert_eq!(reply.rpc_error.as_ref().unwrap()["message"], busy);
        } else {
            assert!(reply.rpc_error.is_none());
            assert_eq!(reply.text, busy);
        }
        assert_eq!(s.count("tools/call"), 1, "{kind}: never asked again");
    }
}

#[test]
fn g_14_http_error_with_json_body_reaches_the_caller_with_its_status() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/call").then(|| {
            json_reply(
                503,
                json!({"error_code": "TEMPORARILY_UNAVAILABLE", "message": "busy"}),
            )
        })
    });
    let r = rig(&s);
    let reply = poll(&r, MOG).unwrap();
    assert_eq!(reply.http_status, 503);
    assert!(reply.is_error);
    assert_eq!(
        reply.rpc_error.unwrap()["error_code"],
        "TEMPORARILY_UNAVAILABLE"
    );
    assert_eq!(s.count("tools/call"), 1);
}

#[test]
fn g_10a_429_is_rate_limited_and_not_retried() {
    let s = genie_one(|x| {
        (x.rpc_method() == "tools/call")
            .then(|| json_reply(429, json!({})).with_header("Retry-After", "3"))
    });
    let r = rig(&s);
    assert_eq!(code(poll(&r, MOG)), "rate_limited");
    assert_eq!(s.count("tools/call"), 1);
}

// ---------- size and time caps ----------

fn small_limits() -> McpLimits {
    McpLimits {
        max_reply_bytes: 2048,
        ..McpLimits::default()
    }
}

#[test]
fn nfr_mcp_reply_larger_than_the_cap_is_refused() {
    for sse in [false, true] {
        let s = genie_one(move |x| {
            (x.rpc_method() == "tools/call").then(|| {
                let mut v = poll_result(&x.rpc_id());
                v["result"]["content"][0]["text"] = json!("x".repeat(5000));
                if sse {
                    sse_reply(&[v])
                } else {
                    json_reply(200, v)
                }
            })
        });
        let r = rig_full(
            &s.origin(),
            &s.url(),
            Arc::new(LoopbackPolicy),
            small_limits(),
        );
        assert_eq!(code(poll(&r, MOG)), "reply_too_large", "sse={sse}");
        assert_eq!(s.count("tools/call"), 1);
    }
}

/// A server whose `tools/call` reply waits until the test releases it.
fn held_server() -> (FakeMcp, mpsc::Sender<()>) {
    let (tx, rx) = mpsc::channel::<()>();
    let rx = Mutex::new(rx);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call").then(|| {
            let _ = rx.lock().unwrap().recv();
            json_reply(200, poll_result(&x.rpc_id()))
        })
    });
    (s, tx)
}

#[test]
fn nfr_mcp_call_that_outlives_the_call_timeout_is_timeout() {
    let (s, release) = held_server();
    let limits = McpLimits {
        call_timeout: Duration::from_millis(300),
        ..McpLimits::default()
    };
    let r = rig_full(&s.origin(), &s.url(), Arc::new(LoopbackPolicy), limits);
    // The server holds the reply until after the client gave up, whatever the machine load.
    let got = poll(&r, MOG);
    release.send(()).unwrap();
    assert_eq!(code(got), "timeout");
    assert_eq!(s.count("tools/call"), 1, "a timeout is never resent");
}

#[test]
fn nfr_mcp_per_call_timeout_is_used_and_capped() {
    let (s, release) = held_server();
    // The default and the max are far above the per-call cap, so only the per-call cap can end
    // the call. A watchdog releases the reply after 60 s: if the cap were ignored, the call
    // would then succeed and the assertion below would fail (no hang, no wall-clock window).
    let limits = McpLimits {
        call_timeout: Duration::from_secs(3600),
        max_call_timeout: Duration::from_secs(7200),
        ..McpLimits::default()
    };
    let r = rig_full(&s.origin(), &s.url(), Arc::new(LoopbackPolicy), limits);
    let watchdog = release.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(60));
        let _ = watchdog.send(());
    });
    let t = tags(MOG);
    let got = r.t.call_tool(
        McpCall::new(PROFILE, "genie_poll_response", poll_args(), &t)
            .timeout(Duration::from_millis(300)),
    );
    let _ = release.send(());
    assert_eq!(code(got), "timeout");
    let l = McpLimits::default();
    assert_eq!(l.effective_timeout(None), l.call_timeout);
    assert_eq!(
        l.effective_timeout(Some(Duration::from_secs(5))),
        Duration::from_secs(5)
    );
    assert_eq!(
        l.effective_timeout(Some(Duration::from_secs(100_000))),
        l.max_call_timeout
    );
}

// ---------- ARC-6, SEC-03: no token or content in logs, Debug or errors ----------

#[test]
fn arc_6_sec_03_no_token_question_or_session_in_logs_debug_or_errors() {
    let calls = AtomicUsize::new(0);
    let s = genie_one(
        move |x| match (x.rpc_method().as_str(), x.tool().as_str()) {
            ("tools/call", "genie_poll_response") => match calls.fetch_add(1, Ordering::SeqCst) {
                0 => Some(empty_reply(401)),
                1 => Some(empty_reply(404)),
                _ => Some(empty_reply(401)),
            },
            _ => None,
        },
    );
    let r = rig(&s);
    let mut texts = Vec::new();
    let t = tags(MOG);
    let call = McpCall::new(PROFILE, "genie_ask", ask_args(), &t);
    texts.push(format!("{call:?}"));
    let reply = r.t.call_tool(call).unwrap();
    texts.push(format!("{reply:?}"));
    for _ in 0..2 {
        if let Err(e) = poll(&r, MOG) {
            texts.push(format!("{e} {e:?} {}", e.code()));
        }
    }
    let bad =
        r.t.call_tool(McpCall::new(PROFILE, "secret_tool_name", ask_args(), &t));
    let e = bad.unwrap_err();
    texts.push(format!("{e} {e:?}"));
    texts.push(format!("{:?}", r.t));
    texts.extend(r.log.all());
    assert!(!r.log.all().is_empty(), "R8 writes content-free log lines");
    let all = texts.join("\n");
    for needle in [
        TOKEN_PREFIX,
        "Bearer",
        QUESTION,
        "revenue",
        "Revenue",
        "sess-1",
        "sess-2",
        "127.0.0.1",
        PROFILE,
        MOG,
        "secret_tool_name",
        "c-1",
    ] {
        assert!(!all.contains(needle), "{needle:?} leaked in:\n{all}");
    }
}

#[test]
fn w_04a_profile_comes_only_from_the_caller() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    ask(&r).unwrap();
    assert!(r.hosts.asked.lock().unwrap().iter().all(|p| p == PROFILE));
    assert!(r.broker.lent().iter().all(|(p, _, _)| p == PROFILE));
    // An empty profile is refused; R8 never falls back to a default profile.
    let t = tags(MOG);
    let got = r.t.call_tool(McpCall::new("", "genie_ask", ask_args(), &t));
    assert_eq!(code(got), "profile_invalid");
    assert!(r.broker.lent().iter().all(|(p, _, _)| p == PROFILE));
}

// ---------- mcp_call through the hardened gateway ----------

#[test]
fn at_07_g_06a_mcp_call_reaches_r8_through_the_gateway() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let gw = hardened_gateway_with(NoLog, McpCallComponent::new(Arc::clone(&r.t)));
    let args = json!({
        "profile": PROFILE, "server": "genie_one", "tool": "genie_poll_response",
        "args": {"conversation_id": "c-1", "response_id": "r-1"},
        "opId": "op-1", "tags": {"app": "bottle", "mog": MOG, "op": "genie"}
    });
    let out = gw.dispatch("brain", "mcp_call", args.clone()).unwrap();
    assert_eq!(out["isError"], false);
    assert_eq!(out["httpStatus"], 200);
    assert_eq!(out["text"], "Revenue was **42**.");
    let mut bad = args.clone();
    bad["tool"] = json!("execute_sql");
    assert_eq!(
        gw.dispatch("brain", "mcp_call", bad).unwrap_err().code(),
        "tool_not_allowed"
    );
    let mut list = args;
    list["tool"] = json!("tools/list");
    list["args"] = json!({});
    let out = gw.dispatch("brain", "mcp_call", list).unwrap();
    assert_eq!(
        out["structuredContent"]["tools"].as_array().unwrap().len(),
        4
    );
    assert_eq!(s.count("tools/call"), 1);
}

// ---------- ARC-F1, G-07b: the in-flight poll guard (#31) ----------

/// Wait (bounded) until the server has seen `n` `tools/call` requests.
fn wait_for_tool_calls(s: &FakeMcp, n: usize) {
    for _ in 0..3000 {
        if s.count("tools/call") >= n {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("the server never saw {n} tools/call");
}

#[test]
fn arc_f1_g_07b_second_poll_on_a_conversation_gets_e_poll_inflight_and_sends_nothing() {
    let (s, release) = held_server();
    let r = rig(&s);
    let t = Arc::clone(&r.t);
    // The PollActor's poll, held open by the server.
    let first = std::thread::spawn(move || {
        let tg = tags(MOG);
        t.call_tool(McpCall::new(
            PROFILE,
            "genie_poll_response",
            poll_args(),
            &tg,
        ))
    });
    wait_for_tool_calls(&s, 1);
    let before = s.seen().len();
    // The main agent polls the same conversation from its own session (ARC-F1: "also for the
    // calls of the main agent"), through `mcp_call` on the gateway.
    let gw = hardened_gateway_with(NoLog, McpCallComponent::new(Arc::clone(&r.t)));
    let args = json!({
        "profile": PROFILE, "server": "genie_one", "tool": "genie_poll_response",
        "args": {"conversation_id": "c-1", "response_id": "r-1"},
        "opId": "op-2", "tags": {"app": "bottle", "mog": MOG_2, "op": "genie"}
    });
    let second = gw.dispatch("brain", "mcp_call", args);
    assert_eq!(code(second), "poll_inflight");
    assert_eq!(code(poll(&r, MOG)), "poll_inflight");
    assert_eq!(s.seen().len(), before, "an overlapping poll sends nothing");
    release.send(()).unwrap();
    assert!(!first.join().unwrap().unwrap().is_error);
    // The slot is free again: the next poll goes out with the same IDs.
    release.send(()).unwrap();
    assert!(!poll(&r, MOG).unwrap().is_error);
    let calls = tool_calls(&s);
    assert_eq!(calls.len(), 2);
    for c in &calls {
        assert_eq!(c.json()["params"]["arguments"], json!(poll_args()));
    }
}

#[test]
fn arc_f1_poll_slot_is_freed_after_a_failed_poll() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| empty_reply(500)));
    let r = rig(&s);
    let first = poll(&r, MOG);
    assert!(first.as_ref().map_or(true, |x| x.is_error), "{first:?}");
    // A failed poll never leaves the conversation locked.
    let second = poll(&r, MOG);
    assert_ne!(second.err().map(|e| e.code()), Some("poll_inflight"));
    assert_eq!(s.count("tools/call"), 2);
}
