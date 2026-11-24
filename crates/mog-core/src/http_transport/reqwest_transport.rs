//! The real R7 HttpTransport (#364, ADR 0046): blocking `reqwest` with `rustls` (ring
//! provider) and no default features.
//!
//! One call, in order (every `Err` before the send sends nothing):
//! 1. Route ID to a fixed method and path; params fill only named slots (4.6 rule 1).
//! 2. [`super::outbound_headers`]: User-Agent and, on Unity Gateway routes, the tags
//!    (SEC-06a, SEC-06b).
//! 3. The profile URL from [`ProfileHosts`]; the [`TokenEgressPolicy`] must allow the token to
//!    go to it (rule 2).
//! 4. The per-profile [`RateBucket`] (G-09a), with the call class of the route (G-09b). A 429
//!    records AIMD and its retry wait; a 503 `Retry-After` holds the profile (G-10a, #138).
//! 5. The token from [`TokenBroker::with_token`] for this one send only. It is never logged.
//! 6. Redirects are off in reqwest. R7 follows a 3xx only to the same host (rule 3).
//! 7. Only a JSON reply reaches the caller (rule 6), scrubbed (rule 7). A page is never retried.
//! 8. A 401 invalidates the token once and retries once (W-05b..d).
//!
//! R7 is also the [`ScimMe`] seam of R6: `GET /api/2.0/preview/scim/v2/Me` through this same
//! pipeline, returning only the `x-databricks-org-id` header (W-06a).
//!
//! Errors are [`CoreError::Unavailable`] with a content-free code: never a URL, host, token
//! or body text.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE, LOCATION, RETRY_AFTER};
use reqwest::Url;
use serde_json::Value;

use super::host_policy::AllowlistEgressPolicy;
use super::rate_bucket::{parse_retry_after, retry_after_from_body, CallClass, RateBucket};
use super::rate_limit::{acquire, acquire_within_or_stop};
use super::routes::{self, Method};
use super::{outbound_headers, route_call_kind, HttpTransport, ID};
use crate::egress_policy::TokenEgressPolicy;
use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::recording_cloud::http::{CallBounds, RecordingHttp, Reply, ReplyBody, ReqBody};
use crate::request_identity::{BottleIdentity, CallKind, RequestDecorator};
use crate::token_broker::{ScimMe, TokenBroker, TokenLease};
use crate::token_guard::scrub_json;
use crate::types::Tags;

/// The profile URL is not a bare `scheme://host[:port]`.
pub const PROFILE_HOST_INVALID: &str = "profile_host_invalid";
/// The egress policy refused to send the token to the profile host (rule 2).
pub const HOST_REFUSED: &str = "host_refused";
/// A 3xx went to another host, or too many hops (rule 3).
pub const REDIRECT_REFUSED: &str = "redirect_refused";
/// The upstream reply was not JSON (rule 6, SEC-05a: the card shows `PROXY_PAGE`).
pub const NON_JSON_REPLY: &str = "non_json_reply";
/// A 401 again after one invalidate and retry.
pub const AUTH_EXPIRED: &str = "auth_expired";
/// A 429. The bucket holds the retry wait.
pub const RATE_LIMITED: &str = "rate_limited";
/// The host did not accept a connection.
pub const OFFLINE: &str = "offline";
/// The call timed out.
pub const TIMEOUT: &str = "timeout";
/// The call failed in transport for another reason.
pub const TRANSPORT_FAILED: &str = "transport_failed";
/// A JSON 4xx reply other than 401 and 429.
pub const UPSTREAM_REJECTED: &str = "upstream_rejected";
/// A JSON 5xx (or other non-2xx) reply.
pub const UPSTREAM_FAILED: &str = "upstream_failed";
/// The reply is larger than [`MAX_REPLY_BYTES`].
pub const REPLY_TOO_LARGE: &str = "reply_too_large";
/// The token bytes cannot go in a header, or the broker lent nothing.
pub const TOKEN_UNUSABLE: &str = "token_unusable";
/// The HTTP client could not start.
pub const CLIENT_INIT_FAILED: &str = "client_init_failed";

/// The largest reply R7 reads.
pub const MAX_REPLY_BYTES: u64 = 8 * 1024 * 1024;
/// The most same-host redirects R7 follows for one call.
pub const MAX_REDIRECTS: u8 = 3;
/// The connect timeout.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The whole-call timeout.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The workspace URL of a CLI profile (`host` in the CLI config).
/// The app's one is T7 `profile_service::ProfileService` (#17): only supported profiles have one.
pub trait ProfileHosts: Send + Sync {
    /// The workspace URL of `profile`, for example `https://<workspace>.cloud.databricks.com`.
    fn workspace_url(&self, profile: &str) -> CoreResult<String>;
}

/// For tests and builds with no T7: every call is [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubProfileHosts;

impl ProfileHosts for StubProfileHosts {
    fn workspace_url(&self, _profile: &str) -> CoreResult<String> {
        Err(CoreError::NotImplemented { component: "R5" })
    }
}

/// How R7 waits for the rate bucket.
pub trait Pause: Send + Sync {
    /// Block the calling thread for `d`.
    fn pause(&self, d: Duration);
}

/// [`Pause`] with `std::thread::sleep`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadPause;

impl Pause for ThreadPause {
    fn pause(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// The seams of [`ReqwestHttpTransport`].
pub struct TransportSeams {
    /// Profile to workspace URL.
    pub hosts: Arc<dyn ProfileHosts>,
    /// R6: lends the token for one send.
    pub tokens: Arc<dyn TokenBroker>,
    /// 4.6 rules 2, 3 and 6.
    pub policy: Arc<dyn TokenEgressPolicy + Send + Sync>,
    /// G-09a..d, G-10a.
    pub bucket: Arc<dyn RateBucket>,
    /// SEC-06a, SEC-06b.
    pub decorator: Arc<dyn RequestDecorator>,
    /// Waits for the bucket.
    pub pause: Arc<dyn Pause>,
}

/// The real R7 HttpTransport. Blocking: call it from a worker thread, not from an async task.
pub struct ReqwestHttpTransport {
    client: Client,
    seams: TransportSeams,
    online: AtomicBool,
}

/// One upstream reply, read in full (bounded).
struct Raw {
    status: u16,
    content_type: Option<String>,
    location: Option<String>,
    retry_after: Option<String>,
    org_id: Option<String>,
    body: Vec<u8>,
}

/// The body of one send: JSON (every IPC route) or raw bytes (a recording upload).
#[derive(Clone, Copy)]
enum SendBody<'a> {
    Json(&'a [u8]),
    Raw(&'a [u8]),
}

/// The whole-call timeout of a recording upload (a 2 h Opus part is about 15 MB).
pub const UPLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);
/// The whole-call timeout of a recording model call (a 2 h transcript is a few minutes).
pub const MODEL_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// The largest `files.download` reply (an audio part read back for Retry).
pub const MAX_DOWNLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// The timeout and reply cap of one send.
#[derive(Clone, Copy)]
struct Limits {
    timeout: Option<Duration>,
    max_reply: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            timeout: None,
            max_reply: MAX_REPLY_BYTES,
        }
    }
}

/// The `x-databricks-org-id` reply header (W-06a).
const ORG_ID_HEADER: &str = "x-databricks-org-id";

fn err(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

fn header_text(
    h: &reqwest::header::HeaderMap,
    name: reqwest::header::HeaderName,
) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

impl ReqwestHttpTransport {
    /// The production transport: [`AllowlistEgressPolicy`], [`BottleIdentity`], [`ThreadPause`].
    pub fn new(
        hosts: Arc<dyn ProfileHosts>,
        tokens: Arc<dyn TokenBroker>,
        bucket: Arc<dyn RateBucket>,
    ) -> CoreResult<Self> {
        Self::with_seams(TransportSeams {
            hosts,
            tokens,
            policy: Arc::new(AllowlistEgressPolicy),
            bucket,
            decorator: Arc::new(BottleIdentity::new()),
            pause: Arc::new(ThreadPause),
        })
    }

    /// A transport on the given seams.
    pub fn with_seams(seams: TransportSeams) -> CoreResult<Self> {
        // Proxies come from `HTTPS_PROXY` / `NO_PROXY` only.
        // TODO(#412): read the OS proxy settings (reqwest `system-proxy`).
        // rustls needs one process-wide crypto provider. An `Err` means one is set already.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|_| err(CLIENT_INIT_FAILED))?;
        Ok(Self {
            client,
            seams,
            online: AtomicBool::new(true),
        })
    }

    /// The profile origin (`scheme://host[:port]`) and its host.
    fn origin(&self, profile: &str) -> CoreResult<(String, String)> {
        let text = self.seams.hosts.workspace_url(profile)?;
        let u = Url::parse(text.trim()).map_err(|_| err(PROFILE_HOST_INVALID))?;
        let bare = (u.path().is_empty() || u.path() == "/")
            && u.query().is_none()
            && u.fragment().is_none()
            && u.username().is_empty()
            && u.password().is_none();
        let host = u
            .host_str()
            .filter(|_| bare)
            .ok_or(err(PROFILE_HOST_INVALID))?;
        let origin = match u.port() {
            Some(p) => format!("{}://{host}:{p}", u.scheme()),
            None => format!("{}://{host}", u.scheme()),
        };
        Ok((origin, host.to_owned()))
    }

    fn wait_for_bucket(&self, profile: &str, class: CallClass) {
        acquire(
            &*self.seams.bucket,
            &*self.seams.pause,
            profile,
            class,
            &mut |_| {},
        );
    }

    /// Send once with the token of `profile`. The token lives only inside the broker call.
    fn send_once(
        &self,
        profile: &str,
        host: &str,
        method: Method,
        url: &str,
        headers: &[(&'static str, String)],
        (body, limits, abandoned): (
            Option<SendBody<'_>>,
            Limits,
            Option<&std::sync::atomic::AtomicBool>,
        ),
    ) -> CoreResult<(Raw, TokenLease)> {
        let mut out: Option<CoreResult<Raw>> = None;
        let lease = self.seams.tokens.with_token(profile, host, &mut |token| {
            // A call that was abandoned while the token was fetched or refreshed is not sent.
            if abandoned.is_some_and(|a| a.load(std::sync::atomic::Ordering::SeqCst)) {
                out = Some(Err(err(TIMEOUT)));
                return;
            }
            out = Some(self.send_with(token, method, url, headers, body, limits));
        })?;
        out.unwrap_or(Err(err(TOKEN_UNUSABLE)))
            .map(|raw| (raw, lease))
    }

    /// Build one request. The `Authorization` value is marked sensitive, so the `Debug` output
    /// of the request (and of any header map that holds it) never shows the token.
    fn build_request(
        &self,
        token: &[u8],
        method: Method,
        url: &str,
        headers: &[(&'static str, String)],
        body: Option<SendBody<'_>>,
        timeout: Option<Duration>,
    ) -> CoreResult<reqwest::blocking::Request> {
        let mut bearer = Vec::with_capacity(7 + token.len());
        bearer.extend_from_slice(b"Bearer ");
        bearer.extend_from_slice(token);
        let auth = HeaderValue::from_bytes(&bearer);
        bearer.fill(0);
        std::hint::black_box(&mut bearer);
        let mut auth = auth.map_err(|_| err(TOKEN_UNUSABLE))?;
        auth.set_sensitive(true);

        let m = match method {
            Method::Get => reqwest::Method::GET,
            Method::Post => reqwest::Method::POST,
            Method::Patch => reqwest::Method::PATCH,
            Method::Delete => reqwest::Method::DELETE,
            Method::Put => reqwest::Method::PUT,
        };
        let mut req = self
            .client
            .request(m, url)
            .header(AUTHORIZATION, auth)
            .header(ACCEPT, "application/json");
        for (name, value) in headers {
            req = req.header(*name, value.as_str());
        }
        match body {
            Some(SendBody::Json(b)) => {
                req = req
                    .header(CONTENT_TYPE, "application/json")
                    .body(b.to_vec());
            }
            Some(SendBody::Raw(b)) => {
                req = req
                    .header(CONTENT_TYPE, "application/octet-stream")
                    .body(b.to_vec());
            }
            None => {}
        }
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        req.build().map_err(|_| err(TRANSPORT_FAILED))
    }

    fn send_with(
        &self,
        token: &[u8],
        method: Method,
        url: &str,
        headers: &[(&'static str, String)],
        body: Option<SendBody<'_>>,
        limits: Limits,
    ) -> CoreResult<Raw> {
        let req = self.build_request(token, method, url, headers, body, limits.timeout)?;
        let resp = self.client.execute(req).map_err(|e| {
            if e.is_connect() {
                self.online.store(false, Ordering::SeqCst);
                err(OFFLINE)
            } else if e.is_timeout() {
                err(TIMEOUT)
            } else {
                err(TRANSPORT_FAILED)
            }
        })?;
        self.online.store(true, Ordering::SeqCst);
        let status = resp.status().as_u16();
        let h = resp.headers();
        let content_type = header_text(h, CONTENT_TYPE);
        let location = header_text(h, LOCATION);
        let retry_after = header_text(h, RETRY_AFTER);
        let org_id = h
            .get(ORG_ID_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let mut bytes = Vec::new();
        resp.take(limits.max_reply + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| err(TRANSPORT_FAILED))?;
        if bytes.len() as u64 > limits.max_reply {
            return Err(err(REPLY_TOO_LARGE));
        }
        Ok(Raw {
            status,
            content_type,
            location,
            retry_after,
            org_id,
            body: bytes,
        })
    }
}

impl std::fmt::Debug for ReqwestHttpTransport {
    /// Shows only the online state: no seam, header or token.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReqwestHttpTransport")
            .field("online", &self.online.load(Ordering::SeqCst))
            .finish_non_exhaustive()
    }
}

impl ReqwestHttpTransport {
    /// Rules 1-8 for one route: the scrubbed JSON reply and its `x-databricks-org-id` header.
    fn request_raw(
        &self,
        profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: Option<&Value>,
        headers: Vec<(&'static str, String)>,
    ) -> CoreResult<(Value, Option<String>)> {
        let route = routes::route(route_id)?;
        let class = CallClass::of_route(route_id);
        let path = routes::build_path(route, params)?;
        let (origin, host) = self.origin(profile)?;
        let mut url = format!("{origin}{path}");
        let policy = &self.seams.policy;
        if !policy.may_send_token(&host, &url) {
            return Err(err(HOST_REFUSED));
        }
        let mut body = match body {
            Some(v) => Some(serde_json::to_vec(v).map_err(|_| err(routes::INVALID_PARAMS))?),
            None => None,
        };
        let mut method = route.method;
        let mut retried_401 = false;
        let mut hops = 0u8;
        loop {
            self.wait_for_bucket(profile, class);
            let (raw, lease) = self.send_once(
                profile,
                &host,
                method,
                &url,
                &headers,
                (body.as_deref().map(SendBody::Json), Limits::default(), None),
            )?;
            if (300..400).contains(&raw.status) {
                // Rule 3: same host only; anything else is dropped, never followed.
                let to = raw
                    .location
                    .as_deref()
                    .and_then(|l| Url::parse(&url).ok()?.join(l).ok())
                    .map(String::from)
                    .ok_or(err(REDIRECT_REFUSED))?;
                if hops >= MAX_REDIRECTS
                    || !policy.may_send_token(&host, &to)
                    || !policy.may_follow_redirect_with_authorization(&host, &url, &to)
                {
                    return Err(err(REDIRECT_REFUSED));
                }
                if matches!(raw.status, 301..=303) {
                    method = Method::Get;
                    body = None;
                }
                url = to;
                hops += 1;
                continue;
            }
            let reply = policy.reply_for_ts(raw.content_type.as_deref(), &raw.body);
            if raw.status == 429 {
                let wait = raw
                    .retry_after
                    .as_deref()
                    .and_then(|v| parse_retry_after(v, SystemTime::now()))
                    .or_else(|| reply.as_ref().and_then(retry_after_from_body));
                self.seams.bucket.on_rate_limited(profile, wait);
                // TODO(#491): the Genie REST ask routes get no G-10b retry here; the UC2 ask
                // retry is in R8 (ADR 0075).
                return Err(err(RATE_LIMITED));
            }
            if raw.status == 503 {
                // G-10a: a 503 `Retry-After` holds the profile, with no AIMD cut (#138).
                if let Some(wait) = raw
                    .retry_after
                    .as_deref()
                    .and_then(|v| parse_retry_after(v, SystemTime::now()))
                {
                    self.seams.bucket.hold(profile, wait);
                }
            }
            // Rule 6: a page never reaches TS and is never retried with the token.
            let Some(json) = reply else {
                return Err(err(NON_JSON_REPLY));
            };
            return match raw.status {
                200..=299 => Ok((scrub_json(&json), raw.org_id)),
                401 if !retried_401 => {
                    // Drop only the token that got the 401, not a newer one (W-05c).
                    self.seams.tokens.invalidate(profile, lease)?;
                    retried_401 = true;
                    continue;
                }
                401 => Err(err(AUTH_EXPIRED)),
                400..=499 => Err(err(UPSTREAM_REJECTED)),
                _ => Err(err(UPSTREAM_FAILED)),
            };
        }
    }
}

impl ReqwestHttpTransport {
    /// Rules 1-8 for one recording route (ADR 0103). Like [`Self::request_raw`], but the body
    /// may be raw bytes, a non-2xx JSON or empty reply comes back with its status (so the
    /// pipeline can map 403/404), and a `files.download` reply may be raw bytes.
    fn recording_raw(
        &self,
        profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: ReqBody<'_>,
        tags: Option<&Tags>,
        bounds: Option<CallBounds<'_>>,
    ) -> CoreResult<Reply> {
        let started = std::time::Instant::now();
        let route = routes::recording_route(route_id)?;
        let class = CallClass::Ask;
        let model = routes::RECORDING_MODEL_ROUTES.contains(&route_id);
        let headers = if model {
            // SEC-06a: the Unity Gateway route carries the tags; no tags, no send.
            let t = tags.ok_or(err(routes::INVALID_PARAMS))?;
            outbound_headers(&*self.seams.decorator, route_id, t)?
        } else {
            self.seams
                .decorator
                .headers(CallKind::Workspace, None)
                .map_err(|e| err(e.code()))?
        };
        let path = routes::build_path(route, params)?;
        let (origin, host) = self.origin(profile)?;
        let mut url = format!("{origin}{path}");
        let policy = &self.seams.policy;
        if !policy.may_send_token(&host, &url) {
            return Err(err(HOST_REFUSED));
        }
        let json_bytes = match body {
            ReqBody::Json(v) => {
                Some(serde_json::to_vec(v).map_err(|_| err(routes::INVALID_PARAMS))?)
            }
            _ => None,
        };
        let mut send_body = match body {
            ReqBody::None => None,
            ReqBody::Json(_) => json_bytes.as_deref().map(SendBody::Json),
            ReqBody::Bytes(b) => Some(SendBody::Raw(b)),
        };
        let mut limits = Limits {
            timeout: if model {
                Some(MODEL_TIMEOUT)
            } else {
                matches!(body, ReqBody::Bytes(_)).then_some(UPLOAD_TIMEOUT)
            },
            max_reply: if route_id == "files.download" {
                MAX_DOWNLOAD_BYTES
            } else {
                MAX_REPLY_BYTES
            },
        };
        let mut method = route.method;
        let mut retried_401 = false;
        let mut hops = 0u8;
        loop {
            match bounds {
                // ADR 0122: the bucket wait ends at the budget, and an abandoned call is not sent.
                Some(b) => {
                    let left = b.budget.saturating_sub(started.elapsed());
                    let stop = || b.abandoned.load(std::sync::atomic::Ordering::SeqCst);
                    let got = acquire_within_or_stop(
                        &*self.seams.bucket,
                        &*self.seams.pause,
                        profile,
                        class,
                        left,
                        &mut |_| {},
                        &stop,
                    );
                    let left = b.budget.saturating_sub(started.elapsed());
                    if got.is_none() || stop() || left.is_zero() {
                        return Err(err(TIMEOUT));
                    }
                    // The HTTP timeout of this attempt is what is left of the budget.
                    if model {
                        limits.timeout = Some(left.min(MODEL_TIMEOUT));
                    }
                }
                None => self.wait_for_bucket(profile, class),
            }
            let (raw, lease) = self.send_once(
                profile,
                &host,
                method,
                &url,
                &headers,
                (send_body, limits, bounds.map(|b| b.abandoned)),
            )?;
            if (300..400).contains(&raw.status) {
                let to = raw
                    .location
                    .as_deref()
                    .and_then(|l| Url::parse(&url).ok()?.join(l).ok())
                    .map(String::from)
                    .ok_or(err(REDIRECT_REFUSED))?;
                if hops >= MAX_REDIRECTS
                    || !policy.may_send_token(&host, &to)
                    || !policy.may_follow_redirect_with_authorization(&host, &url, &to)
                {
                    return Err(err(REDIRECT_REFUSED));
                }
                if matches!(raw.status, 301..=303) {
                    method = Method::Get;
                    send_body = None;
                }
                url = to;
                hops += 1;
                continue;
            }
            if raw.status == 429 {
                let reply = policy.reply_for_ts(raw.content_type.as_deref(), &raw.body);
                let wait = raw
                    .retry_after
                    .as_deref()
                    .and_then(|v| parse_retry_after(v, SystemTime::now()))
                    .or_else(|| reply.as_ref().and_then(retry_after_from_body));
                self.seams.bucket.on_rate_limited(profile, wait);
                return Err(err(RATE_LIMITED));
            }
            if raw.status == 401 {
                if retried_401 {
                    return Err(err(AUTH_EXPIRED));
                }
                self.seams.tokens.invalidate(profile, lease)?;
                retried_401 = true;
                continue;
            }
            if raw.status == 503 {
                if let Some(wait) = raw
                    .retry_after
                    .as_deref()
                    .and_then(|v| parse_retry_after(v, SystemTime::now()))
                {
                    self.seams.bucket.hold(profile, wait);
                }
            }
            if raw.body.is_empty() {
                return Ok(Reply {
                    status: raw.status,
                    body: ReplyBody::Empty,
                });
            }
            let octet = raw
                .content_type
                .as_deref()
                .is_some_and(|c| c.trim().starts_with("application/octet-stream"));
            if route_id == "files.download" && (200..300).contains(&raw.status) && octet {
                return Ok(Reply {
                    status: raw.status,
                    body: ReplyBody::Bytes(raw.body),
                });
            }
            // Rule 6: a page is never read as content and never retried with the token.
            let Some(json) = policy.reply_for_ts(raw.content_type.as_deref(), &raw.body) else {
                return Err(err(NON_JSON_REPLY));
            };
            return Ok(Reply {
                status: raw.status,
                body: ReplyBody::Json(scrub_json(&json)),
            });
        }
    }
}

impl RecordingHttp for ReqwestHttpTransport {
    fn send(
        &self,
        profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: ReqBody<'_>,
        tags: Option<&Tags>,
    ) -> CoreResult<Reply> {
        self.recording_raw(profile, route_id, params, body, tags, None)
    }

    fn send_bounded(
        &self,
        profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: ReqBody<'_>,
        tags: Option<&Tags>,
        bounds: CallBounds<'_>,
    ) -> CoreResult<Reply> {
        self.recording_raw(profile, route_id, params, body, tags, Some(bounds))
    }
}

impl ScimMe for ReqwestHttpTransport {
    fn org_id_header(&self, profile: &str) -> CoreResult<Option<String>> {
        let kind = route_call_kind("scim.me").ok_or(err(super::UNKNOWN_ROUTE))?;
        let headers = self
            .seams
            .decorator
            .headers(kind, None)
            .map_err(|e| err(e.code()))?;
        let (_, org_id) =
            self.request_raw(profile, "scim.me", &JsonObject::new(), None, headers)?;
        Ok(org_id)
    }
}

impl HttpTransport for ReqwestHttpTransport {
    fn request(
        &self,
        profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: Option<&Value>,
        tags: &Tags,
    ) -> CoreResult<Value> {
        let headers = outbound_headers(&*self.seams.decorator, route_id, tags)?;
        self.request_raw(profile, route_id, params, body, headers)
            .map(|(v, _)| v)
    }

    fn is_online(&self) -> CoreResult<bool> {
        Ok(self.online.load(Ordering::SeqCst))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SystemClock;
    use crate::http_transport::rate_bucket::AimdBucket;
    use crate::token_broker::StubTokenBroker;

    const FAKE: &str = "test-token-not-real";

    fn transport() -> ReqwestHttpTransport {
        ReqwestHttpTransport::new(
            Arc::new(StubProfileHosts),
            Arc::new(StubTokenBroker),
            Arc::new(AimdBucket::new(Arc::new(SystemClock::new()))),
        )
        .unwrap()
    }

    #[test]
    fn arc_6_token_never_shows_in_request_debug_output() {
        let t = transport();
        let req = t
            .build_request(
                FAKE.as_bytes(),
                Method::Post,
                "https://dbc-fake.cloud.databricks.com/api/2.0/x",
                &[("User-Agent", "bottle/0 (macos; mog)".into())],
                Some(SendBody::Json(b"{}")),
                None,
            )
            .unwrap();
        assert!(req.headers().contains_key(AUTHORIZATION));
        assert!(req.headers()[AUTHORIZATION].is_sensitive());
        let text = format!(
            "{req:?} {:?} {:?}",
            req.headers(),
            req.headers()[AUTHORIZATION]
        );
        assert!(!text.contains(FAKE), "{text}");
        assert!(!text.contains("Bearer"), "{text}");
    }

    #[test]
    fn arc_6_transport_debug_output_holds_no_seam_or_token() {
        let text = format!("{:?}", transport());
        assert_eq!(text, "ReqwestHttpTransport { online: true, .. }");
    }
}
