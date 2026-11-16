//! `McpClient`: MCP Streamable HTTP (protocol 2025-06-18) over reqwest + rustls.
//!
//! - Each JSON-RPC message is one POST with `Accept: application/json, text/event-stream`.
//! - Each POST asks the `TokenSource` for a token (G-04d), so a token that rotates
//!   in the middle of a session needs no new transport (the point of SP-09, COL-17).
//! - On 401 the client forces one refresh and sends again one time.
//! - On 404 with a session ID it initialises again one time (G-05a); on 404
//!   without a session it reports `GENIE_ONE_OFF` (G-05b).
//! - It never sends a `tools/call` again for any other error (G-15b).
use crate::PROTOCOL_VERSION;
use crate::endpoint::GenieOneEndpoint;
use crate::envelope::{RpcError, Signal, SignalRules, ToolCallOutcome};
use crate::sse::parse_events;
use crate::token::{TokenError, TokenSource};
use serde::Serialize;
use serde_json::{Value, json};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("token: {0}")]
    Token(#[from] TokenError),
    #[error("network: {0}")]
    Network(String),
    #[error("AUTH_EXPIRED: 401 after one forced token refresh")]
    AuthExpired,
    #[error("GENIE_ONE_OFF: HTTP {status} without a session")]
    GenieOneOff { status: u16 },
    #[error("RATE_LIMITED: HTTP 429 (Retry-After {retry_after:?})")]
    RateLimited { retry_after: Option<String> },
    #[error("HTTP {status}")]
    Http {
        status: u16,
        body_signal: Option<Signal>,
        rpc_error: Option<RpcError>,
    },
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("the SSE stream ended before the JSON-RPC response")]
    StreamEndedWithoutResponse,
    #[error("the server named protocol {0}, not 2025-06-18")]
    UnsupportedProtocolVersion(String),
    #[error("call initialize first")]
    NotInitialized,
    #[error("G-06a: tool {0} is not on the allow list")]
    ToolNotAllowed(String),
}

/// One HTTP exchange, with no token, no body and no session ID value.
#[derive(Debug, Clone, Serialize)]
pub struct Exchange {
    pub method: String,
    pub attempt: u8,
    pub forced_refresh: bool,
    pub http_status: u16,
    pub content_type: String,
    pub session_id_sent: bool,
    pub protocol_header_sent: bool,
    pub o_param: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitializeInfo {
    pub protocol_version: String,
    pub session_id_present: bool,
    pub server_name: Option<String>,
    pub content_type: String,
    pub initialized_status: u16,
}

struct Session {
    id: Option<String>,
    initialized: bool,
}

pub struct McpClient<T> {
    http: reqwest::Client,
    endpoint: GenieOneEndpoint,
    tokens: T,
    session: Mutex<Session>,
    next_id: AtomicU64,
    trace: Mutex<Vec<Exchange>>,
    rules: SignalRules,
}

/// What came back for one POST.
struct Reply {
    status: u16,
    message: Option<Value>,
    session_header: Option<String>,
    content_type: String,
}

enum Sent {
    Done(Reply),
    SessionGone,
}

impl<T: TokenSource> McpClient<T> {
    pub fn new(endpoint: GenieOneEndpoint, tokens: T) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            // G-19b / #207 spirit: do not follow redirects off the endpoint.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client");
        McpClient {
            http,
            endpoint,
            tokens,
            session: Mutex::new(Session {
                id: None,
                initialized: false,
            }),
            next_id: AtomicU64::new(1),
            trace: Mutex::new(Vec::new()),
            rules: SignalRules::builtin(),
        }
    }

    pub fn tokens(&self) -> &T {
        &self.tokens
    }

    pub fn session_id(&self) -> Option<String> {
        self.session.lock().expect("lock").id.clone()
    }

    pub fn exchanges(&self) -> Vec<Exchange> {
        self.trace.lock().expect("lock").clone()
    }

    /// `initialize` then `notifications/initialized`.
    pub async fn initialize(&self) -> Result<InitializeInfo, McpError> {
        {
            let mut s = self.session.lock().expect("lock");
            s.id = None;
            s.initialized = false;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let msg = json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "bottle-sp09", "version": env!("CARGO_PKG_VERSION")}
            }
        });
        let reply = match self.send_with_auth(&msg, Some(id), false).await? {
            Sent::Done(r) => r,
            Sent::SessionGone => unreachable!("initialize sends no session ID"),
        };
        let result = reply
            .message
            .as_ref()
            .and_then(|m| m.get("result"))
            .ok_or_else(|| McpError::BadResponse("initialize has no result".into()))?;
        let version = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if version != PROTOCOL_VERSION {
            return Err(McpError::UnsupportedProtocolVersion(version));
        }
        let sid = reply
            .session_header
            .filter(|s| !s.is_empty() && s.bytes().all(|b| (0x21..=0x7e).contains(&b)));
        let server_name = result
            .pointer("/serverInfo/name")
            .and_then(Value::as_str)
            .map(str::to_string);
        {
            let mut s = self.session.lock().expect("lock");
            s.id = sid.clone();
            s.initialized = true;
        }
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let initialized_status = match self.send_with_auth(&note, None, false).await? {
            Sent::Done(r) => r.status,
            Sent::SessionGone => {
                return Err(McpError::BadResponse(
                    "session lost at notifications/initialized".into(),
                ));
            }
        };
        Ok(InitializeInfo {
            protocol_version: version,
            session_id_present: sid.is_some(),
            server_name,
            content_type: reply.content_type,
            initialized_status,
        })
    }

    /// `tools/list`, following `nextCursor` (up to 20 pages). Returns the merged result.
    pub async fn list_tools(&self) -> Result<Value, McpError> {
        let mut tools: Vec<Value> = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..20 {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let msg = self.request("tools/list", params)?;
            let m = self.request_with_reinit(msg).await?;
            if let Some(e) = m.get("error") {
                return Err(McpError::BadResponse(format!(
                    "tools/list error code {}",
                    e["code"]
                )));
            }
            let result = m
                .get("result")
                .ok_or_else(|| McpError::BadResponse("tools/list has no result".into()))?;
            tools.extend(
                result
                    .get("tools")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
            );
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                return Ok(json!({ "tools": tools }));
            }
        }
        Err(McpError::BadResponse(
            "tools/list has more than 20 pages".into(),
        ))
    }

    /// `tools/call` for one tool on the G-06a allow list.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
        meta: Option<Value>,
    ) -> Result<ToolCallOutcome, McpError> {
        if !crate::contract::REQUIRED_TOOLS.contains(&name) {
            return Err(McpError::ToolNotAllowed(name.to_string()));
        }
        let mut params = json!({"name": name, "arguments": arguments});
        if let Some(m) = meta {
            params["_meta"] = m;
        }
        let msg = self.request("tools/call", params)?;
        let m = self.request_with_reinit(msg).await?;
        ToolCallOutcome::from_message(&m).map_err(McpError::BadResponse)
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, McpError> {
        if !self.session.lock().expect("lock").initialized {
            return Err(McpError::NotInitialized);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        Ok(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
    }

    /// Send a request; on 404 with a session, initialise again one time and resend (G-05a).
    async fn request_with_reinit(&self, mut msg: Value) -> Result<Value, McpError> {
        let mut id = msg["id"].as_u64();
        for round in 0..2 {
            match self.send_with_auth(&msg, id, true).await? {
                Sent::Done(r) => {
                    return r
                        .message
                        .ok_or_else(|| McpError::BadResponse("no JSON-RPC message".into()));
                }
                Sent::SessionGone if round == 0 => {
                    self.initialize().await?;
                    let new_id = self.next_id.fetch_add(1, Ordering::Relaxed);
                    msg["id"] = json!(new_id);
                    id = Some(new_id);
                }
                Sent::SessionGone => break,
            }
        }
        Err(McpError::GenieOneOff { status: 404 })
    }

    /// One logical send: a fresh token, and one forced refresh after a 401.
    async fn send_with_auth(
        &self,
        msg: &Value,
        id: Option<u64>,
        after_init: bool,
    ) -> Result<Sent, McpError> {
        let method = msg["method"].as_str().unwrap_or("").to_string();
        for attempt in 0..2u8 {
            let forced = attempt == 1;
            let token = self.tokens.token(forced).await?;
            let session_id = self.session_id();
            let mut req = self
                .http
                .post(self.endpoint.url().clone())
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .bearer_auth(token.expose_secret())
                .body(msg.to_string());
            drop(token);
            if after_init || method != "initialize" {
                req = req.header("MCP-Protocol-Version", PROTOCOL_VERSION);
            }
            if let Some(sid) = &session_id {
                req = req.header("Mcp-Session-Id", sid);
            }
            // reqwest errors can name the URL but never the headers; keep only the kind.
            let resp = req
                .send()
                .await
                .map_err(|e| McpError::Network(net_kind(&e)))?;
            let status = resp.status().as_u16();
            let content_type = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(|v| {
                    v.split(';')
                        .next()
                        .unwrap_or("")
                        .trim()
                        .to_ascii_lowercase()
                })
                .unwrap_or_default();
            let session_header = resp
                .headers()
                .get("mcp-session-id")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let retry_after = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            self.trace.lock().expect("lock").push(Exchange {
                method: method.clone(),
                attempt: attempt + 1,
                forced_refresh: forced,
                http_status: status,
                content_type: content_type.clone(),
                session_id_sent: session_id.is_some(),
                protocol_header_sent: after_init || method != "initialize",
                o_param: self.endpoint.url().query_pairs().any(|(k, _)| k == "o"),
            });
            match status {
                401 if attempt == 0 => continue,
                401 => return Err(McpError::AuthExpired),
                404 if session_id.is_some() && method != "initialize" => {
                    return Ok(Sent::SessionGone);
                }
                404 => return Err(McpError::GenieOneOff { status }),
                429 => return Err(McpError::RateLimited { retry_after }),
                200..=299 => {}
                _ => {
                    let body = resp.text().await.unwrap_or_default();
                    let rpc_error = serde_json::from_str::<Value>(&body)
                        .ok()
                        .and_then(|v| v.get("error").cloned())
                        .and_then(|e| serde_json::from_value::<RpcError>(e).ok());
                    return Err(McpError::Http {
                        status,
                        body_signal: self.rules.classify_http_body(&body),
                        rpc_error,
                    });
                }
            }
            let message = match id {
                None => None, // a notification: 202 with no body
                Some(id) => {
                    let body = resp
                        .text()
                        .await
                        .map_err(|e| McpError::Network(net_kind(&e)))?;
                    Some(read_message(&content_type, &body, id)?)
                }
            };
            return Ok(Sent::Done(Reply {
                status,
                message,
                session_header,
                content_type,
            }));
        }
        Err(McpError::AuthExpired)
    }
}

fn net_kind(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timeout".into()
    } else if e.is_connect() {
        "connect".into()
    } else if e.is_body() || e.is_decode() {
        "body".into()
    } else {
        "request".into()
    }
}

/// Find the JSON-RPC response with `id` in a JSON or SSE body.
fn read_message(content_type: &str, body: &str, id: u64) -> Result<Value, McpError> {
    let is_response = |v: &Value| {
        v.get("id").and_then(Value::as_u64) == Some(id)
            && (v.get("result").is_some() || v.get("error").is_some())
    };
    match content_type {
        "text/event-stream" => parse_events(body)
            .iter()
            .filter_map(|e| serde_json::from_str::<Value>(&e.data).ok())
            .find(is_response)
            .ok_or(McpError::StreamEndedWithoutResponse),
        _ => {
            let v: Value = serde_json::from_str(body).map_err(|_| {
                McpError::BadResponse(format!("body is not JSON (content type {content_type:?})"))
            })?;
            if is_response(&v) {
                Ok(v)
            } else {
                Err(McpError::BadResponse(
                    "the JSON body is not the response to this request".into(),
                ))
            }
        }
    }
}
