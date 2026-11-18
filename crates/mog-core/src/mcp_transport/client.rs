//! The real R8 McpTransport (#30, ADR 0069): JSON-RPC 2.0 over MCP streamable HTTP on blocking
//! `reqwest` with `rustls`. No `rmcp` (G-04a).
//!
//! One `tools/call`, in order (every `Err` before the send sends nothing and lends no token):
//! 1. The tool is on [`ALLOWED_TOOLS`] (G-06a), its arguments match its contract, `profile` keys
//!    are removed (4.6 rule 5), and `_meta` is only `warehouse_id` on `genie_ask` (G-06b, G-07a).
//! 2. The identity headers (SEC-06a, SEC-06b).
//! 3. The Genie One URL of the profile ([`McpEndpoints`]) must be a URL that the
//!    [`TokenEgressPolicy`] lets the token of the profile host go to (4.6 rule 2).
//! 4. The session of (profile, MOG): `initialize` (G-04c) and `notifications/initialized` once,
//!    single-flight; then `Mcp-Session-Id` and `MCP-Protocol-Version` on every request (G-04b).
//! 5. Each HTTP request gets its own token from [`TokenBroker::with_token`] (G-04d). A 401 drops
//!    that lease once and resends once (W-05d); a second 401 is `auth_expired`.
//! 6. A 404 with a session initialises again one time and resends (G-05a). A 404 without a
//!    session is [`GENIE_ONE_OFF`] (G-05b).
//! 7. Rate (#138, ADR 0075), only with [`ReqwestMcpTransport::with_rate_limit`]: each HTTP
//!    request waits in the shared per-profile bucket with the class of its tool (G-09a, G-09b);
//!    a queued ask shows [`BusyState::Queued`]. A 429 cuts the rate (G-09c) and records the
//!    `Retry-After` or FMAPI `error.retry_after` wait (G-10a); a 503 `Retry-After` holds the
//!    profile. Only an ask is sent again: after that wait, or else after the G-BACKOFF wait
//!    (G-10b), [`MAX_ASK_TRIES`] sends or less, with [`BusyState::Retrying`]. A poll or a cancel
//!    gets `rate_limited` and is never sent again here: its next call waits in the bucket.
//!    Every wait of one call fits its time cap: a call that would wait past it is not sent.
//!    TODO(#136): SP-18 confirms the live MCP 429 shape; R8 knows only the HTTP 429 of G-10a.
//! 8. Redirects are never followed (rule 3). A non-JSON page is never echoed or retried
//!    (rule 6). The reply is read up to [`McpLimits::max_reply_bytes`] and within the call time
//!    cap, and is scrubbed (rule 7).
//!
//! Errors are [`CoreError::Unavailable`] with a content-free code: never a URL, host, profile,
//! token, session ID or body text. Log lines name only fixed words and the HTTP status.
//!
//! The 401, 404 and session rules follow `forward` and `call_upstream` of the reference plugin.
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).
//
// TODO(#29): the SP-09 live run confirms the live `tools/list` shape, the busy envelope and a
// mid-session token rotation on a real workspace. Until then, these rules run on doubles only.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use reqwest::blocking::{Client, Response};
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::Url;
use serde_json::{json, Map, Value};

use super::poll::PollInflightGuard;
use super::sse::SseParser;
use super::tools;
use super::{
    err, forwardable_tool_args, outbound_identity_headers, McpCall, McpReply, McpTransport,
    BAD_REPLY, GENIE_ONE_OFF, INVALID_META, INVALID_TOOL_ARGS, PROFILE_INVALID,
    PROTOCOL_UNSUPPORTED, PROTOCOL_VERSION, TOOL_NOT_ALLOWED,
};
use crate::egress_policy::TokenEgressPolicy;
use crate::error::{CoreError, CoreResult};
use crate::http_transport::backoff::{ask_backoff, MAX_ASK_TRIES};
use crate::http_transport::host_policy::AllowlistEgressPolicy;
use crate::http_transport::rate_bucket::{
    parse_retry_after, retry_after_from_body, CallClass, RateBucket,
};
use crate::http_transport::rate_limit::{acquire_within, BusyState, RateLimit};
use crate::http_transport::{
    ProfileHosts, AUTH_EXPIRED, CLIENT_INIT_FAILED, HOST_REFUSED, NON_JSON_REPLY, OFFLINE,
    PROFILE_HOST_INVALID, RATE_LIMITED, REDIRECT_REFUSED, REPLY_TOO_LARGE, TIMEOUT, TOKEN_UNUSABLE,
    TRANSPORT_FAILED, UPSTREAM_FAILED, UPSTREAM_REJECTED,
};
use crate::request_identity::{BottleIdentity, RequestDecorator};
use crate::token_broker::{CliTokenBroker, TokenBroker, TokenLease};
use crate::token_guard::{scrub, scrub_json};
use crate::types::Tags;

/// The `Mcp-Session-Id` header (G-04b).
pub const SESSION_HEADER: &str = "mcp-session-id";
/// The `MCP-Protocol-Version` header (G-04c).
pub const PROTOCOL_HEADER: &str = "mcp-protocol-version";
/// The most `tools/list` pages R8 reads.
pub const MAX_LIST_PAGES: usize = 20;
/// The longest `Mcp-Session-Id` R8 keeps.
const MAX_SESSION_ID: usize = 256;

/// The Genie One MCP URL of a profile (G-02): `https://<host>/ai-gateway/mcp-services/
/// system.ai.genie_one_mcp?o=<workspace_id>`. The production seam is R6
/// ([`CliTokenBroker::genie_one_endpoint`]).
pub trait McpEndpoints: Send + Sync {
    /// The Genie One URL of `profile`.
    fn genie_one_url(&self, profile: &str) -> CoreResult<String>;
}

impl McpEndpoints for CliTokenBroker {
    fn genie_one_url(&self, profile: &str) -> CoreResult<String> {
        self.genie_one_endpoint(profile).map(|e| e.url().to_owned())
    }
}

/// Where R8 writes its content-free log lines (SEC-03).
pub trait McpLog: Send + Sync {
    /// Keep one log line.
    fn record(&self, line: &str);
}

/// A log sink that drops every line.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMcpLog;

impl McpLog for NoMcpLog {
    fn record(&self, _line: &str) {}
}

/// The size and time caps of R8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpLimits {
    /// The largest reply body R8 reads.
    pub max_reply_bytes: u64,
    /// The connect timeout.
    pub connect_timeout: Duration,
    /// The whole-request timeout when the caller gives none.
    pub call_timeout: Duration,
    /// The largest per-call timeout a caller may ask for.
    pub max_call_timeout: Duration,
}

impl Default for McpLimits {
    fn default() -> Self {
        Self {
            max_reply_bytes: 8 * 1024 * 1024,
            connect_timeout: Duration::from_secs(10),
            call_timeout: Duration::from_secs(60),
            max_call_timeout: Duration::from_secs(120),
        }
    }
}

impl McpLimits {
    /// The time cap for one call: the caller's, at most [`McpLimits::max_call_timeout`].
    pub fn effective_timeout(&self, asked: Option<Duration>) -> Duration {
        asked
            .unwrap_or(self.call_timeout)
            .min(self.max_call_timeout)
    }
}

/// The seams of [`ReqwestMcpTransport`].
pub struct McpSeams {
    /// Profile to workspace URL (the token host, 4.6 rule 2).
    pub hosts: Arc<dyn ProfileHosts>,
    /// Profile to Genie One URL (G-02).
    pub endpoints: Arc<dyn McpEndpoints>,
    /// R6: lends the token for one send.
    pub tokens: Arc<dyn TokenBroker>,
    /// 4.6 rules 2, 3 and 6.
    pub policy: Arc<dyn TokenEgressPolicy + Send + Sync>,
    /// SEC-06a, SEC-06b.
    pub decorator: Arc<dyn RequestDecorator>,
    /// Content-free log lines.
    pub log: Arc<dyn McpLog>,
    /// Size and time caps.
    pub limits: McpLimits,
}

/// The state of one MCP session.
#[derive(Default)]
struct Session {
    initialized: bool,
    id: Option<String>,
}

type SessionKey = (String, String);

/// The real R8 McpTransport. Blocking: call it from a worker thread, not from an async task.
pub struct ReqwestMcpTransport {
    client: Client,
    seams: McpSeams,
    sessions: Mutex<HashMap<SessionKey, Arc<Mutex<Session>>>>,
    next_id: AtomicU64,
    /// The rate seams (#138). `None` sends with no bucket (tests of other rules only).
    rate: Option<RateLimit>,
    /// ARC-F1, G-07b: one open poll per conversation, for every caller (#31).
    polls: PollInflightGuard,
}

impl std::fmt::Debug for ReqwestMcpTransport {
    /// Shows only the session count: no seam, session ID, profile or token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestMcpTransport")
            .field("sessions", &lock(&self.sessions).len())
            .finish_non_exhaustive()
    }
}

/// Where one request goes.
struct Target<'a> {
    profile: &'a str,
    host: String,
    url: String,
    headers: Vec<(&'static str, String)>,
    timeout: Duration,
    /// The MOG of the call: only for the busy states, never logged.
    mog: String,
    /// The bucket class of the call (G-09b).
    class: CallClass,
    /// A busy state is on the card for this call and needs a [`BusyState::Clear`].
    busy_shown: Cell<bool>,
    /// The time this call spent waiting for the bucket or a retry, within `timeout`.
    waited: Cell<Duration>,
}

impl Target<'_> {
    /// The wait time left of the call cap.
    fn wait_left(&self) -> Duration {
        self.timeout.saturating_sub(self.waited.get())
    }
}

/// What one HTTP exchange gave.
enum Posted {
    /// A JSON-RPC response to the request, or `None` for a notification.
    Rpc {
        status: u16,
        message: Option<Value>,
        session: Option<String>,
    },
    /// A JSON error body with a non-2xx status (G-14: the brain classifies it).
    HttpError { status: u16, body: Value },
    /// A 404 on a request that carried a session ID (G-05a).
    SessionGone,
    /// A 429, with the clamped retry wait of the reply if it had one (G-10a).
    RateLimited { retry_after: Option<Duration> },
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn header_text(r: &Response, name: &str) -> Option<String> {
    r.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

fn media_type(ct: Option<&str>) -> String {
    ct.and_then(|c| c.split(';').next())
        .map(|c| c.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

/// A session ID R8 can send back: visible ASCII only (MCP spec), bounded.
fn usable_session_id(v: Option<String>) -> Option<String> {
    v.filter(|s| {
        !s.is_empty() && s.len() <= MAX_SESSION_ID && s.bytes().all(|b| (0x21..=0x7e).contains(&b))
    })
}

impl ReqwestMcpTransport {
    /// The production transport over R6: [`AllowlistEgressPolicy`], [`BottleIdentity`], no log
    /// sink, the default caps, and the per-profile `bucket` that R7 shares (G-09a, #138).
    pub fn new(
        hosts: Arc<dyn ProfileHosts>,
        broker: Arc<CliTokenBroker>,
        bucket: Arc<dyn RateBucket>,
    ) -> CoreResult<Self> {
        Self::with_seams(McpSeams {
            hosts,
            endpoints: broker.clone(),
            tokens: broker,
            policy: Arc::new(AllowlistEgressPolicy),
            decorator: Arc::new(BottleIdentity::new()),
            log: Arc::new(NoMcpLog),
            limits: McpLimits::default(),
        })
        .map(|t| t.with_rate_limit(RateLimit::new(bucket)))
    }

    /// A transport on the given seams, with no rate bucket until
    /// [`ReqwestMcpTransport::with_rate_limit`].
    pub fn with_seams(seams: McpSeams) -> CoreResult<Self> {
        // rustls needs one process-wide crypto provider. An `Err` means one is set already.
        let _ = rustls::crypto::ring::default_provider().install_default();
        // TODO(#412): read the OS proxy settings (reqwest `system-proxy`), as R7 will.
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(seams.limits.connect_timeout)
            .timeout(seams.limits.max_call_timeout)
            .build()
            .map_err(|_| err(CLIENT_INIT_FAILED))?;
        Ok(Self {
            client,
            seams,
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            rate: None,
            polls: PollInflightGuard::new(),
        })
    }

    /// Send every request through the shared rate bucket and the 429 rules of `rate` (#138).
    #[must_use]
    pub fn with_rate_limit(mut self, rate: RateLimit) -> Self {
        self.rate = Some(rate);
        self
    }

    /// G-09a, G-09b: wait for a slot in the bucket of the profile. An ask that must wait shows
    /// [`BusyState::Queued`]; the `tools/call` request that gets its slot clears the state.
    /// Every wait of one call counts against its time cap: a call that would wait past it is
    /// not sent and is `rate_limited`, so nothing goes out after the caller stopped waiting.
    fn wait_turn(&self, t: &Target<'_>, is_tool_call: bool) -> CoreResult<()> {
        let Some(rl) = &self.rate else { return Ok(()) };
        let waited = acquire_within(
            &*rl.bucket,
            &*rl.pause,
            t.profile,
            t.class,
            t.wait_left(),
            &mut |_| {
                if t.class == CallClass::Ask && !t.busy_shown.get() {
                    rl.observer.busy(&t.mog, BusyState::Queued);
                    t.busy_shown.set(true);
                }
            },
        );
        let Some(waited) = waited else {
            self.log("mcp refused code=rate_limited wait=over_cap");
            return Err(err(RATE_LIMITED));
        };
        t.waited.set(t.waited.get().saturating_add(waited));
        if is_tool_call {
            self.clear_busy(t);
        }
        Ok(())
    }

    fn clear_busy(&self, t: &Target<'_>) {
        if let Some(rl) = &self.rate {
            if t.busy_shown.replace(false) {
                rl.observer.busy(&t.mog, BusyState::Clear);
            }
        }
    }

    fn log(&self, line: &str) {
        // Every part is a fixed word or a number already; the scrub is a second guard.
        self.seams.log.record(&scrub(line));
    }

    fn refuse(&self, code: &'static str) -> CoreError {
        self.log(&format!("mcp refused code={code}"));
        err(code)
    }

    /// The host of the profile URL (a bare `scheme://host[:port]` URL only).
    fn profile_host(&self, profile: &str) -> CoreResult<String> {
        let text = self.seams.hosts.workspace_url(profile)?;
        let u = Url::parse(text.trim()).map_err(|_| err(PROFILE_HOST_INVALID))?;
        let bare = (u.path().is_empty() || u.path() == "/")
            && u.query().is_none()
            && u.fragment().is_none()
            && u.username().is_empty()
            && u.password().is_none();
        match u.host_str() {
            Some(h) if bare && !h.is_empty() => Ok(h.to_ascii_lowercase()),
            _ => Err(err(PROFILE_HOST_INVALID)),
        }
    }

    /// Steps 2 and 3: the headers, the profile host and the Genie One URL.
    fn target<'a>(
        &self,
        profile: &'a str,
        tags: &Tags,
        timeout: Option<Duration>,
    ) -> CoreResult<Target<'a>> {
        if profile.is_empty() {
            return Err(self.refuse(PROFILE_INVALID));
        }
        let headers = outbound_identity_headers(tags).map_err(|e| err(e.code()))?;
        let host = self.profile_host(profile)?;
        let url = self.seams.endpoints.genie_one_url(profile)?;
        if !self.seams.policy.may_send_token(&host, &url) {
            return Err(self.refuse(HOST_REFUSED));
        }
        Ok(Target {
            profile,
            host,
            url,
            headers,
            timeout: self.seams.limits.effective_timeout(timeout),
            mog: tags.mog.clone(),
            class: CallClass::Ask,
            busy_shown: Cell::new(false),
            waited: Cell::new(Duration::ZERO),
        })
    }

    fn session(&self, profile: &str, mog: &str) -> Arc<Mutex<Session>> {
        let mut all = lock(&self.sessions);
        Arc::clone(all.entry((profile.to_owned(), mog.to_owned())).or_default())
    }

    fn rpc_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Build one request. The `Authorization` value is marked sensitive, so the `Debug` output
    /// of the request never shows the token.
    fn build(
        &self,
        token: &[u8],
        t: &Target<'_>,
        session: Option<&str>,
        after_init: bool,
        body: &[u8],
    ) -> CoreResult<reqwest::blocking::Request> {
        let mut bearer = Vec::with_capacity(7 + token.len());
        bearer.extend_from_slice(b"Bearer ");
        bearer.extend_from_slice(token);
        let auth = HeaderValue::from_bytes(&bearer);
        bearer.fill(0);
        std::hint::black_box(&mut bearer);
        let mut auth = auth.map_err(|_| err(TOKEN_UNUSABLE))?;
        auth.set_sensitive(true);
        let mut req = self
            .client
            .post(t.url.as_str())
            .timeout(t.timeout)
            .header(AUTHORIZATION, auth)
            .header(ACCEPT, "application/json, text/event-stream")
            .header(CONTENT_TYPE, "application/json");
        for (name, value) in &t.headers {
            req = req.header(*name, value.as_str());
        }
        if after_init {
            req = req.header(PROTOCOL_HEADER, PROTOCOL_VERSION);
        }
        if let Some(sid) = session {
            req = req.header(SESSION_HEADER, sid);
        }
        req.body(body.to_vec())
            .build()
            .map_err(|_| err(TRANSPORT_FAILED))
    }

    /// Send once with a token of the profile, lent only for this one send (G-04d).
    fn send_once(
        &self,
        t: &Target<'_>,
        session: Option<&str>,
        after_init: bool,
        body: &[u8],
    ) -> CoreResult<(Response, TokenLease)> {
        let mut out: Option<CoreResult<Response>> = None;
        let lease = self
            .seams
            .tokens
            .with_token(t.profile, &t.host, &mut |token| {
                out = Some(
                    self.build(token, t, session, after_init, body)
                        .and_then(|req| {
                            self.client.execute(req).map_err(|e| {
                                if e.is_timeout() {
                                    err(TIMEOUT)
                                } else if e.is_connect() {
                                    err(OFFLINE)
                                } else {
                                    err(TRANSPORT_FAILED)
                                }
                            })
                        }),
                );
            })?;
        out.unwrap_or(Err(err(TOKEN_UNUSABLE))).map(|r| (r, lease))
    }

    /// One JSON-RPC message: the 401 rule (W-05d), the status rules and the bounded read.
    /// `kind` is a fixed word for the log. `rpc_id` is `None` for a notification.
    fn post(
        &self,
        t: &Target<'_>,
        session: Option<&str>,
        after_init: bool,
        kind: &'static str,
        message: &Value,
        rpc_id: Option<u64>,
    ) -> CoreResult<Posted> {
        let body = serde_json::to_vec(message).map_err(|_| err(TRANSPORT_FAILED))?;
        let mut attempt = 1u8;
        loop {
            self.wait_turn(t, kind == "tools/call")?;
            let sent = self.send_once(t, session, after_init, &body);
            let (resp, lease) = match sent {
                Ok(x) => x,
                Err(e) => {
                    self.log(&format!(
                        "mcp {kind} attempt={attempt} outcome={}",
                        e.code()
                    ));
                    return Err(e);
                }
            };
            let status = resp.status().as_u16();
            self.log(&format!("mcp {kind} attempt={attempt} status={status}"));
            if (300..400).contains(&status) {
                // Rule 3: no redirect is followed, with or without the token.
                return Err(err(REDIRECT_REFUSED));
            }
            if (200..300).contains(&status) {
                return self.read_rpc(resp, status, rpc_id);
            }
            // Rule 6, before any status rule: a non-JSON page never reaches TS and is never
            // retried with the token (also on 401 and 404). An empty body is not a page.
            let ct = header_text(&resp, CONTENT_TYPE.as_str());
            let retry_after = header_text(&resp, "retry-after")
                .and_then(|v| parse_retry_after(&v, SystemTime::now()));
            let bytes = self.read_capped(resp)?;
            let json = self.seams.policy.reply_for_ts(ct.as_deref(), &bytes);
            if status == 429 {
                // G-09c, G-10a: the header wait, else the FMAPI body wait (clamped 1-300 s).
                let wait = retry_after.or_else(|| json.as_ref().and_then(retry_after_from_body));
                if let Some(rl) = &self.rate {
                    rl.bucket.on_rate_limited(t.profile, wait);
                }
                // A 429 page is dropped, never echoed (rule 6): only the status counts.
                return Ok(Posted::RateLimited { retry_after: wait });
            }
            if let (503, Some(wait), Some(rl)) = (status, retry_after, &self.rate) {
                // G-10a: a 503 `Retry-After` holds the profile, with no rate cut.
                rl.bucket.hold(t.profile, wait);
            }
            if json.is_none() && (!bytes.is_empty() || !matches!(status, 401 | 404 | 429)) {
                return Err(err(NON_JSON_REPLY));
            }
            match status {
                401 if attempt == 1 => {
                    // Drop only the token that got the 401, not a newer one (W-05c).
                    self.seams.tokens.invalidate(t.profile, lease)?;
                    attempt += 1;
                }
                401 => return Err(err(AUTH_EXPIRED)),
                404 if session.is_some() => return Ok(Posted::SessionGone),
                404 => return Err(err(GENIE_ONE_OFF)),
                _ => {
                    let body = json.ok_or(err(NON_JSON_REPLY))?;
                    return Ok(Posted::HttpError { status, body });
                }
            }
        }
    }

    fn read_capped(&self, resp: Response) -> CoreResult<Vec<u8>> {
        let max = self.seams.limits.max_reply_bytes;
        if resp.content_length().is_some_and(|n| n > max) {
            return Err(err(REPLY_TOO_LARGE));
        }
        let mut bytes = Vec::new();
        resp.take(max + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| read_error(&e))?;
        if bytes.len() as u64 > max {
            return Err(err(REPLY_TOO_LARGE));
        }
        Ok(bytes)
    }

    /// A 2xx reply: the JSON-RPC response whose `id` is `rpc_id`, from JSON or SSE.
    fn read_rpc(&self, resp: Response, status: u16, rpc_id: Option<u64>) -> CoreResult<Posted> {
        let session = usable_session_id(header_text(&resp, SESSION_HEADER));
        let Some(id) = rpc_id else {
            // A notification: 202 with no body. Nothing is read.
            return Ok(Posted::Rpc {
                status,
                message: None,
                session,
            });
        };
        let ct = header_text(&resp, CONTENT_TYPE.as_str());
        let message = if media_type(ct.as_deref()) == "text/event-stream" {
            self.read_sse(resp, id)?
        } else {
            let bytes = self.read_capped(resp)?;
            let v = self
                .seams
                .policy
                .reply_for_ts(ct.as_deref(), &bytes)
                .ok_or(err(NON_JSON_REPLY))?;
            find_response(v, id).ok_or(err(BAD_REPLY))?
        };
        Ok(Posted::Rpc {
            status,
            message: Some(message),
            session,
        })
    }

    /// Read SSE events until the response with `id` arrives, within the size cap.
    fn read_sse(&self, mut resp: Response, id: u64) -> CoreResult<Value> {
        let max = self.seams.limits.max_reply_bytes;
        let mut parser = SseParser::new();
        let mut events = Vec::new();
        let mut chunk = [0u8; 8192];
        let mut total = 0u64;
        loop {
            let n = resp.read(&mut chunk).map_err(|e| read_error(&e))?;
            if n == 0 {
                parser.finish(&mut events);
            } else {
                total += n as u64;
                if total > max {
                    return Err(err(REPLY_TOO_LARGE));
                }
                parser.push(&chunk[..n], &mut events);
            }
            for e in events.drain(..) {
                if let Some(v) = serde_json::from_str::<Value>(&e.data)
                    .ok()
                    .and_then(|v| find_response(v, id))
                {
                    return Ok(v);
                }
            }
            if n == 0 {
                return Err(err(BAD_REPLY));
            }
        }
    }

    /// Step 4: `initialize` and `notifications/initialized` for one session, while the caller
    /// holds the session lock (single-flight).
    fn initialize(&self, t: &Target<'_>, s: &mut Session) -> CoreResult<()> {
        s.initialized = false;
        s.id = None;
        let id = self.rpc_id();
        let msg = json!({
            "jsonrpc": "2.0", "id": id, "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "bottle", "version": env!("CARGO_PKG_VERSION")}
            }
        });
        let (message, session) = match self.post(t, None, false, "initialize", &msg, Some(id))? {
            Posted::Rpc {
                message, session, ..
            } => (message, session),
            Posted::HttpError { status, .. } => return Err(err(upstream_code(status))),
            Posted::RateLimited { .. } => return Err(err(RATE_LIMITED)),
            // `post` gives this only for a request with a session ID.
            Posted::SessionGone => return Err(err(BAD_REPLY)),
        };
        let version = message
            .as_ref()
            .and_then(|m| m.pointer("/result/protocolVersion"))
            .and_then(Value::as_str)
            .ok_or(err(BAD_REPLY))?;
        if version != PROTOCOL_VERSION {
            return Err(err(PROTOCOL_UNSUPPORTED));
        }
        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        match self.post(
            t,
            session.as_deref(),
            true,
            "notifications/initialized",
            &note,
            None,
        )? {
            Posted::Rpc { .. } => {}
            Posted::SessionGone => return Err(err(GENIE_ONE_OFF)),
            Posted::HttpError { status, .. } => return Err(err(upstream_code(status))),
            Posted::RateLimited { .. } => return Err(err(RATE_LIMITED)),
        }
        s.id = session;
        s.initialized = true;
        Ok(())
    }

    /// One request in the session, with one new `initialize` after a 404 with a session (G-05a).
    fn request(
        &self,
        t: &Target<'_>,
        session: &Mutex<Session>,
        kind: &'static str,
        method: &str,
        params: Value,
    ) -> CoreResult<Posted> {
        for round in 0..2 {
            let sid = {
                let mut s = lock(session);
                if !s.initialized {
                    self.initialize(t, &mut s)?;
                }
                s.id.clone()
            };
            let id = self.rpc_id();
            let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
            match self.post(t, sid.as_deref(), true, kind, &msg, Some(id))? {
                Posted::SessionGone if round == 0 => {
                    let mut s = lock(session);
                    // Another caller may have started a new session already.
                    if s.id == sid {
                        s.initialized = false;
                        s.id = None;
                    }
                }
                Posted::SessionGone => break,
                other => return Ok(other),
            }
        }
        Err(err(GENIE_ONE_OFF))
    }
}

impl ReqwestMcpTransport {
    /// One `tools/call`. G-10b: only an ask that got a 429 is sent again, after the retry wait
    /// of the reply (the bucket holds the profile for it) or else the G-BACKOFF wait, with
    /// [`MAX_ASK_TRIES`] sends or less. A 429 means the server did not take the ask, so a new
    /// send makes no duplicate question (MOG-10b).
    fn call_with_retry(
        &self,
        t: &Target<'_>,
        session: &Mutex<Session>,
        params: &Value,
    ) -> CoreResult<Posted> {
        let mut tries = 1u32;
        loop {
            let posted = self.request(t, session, "tools/call", "tools/call", params.clone())?;
            let Posted::RateLimited { retry_after } = posted else {
                return Ok(posted);
            };
            let rl = match &self.rate {
                Some(rl) if t.class == CallClass::Ask && tries < MAX_ASK_TRIES => rl,
                _ => return Ok(posted),
            };
            let wait = retry_after.unwrap_or_else(|| ask_backoff(tries, &*rl.jitter));
            if wait > t.wait_left() {
                // The retry would go out after the call cap, when the caller no longer waits
                // for it (MOG-10b, B1): give up now, nothing more is sent.
                self.log("mcp tools/call rate_limited wait=over_cap");
                return Ok(posted);
            }
            self.log(&format!("mcp tools/call rate_limited try={tries}"));
            rl.observer.busy(&t.mog, BusyState::Retrying { wait });
            t.busy_shown.set(true);
            if retry_after.is_none() {
                // With a header the bucket holds the profile for the wait already.
                rl.pause.pause(wait);
                t.waited.set(t.waited.get().saturating_add(wait));
            }
            tries += 1;
        }
    }
}

fn read_error(e: &std::io::Error) -> CoreError {
    // reqwest gives a timeout while reading the body as an io error.
    if e.kind() == std::io::ErrorKind::TimedOut
        || std::error::Error::source(e)
            .and_then(|s| s.downcast_ref::<reqwest::Error>())
            .is_some_and(reqwest::Error::is_timeout)
    {
        err(TIMEOUT)
    } else {
        err(TRANSPORT_FAILED)
    }
}

/// The code of a non-2xx JSON reply to `initialize` or the notification.
fn upstream_code(status: u16) -> &'static str {
    if (400..500).contains(&status) {
        UPSTREAM_REJECTED
    } else {
        UPSTREAM_FAILED
    }
}

/// The JSON-RPC response with `id` in one message or a batch.
fn find_response(v: Value, id: u64) -> Option<Value> {
    let is_it = |m: &Value| {
        m.get("id").and_then(Value::as_u64) == Some(id)
            && (m.get("result").is_some() || m.get("error").is_some())
    };
    match v {
        Value::Array(items) => items.into_iter().find(is_it),
        m if is_it(&m) => Some(m),
        _ => None,
    }
}

/// A `tools/call` response as the `mcp_call` reply, scrubbed (rule 7).
fn reply_of(status: u16, message: &Value) -> CoreResult<McpReply> {
    if let Some(e) = message.get("error") {
        return Ok(McpReply {
            structured_content: Value::Null,
            text: String::new(),
            is_error: true,
            http_status: status,
            rpc_error: Some(scrub_json(e)),
        });
    }
    let result = message
        .get("result")
        .and_then(Value::as_object)
        .ok_or(err(BAD_REPLY))?;
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    Ok(McpReply {
        structured_content: result
            .get("structuredContent")
            .map(scrub_json)
            .unwrap_or(Value::Null),
        text: scrub(&text),
        is_error: result.get("isError").and_then(Value::as_bool) == Some(true),
        http_status: status,
        rpc_error: None,
    })
}

impl McpTransport for ReqwestMcpTransport {
    fn call_tool(&self, call: McpCall<'_>) -> CoreResult<McpReply> {
        // Step 1, before any lookup or lend (G-06a).
        let contract = tools::allowed(call.tool).ok_or_else(|| self.refuse(TOOL_NOT_ALLOWED))?;
        let args = forwardable_tool_args(call.args);
        if !tools::args_match(contract, &args) {
            return Err(self.refuse(INVALID_TOOL_ARGS));
        }
        let meta = tools::outbound_meta(contract.name, call.meta)
            .map_err(|()| self.refuse(INVALID_META))?;
        // ARC-F1: before any lookup, lend or send. The slot is held until this call returns.
        let _poll_slot = self
            .polls
            .enter(contract.name, &args)
            .map_err(|e| self.refuse(e.code()))?;
        let mut t = self.target(call.profile, call.tags, call.timeout)?;
        t.class = CallClass::of_tool(contract.name);
        let mut params = Map::new();
        params.insert("name".to_owned(), Value::String(contract.name.to_owned()));
        params.insert(
            "arguments".to_owned(),
            Value::Object(args.into_iter().collect()),
        );
        if let Some(m) = meta {
            params.insert("_meta".to_owned(), m);
        }
        let session = self.session(call.profile, &call.tags.mog);
        let posted = self.call_with_retry(&t, &session, &Value::Object(params));
        self.clear_busy(&t);
        match posted? {
            Posted::Rpc {
                status,
                message: Some(m),
                ..
            } => reply_of(status, &m),
            Posted::HttpError { status, body } => {
                let e = body.get("error").cloned().unwrap_or(body);
                Ok(McpReply {
                    structured_content: Value::Null,
                    text: String::new(),
                    is_error: true,
                    http_status: status,
                    rpc_error: Some(scrub_json(&e)),
                })
            }
            Posted::RateLimited { .. } => Err(err(RATE_LIMITED)),
            _ => Err(err(BAD_REPLY)),
        }
    }

    fn list_tools(&self, profile: &str, tags: &Tags) -> CoreResult<Vec<Value>> {
        let t = self.target(profile, tags, None)?;
        let session = self.session(profile, &tags.mog);
        let mut listed = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_LIST_PAGES {
            let params = match &cursor {
                Some(c) => json!({ "cursor": c }),
                None => json!({}),
            };
            let m = match self.request(&t, &session, "tools/list", "tools/list", params)? {
                Posted::Rpc {
                    message: Some(m), ..
                } => m,
                Posted::RateLimited { .. } => return Err(err(RATE_LIMITED)),
                _ => return Err(err(BAD_REPLY)),
            };
            let result = m.get("result").ok_or(err(BAD_REPLY))?;
            if let Some(tools) = result.get("tools").and_then(Value::as_array) {
                listed.extend(tools.iter().cloned());
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                // G-06a: any tool not on the allowlist is dropped.
                return Ok(tools::keep_allowed(&listed)
                    .iter()
                    .map(scrub_json)
                    .collect());
            }
        }
        Err(err(BAD_REPLY))
    }

    fn start_loopback_proxy(&self) -> CoreResult<u16> {
        // TODO(#139): loopback proxy for the agent's Genie One MCP calls.
        Err(CoreError::NotImplemented { component: "R8" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_04b_session_id_must_be_visible_ascii_and_bounded() {
        assert_eq!(usable_session_id(Some("s-1".into())), Some("s-1".into()));
        for bad in ["", "a b", "a\u{7f}", "é"] {
            assert_eq!(usable_session_id(Some(bad.into())), None, "{bad:?}");
        }
        assert_eq!(usable_session_id(Some("x".repeat(257))), None);
    }

    #[test]
    fn g_04a_find_response_matches_the_id_in_one_message_or_a_batch() {
        let r = json!({"jsonrpc": "2.0", "id": 3, "result": {}});
        assert_eq!(find_response(r.clone(), 3), Some(r.clone()));
        assert_eq!(find_response(r.clone(), 4), None);
        let note = json!({"jsonrpc": "2.0", "method": "x", "id": 3});
        assert_eq!(find_response(json!([note, r.clone()]), 3), Some(r));
    }

    #[test]
    fn arc_6_rule_7_reply_of_scrubs_text_and_errors() {
        let m = json!({"error": {"code": 1, "message": "password=not-real-value"}});
        let r = reply_of(200, &m).unwrap();
        assert!(!r.rpc_error.unwrap().to_string().contains("not-real-value"));
        assert_eq!(reply_of(200, &json!({})).unwrap_err().code(), BAD_REPLY);
    }
}
