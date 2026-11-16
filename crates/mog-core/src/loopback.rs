//! Loopback endpoints for the agent harness (#224, ARC-7, 4.6 rule 9, COL-78).
//!
//! One [`LoopbackSession`] per agent session opens three listeners on `127.0.0.1`:
//! `bottle-mcp` (R17), the Genie One MCP proxy (R8) and the model proxy (R17). The session has
//! one random secret. Every request must carry `Authorization: Bearer <secret>` and a `Host`
//! of `127.0.0.1:<port>` or `localhost:<port>`; otherwise the listener answers 401 or 403 and
//! the handler never runs. The harness gets the URLs and the secret, never a token: the real
//! handlers add the token inside the core. Reply bodies go through the rule-7 scrubber.
//!
//! This is a skeleton. It speaks plain HTTP/1.1 with a `Content-Length` body, one request per
//! connection.
//! TODO(#111): the SP-19 wire details (MCP streamable HTTP and SSE, paths, how the harness
//! passes the header). TODO(#113): the `bottle-mcp` and model proxy handlers.
//! TODO(#139): the Genie One MCP proxy handler.

use std::collections::HashMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::error::{CoreError, CoreResult};
use crate::token_guard::scrub_json;

const ID: &str = "R16";
/// The reason code of [`CoreError::Unavailable`] when a listener cannot open.
pub const UNAVAILABLE: &str = "loopback_unavailable";
const MAX_HEAD: usize = 16 * 1024;
const MAX_BODY: usize = 1024 * 1024;
/// The whole request (head and body) must arrive within this time, or the connection closes.
const REQUEST_DEADLINE: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// The most open connections per endpoint. Further connections to that endpoint close at once.
/// A local process can still fill one endpoint before auth for up to [`REQUEST_DEADLINE`] at a
/// time; TODO(#111): revisit the limits with the SP-19 traffic numbers.
pub const MAX_CONNECTIONS: usize = 32;

/// The three loopback endpoints of ARC-7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopbackEndpoint {
    /// R17 `bottle-mcp`.
    BottleMcp,
    /// R8 Genie One MCP proxy.
    GenieOneMcp,
    /// R17 model proxy to `/ai-gateway/…`.
    ModelProxy,
}

impl LoopbackEndpoint {
    /// All endpoints, in a fixed order.
    pub const ALL: [LoopbackEndpoint; 3] = [
        LoopbackEndpoint::BottleMcp,
        LoopbackEndpoint::GenieOneMcp,
        LoopbackEndpoint::ModelProxy,
    ];

    /// The stable name of the endpoint.
    pub fn name(&self) -> &'static str {
        match self {
            LoopbackEndpoint::BottleMcp => "bottle-mcp",
            LoopbackEndpoint::GenieOneMcp => "genie-one-mcp",
            LoopbackEndpoint::ModelProxy => "model-proxy",
        }
    }
}

/// An authorized request from the harness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopbackRequest {
    /// The endpoint that took the request.
    pub endpoint: LoopbackEndpoint,
    /// The HTTP method.
    pub method: String,
    /// The request path.
    pub path: String,
    /// The request body.
    pub body: Vec<u8>,
}

/// The reply of a handler. The listener scrubs `body` before it sends it (4.6 rule 7).
#[derive(Debug, Clone, PartialEq)]
pub struct LoopbackResponse {
    /// The HTTP status.
    pub status: u16,
    /// The JSON body.
    pub body: Value,
}

/// Serves the authorized requests of one session.
pub trait LoopbackHandler: Send + Sync {
    /// Answer one request.
    fn handle(&self, req: &LoopbackRequest) -> LoopbackResponse;
}

/// The default handler: 501 for every endpoint (TODO(#113), TODO(#139)).
#[derive(Debug, Default, Clone, Copy)]
pub struct NotImplementedHandler;

impl LoopbackHandler for NotImplementedHandler {
    fn handle(&self, req: &LoopbackRequest) -> LoopbackResponse {
        LoopbackResponse {
            status: 501,
            body: json!({"error": "not_implemented", "endpoint": req.endpoint.name()}),
        }
    }
}

/// Bytes that are overwritten with zeros on drop.
struct Wiped(Vec<u8>);

impl Wiped {
    fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("")
    }
}

impl Drop for Wiped {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            *b = 0;
        }
        std::sync::atomic::compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut self.0);
    }
}

/// A new `Authorization` value with a 256-bit secret from the OS RNG, as lowercase hex.
fn new_authorization() -> CoreResult<Wiped> {
    let mut raw = [0u8; 32];
    getrandom::fill(&mut raw).map_err(|_| CoreError::Unavailable {
        component: ID,
        reason: UNAVAILABLE,
    })?;
    let mut v = Vec::with_capacity(7 + 64);
    v.extend_from_slice(b"Bearer ");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for b in raw.iter() {
        v.push(HEX[usize::from(b >> 4)]);
        v.push(HEX[usize::from(b & 0x0f)]);
    }
    raw.fill(0);
    Ok(Wiped(v))
}

/// Compare in time that does not depend on where the bytes differ.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// What the harness gets: the endpoint URLs and the `Authorization` value. No token.
pub struct HarnessConfig {
    endpoints: Vec<(LoopbackEndpoint, String)>,
    authorization: Arc<Wiped>,
}

impl HarnessConfig {
    /// The base URL of each endpoint.
    pub fn endpoints(&self) -> &[(LoopbackEndpoint, String)] {
        &self.endpoints
    }

    /// The `Authorization` header value: `Bearer <session secret>`.
    pub fn authorization(&self) -> &str {
        self.authorization.as_str()
    }
}

impl fmt::Debug for HarnessConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HarnessConfig")
            .field("endpoints", &self.endpoints)
            .field("authorization", &"<redacted>")
            .finish()
    }
}

#[derive(Default)]
struct Conns {
    next_id: u64,
    open: HashMap<u64, (LoopbackEndpoint, TcpStream)>,
    threads: Vec<JoinHandle<()>>,
}

struct Shared {
    authorization: Arc<Wiped>,
    handler: Arc<dyn LoopbackHandler>,
    stop: AtomicBool,
    conns: Mutex<Conns>,
}

impl Shared {
    fn conns(&self) -> MutexGuard<'_, Conns> {
        self.conns.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The three loopback listeners of one agent session.
pub struct LoopbackSession {
    shared: Arc<Shared>,
    addrs: Vec<(LoopbackEndpoint, SocketAddr)>,
    threads: Vec<JoinHandle<()>>,
}

impl fmt::Debug for LoopbackSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LoopbackSession")
            .field("addrs", &self.addrs)
            .finish_non_exhaustive()
    }
}

impl LoopbackSession {
    /// Bind the three endpoints on `127.0.0.1` with new ports and a new session secret.
    pub fn open(handler: Arc<dyn LoopbackHandler>) -> CoreResult<Self> {
        let shared = Arc::new(Shared {
            authorization: Arc::new(new_authorization()?),
            handler,
            stop: AtomicBool::new(false),
            conns: Mutex::new(Conns::default()),
        });
        let mut session = LoopbackSession {
            shared,
            addrs: Vec::new(),
            threads: Vec::new(),
        };
        for endpoint in LoopbackEndpoint::ALL {
            let unavailable = |_| CoreError::Unavailable {
                component: ID,
                reason: UNAVAILABLE,
            };
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(unavailable)?;
            let addr = listener.local_addr().map_err(unavailable)?;
            let shared = Arc::clone(&session.shared);
            let thread = std::thread::Builder::new()
                .name(format!("loopback-{}", endpoint.name()))
                .spawn(move || accept_loop(listener, endpoint, addr.port(), shared))
                .map_err(unavailable)?;
            session.addrs.push((endpoint, addr));
            session.threads.push(thread);
        }
        Ok(session)
    }

    /// The address of `endpoint`.
    pub fn addr(&self, endpoint: LoopbackEndpoint) -> SocketAddr {
        self.addrs
            .iter()
            .find(|(e, _)| *e == endpoint)
            .map(|(_, a)| *a)
            .expect("open binds every endpoint")
    }

    /// The URLs and the secret for the harness of this session.
    pub fn harness_config(&self) -> HarnessConfig {
        HarnessConfig {
            endpoints: self
                .addrs
                .iter()
                .map(|(e, a)| (*e, format!("http://127.0.0.1:{}", a.port())))
                .collect(),
            authorization: Arc::clone(&self.shared.authorization),
        }
    }

    /// The number of open connections (at most [`MAX_CONNECTIONS`] per endpoint).
    pub fn open_connections(&self) -> usize {
        self.shared.conns().open.len()
    }

    /// Stop the listeners, close every open connection and wait for their threads.
    pub fn close(self) {}

    fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        for (_, addr) in &self.addrs {
            // Wake the blocking accept so the thread sees `stop`.
            let _ = TcpStream::connect_timeout(addr, Duration::from_millis(200));
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
        // No accept runs now, so no connection is added after this point.
        let (streams, threads) = {
            let mut c = self.shared.conns();
            (std::mem::take(&mut c.open), std::mem::take(&mut c.threads))
        };
        for (_, s) in streams.values() {
            let _ = s.shutdown(Shutdown::Both);
        }
        for t in threads {
            let _ = t.join();
        }
    }
}

impl Drop for LoopbackSession {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn accept_loop(listener: TcpListener, endpoint: LoopbackEndpoint, port: u16, shared: Arc<Shared>) {
    for stream in listener.incoming() {
        if shared.stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(stream) = stream else { continue };
        let mut c = shared.conns();
        c.threads.retain(|t| !t.is_finished());
        if c.open.values().filter(|(e, _)| *e == endpoint).count() >= MAX_CONNECTIONS {
            // Over the cap: close at once (dropping the stream), before any read.
            continue;
        }
        let Ok(handle) = stream.try_clone() else {
            continue;
        };
        let id = c.next_id;
        c.next_id += 1;
        c.open.insert(id, (endpoint, handle));
        let conn_shared = Arc::clone(&shared);
        // One short-lived thread per connection, so a slow client cannot block the endpoint.
        // The cap and the request deadline bound the threads; `close` joins them.
        let spawned = std::thread::Builder::new()
            .name("loopback-conn".into())
            .spawn(move || {
                // The guard frees the slot even if `serve` panics.
                let _guard = ConnGuard {
                    shared: Arc::clone(&conn_shared),
                    id,
                };
                serve(stream, endpoint, port, &conn_shared);
            });
        match spawned {
            Ok(t) => c.threads.push(t),
            Err(_) => {
                c.open.remove(&id);
            }
        }
    }
}

/// Removes a connection from the open list and shuts its socket when the thread ends.
struct ConnGuard {
    shared: Arc<Shared>,
    id: u64,
}

impl Drop for ConnGuard {
    fn drop(&mut self) {
        if let Some((_, s)) = self.shared.conns().open.remove(&self.id) {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        _ => "Status",
    }
}

fn reply(stream: &mut TcpStream, status: u16, body: &Value) {
    let body = scrub_json(body).to_string();
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

fn error(stream: &mut TcpStream, status: u16, code: &str) {
    reply(stream, status, &json!({ "error": code }));
}

/// Read some bytes before `deadline`. `Err(408)` when the time is up, `Err(400)` on EOF.
fn read_some(stream: &mut TcpStream, buf: &mut [u8], deadline: Instant) -> Result<usize, u16> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(408);
    }
    let _ = stream.set_read_timeout(Some(left));
    match stream.read(buf) {
        Ok(0) => Err(400),
        Ok(n) => Ok(n),
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Err(408)
        }
        Err(_) => Err(400),
    }
}

/// Read the request head (up to the blank line) and any bytes after it.
fn read_head(stream: &mut TcpStream, deadline: Instant) -> Result<(Vec<u8>, Vec<u8>), u16> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    loop {
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(p + 4);
            return Ok((buf, rest));
        }
        if buf.len() > MAX_HEAD {
            return Err(431);
        }
        let n = read_some(stream, &mut chunk, deadline)?;
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn serve(mut stream: TcpStream, endpoint: LoopbackEndpoint, port: u16, shared: &Shared) {
    let deadline = Instant::now() + REQUEST_DEADLINE;
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    if shared.stop.load(Ordering::SeqCst) {
        return;
    }
    let (head, mut body) = match read_head(&mut stream, deadline) {
        Ok(x) => x,
        Err(408) => return error(&mut stream, 408, "timeout"),
        Err(status) => return error(&mut stream, status, "bad_request"),
    };
    let Ok(head) = std::str::from_utf8(&head) else {
        return error(&mut stream, 400, "bad_request");
    };
    let mut lines = head.split("\r\n");
    let mut parts = lines.next().unwrap_or("").split(' ');
    let (Some(method), Some(path), Some(version)) = (parts.next(), parts.next(), parts.next())
    else {
        return error(&mut stream, 400, "bad_request");
    };
    if !version.starts_with("HTTP/1.") {
        return error(&mut stream, 400, "bad_request");
    }
    let mut host = Vec::new();
    let mut auth = Vec::new();
    let mut length: Option<usize> = None;
    let mut chunked = false;
    for line in lines.filter(|l| !l.is_empty()) {
        let Some((name, value)) = line.split_once(':') else {
            return error(&mut stream, 400, "bad_request");
        };
        let value = value.trim();
        match name.trim().to_ascii_lowercase().as_str() {
            "host" => host.push(value),
            "authorization" => auth.push(value),
            "content-length" => match value.parse() {
                Ok(n) if length.is_none() => length = Some(n),
                _ => return error(&mut stream, 400, "bad_request"),
            },
            "transfer-encoding" => chunked = true,
            _ => {}
        }
    }
    // DNS rebinding guard: only a loopback Host with this port.
    let allowed_hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if host.len() != 1
        || !allowed_hosts
            .iter()
            .any(|h| h.eq_ignore_ascii_case(host[0]))
    {
        return error(&mut stream, 403, "forbidden");
    }
    let expected = shared.authorization.as_str().as_bytes();
    if auth.len() != 1 || !ct_eq(auth[0].as_bytes(), expected) {
        return error(&mut stream, 401, "unauthorized");
    }
    if chunked {
        // TODO(#111): streamable HTTP bodies if SP-19 needs them.
        return error(&mut stream, 400, "bad_request");
    }
    let length = length.unwrap_or(0);
    if length > MAX_BODY {
        return error(&mut stream, 413, "too_large");
    }
    let mut chunk = [0u8; 8192];
    while body.len() < length {
        let want = (length - body.len()).min(chunk.len());
        match read_some(&mut stream, &mut chunk[..want], deadline) {
            Ok(n) => body.extend_from_slice(&chunk[..n]),
            Err(408) => return error(&mut stream, 408, "timeout"),
            Err(status) => return error(&mut stream, status, "bad_request"),
        }
    }
    body.truncate(length);
    if shared.stop.load(Ordering::SeqCst) {
        // The session closed while the request was arriving: the handler must not run.
        return;
    }
    let req = LoopbackRequest {
        endpoint,
        method: method.to_owned(),
        path: path.to_owned(),
        body,
    };
    let handler = &shared.handler;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handler.handle(&req))) {
        Ok(resp) => reply(&mut stream, resp.status, &resp.body),
        Err(_) => error(&mut stream, 500, "internal"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_7_ct_eq_compares_whole_values() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"abcd"));
    }

    #[test]
    fn arc_7_secret_is_bearer_and_64_hex() {
        let a = new_authorization().unwrap();
        let s = a.as_str().strip_prefix("Bearer ").unwrap();
        assert_eq!(s.len(), 64);
        assert!(s
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }
}
