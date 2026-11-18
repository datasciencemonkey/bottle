//! The Genie One MCP loopback proxy of the main agent (#139, RTR-27, ARC-7, 4.6 rules 5, 7, 9).
//!
//! The harness reaches Genie One only through the `genie-one-mcp` loopback listener of its
//! session. The proxy forwards each call to the SAME R8 McpTransport that the brain uses, so the
//! agent's calls get the R8 rules: the G-06a allowlist, a fresh token per HTTP request (G-04d),
//! the shared per-profile rate bucket (G-09a), the poll guard (ARC-F1) and the rule-7 scrub.
//!
//! Every test runs against the fake Genie One MCP server on loopback and a fake R6 broker that
//! lends plainly fake tokens (`test-token-not-real-<n>`). No live workspace.

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fake_mcp::*;
use mog_core::bottle_mcp::{
    BottleMcpDeps, BottleMcpServer, UnavailableLocalTools, UnavailableRelay,
};
use mog_core::clock::SimClock;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::backoff::Jitter;
use mog_core::http_transport::rate_bucket::{AimdBucket, CallClass, RateBucket, Ticket};
use mog_core::http_transport::rate_limit::{NoBusyObserver, RateLimit};
use mog_core::http_transport::Pause;
use mog_core::mcp_transport::proxy::{GenieProxyDeps, GenieTarget, GenieTargets};
use mog_core::mcp_transport::{McpCall, McpLimits, McpSeams, McpTransport, ReqwestMcpTransport};
use mog_core::request_identity::{BottleIdentity, TAGS_HEADER};
use serde_json::{json, Value};

/// A wait bound for a broken run only; every wait ends on an event first.
const BOUND: Duration = Duration::from_secs(60);

// ---------- the rig ----------

/// The MOG target of the session: [`PROFILE`] and an optional warehouse.
struct FixedTarget {
    warehouse: Option<String>,
    asked: Mutex<Vec<String>>,
}

impl GenieTargets for FixedTarget {
    fn target(&self, mog_id: &str) -> CoreResult<GenieTarget> {
        self.asked.lock().unwrap().push(mog_id.to_owned());
        Ok(GenieTarget {
            profile: PROFILE.to_owned(),
            warehouse_id: self.warehouse.clone(),
        })
    }
}

/// A MOG with no Genie target yet.
struct NoTarget;

impl GenieTargets for NoTarget {
    fn target(&self, _mog_id: &str) -> CoreResult<GenieTarget> {
        Err(CoreError::Unavailable {
            component: "test",
            reason: "no_target",
        })
    }
}

/// The bucket that R7 and R8 share, with a record of each call that took a place in it.
struct Recording {
    inner: AimdBucket,
    queued: Mutex<Vec<(String, CallClass)>>,
}

impl RateBucket for Recording {
    fn reserve(&self, profile: &str) -> Duration {
        self.inner.reserve(profile)
    }
    fn on_rate_limited(&self, profile: &str, retry_after: Option<Duration>) {
        self.inner.on_rate_limited(profile, retry_after);
    }
    fn enqueue(&self, profile: &str, class: CallClass) -> Ticket {
        self.queued
            .lock()
            .unwrap()
            .push((profile.to_owned(), class));
        self.inner.enqueue(profile, class)
    }
    fn take(&self, profile: &str, ticket: &Ticket) -> Duration {
        self.inner.take(profile, ticket)
    }
    fn leave(&self, profile: &str, ticket: Ticket) {
        self.inner.leave(profile, ticket);
    }
    fn hold(&self, profile: &str, wait: Duration) {
        self.inner.hold(profile, wait);
    }
}

#[derive(Default)]
struct SimPause {
    clock: SimClock,
    pauses: Mutex<Vec<Duration>>,
}

impl SimPause {
    fn total(&self) -> Duration {
        self.pauses.lock().unwrap().iter().sum()
    }
}

impl Pause for SimPause {
    fn pause(&self, d: Duration) {
        self.pauses.lock().unwrap().push(d);
        self.clock.advance(d);
    }
}

struct Half;

impl Jitter for Half {
    fn unit(&self) -> f64 {
        0.5
    }
}

struct Rig {
    /// The ONE R8 that the brain and the proxy share.
    r8: Arc<ReqwestMcpTransport>,
    broker: Arc<FakeBroker>,
    hosts: Arc<Hosts>,
    bucket: Arc<Recording>,
    pause: Arc<SimPause>,
    target: Arc<FixedTarget>,
    /// The session's `bottle-mcp` server; it also serves the `genie-one-mcp` listener.
    server: Option<BottleMcpServer>,
}

impl Rig {
    fn server(&self) -> &BottleMcpServer {
        self.server.as_ref().expect("the server runs")
    }
    fn url(&self) -> String {
        self.server()
            .genie_one_url()
            .expect("the session has a Genie One proxy")
    }
    fn auth(&self) -> String {
        self.server().authorization().to_owned()
    }
    /// One JSON-RPC request from the harness, with the session secret.
    fn rpc(&self, msg: Value) -> Answer {
        post(&self.url(), Some(&self.auth()), None, &msg.to_string())
    }
    fn call(&self, tool: &str, args: Value) -> Answer {
        self.rpc(json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
                        "params": {"name": tool, "arguments": args}}))
    }
}

fn rig_with(s: &FakeMcp, warehouse: Option<&str>, targets: Option<Arc<dyn GenieTargets>>) -> Rig {
    let broker = Arc::new(FakeBroker::default());
    let hosts = Hosts::one(&s.origin());
    let pause = Arc::new(SimPause::default());
    let bucket = Arc::new(Recording {
        inner: AimdBucket::new(Arc::new(pause.clock.clone())),
        queued: Mutex::new(Vec::new()),
    });
    let r8 = Arc::new(
        ReqwestMcpTransport::with_seams(McpSeams {
            hosts: hosts.clone(),
            endpoints: Endpoints::one(&s.url()),
            tokens: broker.clone(),
            policy: Arc::new(LoopbackPolicy),
            decorator: Arc::new(BottleIdentity::new()),
            log: Arc::new(Lines::default()),
            limits: McpLimits::default(),
        })
        .unwrap()
        .with_rate_limit(RateLimit {
            bucket: bucket.clone(),
            pause: pause.clone(),
            jitter: Arc::new(Half),
            observer: Arc::new(NoBusyObserver),
        }),
    );
    let target = Arc::new(FixedTarget {
        warehouse: warehouse.map(str::to_owned),
        asked: Mutex::new(Vec::new()),
    });
    let targets = targets.unwrap_or_else(|| target.clone());
    let transport: Arc<dyn McpTransport> = r8.clone();
    let deps = BottleMcpDeps::new(Arc::new(UnavailableLocalTools), Arc::new(UnavailableRelay))
        .with_genie_proxy(GenieProxyDeps::new(transport, targets));
    let server = BottleMcpServer::start(MOG, &deps).expect("the session listeners bind");
    Rig {
        r8,
        broker,
        hosts,
        bucket,
        pause,
        target,
        server: Some(server),
    }
}

fn rig(s: &FakeMcp) -> Rig {
    rig_with(s, None, None)
}

/// What the proxy answered: the HTTP status, the JSON body and every raw byte.
struct Answer {
    status: u16,
    body: Value,
    raw: String,
}

impl Answer {
    fn result(&self) -> &Value {
        &self.body["result"]
    }
    fn rpc_error(&self) -> &Value {
        &self.body["error"]
    }
    fn tool_error_text(&self) -> String {
        assert_eq!(self.status, 200, "{}", self.raw);
        assert_eq!(self.result()["isError"], json!(true), "{}", self.raw);
        self.result()["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_owned()
    }
}

/// POST `body` to `url` with `auth` (or none) and a `Host` header (the URL's, or `host`).
fn post(url: &str, auth: Option<&str>, host: Option<&str>, body: &str) -> Answer {
    let rest = url.strip_prefix("http://").expect("http url");
    let (authority, path) = rest.split_once('/').expect("url path");
    let mut req = format!(
        "POST /{path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        host.unwrap_or(authority),
        body.len()
    );
    if let Some(a) = auth {
        req.push_str(&format!("Authorization: {a}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    let Ok(mut s) = TcpStream::connect(authority) else {
        return Answer {
            status: 0,
            body: Value::Null,
            raw: String::new(),
        };
    };
    s.set_read_timeout(Some(BOUND)).unwrap();
    let _ = s.write_all(req.as_bytes());
    let mut out = Vec::new();
    let _ = s.read_to_end(&mut out);
    let raw = String::from_utf8_lossy(&out).into_owned();
    let status = raw
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0);
    let body = raw
        .split_once("\r\n\r\n")
        .and_then(|(_, b)| serde_json::from_str(b).ok())
        .unwrap_or(Value::Null);
    Answer { status, body, raw }
}

fn tool_calls(s: &FakeMcp) -> Vec<Seen> {
    s.seen()
        .into_iter()
        .filter(|x| x.rpc_method() == "tools/call")
        .collect()
}

fn ask_args() -> Value {
    json!({"question": QUESTION})
}

fn poll_args() -> Value {
    json!({"conversation_id": "c-1", "response_id": "r-1"})
}

// ---------- RTR-27: the proxy adds the token, the harness never holds one ----------

#[test]
fn rtr_27_at_42_agent_ask_goes_upstream_with_a_fresh_token_on_each_request() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let a = r.call("genie_ask", ask_args());
    assert_eq!(a.status, 200, "{}", a.raw);
    assert_eq!(a.result()["isError"], json!(false), "{}", a.raw);
    assert_eq!(
        a.result()["structuredContent"]["status"],
        json!("completed")
    );
    assert!(a.result()["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("42"));

    // initialize, the notification and the tool call: each its own lend and its own token.
    let seen = s.seen();
    assert_eq!(
        s.methods(),
        ["initialize", "notifications/initialized", "tools/call"]
    );
    let bearers: Vec<String> = seen.iter().map(|x| x.bearer().unwrap()).collect();
    assert_eq!(bearers, [token(1), token(2), token(3)]);
    assert_eq!(r.broker.lend_count(), 3);
    for (profile, host, _) in r.broker.lent() {
        assert_eq!(profile, PROFILE, "the MOG's profile");
        assert_eq!(host, "127.0.0.1", "only the profile host (4.6 rule 2)");
    }

    // The harness secret is never sent upstream, and no token comes back to the harness.
    let secret = r.auth()["Bearer ".len()..].to_owned();
    for x in &seen {
        let text = format!("{:?} {}", x.headers, String::from_utf8_lossy(&x.body));
        assert!(!text.contains(&secret), "the session secret went upstream");
    }
    assert!(!a.raw.contains(TOKEN_PREFIX), "{}", a.raw);
    assert_eq!(seen[2].tool(), "genie_ask");
    assert_eq!(seen[2].json()["params"]["arguments"], ask_args());
}

#[test]
fn rtr_27_g_04c_initialize_and_ping_are_answered_by_the_proxy() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let a = r.rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                         "params": {"protocolVersion": "2025-06-18", "capabilities": {}}}));
    assert_eq!(a.status, 200, "{}", a.raw);
    assert_eq!(a.result()["protocolVersion"], json!("2025-06-18"));
    assert!(a.result()["capabilities"]["tools"].is_object());
    let p = r.rpc(json!({"jsonrpc": "2.0", "id": 2, "method": "ping"}));
    assert_eq!(p.result(), &json!({}));
    let n = r.rpc(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert_eq!(n.status, 202);
    let m = r.rpc(json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}));
    assert_eq!(m.rpc_error()["code"], json!(-32601));
    assert!(s.seen().is_empty(), "nothing went to Genie One");
    assert_eq!(r.broker.lend_count(), 0, "no token was lent");
}

// ---------- G-06a: the tool allowlist ----------

#[test]
fn g_06a_rtr_27_proxy_refuses_every_tool_off_the_allowlist_before_any_send() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    for tool in [
        "view_ask",
        "genie_delete_conversation",
        "publish_page",
        "memory.write",
        "Genie_Ask",
        "",
    ] {
        let a = r.call(tool, ask_args());
        assert_eq!(a.status, 200, "{}", a.raw);
        assert_eq!(a.rpc_error()["code"], json!(-32602), "{tool}: {}", a.raw);
    }
    let no_name = r.rpc(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {}}));
    assert_eq!(no_name.rpc_error()["code"], json!(-32602));
    assert!(s.seen().is_empty(), "nothing went to Genie One");
    assert_eq!(r.broker.lend_count(), 0, "no token was lent");
}

#[test]
fn g_06a_tools_list_shows_only_the_allowed_tools() {
    let s = genie_one(|seen| {
        (seen.rpc_method() == "tools/list").then(|| {
            json_reply(
                200,
                tools_result(
                    &seen.rpc_id(),
                    &["genie_ask", "view_ask", "genie_poll_response", "drop_table"],
                ),
            )
        })
    });
    let r = rig(&s);
    let a = r.rpc(json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list"}));
    assert_eq!(a.status, 200, "{}", a.raw);
    let names: Vec<&str> = a.result()["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["genie_ask", "genie_poll_response"]);
}

#[test]
fn g_06a_tool_arguments_must_match_the_contract() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    for args in [
        json!({}),
        json!({"question": 5}),
        json!({"question": "q", "sql": "drop table x"}),
        json!("not an object"),
    ] {
        let a = r.call("genie_ask", args.clone());
        assert_eq!(a.rpc_error()["code"], json!(-32602), "{args}: {}", a.raw);
    }
    assert!(tool_calls(&s).is_empty());
}

// ---------- 4.6 rule 5, RTR-29d, G-06b, G-07a: the session decides, not the agent ----------

#[test]
fn rtr_29d_rule_5_the_agent_cannot_pick_the_profile() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let a = r.rpc(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "genie_ask", "profile": "other-workspace",
                   "arguments": {"question": QUESTION, "profile": "other-workspace"}}}));
    assert_eq!(a.result()["isError"], json!(false), "{}", a.raw);
    let calls = tool_calls(&s);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].json()["params"]["arguments"], ask_args());
    assert!(calls[0].json()["params"].get("profile").is_none());
    let asked = r.hosts.asked.lock().unwrap().clone();
    assert!(asked.iter().all(|p| p == PROFILE), "{asked:?}");
    assert!(r.broker.lent().iter().all(|(p, _, _)| p == PROFILE));
    assert_eq!(r.target.asked.lock().unwrap().as_slice(), [MOG]);
}

#[test]
fn g_06b_g_07a_warehouse_comes_from_the_mog_and_the_agent_meta_is_dropped() {
    let s = genie_one(|_| None);
    let r = rig_with(&s, Some("abc123"), None);
    let ask = r.rpc(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "genie_ask", "arguments": ask_args(),
                   "_meta": {"warehouse_id": "evil", "progressToken": 5}}}));
    assert_eq!(ask.result()["isError"], json!(false), "{}", ask.raw);
    let poll = r.rpc(json!({"jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": {"name": "genie_poll_response", "arguments": poll_args(),
                   "_meta": {"progressToken": 6}}}));
    assert_eq!(poll.result()["isError"], json!(false), "{}", poll.raw);
    let calls = tool_calls(&s);
    assert_eq!(
        calls[0].json()["params"]["_meta"],
        json!({"warehouse_id": "abc123"})
    );
    assert!(calls[1].json()["params"].get("_meta").is_none());
    for c in &calls {
        assert!(!String::from_utf8_lossy(&c.body).contains("progressToken"));
    }
}

#[test]
fn sec_06a_upstream_tags_name_the_session_mog_and_the_genie_op() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let a = r.rpc(json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "genie_ask", "arguments": ask_args(),
                   "tags": {"app": "bottle", "mog": MOG_2, "op": "page"}}}));
    assert_eq!(a.result()["isError"], json!(false), "{}", a.raw);
    for x in s.seen() {
        assert_eq!(
            x.header(TAGS_HEADER),
            Some(format!(r#"{{"app":"bottle","mog":"{MOG}","op":"genie"}}"#).as_str())
        );
    }
}

#[test]
fn g_20b_query_result_item_ids_must_match_pub_output() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    for bad in [
        "x",
        "pub_1",
        "pub__output",
        "pub_1_output\n",
        "item_1_output",
    ] {
        let a = r.call(
            "genie_get_query_result",
            json!({"conversation_id": "c-1", "response_id": "r-1", "item_id": bad}),
        );
        assert_eq!(a.rpc_error()["code"], json!(-32602), "{bad:?}: {}", a.raw);
    }
    assert!(tool_calls(&s).is_empty());
    let ok = r.call(
        "genie_get_query_result",
        json!({"conversation_id": "c-1", "response_id": "r-1", "item_id": "pub_7_output"}),
    );
    assert_eq!(ok.result()["isError"], json!(false), "{}", ok.raw);
    assert_eq!(tool_calls(&s).len(), 1);
}

// ---------- G-09a, G-09b: the shared per-profile bucket ----------

#[test]
fn g_09a_g_09b_agent_calls_wait_in_the_bucket_the_brain_uses() {
    let s = genie_one(|seen| {
        (seen.rpc_method() == "tools/call" && seen.tool() == "genie_cancel_response").then(|| {
            json_reply(429, json!({"error": {"code": "RATE_LIMITED"}}))
                .with_header("Retry-After", "30")
        })
    });
    let r = rig(&s);
    assert_eq!(
        r.call("genie_ask", ask_args()).result()["isError"],
        json!(false)
    );
    assert_eq!(
        r.call("genie_poll_response", poll_args()).result()["isError"],
        json!(false)
    );
    let queued = r.bucket.queued.lock().unwrap().clone();
    assert!(queued.iter().all(|(p, _)| p == PROFILE), "{queued:?}");
    assert!(queued.contains(&(PROFILE.to_owned(), CallClass::Ask)));
    assert!(queued.contains(&(PROFILE.to_owned(), CallClass::Poll)));

    // A 429 with Retry-After on a BRAIN call holds the profile; the next AGENT call waits for it.
    let t = tags(MOG_2);
    let brain = r.r8.call_tool(McpCall::new(
        PROFILE,
        "genie_cancel_response",
        obj(json!({"conversation_id": "c-9"})),
        &t,
    ));
    assert_eq!(brain.expect_err("429").code(), "rate_limited");
    let before = r.pause.total();
    assert_eq!(
        r.call("genie_poll_response", poll_args()).result()["isError"],
        json!(false)
    );
    assert!(
        r.pause.total() - before >= Duration::from_secs(29),
        "the agent poll waited for the brain's Retry-After: {:?}",
        r.pause.total() - before
    );
}

// ---------- 4.6 rule 7: replies to the harness are scrubbed ----------

#[test]
fn arc_6_rule_7_replies_to_the_harness_are_scrubbed() {
    // A JWT-shaped value, built at run time so no credential-shaped literal is in the repo.
    let jwt = format!("{}{}.{}.sig", "ey", "JhbGciOiJub25lIn0", "eyJzdWIiOiJ4In0");
    let leak = jwt.clone();
    let s = genie_one(move |seen| {
        (seen.rpc_method() == "tools/call").then(|| {
            json_reply(200, json!({"jsonrpc": "2.0", "id": seen.rpc_id(), "result": {
                "content": [{"type": "text", "text": format!("token {leak} password=not-real-pass-1")}],
                "structuredContent": {"status": "completed", "note": format!("Bearer {leak}")},
                "isError": false
            }}))
        })
    });
    let r = rig(&s);
    let a = r.call("genie_ask", ask_args());
    assert_eq!(a.status, 200, "{}", a.raw);
    assert!(!a.raw.contains(&jwt[..20]), "{}", a.raw);
    assert!(!a.raw.contains("not-real-pass-1"), "{}", a.raw);
    assert_eq!(
        a.result()["structuredContent"]["status"],
        json!("completed")
    );
}

// ---------- 4.6 rule 9, ARC-7: the per-session secret ----------

#[test]
fn arc_7_rule_9_the_proxy_needs_the_session_secret_and_a_loopback_host() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    let other = rig(&s);
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": {"name": "genie_ask", "arguments": ask_args()}})
    .to_string();
    assert_eq!(post(&r.url(), None, None, &body).status, 401);
    assert_eq!(
        post(&r.url(), Some("Bearer not-the-secret"), None, &body).status,
        401
    );
    assert_eq!(post(&r.url(), Some(&other.auth()), None, &body).status, 401);
    let port = r
        .url()
        .split(':')
        .nth(2)
        .unwrap()
        .split('/')
        .next()
        .unwrap()
        .to_owned();
    assert_eq!(
        post(
            &r.url(),
            Some(&r.auth()),
            Some(&format!("evil.example:{port}")),
            &body
        )
        .status,
        403
    );
    assert!(s.seen().is_empty(), "nothing went to Genie One");
    assert_eq!(r.broker.lend_count() + other.broker.lend_count(), 0);
    assert!(r.url().starts_with("http://127.0.0.1:"), "{}", r.url());
    assert_ne!(
        r.url(),
        r.server().url(),
        "Genie One has its own listener, not the bottle-mcp one"
    );
}

#[test]
fn rtr_27_a_stopped_session_proxy_opens_nothing() {
    let s = genie_one(|_| None);
    let mut r = rig(&s);
    let (url, auth) = (r.url(), r.auth());
    drop(r.server.take());
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}).to_string();
    assert_ne!(post(&url, Some(&auth), None, &body).status, 200);
    assert!(s.seen().is_empty());
}

// ---------- failures are content-free codes ----------

#[test]
fn rtr_27_upstream_failures_reach_the_harness_as_content_free_codes() {
    // A 404 with no session: Genie One is off in the workspace (G-05b).
    let s = genie_one(|seen| (seen.rpc_method() == "initialize").then(|| empty_reply(404)));
    let r = rig(&s);
    let text = r.call("genie_ask", ask_args()).tool_error_text();
    assert_eq!(text, "genie_one_off");
    let host = s.origin();
    assert!(!text.contains(&host[7..]));
}

#[test]
fn rtr_27_a_mog_without_a_genie_target_sends_nothing() {
    let s = genie_one(|_| None);
    let r = rig_with(&s, None, Some(Arc::new(NoTarget)));
    let text = r.call("genie_ask", ask_args()).tool_error_text();
    assert_eq!(text, "genie_target_unavailable");
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lend_count(), 0);
}

#[test]
fn rtr_27_a_session_stop_does_not_wait_for_a_slow_genie_call() {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Mutex::new(release_rx);
    let s = genie_one(move |seen| {
        if seen.rpc_method() == "tools/call" {
            // Hold the Genie answer until the test lets it go.
            let _ = release_rx.lock().unwrap().recv_timeout(BOUND);
        }
        None
    });
    let mut r = rig(&s);
    let (url, auth) = (r.url(), r.auth());
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
                      "params": {"name": "genie_ask", "arguments": ask_args()}})
    .to_string();
    let caller = std::thread::spawn(move || post(&url, Some(&auth), None, &body));
    let start = Instant::now();
    while tool_calls(&s).is_empty() {
        assert!(start.elapsed() < BOUND, "the ask reached Genie One");
        std::thread::sleep(Duration::from_millis(10));
    }
    let stop = Instant::now();
    drop(r.server.take());
    let stopped_in = stop.elapsed();
    let _ = release_tx.send(());
    let a = caller.join().unwrap();
    assert!(
        stopped_in < Duration::from_secs(10),
        "the stop waited {stopped_in:?} for Genie"
    );
    assert_ne!(a.result()["isError"], json!(false), "{}", a.raw);
}
