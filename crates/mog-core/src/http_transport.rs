//! R7 HttpTransport (section 4.3).
//!
//! Responsibilities: `reqwest` + `rustls`. Host allowlist, no credentialed cross-host redirect, request tags and User-Agent, shared per-profile bucket (40 calls/min, AIMD), 429/Retry-After, online/offline detection.

use crate::error::{CoreError, CoreResult};
use crate::request_identity::{CallKind, RequestDecorator};
use crate::types::Tags;

const ID: &str = "R7";

/// The fixed route map of R7 (#342, ADR 0033): every route ID that R7 sends, with its call
/// kind. The `ipc::RouteId` wire names (4.6) are workspace REST calls. The two Unity Gateway
/// model routes are `POST /ai-gateway/mlflow/v1/chat/completions` and `/embeddings`
/// (NFR-21a, D57); `model_invoke` and the cloud ASR path use them. Any other ID is refused.
pub const ROUTES: &[(&str, CallKind)] = &[
    ("scim.me", CallKind::Workspace),
    ("genie.start_conversation", CallKind::Workspace),
    ("genie.message", CallKind::Workspace),
    ("genie.get_message", CallKind::Workspace),
    ("genie.query_result", CallKind::Workspace),
    ("genie.feedback", CallKind::Workspace),
    ("genie.download_viz", CallKind::Workspace),
    ("genie.delete_conversation", CallKind::Workspace),
    ("genie_one.list_conversations", CallKind::Workspace),
    ("genie_one.delete_conversation", CallKind::Workspace),
    ("apps.invocations", CallKind::Workspace),
    ("apps.events", CallKind::Workspace),
    ("apps.responses", CallKind::Workspace),
    ("knowledge.create_page", CallKind::Workspace),
    ("knowledge.get_page", CallKind::Workspace),
    ("knowledge.list_domains", CallKind::Workspace),
    ("knowledge.update_page", CallKind::Workspace),
    ("knowledge.request_publish", CallKind::Workspace),
    ("postgres.credentials", CallKind::Workspace),
    ("mlflow.chat_completions", CallKind::UnityGateway),
    ("mlflow.embeddings", CallKind::UnityGateway),
];

/// The content-free code for a route ID that is not in [`ROUTES`].
pub const UNKNOWN_ROUTE: &str = "unknown_route";

/// The call kind of a route ID, or `None` if R7 does not send it (exact match only).
pub fn route_call_kind(route_id: &str) -> Option<CallKind> {
    ROUTES
        .iter()
        .find(|(r, _)| *r == route_id)
        .map(|(_, kind)| *kind)
}

/// The request identity headers for one R7 request (SEC-06a, SEC-06b). Every
/// [`HttpTransport`] implementation calls this (or [`RequestDecorator::decorate`] with the
/// kind from [`route_call_kind`]) for each request it sends, and sends nothing on `Err`.
/// Errors are content-free: `unknown_route` or a `tags_*` code of
/// [`crate::request_identity::IdentityError`].
pub fn outbound_headers(
    decorator: &dyn RequestDecorator,
    route_id: &str,
    tags: &Tags,
) -> CoreResult<Vec<(&'static str, String)>> {
    let kind = route_call_kind(route_id).ok_or(CoreError::Unavailable {
        component: ID,
        reason: UNKNOWN_ROUTE,
    })?;
    decorator
        .headers(kind, Some(tags))
        .map_err(|e| CoreError::Unavailable {
            component: ID,
            reason: e.code(),
        })
}

/// HTTPS to the workspace of a profile. Callers give a fixed route ID, never a URL (4.6 rule 1),
/// and the `tags` of the IPC call (`dbx_http`, `model_invoke`, `stt_transcribe`).
///
/// Every implementation must send the headers of [`outbound_headers`] on each request: the
/// Bottle User-Agent on every call (SEC-06b) and the tags on each Unity Gateway call
/// (SEC-06a). TODO(#364): the real `reqwest` + `rustls` client.
pub trait HttpTransport: Send + Sync {
    /// Send one request on a fixed route ID for `profile` and return the JSON reply.
    fn request(
        &self,
        profile: &str,
        route_id: &str,
        body: Option<&serde_json::Value>,
        tags: &Tags,
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
        _tags: &Tags,
    ) -> CoreResult<serde_json::Value> {
        // TODO(#207): host allowlist, redirect and non-JSON page rules (rate bucket is #138).
        Err(CoreError::NotImplemented { component: ID })
    }

    fn is_online(&self) -> CoreResult<bool> {
        // TODO(#207): online/offline detection.
        Err(CoreError::NotImplemented { component: ID })
    }
}
