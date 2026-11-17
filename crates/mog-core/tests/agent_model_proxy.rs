//! The loopback model proxy of R16 AgentHost (#112, RTR-23, ARC-7, RTR-28).
//!
//! The harness talks to the proxy on `127.0.0.1` with a per-session secret. The proxy checks
//! the secret, allows only the model paths, drops the inbound credential and adds the token on
//! the outbound hop only. The outbound hop is a fake upstream: no test reaches a real model.
//! TODO(#111): the live SP-19 run checks the same route against the AI Gateway.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use mog_core::agent_host::model_proxy::{
    ModelProxy, ModelRoute, ModelTarget, SessionSecret, Upstream, UpstreamError, UpstreamRequest,
    UpstreamResponse,
};
use mog_core::agent_host::{AgentError, Harness};
use mog_core::error::CoreResult;
use mog_core::token_broker::TokenBroker;

const FAKE_TOKEN: &str = "dapi-FAKE-TOKEN-0123456789abcdef";
const HOST: &str = "example.cloud.databricks.com";

struct FakeTokens {
    profiles: Mutex<Vec<String>>,
}

impl TokenBroker for FakeTokens {
    fn with_token(&self, profile: &str, _host: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<()> {
        self.profiles.lock().unwrap().push(profile.to_owned());
        f(FAKE_TOKEN.as_bytes());
        Ok(())
    }
    fn invalidate(&self, _profile: &str) -> CoreResult<()> {
        Ok(())
    }
    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        Ok("123".into())
    }
}

/// What the fake upstream saw. The test copies the token only to compare it.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    url: String,
    headers: Vec<(String, String)>,
    authorization: Option<String>,
    body: Vec<u8>,
    mog_id: String,
}

#[derive(Default)]
struct FakeUpstream {
    seen: Mutex<Vec<Seen>>,
    calls: AtomicUsize,
}

impl Upstream for FakeUpstream {
    fn forward(&self, req: &UpstreamRequest<'_>) -> Result<UpstreamResponse, UpstreamError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(Seen {
            method: req.method.to_owned(),
            url: req.url.clone(),
            headers: req.headers.clone(),
            authorization: req
                .authorization
                .map(|b| String::from_utf8(b.to_vec()).unwrap()),
            body: req.body.to_vec(),
            mog_id: req.mog_id.to_owned(),
        });
        Ok(UpstreamResponse {
            status: 200,
            content_type: Some("application/json".into()),
            body: br#"{"ok":true}"#.to_vec(),
        })
    }
}

struct Rig {
    proxy: ModelProxy,
    tokens: Arc<FakeTokens>,
    upstream: Arc<FakeUpstream>,
}

fn rig() -> Rig {
    let tokens = Arc::new(FakeTokens {
        profiles: Mutex::new(Vec::new()),
    });
    let upstream = Arc::new(FakeUpstream::default());
    let proxy = ModelProxy::start(tokens.clone(), upstream.clone()).expect("binds loopback");
    Rig {
        proxy,
        tokens,
        upstream,
    }
}

fn gateway(mog: &str, h: Harness) -> ModelRoute {
    ModelRoute {
        mog_id: mog.into(),
        harness: h,
        target: ModelTarget::Gateway {
            profile: "bottle-home".into(),
            host: HOST.into(),
        },
    }
}

/// Send one raw HTTP/1.1 request; return (status, body).
fn raw(proxy: &ModelProxy, request: &[u8]) -> (u16, String) {
    let addr = proxy.base_url().trim_start_matches("http://").to_owned();
    let mut s = TcpStream::connect(addr).unwrap();
    s.write_all(request).unwrap();
    let mut out = Vec::new();
    s.read_to_end(&mut out).unwrap();
    let text = String::from_utf8_lossy(&out).to_string();
    let status: u16 = text
        .split(' ')
        .nth(1)
        .and_then(|c| c.parse().ok())
        .expect("status line");
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
    (status, body)
}

/// The Host header value of the proxy listener.
fn host_of(proxy: &ModelProxy) -> String {
    proxy.base_url().trim_start_matches("http://").to_owned()
}

fn post(proxy: &ModelProxy, path: &str, headers: &[(&str, &str)], body: &str) -> (u16, String) {
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: {}\r\n", host_of(proxy));
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    raw(proxy, req.as_bytes())
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

// ---------- RTR-23: routes through the AI Gateway with the user's token ----------

#[test]
fn rtr_23_claude_route_goes_to_ai_gateway_anthropic_with_the_token() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    let (status, body) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[
            ("x-api-key", s.as_str()),
            ("content-type", "application/json"),
            ("anthropic-version", "2023-06-01"),
        ],
        r#"{"model":"m","messages":[]}"#,
    );
    assert_eq!(status, 200);
    assert_eq!(body, r#"{"ok":true}"#);
    let seen = r.upstream.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.method, "POST");
    assert_eq!(
        seen.url,
        format!("https://{HOST}/ai-gateway/anthropic/v1/messages")
    );
    assert_eq!(seen.authorization.as_deref(), Some(FAKE_TOKEN));
    assert_eq!(seen.body, br#"{"model":"m","messages":[]}"#);
    assert_eq!(seen.mog_id, "mog-a");
    assert_eq!(header(&seen, "anthropic-version"), Some("2023-06-01"));
    assert_eq!(header(&seen, "content-type"), Some("application/json"));
    assert_eq!(
        header(&seen, "x-api-key"),
        None,
        "the session secret stays local"
    );
    assert_eq!(
        header(&seen, "authorization"),
        None,
        "auth is only the token field"
    );
    assert_eq!(*r.tokens.profiles.lock().unwrap(), vec!["bottle-home"]);
}

#[test]
fn rtr_23_codex_route_goes_to_ai_gateway_mlflow_v1_with_the_token() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-b", Harness::Codex))
        .unwrap();
    let bearer = format!("Bearer {}", s.as_str());
    let (status, _) = post(
        &r.proxy,
        "/openai/v1/chat/completions",
        &[
            ("Authorization", &bearer),
            ("content-type", "application/json"),
        ],
        "{}",
    );
    assert_eq!(status, 200);
    let seen = r.upstream.seen.lock().unwrap()[0].clone();
    assert_eq!(
        seen.url,
        format!("https://{HOST}/ai-gateway/mlflow/v1/chat/completions")
    );
    assert_eq!(seen.authorization.as_deref(), Some(FAKE_TOKEN));
    assert_eq!(header(&seen, "authorization"), None);
}

#[test]
fn rtr_23_local_model_route_gets_no_token() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(
            &s,
            ModelRoute {
                mog_id: "mog-l".into(),
                harness: Harness::Claude,
                target: ModelTarget::Local {
                    base_url: "http://127.0.0.1:11434".into(),
                },
            },
        )
        .unwrap();
    let (status, _) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str())],
        "{}",
    );
    assert_eq!(status, 200);
    let seen = r.upstream.seen.lock().unwrap()[0].clone();
    assert_eq!(seen.url, "http://127.0.0.1:11434/v1/messages");
    assert_eq!(seen.authorization, None);
    assert!(r.tokens.profiles.lock().unwrap().is_empty());
}

#[test]
fn rtr_23_local_model_must_be_on_loopback() {
    let r = rig();
    let s = SessionSecret::generate();
    let e = r
        .proxy
        .register(
            &s,
            ModelRoute {
                mog_id: "m".into(),
                harness: Harness::Claude,
                target: ModelTarget::Local {
                    base_url: "http://model.example.com:8080".into(),
                },
            },
        )
        .unwrap_err();
    assert_eq!(e, AgentError::InvalidTarget);
}

#[test]
fn rtr_23_gateway_host_must_be_a_bare_host_name() {
    let r = rig();
    for bad in [
        "evil.com/x",
        "a@b.com",
        "h:443",
        "",
        "h h",
        "evil.com?x=1",
        "169.254.169.254",
        "10.0.0.1",
        "localhost",
        "[::1]",
    ] {
        let s = SessionSecret::generate();
        let route = ModelRoute {
            mog_id: "m".into(),
            harness: Harness::Claude,
            target: ModelTarget::Gateway {
                profile: "p".into(),
                host: bad.into(),
            },
        };
        assert_eq!(
            r.proxy.register(&s, route).unwrap_err(),
            AgentError::InvalidTarget,
            "{bad}"
        );
    }
}

#[test]
fn rtr_23_proxy_listens_on_loopback_only() {
    let r = rig();
    assert!(r.proxy.base_url().starts_with("http://127.0.0.1:"));
}

// ---------- ARC-7 / RTR-28: the secret gate and the path allowlist ----------

#[test]
fn arc_7_rtr_28_unknown_secret_is_refused_and_never_forwarded() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    for headers in [
        vec![("x-api-key", "wrong")],
        vec![("Authorization", "Bearer wrong")],
        vec![],
    ] {
        let (status, body) = post(&r.proxy, "/anthropic/v1/messages", &headers, "{}");
        assert_eq!(status, 401);
        assert!(!body.contains(FAKE_TOKEN));
    }
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
    assert!(
        r.tokens.profiles.lock().unwrap().is_empty(),
        "no token is read"
    );
}

#[test]
fn arc_7_unregistered_secret_is_refused() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    r.proxy.unregister(&s);
    let (status, _) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str())],
        "{}",
    );
    assert_eq!(status, 401);
    assert_eq!(r.proxy.session_count(), 0);
}

#[test]
fn arc_7_rtr_29c_only_model_paths_are_forwarded() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    for path in [
        "/anthropic/api/2.0/knowledge/pages",
        "/anthropic/v1/messages/../../api/2.0/knowledge/pages",
        "/anthropic/v1/files",
        "/openai/v1/chat/completions", // a Claude session may not use the Codex route
        "/api/2.0/knowledge/pages",
        "/anthropic/v1/messages?x=https://evil.example",
    ] {
        let (status, _) = post(&r.proxy, path, &[("x-api-key", s.as_str())], "{}");
        assert_eq!(status, 404, "{path}");
    }
    let get = format!(
        "GET /anthropic/v1/messages HTTP/1.1\r\nHost: {}\r\nx-api-key: {}\r\n\r\n",
        host_of(&r.proxy),
        s.as_str()
    );
    assert_eq!(raw(&r.proxy, get.as_bytes()).0, 405);
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_chunked_and_oversize_bodies_are_refused() {
    let r = rig();
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    let chunked = format!(
        "POST /anthropic/v1/messages HTTP/1.1\r\nHost: {}\r\nx-api-key: {}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        host_of(&r.proxy),
        s.as_str()
    );
    assert_eq!(raw(&r.proxy, chunked.as_bytes()).0, 411);
    let huge = format!(
        "POST /anthropic/v1/messages HTTP/1.1\r\nHost: {}\r\nx-api-key: {}\r\nContent-Length: 999999999\r\n\r\n",
        host_of(&r.proxy),
        s.as_str()
    );
    assert_eq!(raw(&r.proxy, huge.as_bytes()).0, 413);
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_upstream_failure_gives_a_fixed_content_free_error() {
    struct Down;
    impl Upstream for Down {
        fn forward(&self, _r: &UpstreamRequest<'_>) -> Result<UpstreamResponse, UpstreamError> {
            Err(UpstreamError)
        }
    }
    let tokens = Arc::new(FakeTokens {
        profiles: Mutex::new(Vec::new()),
    });
    let proxy = ModelProxy::start(tokens, Arc::new(Down)).unwrap();
    let s = SessionSecret::generate();
    proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    let (status, body) = post(
        &proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str())],
        r#"{"secret question":"revenue"}"#,
    );
    assert_eq!(status, 502);
    assert!(!body.contains("revenue") && !body.contains(FAKE_TOKEN));
}

#[test]
fn arc_7_session_secret_debug_is_redacted_and_secrets_differ() {
    let a = SessionSecret::generate();
    let b = SessionSecret::generate();
    assert_ne!(a.as_str(), b.as_str());
    assert!(a.as_str().len() >= 43, "at least 256 bits");
    assert!(!format!("{a:?}").contains(a.as_str()));
}

// ---------- ARC-7: strict request parsing (review of #309) ----------

/// A registered Claude session and a raw request builder with the right Host header.
fn claude_session(r: &Rig) -> SessionSecret {
    let s = SessionSecret::generate();
    r.proxy
        .register(&s, gateway("mog-a", Harness::Claude))
        .unwrap();
    s
}

#[test]
fn arc_7_bare_line_feed_or_control_char_in_a_header_is_refused() {
    let r = rig();
    let s = claude_session(&r);
    for evil in [
        "content-type: application/json\nx-injected: 1",
        "content-type: application/json\rx-injected: 1",
        "anthropic-beta: a\u{0}b",
    ] {
        let req = format!(
            "POST /anthropic/v1/messages HTTP/1.1\r\nHost: {}\r\nx-api-key: {}\r\n{evil}\r\nContent-Length: 2\r\n\r\n{{}}",
            host_of(&r.proxy),
            s.as_str()
        );
        assert_eq!(raw(&r.proxy, req.as_bytes()).0, 400, "{evil:?}");
    }
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_folded_header_is_refused() {
    let r = rig();
    let s = claude_session(&r);
    let req = format!(
        "POST /anthropic/v1/messages HTTP/1.1\r\nHost: {}\r\nx-api-key: {}\r\nanthropic-beta: a\r\n x-injected: 1\r\nContent-Length: 2\r\n\r\n{{}}",
        host_of(&r.proxy),
        s.as_str()
    );
    assert_eq!(raw(&r.proxy, req.as_bytes()).0, 400);
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_duplicate_length_or_credential_headers_are_refused() {
    let r = rig();
    let s = claude_session(&r);
    let (status, _) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str()), ("Content-Length", "2")],
        "{}",
    );
    assert_eq!(status, 400, "two Content-Length headers");
    let (status, _) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str()), ("x-api-key", "other")],
        "{}",
    );
    assert_eq!(status, 400, "two x-api-key headers");
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_transfer_encoding_with_content_length_is_refused() {
    let r = rig();
    let s = claude_session(&r);
    let (status, _) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str()), ("Transfer-Encoding", "chunked")],
        "{}",
    );
    assert_eq!(status, 411);
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn arc_7_host_header_must_name_the_loopback_listener() {
    let r = rig();
    let s = claude_session(&r);
    let port = host_of(&r.proxy).rsplit(':').next().unwrap().to_owned();
    for host in [
        Some("evil.example".to_owned()),
        Some(format!("evil.example:{port}")),
        Some("127.0.0.1:1".to_owned()),
        None,
    ] {
        let mut req = String::from("POST /anthropic/v1/messages HTTP/1.1\r\n");
        if let Some(h) = &host {
            req.push_str(&format!("Host: {h}\r\n"));
        }
        req.push_str(&format!(
            "x-api-key: {}\r\nContent-Length: 2\r\n\r\n{{}}",
            s.as_str()
        ));
        assert_eq!(raw(&r.proxy, req.as_bytes()).0, 403, "{host:?}");
    }
    let localhost = format!("localhost:{port}");
    let req = format!(
        "POST /anthropic/v1/messages HTTP/1.1\r\nHost: {localhost}\r\nx-api-key: {}\r\nContent-Length: 2\r\n\r\n{{}}",
        s.as_str()
    );
    assert_eq!(raw(&r.proxy, req.as_bytes()).0, 200);
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn arc_7_connection_cap_refuses_extra_connections_at_once() {
    let r = rig();
    let s = claude_session(&r);
    let addr = host_of(&r.proxy);
    let idle: Vec<TcpStream> = (0..mog_core::agent_host::model_proxy::MAX_CONNECTIONS)
        .map(|_| TcpStream::connect(&addr).unwrap())
        .collect();
    std::thread::sleep(std::time::Duration::from_millis(200));
    let t = std::time::Instant::now();
    let (status, _) = post(
        &r.proxy,
        "/anthropic/v1/messages",
        &[("x-api-key", s.as_str())],
        "{}",
    );
    assert_eq!(status, 503);
    assert!(t.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(r.upstream.calls.load(Ordering::SeqCst), 0);
    drop(idle);
    let mut ok = false;
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(50));
        let (status, _) = post(
            &r.proxy,
            "/anthropic/v1/messages",
            &[("x-api-key", s.as_str())],
            "{}",
        );
        if status == 200 {
            ok = true;
            break;
        }
    }
    assert!(ok, "slots free up when the idle clients leave");
}

#[test]
fn arc_7_whole_head_deadline_stops_a_client_that_trickles_bytes() {
    let tokens = Arc::new(FakeTokens {
        profiles: Mutex::new(Vec::new()),
    });
    let proxy = ModelProxy::start_with_head_timeout(
        tokens,
        Arc::new(FakeUpstream::default()),
        std::time::Duration::from_millis(300),
    )
    .unwrap();
    let mut c = TcpStream::connect(host_of(&proxy)).unwrap();
    c.set_read_timeout(Some(std::time::Duration::from_secs(5)))
        .unwrap();
    let t = std::time::Instant::now();
    // One byte each 50 ms: each read is fast, but the whole head never ends in time.
    let writer = {
        let mut w = c.try_clone().unwrap();
        std::thread::spawn(move || {
            for b in b"POST /anthropic/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nx-a: ".iter() {
                if w.write_all(&[*b]).is_err() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        })
    };
    let mut out = Vec::new();
    let _ = c.read_to_end(&mut out);
    let elapsed = t.elapsed();
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 400"));
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "closed after the head deadline, not per read ({elapsed:?})"
    );
    let _ = writer.join();
}
