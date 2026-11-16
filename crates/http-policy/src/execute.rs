use url::Url;

use crate::{PolicyError, ResponseClass, TokenPolicy, TokenTarget};

/// The most attempts (first try plus retries) for one request.
pub const MAX_ATTEMPTS: u8 = 2;

/// One request handed to the injected sender. The sender attaches the token
/// only when `with_token` is true.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboundRequest {
    pub url: Url,
    pub with_token: bool,
}

/// A raw upstream response as seen by the transport.
#[derive(Clone, Debug, Default)]
pub struct RawResponse {
    pub status: u16,
    pub content_type: Option<String>,
    pub location: Option<String>,
    pub body: Vec<u8>,
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
pub fn execute<F>(policy: &TokenPolicy, url: &str, send: F) -> ExecuteOutcome
where
    F: FnMut(&OutboundRequest) -> RawResponse,
{
    let _ = (policy, url, send, TokenTarget::ProfileHost);
    todo!("not implemented")
}
