//! The real R7 HttpTransport (#364): `reqwest` + `rustls`, on a std `TcpListener` loopback
//! server. Loopback is allowed ONLY by the test double [`LoopbackPolicy`]; the production
//! policy refuses it (see `arc_6_rule_2_production_policy_refuses_loopback_and_accounts`).
//!
//! Every token here is a plainly fake value (`test-token-not-real`), never a real format.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::egress_policy::{contract_violations, TokenEgressPolicy};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::host_policy::AllowlistEgressPolicy;
use mog_core::http_transport::rate_bucket::{
    parse_retry_after, AimdBucket, RateBucket, DEFAULT_RATE_PER_MIN,
};
use mog_core::http_transport::routes::{route, Method};
use mog_core::http_transport::{
    HttpTransport, Pause, ProfileHosts, ReqwestHttpTransport, TransportSeams, ROUTES,
};
use mog_core::ipc::JsonObject;
use mog_core::request_identity::{user_agent, BottleIdentity};
use mog_core::token_broker::{TokenBroker, TokenLease};
use mog_core::types::Tags;
use serde_json::{json, Value};

const TOKEN: &str = "test-token-not-real";
const TOKEN_2: &str = "test-token-not-real-2";
const PROFILE: &str = "dev";

// ---------- loopback test server (std only) ----------

#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn json_reply(status: u16, v: Value) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: serde_json::to_vec(&v).unwrap(),
    }
}

fn page_reply(status: u16) -> Reply {
    Reply {
        status,
        headers: vec![("Content-Type".into(), "text/html; charset=utf-8".into())],
        body: b"<html><body>Sign in to the proxy</body></html>".to_vec(),
    }
}

fn with_header(mut r: Reply, name: &str, value: &str) -> Reply {
    r.headers.push((name.into(), value.into()));
    r
}

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

struct Server {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.addr.port())
    }
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

fn read_request(stream: &mut TcpStream) -> Option<Seen> {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
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

/// Serve `replies` in order, one per connection, and record each request.
fn serve(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rec = Arc::clone(&seen);
    thread::spawn(move || {
        for reply in replies {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            if let Some(req) = read_request(&mut stream) {
                rec.lock().unwrap().push(req);
            }
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
    Server { addr, seen }
}

// ---------- doubles ----------

struct Hosts(BTreeMap<String, String>);

impl ProfileHosts for Hosts {
    fn workspace_url(&self, profile: &str) -> CoreResult<String> {
        self.0.get(profile).cloned().ok_or(CoreError::Unavailable {
            component: "test",
            reason: "no_profile",
        })
    }
}

fn hosts(url: &str) -> Arc<Hosts> {
    Arc::new(Hosts(BTreeMap::from([(
        PROFILE.to_owned(),
        url.to_owned(),
    )])))
}

/// Lends `TOKEN`, then `TOKEN_2` after an invalidate.
#[derive(Default)]
struct FakeBroker {
    lends: AtomicUsize,
    invalidations: AtomicUsize,
}

impl TokenBroker for FakeBroker {
    fn with_token(
        &self,
        _profile: &str,
        _host: &str,
        f: &mut dyn FnMut(&[u8]),
    ) -> CoreResult<TokenLease> {
        self.lends.fetch_add(1, Ordering::SeqCst);
        if self.invalidations.load(Ordering::SeqCst) == 0 {
            f(TOKEN.as_bytes());
        } else {
            f(TOKEN_2.as_bytes());
        }
        Ok(TokenLease::default())
    }
    fn invalidate(&self, _profile: &str, _lease: TokenLease) -> CoreResult<()> {
        self.invalidations.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        Ok("1".into())
    }
}

/// TEST ONLY: allows plain HTTP to 127.0.0.1 so the loopback server can stand in for a
/// workspace. Rule 6 is the production rule ([`AllowlistEgressPolicy`]).
struct LoopbackPolicy;

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

/// Records each pause and moves the shared sim clock on by it.
#[derive(Debug, Default)]
struct SimPause {
    clock: SimClock,
    pauses: Mutex<Vec<Duration>>,
}

impl Pause for SimPause {
    fn pause(&self, d: Duration) {
        self.pauses.lock().unwrap().push(d);
        self.clock.advance(d);
    }
}

struct Rig {
    transport: ReqwestHttpTransport,
    broker: Arc<FakeBroker>,
    pause: Arc<SimPause>,
    bucket: Arc<AimdBucket>,
}

fn rig_with(url: &str, policy: Arc<dyn TokenEgressPolicy + Send + Sync>) -> Rig {
    let broker = Arc::new(FakeBroker::default());
    let pause = Arc::new(SimPause::default());
    let clock: Arc<dyn MonotonicClock> = Arc::new(pause.clock.clone());
    let bucket = Arc::new(AimdBucket::new(clock));
    let transport = ReqwestHttpTransport::with_seams(TransportSeams {
        hosts: hosts(url),
        tokens: broker.clone(),
        policy,
        bucket: bucket.clone(),
        decorator: Arc::new(BottleIdentity::new()),
        pause: pause.clone(),
    })
    .unwrap();
    Rig {
        transport,
        broker,
        pause,
        bucket,
    }
}

fn rig(server: &Server) -> Rig {
    rig_with(&server.origin(), Arc::new(LoopbackPolicy))
}

fn tags(mog: &str, op: &str) -> Tags {
    serde_json::from_value(json!({"app": "bottle", "mog": mog, "op": op})).unwrap()
}

fn params(v: Value) -> JsonObject {
    serde_json::from_value(v).unwrap()
}

fn none() -> JsonObject {
    JsonObject::new()
}

fn code(r: CoreResult<Value>) -> &'static str {
    r.expect_err("the call must fail").code()
}

// ---------- SEC-06a / SEC-06b: headers on the wire ----------

#[test]
fn sec_06b_user_agent_and_token_on_the_wire_for_a_workspace_route() {
    let s = serve(vec![json_reply(200, json!({"userName": "a"}))]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
        .unwrap();
    assert_eq!(got, json!({"userName": "a"}));
    let seen = s.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "GET");
    assert_eq!(seen[0].target, "/api/2.0/preview/scim/v2/Me");
    assert_eq!(seen[0].header("user-agent"), Some(user_agent().as_str()));
    assert_eq!(
        seen[0].header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    // Data minimisation: no tags header on a workspace call.
    assert_eq!(seen[0].header("databricks-ai-gateway-request-tags"), None);
}

#[test]
fn sec_06a_nfr_21a_tags_header_on_the_wire_for_a_unity_gateway_route() {
    let s = serve(vec![json_reply(200, json!({"choices": []}))]);
    let r = rig(&s);
    let body = json!({"model": "system.ai.fake-model", "messages": []});
    r.transport
        .request(
            PROFILE,
            "mlflow.chat_completions",
            &none(),
            Some(&body),
            &tags("m1", "cleanup"),
        )
        .unwrap();
    let seen = s.seen();
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].target, "/ai-gateway/mlflow/v1/chat/completions");
    assert_eq!(
        seen[0].header("databricks-ai-gateway-request-tags"),
        Some(r#"{"app":"bottle","mog":"m1","op":"cleanup"}"#)
    );
    assert_eq!(seen[0].header("user-agent"), Some(user_agent().as_str()));
    assert_eq!(seen[0].header("content-type"), Some("application/json"));
    assert_eq!(
        serde_json::from_slice::<Value>(&seen[0].body).unwrap(),
        body
    );
}

#[test]
fn sec_06a_bad_tags_send_nothing_and_lend_no_token() {
    let s = serve(vec![json_reply(200, json!({}))]);
    let r = rig(&s);
    let got = r.transport.request(
        PROFILE,
        "mlflow.embeddings",
        &none(),
        Some(&json!({})),
        &tags("bad mog!", "embed"),
    );
    assert_eq!(code(got), "tags_mog_chars");
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lends.load(Ordering::SeqCst), 0);
}

// ---------- ARC-6 / 4.6 rule 1: route IDs and params, never a URL ----------

#[test]
fn arc_6_rule_1_unknown_route_sends_nothing() {
    let s = serve(vec![json_reply(200, json!({}))]);
    let r = rig(&s);
    for id in ["https://evil.example/x", "/api/2.0/x", "scim.me/../x", ""] {
        let got = r
            .transport
            .request(PROFILE, id, &none(), None, &tags("m1", "genie"));
        assert_eq!(code(got), "unknown_route", "route {id:?}");
    }
    assert!(s.seen().is_empty());
}

#[test]
fn arc_6_rule_1_params_fill_only_the_named_slots() {
    let s = serve(vec![json_reply(200, json!({"status": "COMPLETED"}))]);
    let r = rig(&s);
    let p = params(json!({"space_id": "s1", "conversation_id": "c-2", "message_id": "m_3"}));
    r.transport
        .request(PROFILE, "genie.get_message", &p, None, &tags("m1", "genie"))
        .unwrap();
    let seen = s.seen();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(
        seen[0].target,
        "/api/2.0/genie/spaces/s1/conversations/c-2/messages/m_3"
    );
}

#[test]
fn arc_6_rule_1_missing_or_unknown_params_are_invalid_params() {
    let s = serve(vec![json_reply(200, json!({}))]);
    let r = rig(&s);
    let t = tags("m1", "genie");
    let missing = params(json!({"space_id": "s1"}));
    assert_eq!(
        code(
            r.transport
                .request(PROFILE, "genie.message", &missing, None, &t)
        ),
        "invalid_params"
    );
    let extra = params(json!({"space_id": "s1", "host": "evil"}));
    assert_eq!(
        code(
            r.transport
                .request(PROFILE, "genie.start_conversation", &extra, None, &t)
        ),
        "invalid_params"
    );
    let on_slotless = params(json!({"space_id": "s1"}));
    assert_eq!(
        code(
            r.transport
                .request(PROFILE, "scim.me", &on_slotless, None, &t)
        ),
        "invalid_params"
    );
    assert!(s.seen().is_empty());
    assert_eq!(r.broker.lends.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_6_rule_1_no_url_or_path_from_params() {
    let s = serve(vec![json_reply(200, json!({}))]);
    let r = rig(&s);
    let t = tags("m1", "genie");
    for bad in [
        json!("../admin"),
        json!("a/b"),
        json!("https://evil.example"),
        json!("a?b=c"),
        json!("a#b"),
        json!("a%2fb"),
        json!("a b"),
        json!("a.b"),
        json!(""),
        json!("x".repeat(200)),
        json!({"nested": "s1"}),
        json!(null),
        json!(["s1"]),
    ] {
        let p = params(json!({ "space_id": bad.clone() }));
        let got = r
            .transport
            .request(PROFILE, "genie.start_conversation", &p, None, &t);
        assert_eq!(code(got), "invalid_params", "value {bad}");
    }
    assert!(s.seen().is_empty());
}

#[test]
fn arc_6_rule_1_query_params_are_percent_encoded() {
    let s = serve(vec![json_reply(200, json!({"conversations": []}))]);
    let r = rig(&s);
    let p = params(json!({"page_size": 20, "page_token": "ab+/c="}));
    r.transport
        .request(
            PROFILE,
            "genie_one.list_conversations",
            &p,
            None,
            &tags("m1", "genie"),
        )
        .unwrap();
    assert_eq!(
        s.seen()[0].target,
        "/api/2.0/genie/one/conversations?page_size=20&page_token=ab%2B%2Fc%3D"
    );
}

#[test]
fn arc_6_nfr_21a_nfr_21b_every_route_has_a_fixed_method_and_path() {
    for (id, _) in ROUTES {
        match route(id) {
            Ok(r) => {
                assert!(r.path.starts_with('/'), "{id}");
                assert!(!r.path.contains("/serving-endpoints"), "{id}: NFR-21b");
                assert!(!r.path.contains("://"), "{id}");
            }
            Err(e) => assert_eq!(e.code(), "route_not_wired", "{id}"),
        }
    }
    let chat = route("mlflow.chat_completions").unwrap();
    assert_eq!(
        (chat.method, chat.path),
        (Method::Post, "/ai-gateway/mlflow/v1/chat/completions")
    );
    let emb = route("mlflow.embeddings").unwrap();
    assert_eq!(
        (emb.method, emb.path),
        (Method::Post, "/ai-gateway/mlflow/v1/embeddings")
    );
    let me = route("scim.me").unwrap();
    assert_eq!(
        (me.method, me.path),
        (Method::Get, "/api/2.0/preview/scim/v2/Me")
    );
    assert_eq!(route("nope").unwrap_err().code(), "unknown_route");
}

// ---------- 4.6 rule 2: token only to the profile host ----------

#[test]
fn arc_6_rule_2_production_policy_refuses_loopback_and_accounts() {
    let p = AllowlistEgressPolicy;
    assert!(
        contract_violations(&p).is_empty(),
        "{:?}",
        contract_violations(&p)
    );
    for (host, url) in [
        ("127.0.0.1", "http://127.0.0.1:8080/api/2.0/x"),
        ("127.0.0.1", "https://127.0.0.1/api/2.0/x"),
        ("localhost", "https://localhost/api/2.0/x"),
        ("[::1]", "https://[::1]/api/2.0/x"),
        (
            "accounts.cloud.databricks.com",
            "https://accounts.cloud.databricks.com/api/2.0/x",
        ),
        (
            "accounts.azuredatabricks.net",
            "https://accounts.azuredatabricks.net/api/2.0/x",
        ),
        (
            "accounts-dod.cloud.databricks.us",
            "https://accounts-dod.cloud.databricks.us/api/2.0/x",
        ),
    ] {
        assert!(!p.may_send_token(host, url), "{url}");
    }
    let ok = "dbc-fake.cloud.databricks.com";
    assert!(p.may_send_token(ok, &format!("https://{ok}/api/2.0/x?o=1")));
    assert!(p.may_send_token(ok, &format!("https://{ok}:443/api/2.0/x")));
}

#[test]
fn arc_6_rule_2_rule_3_production_policy_refuses_another_databricks_host() {
    let p = AllowlistEgressPolicy;
    let a = "dbc-a.cloud.databricks.com";
    let b = "dbc-b.cloud.databricks.com";
    let from = format!("https://{a}/api/2.0/x");
    assert!(!p.may_send_token(a, &format!("https://{b}/api/2.0/x")));
    assert!(!p.may_follow_redirect_with_authorization(a, &from, &format!("https://{b}/x")));
    assert!(p.may_follow_redirect_with_authorization(a, &from, &format!("https://{a}/y")));
}

#[test]
fn arc_6_rule_2_production_transport_refuses_a_loopback_profile_host() {
    let s = serve(vec![json_reply(200, json!({}))]);
    let broker = Arc::new(FakeBroker::default());
    let pause = Arc::new(SimPause::default());
    let clock: Arc<dyn MonotonicClock> = Arc::new(pause.clock.clone());
    let t = ReqwestHttpTransport::new(
        hosts(&s.origin()),
        broker.clone(),
        Arc::new(AimdBucket::new(clock)),
    )
    .unwrap();
    let got = t.request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
    assert_eq!(code(got), "host_refused");
    assert!(s.seen().is_empty());
    assert_eq!(broker.lends.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_6_rule_2_a_profile_url_with_a_path_or_user_is_refused() {
    let s = serve(vec![json_reply(200, json!({}))]);
    for url in [
        format!("{}/api/x", s.origin()),
        format!("{}?a=b", s.origin()),
        format!("http://u@127.0.0.1:{}", s.addr.port()),
        "not a url".to_owned(),
    ] {
        let r = rig_with(&url, Arc::new(LoopbackPolicy));
        let got = r
            .transport
            .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
        assert_eq!(code(got), "profile_host_invalid", "{url}");
    }
    assert!(s.seen().is_empty());
}

// ---------- 4.6 rule 3: redirects ----------

#[test]
fn arc_6_rule_3_cross_host_redirect_is_not_followed() {
    let other = serve(vec![json_reply(200, json!({"stolen": true}))]);
    let to = format!("http://localhost:{}/api/2.0/x", other.addr.port());
    let s = serve(vec![with_header(
        json_reply(302, json!({})),
        "Location",
        &to,
    )]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
    assert_eq!(code(got), "redirect_refused");
    assert_eq!(s.seen().len(), 1);
    thread::sleep(Duration::from_millis(100));
    assert!(
        other.seen().is_empty(),
        "no request may reach the other host"
    );
}

#[test]
fn arc_6_rule_3_same_host_redirect_is_followed_with_the_token() {
    let s = serve(vec![
        with_header(
            json_reply(307, json!({})),
            "Location",
            "/api/2.0/preview/scim/v2/Me?moved=1",
        ),
        json_reply(200, json!({"ok": 1})),
    ]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
        .unwrap();
    assert_eq!(got, json!({"ok": 1}));
    let seen = s.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].target, "/api/2.0/preview/scim/v2/Me?moved=1");
    assert!(seen[1].header("authorization").is_some());
}

// ---------- 4.6 rule 6: non-JSON pages ----------

#[test]
fn arc_6_rule_6_sec_05a_non_json_page_is_refused_and_not_retried() {
    for status in [200, 401, 403, 502] {
        let s = serve(vec![page_reply(status), json_reply(200, json!({}))]);
        let r = rig(&s);
        let err = r
            .transport
            .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
            .expect_err("a page must fail");
        assert_eq!(err.code(), "non_json_reply", "status {status}");
        assert!(!err.to_string().contains("Sign in"), "content-free error");
        assert_eq!(s.seen().len(), 1, "status {status}: no retry");
        assert_eq!(r.broker.invalidations.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn arc_6_rule_6_html_labelled_as_json_is_refused() {
    let s = serve(vec![Reply {
        status: 200,
        headers: vec![("Content-Type".into(), "application/json".into())],
        body: b"<html>proxy</html>".to_vec(),
    }]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
    assert_eq!(code(got), "non_json_reply");
}

// ---------- 4.6 rule 7: scrubbed replies ----------

#[test]
fn arc_6_rule_7_reply_strings_are_scrubbed() {
    let s = serve(vec![json_reply(
        200,
        json!({"note": "conn password=hunter2 here", "list": ["-----BEGIN KEY"]}),
    )]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
        .unwrap();
    let text = got.to_string();
    assert!(!text.contains("hunter2"), "{text}");
    assert!(!text.contains("-----BEGIN"), "{text}");
    assert!(text.contains("[REDACTED]"), "{text}");
}

// ---------- R6 / R7: 401 ----------

#[test]
fn r7_401_invalidates_the_token_once_and_retries_once() {
    let s = serve(vec![
        json_reply(401, json!({"error_code": "UNAUTHENTICATED"})),
        json_reply(200, json!({"ok": true})),
    ]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
        .unwrap();
    assert_eq!(got, json!({"ok": true}));
    assert_eq!(r.broker.invalidations.load(Ordering::SeqCst), 1);
    let seen = s.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        seen[1].header("authorization"),
        Some(format!("Bearer {TOKEN_2}").as_str())
    );
}

#[test]
fn r7_a_second_401_gives_auth_expired() {
    let s = serve(vec![
        json_reply(401, json!({})),
        json_reply(401, json!({})),
        json_reply(200, json!({})),
    ]);
    let r = rig(&s);
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
    assert_eq!(code(got), "auth_expired");
    assert_eq!(s.seen().len(), 2);
    assert_eq!(r.broker.invalidations.load(Ordering::SeqCst), 1);
}

// ---------- G-09a..d, G-10a: rate bucket and 429 ----------

#[test]
fn g_10a_429_retry_after_is_honoured_by_the_next_call() {
    let s = serve(vec![
        with_header(json_reply(429, json!({})), "Retry-After", "7"),
        json_reply(200, json!({"ok": 1})),
    ]);
    let r = rig(&s);
    let t = tags("m1", "genie");
    let first = r.transport.request(PROFILE, "scim.me", &none(), None, &t);
    assert_eq!(code(first), "rate_limited");
    assert_eq!(s.seen().len(), 1, "a 429 is not retried at once");
    r.transport
        .request(PROFILE, "scim.me", &none(), None, &t)
        .unwrap();
    let waited: Duration = r.pause.pauses.lock().unwrap().iter().sum();
    assert!(waited >= Duration::from_secs(7), "waited {waited:?}");
    assert!(waited <= Duration::from_secs(8), "waited {waited:?}");
}

#[test]
fn g_10a_fmapi_error_retry_after_is_honoured() {
    let s = serve(vec![
        json_reply(429, json!({"error": {"retry_after": 3}})),
        json_reply(200, json!({})),
    ]);
    let r = rig(&s);
    let t = tags("m1", "cleanup");
    let body = json!({});
    let first = r
        .transport
        .request(PROFILE, "mlflow.chat_completions", &none(), Some(&body), &t);
    assert_eq!(code(first), "rate_limited");
    r.transport
        .request(PROFILE, "mlflow.chat_completions", &none(), Some(&body), &t)
        .unwrap();
    let waited: Duration = r.pause.pauses.lock().unwrap().iter().sum();
    assert!(waited >= Duration::from_secs(3), "waited {waited:?}");
}

#[test]
fn g_09c_a_429_halves_the_profile_rate() {
    let s = serve(vec![json_reply(429, json!({}))]);
    let r = rig(&s);
    let _ = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
    assert_eq!(r.bucket.rate_per_min(PROFILE), DEFAULT_RATE_PER_MIN / 2);
}

#[test]
fn g_10a_retry_after_parses_seconds_and_http_dates_and_clamps() {
    let now = UNIX_EPOCH + Duration::from_secs(1_445_412_480); // Wed, 21 Oct 2015 07:28:00 GMT
    assert_eq!(parse_retry_after("7", now), Some(Duration::from_secs(7)));
    assert_eq!(parse_retry_after("0", now), Some(Duration::from_secs(1)));
    assert_eq!(
        parse_retry_after("9999", now),
        Some(Duration::from_secs(300))
    );
    assert_eq!(
        parse_retry_after("Wed, 21 Oct 2015 07:28:30 GMT", now),
        Some(Duration::from_secs(30))
    );
    assert_eq!(
        parse_retry_after("Wed, 21 Oct 2015 07:20:00 GMT", now),
        Some(Duration::from_secs(1))
    );
    assert_eq!(parse_retry_after("soon", now), None);
    assert_eq!(parse_retry_after("-5", now), None);
    let _ = SystemTime::now();
}

#[test]
fn g_09a_bucket_allows_40_calls_per_minute_per_profile() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    for i in 0..40 {
        assert_eq!(b.reserve("a"), Duration::ZERO, "call {i}");
    }
    let wait = b.reserve("a");
    assert!(wait > Duration::ZERO && wait <= Duration::from_secs(60));
    // The bucket is per profile.
    assert_eq!(b.reserve("b"), Duration::ZERO);
    clock.advance(Duration::from_secs(60));
    assert_eq!(b.reserve("a"), Duration::ZERO);
}

#[test]
fn g_09a_a_load_stays_at_40_calls_per_minute_or_less() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    let mut sent: Vec<Duration> = Vec::new();
    while sent.len() < 200 {
        let w = b.reserve("a");
        if w.is_zero() {
            sent.push(clock.now());
        } else {
            clock.advance(w);
        }
    }
    for (i, t) in sent.iter().enumerate() {
        let in_window = sent[i..]
            .iter()
            .take_while(|u| **u < *t + Duration::from_secs(60))
            .count();
        assert!(in_window <= 40, "{in_window} calls in one minute");
    }
}

#[test]
fn g_09c_g_09d_rate_halves_for_5_min_then_adds_5_per_minute() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    assert_eq!(b.rate_per_min("a"), 40);
    b.on_rate_limited("a", None);
    assert_eq!(b.rate_per_min("a"), 20);
    clock.advance(Duration::from_secs(299));
    assert_eq!(b.rate_per_min("a"), 20);
    clock.advance(Duration::from_secs(1));
    assert_eq!(b.rate_per_min("a"), 20);
    for want in [25, 30, 35, 40, 40] {
        clock.advance(Duration::from_secs(60));
        assert_eq!(b.rate_per_min("a"), want);
    }
    assert_eq!(b.rate_per_min("other"), 40);
}

#[test]
fn g_10a_bucket_waits_for_retry_after() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    b.on_rate_limited("a", Some(Duration::from_secs(12)));
    let w = b.reserve("a");
    assert_eq!(w, Duration::from_secs(12));
    clock.advance(w);
    assert_eq!(b.reserve("a"), Duration::ZERO);
}

// ---------- online state and content-free errors ----------

#[test]
fn r7_is_online_reports_the_last_transport_outcome() {
    let closed = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    let r = rig_with(
        &format!("http://127.0.0.1:{port}"),
        Arc::new(LoopbackPolicy),
    );
    assert!(
        r.transport.is_online().unwrap(),
        "online until a call fails"
    );
    let got = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
    let err = got.expect_err("connect must fail");
    assert_eq!(err.code(), "offline");
    assert!(!err.to_string().contains("127.0.0.1"));
    assert!(!r.transport.is_online().unwrap());

    let s = serve(vec![json_reply(200, json!({}))]);
    let ok = rig(&s);
    ok.transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
        .unwrap();
    assert!(ok.transport.is_online().unwrap());
}

#[test]
fn arc_6_errors_carry_no_host_token_or_body() {
    let s = serve(vec![json_reply(
        403,
        json!({"message": "denied for test-token-not-real at 127.0.0.1"}),
    )]);
    let r = rig(&s);
    let err = r
        .transport
        .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
        .expect_err("403 must fail");
    assert_eq!(err.code(), "upstream_rejected");
    let text = format!("{err} {err:?}");
    for bad in ["127.0.0.1", TOKEN, "denied", "/api/"] {
        assert!(!text.contains(bad), "{text}");
    }
}

// ---------- #18: the real R6 TokenBroker behind the real R7 ----------

mod real_r6 {
    use super::*;
    use mog_core::cli_runner::{refuse_token_command, CliRunner, SecretStdout};
    use mog_core::clock::{SimWallClock, WallClock};
    use mog_core::token_broker::{CliTokenBroker, ScimMe};
    use std::path::PathBuf;
    use std::sync::Weak;

    /// 2026-11-17T12:00:00Z in Unix milliseconds.
    const T0_MS: i64 = 1_794_916_800_000;

    /// A fake R5 that hands out TOKEN, then TOKEN_2, ... and counts the `auth token` runs.
    #[derive(Default)]
    struct CountingCli {
        runs: AtomicUsize,
    }

    impl CliRunner for CountingCli {
        fn resolve(&self) -> CoreResult<PathBuf> {
            Ok(PathBuf::from("/usr/local/bin/databricks"))
        }
        fn run_json(&self, args: &[&str]) -> CoreResult<Value> {
            refuse_token_command(args)?;
            panic!("R6 must not call run_json");
        }
        fn run_auth_token(&self, _profile: &str) -> CoreResult<SecretStdout> {
            let n = self.runs.fetch_add(1, Ordering::SeqCst);
            let token = if n == 0 { TOKEN } else { TOKEN_2 };
            Ok(SecretStdout::new(
                format!(
                    r#"{{"access_token":"{token}","token_type":"Bearer","expiry":"2026-11-17T13:00:00Z"}}"#
                )
                .into_bytes(),
            ))
        }
    }

    struct RealRig {
        transport: Arc<ReqwestHttpTransport>,
        r6: Arc<CliTokenBroker>,
        cli: Arc<CountingCli>,
    }

    fn real_rig(server: &Server) -> RealRig {
        let cli = Arc::new(CountingCli::default());
        let pause = Arc::new(SimPause::default());
        let clock: Arc<dyn MonotonicClock> = Arc::new(pause.clock.clone());
        let wall: Arc<dyn WallClock> = Arc::new(SimWallClock::at(T0_MS));
        let h = hosts(&server.origin());
        let r6 = Arc::new(CliTokenBroker::new(
            cli.clone(),
            h.clone(),
            clock.clone(),
            wall,
        ));
        let transport = Arc::new(
            ReqwestHttpTransport::with_seams(TransportSeams {
                hosts: h,
                tokens: r6.clone(),
                policy: Arc::new(LoopbackPolicy),
                bucket: Arc::new(AimdBucket::new(clock)),
                decorator: Arc::new(BottleIdentity::new()),
                pause,
            })
            .unwrap(),
        );
        let weak: Weak<ReqwestHttpTransport> = Arc::downgrade(&transport);
        r6.bind_scim(weak);
        RealRig { transport, r6, cli }
    }

    #[test]
    fn w_05b_w_05c_w_05d_a_401_drops_the_token_runs_the_cli_once_and_retries_once() {
        let s = serve(vec![
            json_reply(401, json!({"error_code": "UNAUTHENTICATED"})),
            json_reply(200, json!({"userName": "a"})),
        ]);
        let r = real_rig(&s);
        let got = r
            .transport
            .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"))
            .unwrap();
        assert_eq!(got, json!({"userName": "a"}));
        assert_eq!(
            r.cli.runs.load(Ordering::SeqCst),
            2,
            "one CLI run after the 401"
        );
        let seen = s.seen();
        assert_eq!(seen.len(), 2, "one retry");
        assert_eq!(
            seen[0].header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(
            seen[1].header("authorization"),
            Some(format!("Bearer {TOKEN_2}").as_str())
        );
    }

    #[test]
    fn w_05d_a_second_401_gives_auth_expired_after_one_retry_only() {
        let s = serve(vec![
            json_reply(401, json!({})),
            json_reply(401, json!({})),
            json_reply(200, json!({})),
        ]);
        let r = real_rig(&s);
        let e = r
            .transport
            .request(PROFILE, "scim.me", &none(), None, &tags("m1", "genie"));
        assert_eq!(code(e), "auth_expired");
        assert_eq!(s.seen().len(), 2, "exactly one retry");
        assert_eq!(r.cli.runs.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn w_06a_w_06b_r7_reads_x_databricks_org_id_for_r6_once_per_profile() {
        let s = serve(vec![
            with_header(
                json_reply(200, json!({"userName": "a"})),
                "x-databricks-org-id",
                "1234",
            ),
            json_reply(200, json!({})),
        ]);
        let r = real_rig(&s);
        assert_eq!(r.r6.workspace_id(PROFILE).unwrap(), "1234");
        assert_eq!(r.r6.workspace_id(PROFILE).unwrap(), "1234");
        let seen = s.seen();
        assert_eq!(seen.len(), 1, "one SCIM call per profile per launch");
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].target, "/api/2.0/preview/scim/v2/Me");
        assert_eq!(
            seen[0].header("authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(
            r.transport.org_id_header(PROFILE).ok().flatten(),
            None,
            "the second reply has no header"
        );
    }
}
