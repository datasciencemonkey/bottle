//! The loopback model proxy of the AgentHost (ARC-7, RTR-23, RTR-28; R17 names the role).
//!
//! The harness sends its model calls to `http://127.0.0.1:<port>/<prefix>/...` with its
//! per-session secret (`x-api-key` or `Authorization: Bearer`). The proxy:
//! 1. refuses a request without a registered secret (401) and never reads a token for it;
//! 2. forwards only `POST` to the model paths of the session's harness (RTR-29c: no Page,
//!    workspace or other Databricks path is reachable through it);
//! 3. drops the inbound credential and every header that is not on a short allowlist;
//! 4. adds the user's token on the outbound hop only, lent by R6 TokenBroker for one call.
//!
//! Gateway targets go to `https://<host>/ai-gateway/anthropic` (Claude) or
//! `https://<host>/ai-gateway/mlflow/v1` (Codex) with the user's identity (RTR-23). A local
//! model target stays on loopback and gets no token. The proxy never logs a request, a
//! response, a secret or a token; its error bodies are fixed strings.
//!
//! TODO(#111): the real outbound [`Upstream`] (HTTP client with the R4 host policy) and
//! streaming (SSE) responses come with SP-19. Until then [`UnavailableUpstream`] answers 502.

use std::fmt;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::adapter::adapter;
use super::{AgentError, Harness};
use crate::token_broker::TokenBroker;

/// The largest request body the proxy forwards.
pub const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_HEAD: usize = 64 * 1024;
/// The read timeout until the secret is checked. It is short, so an unauthenticated client cannot
/// hold a connection for long.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// The read and write timeout of an authenticated model call.
const IO_TIMEOUT: Duration = Duration::from_secs(120);
/// The most connections served at the same time; more get 503 at once.
pub const MAX_CONNECTIONS: usize = 32;
/// The request headers that pass to the outbound hop. All others are dropped, including the
/// inbound `authorization` and `x-api-key` (the session secret).
const FORWARD_HEADERS: [&str; 4] = [
    "content-type",
    "accept",
    "anthropic-version",
    "anthropic-beta",
];

/// The per-session connection secret of the harness (ARC-7). 256 random bits, hex encoded.
/// It is not a Databricks token; it only opens the loopback proxy for one session.
#[derive(Clone, PartialEq, Eq)]
pub struct SessionSecret(String);

impl SessionSecret {
    /// A new random secret from the OS random source.
    pub fn generate() -> Self {
        let mut b = [0u8; 32];
        getrandom::fill(&mut b).expect("the OS random source is available");
        Self(b.iter().map(|x| format!("{x:02x}")).collect())
    }

    /// The secret text, for the session frame and for tests.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SessionSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SessionSecret(<redacted>)")
    }
}

/// Where the model calls of a session go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelTarget {
    /// The Databricks AI Gateway of `host`, with the token of `profile` (the MOG's home).
    Gateway {
        /// The CLI profile whose token the proxy adds. Never a token.
        profile: String,
        /// The workspace host name, for example `example.cloud.databricks.com`.
        host: String,
    },
    /// A local model on loopback, for example `http://127.0.0.1:11434`. No token is added.
    Local {
        /// The loopback base URL.
        base_url: String,
    },
}

/// The model route of one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRoute {
    /// The MOG of the session (for the content-free audit and the usage tags).
    pub mog_id: String,
    /// The harness kind; it decides the proxy prefix and the allowed paths.
    pub harness: Harness,
    /// The target.
    pub target: ModelTarget,
}

/// One outbound model call. `authorization` holds the token bytes lent by R6 for this call
/// only; the implementation sends them as `Authorization: Bearer <token>` and must not keep them.
pub struct UpstreamRequest<'a> {
    /// The HTTP method (always `POST`).
    pub method: &'a str,
    /// The full outbound URL.
    pub url: String,
    /// The forwarded request headers (lowercase names, allowlisted).
    pub headers: Vec<(String, String)>,
    /// The token for the outbound hop; `None` for a local model.
    pub authorization: Option<&'a [u8]>,
    /// The request body.
    pub body: &'a [u8],
    /// The MOG of the session.
    pub mog_id: &'a str,
}

impl fmt::Debug for UpstreamRequest<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UpstreamRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("authorization", &self.authorization.map(|_| "<redacted>"))
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// The answer of the outbound hop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamResponse {
    /// The HTTP status.
    pub status: u16,
    /// The content type, if any.
    pub content_type: Option<String>,
    /// The body.
    pub body: Vec<u8>,
}

/// The outbound hop failed. It carries no detail, so nothing can leak through it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpstreamError;

/// The outbound hop of the model proxy.
pub trait Upstream: Send + Sync {
    /// Send one model call.
    fn forward(&self, req: &UpstreamRequest<'_>) -> Result<UpstreamResponse, UpstreamError>;
}

/// The outbound hop until SP-19 lands: every call fails with 502. TODO(#111).
#[derive(Debug, Default, Clone, Copy)]
pub struct UnavailableUpstream;

impl Upstream for UnavailableUpstream {
    fn forward(&self, _req: &UpstreamRequest<'_>) -> Result<UpstreamResponse, UpstreamError> {
        Err(UpstreamError)
    }
}

struct Shared {
    tokens: Arc<dyn TokenBroker>,
    upstream: Arc<dyn Upstream>,
    routes: Mutex<Vec<(SessionSecret, ModelRoute)>>,
    active: AtomicUsize,
    port: u16,
}

/// The loopback model proxy. It listens on `127.0.0.1` on a free port until it drops.
pub struct ModelProxy {
    addr: SocketAddr,
    shared: Arc<Shared>,
    stop: Arc<AtomicBool>,
}

impl fmt::Debug for ModelProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelProxy")
            .field("addr", &self.addr)
            .finish()
    }
}

/// A gateway host must be a bare DNS name with two or more labels: no IP literal, no port, no
/// path and no single-label name such as `localhost`. TODO(#207): the R4 host allowlist also
/// checks that the host is a Databricks workspace host.
fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !h.starts_with(['.', '-'])
        && !h.ends_with(['.', '-'])
        && h.contains('.')
        && !h.bytes().all(|b| b.is_ascii_digit() || b == b'.')
}

fn valid_local(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    if rest.contains(['?', '#', '@']) {
        return false;
    }
    let authority = rest.split('/').next().unwrap_or("");
    let host = match authority.strip_prefix("[::1]") {
        Some(port) => return port.is_empty() || valid_port(port),
        None => authority.split(':').next().unwrap_or(""),
    };
    let port = &authority[host.len()..];
    matches!(host, "127.0.0.1" | "localhost") && (port.is_empty() || valid_port(port))
}

fn valid_port(p: &str) -> bool {
    p.strip_prefix(':')
        .is_some_and(|d| !d.is_empty() && d.len() <= 5 && d.bytes().all(|b| b.is_ascii_digit()))
}

impl ModelProxy {
    /// Bind `127.0.0.1:0` and serve until the proxy drops.
    pub fn start(
        tokens: Arc<dyn TokenBroker>,
        upstream: Arc<dyn Upstream>,
    ) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared {
            tokens,
            upstream,
            routes: Mutex::new(Vec::new()),
            active: AtomicUsize::new(0),
            port: addr.port(),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name("bottle-model-proxy".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if st.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(mut conn) = conn else { continue };
                    if s.active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        s.active.fetch_sub(1, Ordering::SeqCst);
                        fail(&mut conn, 503, "busy");
                        continue;
                    }
                    let worker = s.clone();
                    let spawned = std::thread::Builder::new()
                        .name("bottle-model-proxy-conn".into())
                        .spawn(move || {
                            serve(&worker, conn);
                            worker.active.fetch_sub(1, Ordering::SeqCst);
                        });
                    if spawned.is_err() {
                        s.active.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })?;
        Ok(Self { addr, shared, stop })
    }

    /// The base URL of the proxy, `http://127.0.0.1:<port>`.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Let `secret` use `route`. A gateway host must be a bare host name; a local model must
    /// be on loopback.
    pub fn register(&self, secret: &SessionSecret, route: ModelRoute) -> Result<(), AgentError> {
        let ok = match &route.target {
            ModelTarget::Gateway { host, .. } => valid_host(host),
            ModelTarget::Local { base_url } => valid_local(base_url),
        };
        if !ok {
            return Err(AgentError::InvalidTarget);
        }
        let mut routes = self.shared.routes.lock().unwrap_or_else(|p| p.into_inner());
        routes.retain(|(s, _)| s != secret);
        routes.push((secret.clone(), route));
        Ok(())
    }

    /// Revoke `secret`. Later requests with it get 401.
    pub fn unregister(&self, secret: &SessionSecret) {
        self.shared
            .routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .retain(|(s, _)| s != secret);
    }

    /// The number of registered sessions.
    pub fn session_count(&self) -> usize {
        self.shared
            .routes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .len()
    }
}

impl Drop for ModelProxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the accept loop so it sees the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(200));
    }
}

/// Constant-time equality, so the secret check leaks no timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

struct Head {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    rest: Vec<u8>,
}

fn find_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn read_head(conn: &mut TcpStream) -> Option<Head> {
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(e) = find_end(&buf) {
            break e;
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        let n = conn.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let text = std::str::from_utf8(&buf[..end]).ok()?;
    let mut lines = text.split("\r\n");
    // No control character (a bare CR or LF included) may hide in a line; tab only in values.
    let bad = |l: &str| l.chars().any(|c| c.is_control() && c != '\t');
    let first_line = lines.next()?;
    if bad(first_line) || first_line.contains('\t') {
        return None;
    }
    let mut first = first_line.split(' ');
    let method = first.next()?.to_owned();
    let path = first.next()?.to_owned();
    first.next()?;
    if first.next().is_some() {
        return None;
    }
    let mut headers = Vec::new();
    for l in lines {
        // Folded (obsolete line-continuation) headers are refused.
        if bad(l) || l.starts_with([' ', '\t']) {
            return None;
        }
        let (k, v) = l.split_once(':')?;
        if k.is_empty() || k.contains([' ', '\t']) {
            return None;
        }
        headers.push((k.to_ascii_lowercase(), v.trim().to_owned()));
    }
    Some(Head {
        method,
        path,
        headers,
        rest: buf[end + 4..].to_vec(),
    })
}

fn count(h: &Head, name: &str) -> usize {
    h.headers.iter().filter(|(k, _)| k == name).count()
}

fn header<'a>(h: &'a Head, name: &str) -> Option<&'a str> {
    h.headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        403 => "Forbidden",
        411 => "Length Required",
        413 => "Payload Too Large",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Status",
    }
}

fn respond(conn: &mut TcpStream, status: u16, content_type: Option<&str>, body: &[u8]) {
    let mut head = format!("HTTP/1.1 {status} {}\r\n", reason(status));
    if let Some(ct) = content_type.filter(|c| !c.contains(['\r', '\n'])) {
        head.push_str(&format!("Content-Type: {ct}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    ));
    let _ = conn.write_all(head.as_bytes());
    let _ = conn.write_all(body);
    let _ = conn.flush();
    let _ = conn.shutdown(Shutdown::Write);
    // Drain what the client still sends, so closing does not reset the connection.
    let _ = conn.set_read_timeout(Some(Duration::from_millis(200)));
    let mut sink = [0u8; 4096];
    let mut left = MAX_HEAD;
    while let Ok(n) = conn.read(&mut sink) {
        if n == 0 || n >= left {
            break;
        }
        left -= n;
    }
}

fn fail(conn: &mut TcpStream, status: u16, code: &str) {
    let body = format!("{{\"error\":\"{code}\"}}");
    respond(conn, status, Some("application/json"), body.as_bytes());
}

fn secret_of(h: &Head) -> Option<&str> {
    header(h, "x-api-key").or_else(|| {
        header(h, "authorization").and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
    })
}

fn serve(shared: &Shared, mut conn: TcpStream) {
    let _ = conn.set_read_timeout(Some(HEAD_TIMEOUT));
    let _ = conn.set_write_timeout(Some(HEAD_TIMEOUT));
    let Some(head) = read_head(&mut conn) else {
        return fail(&mut conn, 400, "bad_request");
    };
    // One value for each credential and length header: no ambiguity between two parsers.
    if [
        "x-api-key",
        "authorization",
        "content-length",
        "transfer-encoding",
        "host",
    ]
    .iter()
    .any(|n| count(&head, n) > 1)
    {
        return fail(&mut conn, 400, "bad_request");
    }

    // 0. The Host header must name this loopback listener (DNS rebinding).
    let host_ok = header(&head, "host").is_some_and(|h| {
        let port = shared.port.to_string();
        h.strip_prefix("127.0.0.1:")
            .or_else(|| h.strip_prefix("localhost:"))
            .is_some_and(|p| p == port)
    });
    if !host_ok {
        return fail(&mut conn, 403, "host_not_allowed");
    }

    // 1. The session secret.
    let route = secret_of(&head).and_then(|presented| {
        let routes = shared.routes.lock().unwrap_or_else(|p| p.into_inner());
        let mut hit = None;
        for (s, r) in routes.iter() {
            if ct_eq(s.as_str().as_bytes(), presented.as_bytes()) {
                hit = Some(r.clone());
            }
        }
        hit
    });
    let Some(route) = route else {
        return fail(&mut conn, 401, "unauthorized");
    };
    let _ = conn.set_read_timeout(Some(IO_TIMEOUT));
    let _ = conn.set_write_timeout(Some(IO_TIMEOUT));

    // 2. Method and path allowlist of the session's harness.
    if head.method != "POST" {
        return fail(&mut conn, 405, "method_not_allowed");
    }
    let a = adapter(route.harness);
    let rest = head
        .path
        .strip_prefix(a.proxy_prefix())
        .filter(|r| a.model_paths().contains(r));
    let Some(rest) = rest else {
        return fail(&mut conn, 404, "route_not_allowed");
    };

    // 3. The body: Content-Length only, with a cap.
    if header(&head, "transfer-encoding").is_some() {
        return fail(&mut conn, 411, "length_required");
    }
    let Some(len) = header(&head, "content-length").and_then(|v| v.parse::<usize>().ok()) else {
        return fail(&mut conn, 411, "length_required");
    };
    if len > MAX_BODY {
        return fail(&mut conn, 413, "body_too_large");
    }
    let mut body = head.rest.clone();
    if body.len() < len {
        let mut more = vec![0u8; len - body.len()];
        if conn.read_exact(&mut more).is_err() {
            return fail(&mut conn, 400, "bad_request");
        }
        body.extend_from_slice(&more);
    }
    body.truncate(len);

    let headers: Vec<(String, String)> = head
        .headers
        .iter()
        .filter(|(k, _)| FORWARD_HEADERS.contains(&k.as_str()))
        .cloned()
        .collect();

    // 4. The outbound hop; the token is added here and only here.
    let result = match &route.target {
        ModelTarget::Local { base_url } => {
            let req = UpstreamRequest {
                method: "POST",
                url: format!("{}{rest}", base_url.trim_end_matches('/')),
                headers,
                authorization: None,
                body: &body,
                mog_id: &route.mog_id,
            };
            Some(shared.upstream.forward(&req))
        }
        ModelTarget::Gateway { profile, host } => {
            let url = format!("https://{host}{}{rest}", a.gateway_route());
            let mut out = None;
            let lent = shared.tokens.with_token(profile, &mut |token| {
                let req = UpstreamRequest {
                    method: "POST",
                    url: url.clone(),
                    headers: headers.clone(),
                    authorization: Some(token),
                    body: &body,
                    mog_id: &route.mog_id,
                };
                out = Some(shared.upstream.forward(&req));
            });
            if lent.is_err() {
                return fail(&mut conn, 502, "token_unavailable");
            }
            out
        }
    };
    match result {
        Some(Ok(r)) => respond(&mut conn, r.status, r.content_type.as_deref(), &r.body),
        _ => fail(&mut conn, 502, "upstream_unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_7_constant_time_compare() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab"));
    }

    #[test]
    fn rtr_23_local_targets_are_loopback_only() {
        for ok in [
            "http://127.0.0.1:11434",
            "http://localhost:8080/v1",
            "http://[::1]:9000",
            "http://127.0.0.1",
        ] {
            assert!(valid_local(ok), "{ok}");
        }
        for bad in [
            "https://127.0.0.1:1",
            "http://127.0.0.1.evil.com",
            "http://localhost@evil.com",
            "http://evil.com",
            "http://127.0.0.1:99999999",
            "http://127.0.0.1:1/?x",
            "http://[::1]:9/x?y@evil",
            "http://[::1]:9/x#y",
        ] {
            assert!(!valid_local(bad), "{bad}");
        }
    }
}
