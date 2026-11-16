use url::Url;

use crate::{classify_response, PolicyError, RedirectDecision, ResponseClass, TokenPolicy};

/// The most attempts (first try plus retries) for one request. Only a JSON
/// 502/503/504 is retried; a non-JSON page never is (SEC-05b).
pub const MAX_ATTEMPTS: u8 = 2;

/// One request handed to the injected sender. The sender attaches the token
/// only when `with_token` is true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundRequest {
    pub url: Url,
    pub with_token: bool,
}

/// A raw upstream response as seen by the transport.
#[derive(Clone, Default)]
pub struct RawResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// `Debug` never prints the location or the body.
impl std::fmt::Debug for RawResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawResponse")
            .field("status", &self.status)
            .field("content_type", &self.content_type)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

/// One content-free trace entry: what was sent and the status that came back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    pub host: String,
    pub with_token: bool,
    pub status: u16,
}

/// The result of [`execute`].
#[derive(Debug)]
pub struct ExecuteOutcome {
    /// The final verdict: the usable response class and body, or the refusal.
    pub result: Result<(ResponseClass, Vec<u8>), PolicyError>,
    /// The content-free trace of every request sent.
    pub trace: Vec<Attempt>,
}

/// Runs one token-bearing request through the policy with an injected `send`.
///
/// - The URL must pass [`TokenPolicy::check_request`]; otherwise nothing is sent.
/// - A 3xx is followed only when [`TokenPolicy::check_redirect`] allows it.
/// - A non-JSON page ends the request with `PROXY_PAGE` and drops the body.
/// - A JSON 502/503/504 is sent again, up to [`MAX_ATTEMPTS`] in total.
pub fn execute<F>(policy: &TokenPolicy, url: &str, mut send: F) -> ExecuteOutcome
where
    F: FnMut(&OutboundRequest) -> RawResponse,
{
    let mut trace = Vec::new();
    let result = run(policy, url, &mut send, &mut trace);
    ExecuteOutcome { result, trace }
}

fn run<F>(
    policy: &TokenPolicy,
    url: &str,
    send: &mut F,
    trace: &mut Vec<Attempt>,
) -> Result<(ResponseClass, Vec<u8>), PolicyError>
where
    F: FnMut(&OutboundRequest) -> RawResponse,
{
    policy.check_request(url)?;
    let mut url = Url::parse(url).map_err(|_| PolicyError::InvalidUrl)?;
    let mut hops = 0u8;
    let mut attempts = 0u8;
    loop {
        let req = OutboundRequest {
            url,
            with_token: true,
        };
        let resp = send(&req);
        url = req.url;
        trace.push(Attempt {
            host: url.host_str().unwrap_or_default().to_owned(),
            with_token: req.with_token,
            status: resp.status,
        });
        if is_redirect(resp.status) {
            let RedirectDecision::Follow(next) =
                policy.check_redirect(&url, resp.location.as_deref(), hops)?;
            url = next;
            hops += 1;
            continue;
        }
        let class = classify_response(resp.status, resp.content_type.as_deref(), &resp.body)?;
        attempts += 1;
        if class == ResponseClass::Json && is_transient(resp.status) && attempts < MAX_ATTEMPTS {
            continue;
        }
        return Ok((class, resp.body));
    }
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}

fn is_transient(status: u16) -> bool {
    matches!(status, 502..=504)
}
