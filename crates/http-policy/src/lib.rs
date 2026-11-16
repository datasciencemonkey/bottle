//! Pure policy layer of Bottle's HttpTransport (component R7).
//!
//! This crate decides, without any network I/O:
//! - whether a bearer token may be attached to a request URL (SEC-04a),
//! - whether a redirect may be followed with the token (SEC-04b),
//! - whether an upstream response is a JSON API answer or a non-JSON page that
//!   must be reported as `PROXY_PAGE`, never shown, logged or retried (SEC-05a/b).
//!
//! See README.md for the API and the out-of-scope list.

mod error;
mod execute;
mod host;
mod response;

pub use error::PolicyError;
pub use execute::{execute, Attempt, ExecuteOutcome, OutboundRequest, RawResponse, MAX_ATTEMPTS};
pub use host::{RedirectDecision, TokenPolicy, TokenTarget, DATABRICKS_DOMAINS, MAX_REDIRECTS};
pub use response::{classify_response, ResponseClass, PROXY_PAGE_CARD_TEXT, PROXY_PAGE_CODE};
