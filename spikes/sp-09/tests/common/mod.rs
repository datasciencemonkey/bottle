//! A fake Genie One MCP server on loopback (wiremock) for SP-09 (#29).
//! It speaks MCP Streamable HTTP: JSON or SSE replies, a session ID, token checks,
//! the busy error in both envelope forms, and a clarify poll.
#![allow(dead_code)]

use serde_json::{Value, json};
use sp09::endpoint::{GENIE_ONE_PATH, GenieOneEndpoint};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub const WORKSPACE_ID: &str = "1234567890";
pub const BUSY_TEXT: &str =
    "asking Genie failed: OneChat stream closed before response.created event. Retry may succeed.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reply {
    Json,
    Sse,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Busy {
    /// Never busy.
    None,
    /// A genie_ask that names a conversation gets the busy text as isError.
    IsError,
    /// A genie_ask that names a conversation gets the busy text as a JSON-RPC error.
    RpcError,
}

pub struct State {
    pub accepted_tokens: HashSet<String>,
    pub session: Option<String>,
    pub sessions_issued: u32,
    pub expire_next_call: bool,
    pub reply: Reply,
    pub busy: Busy,
    pub poll: Value,
    pub server_protocol: String,
    /// Serve tools/list in two pages with nextCursor.
    pub paginate: bool,
    /// Extra structuredContent on the busy isError result.
    pub busy_extra: Option<Value>,
    /// `_meta` on the busy isError result.
    pub busy_meta: Option<Value>,
    /// `data` on the busy JSON-RPC error.
    pub busy_rpc_data: Option<Value>,
    /// Send the poll as JSON text with no structuredContent.
    pub poll_as_text: bool,
    pub server_name: String,
    /// One more tool in tools/list.
    pub extra_tool: Option<Value>,
    /// tools/list answers with this JSON-RPC error code.
    pub tools_list_error_code: Option<Value>,
    /// Replace the tools/list array.
    pub tools_override: Option<Value>,
    /// tools/call answers 429 with this Retry-After value.
    pub rate_limit: Option<String>,
}

#[derive(Clone)]
pub struct FakeGenie(pub Arc<Mutex<State>>);

impl FakeGenie {
    pub fn new(tokens: &[&str]) -> Self {
        FakeGenie(Arc::new(Mutex::new(State {
            accepted_tokens: tokens.iter().map(|t| t.to_string()).collect(),
            session: None,
            sessions_issued: 0,
            expire_next_call: false,
            reply: Reply::Json,
            busy: Busy::None,
            poll: json!({"status": "in_progress", "progress_steps": null}),
            server_protocol: "2025-06-18".into(),
            paginate: false,
            busy_extra: None,
            busy_meta: None,
            busy_rpc_data: None,
            poll_as_text: false,
            server_name: "fake-genie-one".into(),
            extra_tool: None,
            tools_list_error_code: None,
            rate_limit: None,
            tools_override: None,
        })))
    }
    pub fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.0.lock().unwrap())
    }
}

pub fn fixture_tools() -> Value {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/genie_tools.json"
    ))
    .unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    json!({ "tools": v["tools"] })
}

fn rpc_result(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn reply(state: &State, msg: Value) -> ResponseTemplate {
    match state.reply {
        Reply::Json => ResponseTemplate::new(200).set_body_raw(msg.to_string(), "application/json"),
        Reply::Sse => {
            // A progress notification first, then the response, as the spec allows.
            let note = json!({"jsonrpc": "2.0", "method": "notifications/progress",
                "params": {"progressToken": "p", "progress": 1}});
            let body = format!(
                ": keep-alive\r\nevent: message\r\nid: 1\r\ndata: {note}\r\n\r\nevent: message\nid: 2\ndata: {msg}\n\n"
            );
            ResponseTemplate::new(200).set_body_raw(body, "text/event-stream")
        }
    }
}

impl Respond for FakeGenie {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let mut s = self.0.lock().unwrap();
        let auth = req
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let token = auth.strip_prefix("Bearer ").unwrap_or("");
        if !s.accepted_tokens.contains(token) {
            return ResponseTemplate::new(401)
                .set_body_string("{\"error_code\":\"UNAUTHENTICATED\"}");
        }
        let body: Value = match serde_json::from_slice(&req.body) {
            Ok(v) => v,
            Err(_) => return ResponseTemplate::new(400),
        };
        let method = body["method"].as_str().unwrap_or("").to_string();
        let id = body["id"].clone();
        if method == "initialize" {
            s.sessions_issued += 1;
            let sid = format!("sess-{}", s.sessions_issued);
            s.session = Some(sid.clone());
            let msg = rpc_result(
                &id,
                json!({"protocolVersion": s.server_protocol, "capabilities": {"tools": {}},
                       "serverInfo": {"name": s.server_name, "version": "0"}}),
            );
            return reply(&s, msg).insert_header("mcp-session-id", sid.as_str());
        }
        let sent = req
            .headers
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        match (&sent, &s.session) {
            (None, _) => return ResponseTemplate::new(400),
            (Some(a), Some(b)) if a == b => {}
            _ => return ResponseTemplate::new(404),
        }
        if s.expire_next_call {
            s.expire_next_call = false;
            s.session = None;
            return ResponseTemplate::new(404);
        }
        if req
            .headers
            .get("mcp-protocol-version")
            .and_then(|v| v.to_str().ok())
            != Some("2025-06-18")
        {
            return ResponseTemplate::new(400);
        }
        match method.as_str() {
            "notifications/initialized" => ResponseTemplate::new(202),
            "tools/list" if s.paginate => {
                let all = fixture_tools()["tools"].as_array().unwrap().clone();
                let page = if body["params"]["cursor"] == "page-2" {
                    json!({"tools": all[2..]})
                } else {
                    json!({"tools": all[..2], "nextCursor": "page-2"})
                };
                reply(&s, rpc_result(&id, page))
            }
            "tools/list" if s.tools_list_error_code.is_some() => reply(
                &s,
                json!({"jsonrpc": "2.0", "id": id,
                       "error": {"code": s.tools_list_error_code, "message": "no"}}),
            ),
            "tools/list" => {
                let mut t = fixture_tools();
                if let Some(o) = &s.tools_override {
                    t["tools"] = o.clone();
                }
                if let Some(x) = &s.extra_tool {
                    t["tools"].as_array_mut().unwrap().push(x.clone());
                }
                reply(&s, rpc_result(&id, t))
            }
            "tools/call" if s.rate_limit.is_some() => ResponseTemplate::new(429)
                .insert_header("retry-after", s.rate_limit.clone().unwrap().as_str()),
            "tools/call" => {
                let name = body["params"]["name"].as_str().unwrap_or("");
                match name {
                    "genie_ask" => match (
                        s.busy,
                        body["params"]["arguments"].get("conversation_id").is_some(),
                    ) {
                        (Busy::IsError, true) => reply(
                            &s,
                            rpc_result(&id, {
                                let mut r = json!({"content": [{"type": "text", "text": BUSY_TEXT}], "isError": true});
                                if let Some(x) = &s.busy_extra {
                                    r["structuredContent"] = x.clone();
                                }
                                if let Some(x) = &s.busy_meta {
                                    r["_meta"] = x.clone();
                                }
                                r
                            }),
                        ),
                        (Busy::RpcError, true) => reply(&s, {
                            let mut e = json!({"code": -32603, "message": BUSY_TEXT});
                            if let Some(d) = &s.busy_rpc_data {
                                e["data"] = d.clone();
                            }
                            json!({"jsonrpc": "2.0", "id": id, "error": e})
                        }),
                        _ => {
                            let sc = json!({"conversation_id": "conv-1", "response_id": "resp-1",
                                            "status": "in_progress"});
                            reply(
                                &s,
                                rpc_result(
                                    &id,
                                    json!({"content": [{"type": "text", "text": sc.to_string()}],
                                           "structuredContent": sc, "isError": false}),
                                ),
                            )
                        }
                    },
                    "genie_poll_response" => {
                        let sc = s.poll.clone();
                        let result = if s.poll_as_text {
                            json!({"content": [{"type": "text", "text": sc.to_string()}]})
                        } else {
                            json!({"content": [{"type": "text", "text": "poll"}], "structuredContent": sc})
                        };
                        reply(&s, rpc_result(&id, result))
                    }
                    _ => reply(
                        &s,
                        json!({"jsonrpc": "2.0", "id": id,
                               "error": {"code": -32602, "message": "unknown tool"}}),
                    ),
                }
            }
            _ => reply(
                &s,
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "no method"}}),
            ),
        }
    }
}

pub async fn start(fake: &FakeGenie) -> (MockServer, GenieOneEndpoint) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(GENIE_ONE_PATH))
        .and(query_param("o", WORKSPACE_ID))
        .respond_with(fake.clone())
        .mount(&server)
        .await;
    let url = format!("{}{}?o={}", server.uri(), GENIE_ONE_PATH, WORKSPACE_ID);
    let ep = GenieOneEndpoint::parse_loopback_for_tests(&url).unwrap();
    (server, ep)
}

/// JSON-RPC method of each request the server got.
pub async fn methods(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|r| {
            let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            let m = v["method"].as_str().unwrap_or("").to_string();
            if m == "tools/call" {
                format!("tools/call:{}", v["params"]["name"].as_str().unwrap_or(""))
            } else {
                m
            }
        })
        .collect()
}
