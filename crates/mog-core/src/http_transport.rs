//! R7 HttpTransport (section 4.3).
//!
//! Responsibilities: `reqwest` + `rustls`. Host allowlist, no credentialed cross-host redirect, request tags and User-Agent, shared per-profile bucket (40 calls/min, AIMD), 429/Retry-After, online/offline detection.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R7";

/// HTTPS to the workspace of a profile. Callers give a fixed route ID, never a URL (4.6 rule 1).
///
/// Every implementation must add the request identity headers of
/// [`crate::request_identity::RequestDecorator`] to each request: the Bottle User-Agent on
/// every call (SEC-06b) and the tags from the IPC `tags` argument on each Unity Gateway call
/// (SEC-06a). This trait method takes no `Tags` and no call kind, so the real transport needs
/// them from the IPC `tags` argument and a route-ID-to-[`crate::request_identity::CallKind`]
/// map: a trait change through the contract owner. TODO(#197): no issue owns the reqwest
/// wiring yet (#207 / PR #256 owns only the host policy and lists SEC-06a/b out of scope).
pub trait HttpTransport: Send + Sync {
    /// Send one request on a fixed route ID for `profile` and return the JSON reply.
    fn request(
        &self,
        profile: &str,
        route_id: &str,
        body: Option<&serde_json::Value>,
    ) -> CoreResult<serde_json::Value>;
    /// Report whether the network is online.
    fn is_online(&self) -> CoreResult<bool>;
}

/// The R7 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubHttpTransport;

impl HttpTransport for StubHttpTransport {
    fn request(
        &self,
        _profile: &str,
        _route_id: &str,
        _body: Option<&serde_json::Value>,
    ) -> CoreResult<serde_json::Value> {
        // TODO(#207): host allowlist, redirect and non-JSON page rules (rate bucket is #138).
        Err(CoreError::NotImplemented { component: ID })
    }

    fn is_online(&self) -> CoreResult<bool> {
        // TODO(#207): online/offline detection.
        Err(CoreError::NotImplemented { component: ID })
    }
}
