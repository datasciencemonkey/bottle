//! Test doubles for R8 McpTransport (#30): a fake Genie One MCP server on loopback (std only),
//! a fake R6 broker, a loopback-only egress policy and fixed profile hosts.
//!
//! No live workspace. Every token is a plainly fake value (`test-token-not-real-<n>`).
//! Loopback is allowed ONLY by [`LoopbackPolicy`]; the production policy refuses it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use mog_core::egress_policy::TokenEgressPolicy;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::host_policy::AllowlistEgressPolicy;
use mog_core::http_transport::ProfileHosts;
use mog_core::mcp_transport::{McpEndpoints, McpLog};
use mog_core::token_broker::{TokenBroker, TokenLease};
use mog_core::types::Tags;
use serde_json::{json, Value};

pub const PROFILE: &str = "dev";
pub const MOG: &str = "00000000-0000-4000-8000-0000000000a1";
pub const MOG_2: &str = "00000000-0000-4000-8000-0000000000a2";
pub const TOKEN_PREFIX: &str = "test-token-not-real-";
pub const GENIE_ONE_PATH: &str = "/ai-gateway/mcp-services/system.ai.genie_one_mcp";
pub const QUESTION: &str = "What was revenue last quarter in the fake region?";

pub fn token(n: usize) -> String {
    format!("{TOKEN_PREFIX}{n}")
}

// ---------- the fake server ----------

#[derive(Clone, Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

pub fn json_reply(status: u16, v: Value) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: serde_json::to_vec(&v).unwrap(),
    }
}

/// One SSE stream that holds `messages` in order, with CRLF line ends and a comment line.
pub fn sse_reply(messages: &[Value]) -> Reply {
    let mut body = String::from(": fake stream\r\n");
    for m in messages {
        body.push_str("event: message\r\n");
        body.push_str(&format!("data: {}\r\n\r\n", m));
    }
    Reply {
        status: 200,
        headers: vec![("Content-Type".into(), "text/event-stream".into())],
        body: body.into_bytes(),
    }
}

pub fn empty_reply(status: u16) -> Reply {
    Reply {
        status,
        headers: vec![],
        body: vec![],
    }
}

pub fn page_reply(status: u16) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
        body: b"<html><body>Sign in to the proxy at secret-page-text</body></html>".to_vec(),
    }
}

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
    /// The JSON-RPC method of the request body.
    pub fn rpc_method(&self) -> String {
        self.json()["method"].as_str().unwrap_or("").to_owned()
    }
    pub fn rpc_id(&self) -> Value {
        self.json()["id"].clone()
    }
    pub fn tool(&self) -> String {
        self.json()["params"]["name"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }
    pub fn bearer(&self) -> Option<String> {
        self.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(str::to_owned)
    }
}

pub type Handler = Box<dyn FnMut(&Seen) -> Reply + Send>;

pub struct FakeMcp {
    pub addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl FakeMcp {
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }
    /// The Genie One URL on this server, with `?o=`.
    pub fn url(&self) -> String {
        format!("{}{GENIE_ONE_PATH}?o=1234", self.origin())
    }
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
    pub fn methods(&self) -> Vec<String> {
        self.seen().iter().map(Seen::rpc_method).collect()
    }
    pub fn count(&self, method: &str) -> usize {
        self.methods().iter().filter(|m| *m == method).count()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .ok()?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter(|l| !l.is_empty())
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_owned(), v.trim().to_owned()))
        .collect();
    let len = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse::<usize>().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some(Seen {
        method,
        target,
        headers,
        body,
    })
}

/// Serve every connection with `handler` (one request per connection), and record each request
/// before the handler runs.
pub fn serve(mut handler: Handler) -> FakeMcp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&seen);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                return;
            };
            let Some(req) = read_request(&mut stream) else {
                continue;
            };
            rec.lock().unwrap().push(req.clone());
            let reply = handler(&req);
            let mut out = format!(
                "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
                reply.status,
                reply.body.len()
            );
            for (n, v) in &reply.headers {
                out.push_str(&format!("{n}: {v}\r\n"));
            }
            out.push_str("\r\n");
            let _ = stream.write_all(out.as_bytes());
            let _ = stream.write_all(&reply.body);
            let _ = stream.flush();
        }
    });
    FakeMcp { addr, seen }
}

/// The `initialize` result for request `id`.
pub fn init_result(id: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": {
        "protocolVersion": "2025-06-18",
        "capabilities": {"tools": {}},
        "serverInfo": {"name": "fake-genie-one", "version": "0"}
    }})
}

/// A completed poll result for request `id`.
pub fn poll_result(id: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": {
        "content": [{"type": "text", "text": "Revenue was **42**."}],
        "structuredContent": {
            "status": "completed", "conversation_id": "c-1", "response_id": "r-1",
            "final_answer": "Revenue was 42."
        },
        "isError": false
    }})
}

pub fn tools_result(id: &Value, names: &[&str]) -> Value {
    let tools: Vec<Value> = names
        .iter()
        .map(|n| json!({"name": n, "description": "d", "inputSchema": {"type": "object"}}))
        .collect();
    json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}})
}

/// The usual Genie One answers: `initialize` gives session `sess-<n>` (a new one each time),
/// the notification gets 202, `tools/list` lists the four tools, `tools/call` a completed poll.
pub fn standard(n_init: &AtomicUsize, seen: &Seen) -> Reply {
    let id = seen.rpc_id();
    match seen.rpc_method().as_str() {
        "initialize" => {
            let n = n_init.fetch_add(1, Ordering::SeqCst) + 1;
            json_reply(200, init_result(&id)).with_header("Mcp-Session-Id", &format!("sess-{n}"))
        }
        "notifications/initialized" => empty_reply(202),
        "tools/list" => json_reply(
            200,
            tools_result(
                &id,
                &[
                    "genie_ask",
                    "genie_poll_response",
                    "genie_get_query_result",
                    "genie_cancel_response",
                ],
            ),
        ),
        "tools/call" => json_reply(200, poll_result(&id)),
        _ => json_reply(
            200,
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "no"}}),
        ),
    }
}

/// A server that answers with [`standard`] unless `custom` returns a reply first.
pub fn genie_one(mut custom: impl FnMut(&Seen) -> Option<Reply> + Send + 'static) -> FakeMcp {
    let n = AtomicUsize::new(0);
    serve(Box::new(move |s| {
        custom(s).unwrap_or_else(|| standard(&n, s))
    }))
}

// ---------- doubles ----------

/// Fixed profile hosts: `PROFILE` is the loopback server origin.
pub struct Hosts {
    pub map: BTreeMap<String, String>,
    pub asked: Mutex<Vec<String>>,
}

impl Hosts {
    pub fn one(url: &str) -> Arc<Self> {
        Arc::new(Self {
            map: BTreeMap::from([(PROFILE.to_owned(), url.to_owned())]),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl ProfileHosts for Hosts {
    fn workspace_url(&self, profile: &str) -> CoreResult<String> {
        self.asked.lock().unwrap().push(profile.to_owned());
        self.map
            .get(profile)
            .cloned()
            .ok_or(CoreError::Unavailable {
                component: "test",
                reason: "no_profile",
            })
    }
}

/// Fixed Genie One URLs per profile.
pub struct Endpoints(pub BTreeMap<String, String>);

impl Endpoints {
    pub fn one(url: &str) -> Arc<Self> {
        Arc::new(Self(BTreeMap::from([(PROFILE.to_owned(), url.to_owned())])))
    }
}

impl McpEndpoints for Endpoints {
    fn genie_one_url(&self, profile: &str) -> CoreResult<String> {
        self.0.get(profile).cloned().ok_or(CoreError::Unavailable {
            component: "test",
            reason: "no_profile",
        })
    }
}

/// Lends a new fake token on every call (`test-token-not-real-<n>`, lease `n`), and records the
/// profile and host of each lend and each invalidated lease.
#[derive(Default)]
pub struct FakeBroker {
    pub lends: AtomicUsize,
    pub lent: Mutex<Vec<(String, String, String)>>,
    pub invalidated: Mutex<Vec<TokenLease>>,
}

impl FakeBroker {
    pub fn lend_count(&self) -> usize {
        self.lends.load(Ordering::SeqCst)
    }
    pub fn invalidations(&self) -> Vec<TokenLease> {
        self.invalidated.lock().unwrap().clone()
    }
    pub fn lent(&self) -> Vec<(String, String, String)> {
        self.lent.lock().unwrap().clone()
    }
}

impl TokenBroker for FakeBroker {
    fn with_token(
        &self,
        profile: &str,
        host: &str,
        f: &mut dyn FnMut(&[u8]),
    ) -> CoreResult<TokenLease> {
        let n = self.lends.fetch_add(1, Ordering::SeqCst) + 1;
        let t = token(n);
        self.lent
            .lock()
            .unwrap()
            .push((profile.to_owned(), host.to_owned(), t.clone()));
        f(t.as_bytes());
        Ok(TokenLease::new(n as u64))
    }
    fn invalidate(&self, _profile: &str, lease: TokenLease) -> CoreResult<()> {
        self.invalidated.lock().unwrap().push(lease);
        Ok(())
    }
    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        Ok("1234".into())
    }
}

/// TEST ONLY: plain HTTP to 127.0.0.1, and only to the profile host.
pub struct LoopbackPolicy;

fn host_port(url: &str) -> Option<(String, String)> {
    let rest = url.strip_prefix("http://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.contains('@') {
        return None;
    }
    let (h, p) = authority.rsplit_once(':')?;
    Some((h.to_owned(), p.to_owned()))
}

impl TokenEgressPolicy for LoopbackPolicy {
    fn may_send_token(&self, profile_host: &str, url: &str) -> bool {
        matches!(host_port(url), Some((h, _)) if h == "127.0.0.1" && h == profile_host)
    }
    fn may_follow_redirect_with_authorization(&self, p: &str, from: &str, to: &str) -> bool {
        self.may_send_token(p, to) && host_port(from) == host_port(to)
    }
    fn reply_for_ts(&self, content_type: Option<&str>, body: &[u8]) -> Option<Value> {
        AllowlistEgressPolicy.reply_for_ts(content_type, body)
    }
    fn may_retry_with_token_after_non_json(&self) -> bool {
        false
    }
}

/// Keeps every R8 log line.
#[derive(Default)]
pub struct Lines(pub Mutex<Vec<String>>);

impl Lines {
    pub fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl McpLog for Lines {
    fn record(&self, line: &str) {
        self.0.lock().unwrap().push(line.to_owned());
    }
}

pub fn tags(mog: &str) -> Tags {
    serde_json::from_value(json!({"app": "bottle", "mog": mog, "op": "genie"})).unwrap()
}

pub fn obj(v: Value) -> mog_core::ipc::JsonObject {
    serde_json::from_value(v).unwrap()
}
