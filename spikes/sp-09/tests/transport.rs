//! SP-09 (#29) transport tests against a fake Genie One MCP server on loopback.
mod common;

use common::{BUSY_TEXT, Busy, FakeGenie, Reply, WORKSPACE_ID, methods, start};
use serde_json::json;
use sp09::contract;
use sp09::envelope::{EnvelopeKind, Signal, SignalRules, TextSource, ToolCallOutcome};
use sp09::token::RotatingTokens;
use sp09::transport::{McpClient, McpError};

async fn ready(
    fake: &FakeGenie,
    tokens: RotatingTokens,
) -> (wiremock::MockServer, McpClient<RotatingTokens>) {
    let (server, ep) = start(fake).await;
    let client = McpClient::new(ep, tokens);
    client.initialize().await.expect("initialize");
    (server, client)
}

#[tokio::test]
async fn g_04c_initialize_names_protocol_2025_06_18() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, ep) = start(&fake).await;
    let client = McpClient::new(ep, RotatingTokens::new(&["t1"]));
    let info = client.initialize().await.unwrap();
    assert_eq!(info.protocol_version, "2025-06-18");
    assert_eq!(info.initialized_status, 202);
    assert!(info.session_id_present);
    let reqs = server.received_requests().await.unwrap();
    let init: serde_json::Value = serde_json::from_slice(&reqs[0].body).unwrap();
    assert_eq!(init["method"], "initialize");
    assert_eq!(init["params"]["protocolVersion"], "2025-06-18");
    let accept = reqs[0].headers.get("accept").unwrap().to_str().unwrap();
    assert!(accept.contains("application/json") && accept.contains("text/event-stream"));
    // Every request after initialize names the protocol version in the header.
    for r in &reqs[1..] {
        assert_eq!(r.headers.get("mcp-protocol-version").unwrap(), "2025-06-18");
    }
}

#[tokio::test]
async fn g_04c_rejects_other_server_protocol_version() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.server_protocol = "2025-03-26".into());
    let (_server, ep) = start(&fake).await;
    let client = McpClient::new(ep, RotatingTokens::new(&["t1"]));
    let err = client.initialize().await.unwrap_err();
    assert!(
        matches!(err, McpError::UnsupportedProtocolVersion { date: Some(v), len: 10 } if v == "2025-03-26")
    );
}

#[tokio::test]
async fn g_04b_session_id_kept_on_every_call_after_initialize() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    client.list_tools().await.unwrap();
    client
        .call_tool("genie_ask", contract::ask_args("q", None), None)
        .await
        .unwrap();
    assert_eq!(client.session_id().as_deref(), Some("sess-1"));
    let reqs = server.received_requests().await.unwrap();
    assert!(
        reqs[0].headers.get("mcp-session-id").is_none(),
        "initialize has no session ID"
    );
    for r in &reqs[1..] {
        assert_eq!(r.headers.get("mcp-session-id").unwrap(), "sess-1");
    }
    assert!(client.exchanges()[1..].iter().all(|e| e.session_id_sent));
}

#[tokio::test]
async fn g_02_every_mcp_call_carries_o_param() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    client.list_tools().await.unwrap();
    for r in server.received_requests().await.unwrap() {
        let o = r
            .url
            .query_pairs()
            .find(|(k, _)| k == "o")
            .map(|(_, v)| v.to_string());
        assert_eq!(o.as_deref(), Some(WORKSPACE_ID));
    }
    assert!(client.exchanges().iter().all(|e| e.o_param));
}

#[tokio::test]
async fn sp_09_parses_json_reply() {
    let fake = FakeGenie::new(&["t1"]);
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools["tools"], common::fixture_tools()["tools"]);
    assert!(
        client
            .exchanges()
            .iter()
            .any(|e| e.content_type == "application/json")
    );
}

#[tokio::test]
async fn sp_09_parses_sse_reply_with_notification_before_response() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.reply = Reply::Sse);
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    assert_eq!(client.session_id().as_deref(), Some("sess-1"));
    let tools = client.list_tools().await.unwrap();
    assert!(
        tools["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["name"] == "genie_ask")
    );
    let out = client
        .call_tool("genie_ask", contract::ask_args("q", None), None)
        .await
        .unwrap();
    let ToolCallOutcome::Ok(r) = out else {
        panic!("expected a result")
    };
    assert_eq!(r.structured().unwrap()["conversation_id"], "conv-1");
    assert!(
        client
            .exchanges()
            .iter()
            .any(|e| e.content_type == "text/event-stream")
    );
}

#[tokio::test]
async fn g_04d_fresh_token_per_request_without_new_transport() {
    let fake = FakeGenie::new(&["a", "b", "c", "d", "e"]);
    let tokens = RotatingTokens::fresh_each_call(&["a", "b", "c", "d", "e"]);
    let (server, client) = ready(&fake, tokens).await;
    client.list_tools().await.unwrap();
    client
        .call_tool(
            "genie_poll_response",
            contract::poll_args("conv-1", "resp-1"),
            None,
        )
        .await
        .unwrap();
    // One token call per request, no forced refresh, one session only.
    assert_eq!(client.tokens().calls(), vec![false, false, false, false]);
    let auths: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|r| {
            r.headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(auths, vec!["Bearer a", "Bearer b", "Bearer c", "Bearer d"]);
    assert_eq!(fake.with(|s| s.sessions_issued), 1);
}

#[tokio::test]
async fn sp_09_401_after_rotation_retries_once_with_forced_refresh() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1", "t2"])).await;
    // The server rotates: the old token is no longer valid mid-session.
    fake.with(|s| s.accepted_tokens = ["t2".to_string()].into_iter().collect());
    let out = client
        .call_tool(
            "genie_poll_response",
            contract::poll_args("conv-1", "resp-1"),
            None,
        )
        .await
        .unwrap();
    assert_eq!(out.kind(), EnvelopeKind::Result);
    assert_eq!(client.tokens().calls(), vec![false, false, false, true]);
    assert_eq!(
        fake.with(|s| s.sessions_issued),
        1,
        "no new session or transport"
    );
    assert_eq!(client.session_id().as_deref(), Some("sess-1"));
    let ex = client.exchanges();
    let last2: Vec<(u16, bool)> = ex[ex.len() - 2..]
        .iter()
        .map(|e| (e.http_status, e.forced_refresh))
        .collect();
    assert_eq!(last2, vec![(401, false), (200, true)]);
    assert_eq!(
        methods(&server).await,
        vec![
            "initialize",
            "notifications/initialized",
            "tools/call:genie_poll_response",
            "tools/call:genie_poll_response"
        ]
    );
}

#[tokio::test]
async fn sp_09_second_401_is_auth_expired_without_loop() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1", "t2", "t3"])).await;
    fake.with(|s| s.accepted_tokens.clear());
    let err = client.list_tools().await.unwrap_err();
    assert!(matches!(err, McpError::AuthExpired));
    assert_eq!(
        methods(&server)
            .await
            .iter()
            .filter(|m| *m == "tools/list")
            .count(),
        2
    );
}

#[tokio::test]
async fn g_05a_404_with_session_initialises_again_once() {
    let fake = FakeGenie::new(&["t1"]);
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    fake.with(|s| s.expire_next_call = true);
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools["tools"], common::fixture_tools()["tools"]);
    assert_eq!(fake.with(|s| s.sessions_issued), 2);
    assert_eq!(client.session_id().as_deref(), Some("sess-2"));
}

#[tokio::test]
async fn g_05b_404_without_session_maps_to_genie_one_off() {
    let server = wiremock::MockServer::start().await;
    // Nothing is mounted: every POST gets 404 (Genie One not available).
    let url = format!(
        "{}{}?o={}",
        server.uri(),
        sp09::endpoint::GENIE_ONE_PATH,
        WORKSPACE_ID
    );
    let ep = sp09::endpoint::GenieOneEndpoint::parse_loopback_for_tests(&url).unwrap();
    let client = McpClient::new(ep, RotatingTokens::new(&["t1"]));
    let err = client.initialize().await.unwrap_err();
    assert!(matches!(err, McpError::GenieOneOff { status: 404 }));
}

#[tokio::test]
async fn g_15a_busy_as_is_error_result() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.busy = Busy::IsError);
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let out = client
        .call_tool("genie_ask", contract::ask_args("q2", Some("conv-1")), None)
        .await
        .unwrap();
    assert_eq!(out.kind(), EnvelopeKind::IsError);
    let sig = SignalRules::builtin().classify_call(&out);
    assert_eq!(
        sig,
        Some(Signal::ConversationBusy {
            rule_id: "busy-stream-closed".into(),
            source: TextSource::IsError
        })
    );
}

#[tokio::test]
async fn g_15a_busy_as_json_rpc_error_over_sse() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| {
        s.busy = Busy::RpcError;
        s.reply = Reply::Sse;
    });
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let out = client
        .call_tool("genie_ask", contract::ask_args("q2", Some("conv-1")), None)
        .await
        .unwrap();
    assert_eq!(out.kind(), EnvelopeKind::JsonRpcError);
    let ToolCallOutcome::RpcError(e) = &out else {
        panic!()
    };
    assert_eq!(e.message, BUSY_TEXT);
    assert_eq!(
        SignalRules::builtin().classify_call(&out),
        Some(Signal::ConversationBusy {
            rule_id: "busy-stream-closed".into(),
            source: TextSource::JsonRpcError
        })
    );
}

#[tokio::test]
async fn g_15b_busy_ask_is_not_retried() {
    for busy in [Busy::IsError, Busy::RpcError] {
        let fake = FakeGenie::new(&["t1"]);
        fake.with(|s| s.busy = busy);
        let (server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
        let _ = client
            .call_tool("genie_ask", contract::ask_args("q2", Some("conv-1")), None)
            .await
            .unwrap();
        let asks = methods(&server)
            .await
            .iter()
            .filter(|m| *m == "tools/call:genie_ask")
            .count();
        assert_eq!(asks, 1, "{busy:?}: the client sent the busy ask again");
    }
}

#[tokio::test]
async fn g_22_clarify_signal_from_poll() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| {
        s.poll = json!({"status": "completed", "final_answer": "Which region do you mean, EMEA or AMER?",
                        "query_items": null, "progress_steps": null, "deep_link": null})
    });
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let out = client
        .call_tool(
            "genie_poll_response",
            contract::poll_args("conv-1", "resp-1"),
            None,
        )
        .await
        .unwrap();
    let ToolCallOutcome::Ok(r) = out else {
        panic!()
    };
    let sig = SignalRules::builtin().clarify(&r.structured().unwrap());
    assert!(
        matches!(
            sig,
            Some(Signal::Clarify {
                question_chars: 39,
                ..
            })
        ),
        "{sig:?}"
    );
}

#[tokio::test]
async fn g_06a_client_rejects_other_tool_names() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let err = client
        .call_tool("view_ask", json!({"question": "q"}), None)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::ToolNotAllowed(n) if n == "view_ask"));
    assert!(
        !methods(&server)
            .await
            .iter()
            .any(|m| m.contains("view_ask"))
    );
}

#[tokio::test]
async fn g_06b_g_07a_ask_meta_carries_warehouse_id_and_no_progress_token() {
    let fake = FakeGenie::new(&["t1"]);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    client
        .call_tool(
            "genie_ask",
            contract::ask_args("q", None),
            contract::ask_meta(Some("wh-1")),
        )
        .await
        .unwrap();
    let reqs = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&reqs.last().unwrap().body).unwrap();
    assert_eq!(body["params"]["_meta"]["warehouse_id"], "wh-1");
    assert!(body["params"]["_meta"].get("progressToken").is_none());
}

#[tokio::test]
async fn sp_09_calls_before_initialize_fail() {
    let fake = FakeGenie::new(&["t1"]);
    let (_server, ep) = start(&fake).await;
    let client = McpClient::new(ep, RotatingTokens::new(&["t1"]));
    assert!(matches!(
        client.list_tools().await.unwrap_err(),
        McpError::NotInitialized
    ));
}

#[tokio::test]
async fn sp_09_trace_and_errors_hold_no_token() {
    let fake = FakeGenie::new(&["secret-token-xyz"]);
    let (_server, client) = ready(
        &fake,
        RotatingTokens::new(&["secret-token-xyz", "next-secret-abc"]),
    )
    .await;
    fake.with(|s| s.accepted_tokens.clear());
    let err = client.list_tools().await.unwrap_err();
    let dump = format!("{err:?} {err} {:?}", client.exchanges());
    let json = serde_json::to_string(&client.exchanges()).unwrap();
    for secret in ["secret-token-xyz", "next-secret-abc"] {
        assert!(!dump.contains(secret) && !json.contains(secret));
    }
}

#[tokio::test]
async fn sp_09_tools_list_follows_next_cursor() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.paginate = true);
    let (server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let tools = client.list_tools().await.unwrap();
    assert_eq!(tools["tools"], common::fixture_tools()["tools"]);
    let reqs = server.received_requests().await.unwrap();
    let lists: Vec<serde_json::Value> = reqs
        .iter()
        .map(|r| serde_json::from_slice::<serde_json::Value>(&r.body).unwrap())
        .filter(|b| b["method"] == "tools/list")
        .collect();
    assert_eq!(lists.len(), 2);
    assert!(lists[0]["params"].get("cursor").is_none());
    assert_eq!(lists[1]["params"]["cursor"], "page-2");
}

#[tokio::test]
async fn sp_09_redirects_are_not_followed() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(sp09::endpoint::GENIE_ONE_PATH))
        .respond_with(ResponseTemplate::new(307).insert_header("location", "/evil"))
        .mount(&server)
        .await;
    Mock::given(path("/evil"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let url = format!(
        "{}{}?o={}",
        server.uri(),
        sp09::endpoint::GENIE_ONE_PATH,
        WORKSPACE_ID
    );
    let ep = sp09::endpoint::GenieOneEndpoint::parse_loopback_for_tests(&url).unwrap();
    let client = McpClient::new(ep, RotatingTokens::new(&["t1"]));
    let err = client.initialize().await.unwrap_err();
    assert!(matches!(err, McpError::Http { status: 307, .. }), "{err:?}");
    let reqs = server.received_requests().await.unwrap();
    assert_eq!(reqs.len(), 1);
    assert!(reqs.iter().all(|r| r.url.path() != "/evil"));
}

const LEAK: [&str; 3] = ["alice@corp.com", "9876543210123456", "azuredatabricks"];
const LEAKY: &str = "alice@corp.com adb-9876543210123456.7.azuredatabricks.net";

fn no_leak(text: &str) {
    for l in LEAK {
        assert!(!text.contains(l), "holds {l:?}: {text}");
    }
}

#[tokio::test]
async fn sp_09_unsupported_protocol_version_keeps_no_server_text() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.server_protocol = LEAKY.into());
    let (_server, ep) = start(&fake).await;
    let client = McpClient::new(ep, RotatingTokens::new(&["t1"]));
    let err = client.initialize().await.unwrap_err();
    assert!(
        matches!(err, McpError::UnsupportedProtocolVersion { date: None, len } if len == LEAKY.len())
    );
    no_leak(&format!("{err} {err:?}"));
}

#[tokio::test]
async fn sp_09_server_name_and_content_type_keep_no_server_text() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.server_name = LEAKY.into());
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let _ = client.list_tools().await.unwrap();
    let info = client.initialize().await.unwrap();
    assert_eq!(info.server_name_len, Some(LEAKY.len()));
    no_leak(&serde_json::to_string(&info).unwrap());
    no_leak(&serde_json::to_string(&client.exchanges()).unwrap());
}

#[tokio::test]
async fn sp_09_tools_list_string_error_code_is_not_echoed() {
    let fake = FakeGenie::new(&["t1"]);
    fake.with(|s| s.tools_list_error_code = Some(json!(LEAKY)));
    let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
    let err = client.list_tools().await.unwrap_err();
    no_leak(&format!("{err} {err:?}"));
    fake.with(|s| s.tools_list_error_code = Some(json!(-32000)));
    let err = client.list_tools().await.unwrap_err();
    assert!(err.to_string().contains("-32000"), "{err}");
}

#[tokio::test]
async fn sp_09_retry_after_on_429_is_parsed_not_echoed() {
    use sp09::transport::RetryAfter;
    for (raw, want) in [
        ("30", RetryAfter::Seconds(30)),
        ("alice@corp.com", RetryAfter::Invalid),
    ] {
        let fake = FakeGenie::new(&["t1"]);
        let (_server, client) = ready(&fake, RotatingTokens::new(&["t1"])).await;
        fake.with(|s| s.rate_limit = Some(raw.into()));
        let err = client
            .call_tool("genie_poll_response", contract::poll_args("c", "r"), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, McpError::RateLimited { retry_after: Some(r) } if r == want),
            "{err:?}"
        );
        no_leak(&format!("{err} {err:?}"));
    }
}
