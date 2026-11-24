//! The R7 route table (#364, ADR 0046): each route ID of [`super::ROUTES`] gets a fixed method
//! and a fixed path template (4.6 rule 1, NFR-21a).
//!
//! `params` fill only the named `{slot}`s of the template and the listed query keys. A path
//! value must be a short slug (`[A-Za-z0-9_-]{1,128}`), so it can never add a path segment, a
//! host, a query or a fragment. A query value is a short token or a non-negative integer and is
//! percent-encoded. An unknown, missing or bad param gives `invalid_params`.

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;

use super::{ID, UNKNOWN_ROUTE};

/// The content-free code for a missing, unknown or bad route param.
pub const INVALID_PARAMS: &str = "invalid_params";
/// The content-free code for a route ID of [`super::ROUTES`] that R7 does not send yet.
pub const ROUTE_NOT_WIRED: &str = "route_not_wired";

/// The longest path slot value.
pub const MAX_SLOT_LEN: usize = 128;
/// The longest query value.
pub const MAX_QUERY_LEN: usize = 1024;

/// The HTTP method of a route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`
    Get,
    /// `POST`
    Post,
    /// `PATCH`
    Patch,
    /// `DELETE`
    Delete,
    /// `PUT` (recording routes only: Files API upload and directory create, ADR 0103).
    Put,
}

/// One fixed route: method, path template and the query keys it accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Route {
    /// The route ID (a key of [`super::ROUTES`]).
    pub id: &'static str,
    /// The fixed method.
    pub method: Method,
    /// The path template, with `{slot}` names for path params.
    pub path: &'static str,
    /// The optional query keys, in the order they go on the wire.
    pub query: &'static [&'static str],
}

const fn r(id: &'static str, method: Method, path: &'static str) -> Route {
    Route {
        id,
        method,
        path,
        query: &[],
    }
}

const PAGING: &[&str] = &["page_size", "page_token"];
const MSG: &str = "/api/2.0/genie/spaces/{space_id}/conversations/{conversation_id}/messages";

/// The wired routes. Paths come from section 4.6, NFR-21a, PG-20 and the Databricks REST
/// reference pages that the requirements link (Genie conversation API, Genie One API,
/// Unity Catalog Pages, postgres credentials).
pub const ROUTE_TABLE: &[Route] = &[
    r("scim.me", Method::Get, "/api/2.0/preview/scim/v2/Me"),
    r(
        "genie.start_conversation",
        Method::Post,
        "/api/2.0/genie/spaces/{space_id}/start-conversation",
    ),
    r("genie.message", Method::Post, MSG),
    r(
        "genie.get_message",
        Method::Get,
        "/api/2.0/genie/spaces/{space_id}/conversations/{conversation_id}/messages/{message_id}",
    ),
    r(
        "genie.query_result",
        Method::Get,
        "/api/2.0/genie/spaces/{space_id}/conversations/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/query-result",
    ),
    r(
        "genie.feedback",
        Method::Post,
        "/api/2.0/genie/spaces/{space_id}/conversations/{conversation_id}/messages/{message_id}/feedback",
    ),
    // The reply is a PNG, so rule 6 refuses it with `non_json_reply` (ADR 0046).
    // TODO(#411): a binary reply for this route (G-42), through the contract owner.
    r(
        "genie.download_viz",
        Method::Get,
        "/api/2.0/genie/spaces/{space_id}/conversations/{conversation_id}/messages/{message_id}/attachments/{attachment_id}/download-visualization",
    ),
    r(
        "genie.delete_conversation",
        Method::Delete,
        "/api/2.0/genie/spaces/{space_id}/conversations/{conversation_id}",
    ),
    Route {
        id: "genie_one.list_conversations",
        method: Method::Get,
        path: "/api/2.0/genie/one/conversations",
        query: PAGING,
    },
    r(
        "genie_one.delete_conversation",
        Method::Delete,
        "/api/2.0/genie/one/conversations/{conversation_id}",
    ),
    r(
        "knowledge.create_page",
        Method::Post,
        "/api/2.0/knowledge/pages",
    ),
    r(
        "knowledge.get_page",
        Method::Get,
        "/api/2.0/knowledge/pages/{page_id}",
    ),
    Route {
        id: "knowledge.list_domains",
        method: Method::Get,
        path: "/api/2.0/domains",
        query: PAGING,
    },
    Route {
        id: "knowledge.update_page",
        method: Method::Patch,
        path: "/api/2.0/knowledge/pages/{page_id}",
        query: &["update_mask"],
    },
    r(
        "postgres.credentials",
        Method::Post,
        "/api/2.0/postgres/credentials",
    ),
    r(
        "mlflow.chat_completions",
        Method::Post,
        "/ai-gateway/mlflow/v1/chat/completions",
    ),
    r(
        "mlflow.responses",
        Method::Post,
        "/ai-gateway/mlflow/v1/responses",
    ),
    r(
        "mlflow.embeddings",
        Method::Post,
        "/ai-gateway/mlflow/v1/embeddings",
    ),
];

/// Route IDs of [`super::ROUTES`] that R7 refuses with `route_not_wired`.
/// - `apps.*`: the app host is `*.databricksapps.com`, not the profile host (4.6 rule 1
///   exception). TODO(#158): the custom_agent client owns the app host check.
/// - `knowledge.request_publish`: no publish endpoint is documented (publish is an
///   `update_mask=state` PATCH, and B3 allows DRAFT Pages only). TODO(#219).
pub const NOT_WIRED: &[&str] = &[
    "apps.invocations",
    "apps.events",
    "apps.responses",
    "knowledge.request_publish",
];

fn err(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// The recording routes (ADR 0103, PR C): the Files API, Unity Catalog volume create and SCIM
/// `Me`. They are NOT in [`super::ROUTES`], so `dbx_http` from the brain can never reach them;
/// only the Rust recording pipeline ([`crate::recording_cloud`]) sends them, through
/// [`recording_route`]. Its model calls use the existing `mlflow.chat_completions` route
/// ([`RECORDING_MODEL_ROUTES`]). A `{*path}` slot takes a validated volume path (see
/// [`is_volume_path`]), never a host, query or fragment.
pub const RECORDING_ROUTE_TABLE: &[Route] = &[
    r("recording.me", Method::Get, "/api/2.0/preview/scim/v2/Me"),
    Route {
        id: "files.upload",
        method: Method::Put,
        path: "/api/2.0/fs/files{*path}",
        query: &["overwrite"],
    },
    r("files.download", Method::Get, "/api/2.0/fs/files{*path}"),
    r("files.delete", Method::Delete, "/api/2.0/fs/files{*path}"),
    Route {
        id: "files.list_directory",
        method: Method::Get,
        path: "/api/2.0/fs/directories{*path}",
        query: &["page_token"],
    },
    r(
        "files.create_directory",
        Method::Put,
        "/api/2.0/fs/directories{*path}",
    ),
    r(
        "volumes.create",
        Method::Post,
        "/api/2.1/unity-catalog/volumes",
    ),
];

/// The IPC routes of [`ROUTE_TABLE`] that the recording pipeline may also send (with a longer
/// timeout): the Unity Gateway chat route (NFR-21a; never `/serving-endpoints`, NFR-21b), and the
/// Responses route of the orchestrator model (ADR 0122), which the Rust `model_gateway` client
/// sends through the same call.
pub const RECORDING_MODEL_ROUTES: &[&str] = &["mlflow.chat_completions", "mlflow.responses"];

/// The fixed recording route of `route_id` (a [`RECORDING_ROUTE_TABLE`] entry or a
/// [`RECORDING_MODEL_ROUTES`] entry), or `unknown_route`.
pub fn recording_route(route_id: &str) -> CoreResult<&'static Route> {
    if RECORDING_MODEL_ROUTES.contains(&route_id) {
        return route(route_id);
    }
    RECORDING_ROUTE_TABLE
        .iter()
        .find(|r| r.id == route_id)
        .ok_or(err(UNKNOWN_ROUTE))
}

/// The longest volume path.
pub const MAX_VOLUME_PATH_LEN: usize = 1024;

/// A volume path that may fill a `{*path}` slot: `/Volumes/<catalog>/<schema>/<volume>[/...]`,
/// only `[A-Za-z0-9_.@-]` and `/`, no empty, `.` or `..` segment (a trailing `/` is allowed).
pub fn is_volume_path(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("/Volumes/") else {
        return false;
    };
    if s.len() > MAX_VOLUME_PATH_LEN
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.@-/".contains(&b))
    {
        return false;
    }
    let body = rest.strip_suffix('/').unwrap_or(rest);
    let segs: Vec<&str> = body.split('/').collect();
    segs.len() >= 3
        && segs
            .iter()
            .all(|g| !g.is_empty() && *g != "." && *g != "..")
}

/// The fixed route of `route_id`: `unknown_route` if it is not in [`super::ROUTES`],
/// `route_not_wired` if R7 does not send it yet.
pub fn route(route_id: &str) -> CoreResult<&'static Route> {
    if super::route_call_kind(route_id).is_none() {
        return Err(err(UNKNOWN_ROUTE));
    }
    ROUTE_TABLE
        .iter()
        .find(|r| r.id == route_id)
        .ok_or(err(ROUTE_NOT_WIRED))
}

fn is_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SLOT_LEN
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn query_value(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s)
            if !s.is_empty()
                && s.len() <= MAX_QUERY_LEN
                && s.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._~+/=,-".contains(&b)) =>
        {
            Some(percent_encode(s))
        }
        serde_json::Value::Number(n) => n.as_u64().map(|n| n.to_string()),
        _ => None,
    }
}

/// Percent-encode every byte that is not unreserved (RFC 3986).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The path and query of `route` with `params` placed. Every param must be a named slot or a
/// listed query key; every slot must be filled.
pub fn build_path(route: &Route, params: &JsonObject) -> CoreResult<String> {
    let mut used = 0usize;
    let mut out = String::with_capacity(route.path.len() + 64);
    let mut rest = route.path;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').ok_or(err(INVALID_PARAMS))? + open;
        let raw_name = &rest[open + 1..close];
        let (name, is_path) = match raw_name.strip_prefix('*') {
            Some(n) => (n, true),
            None => (raw_name, false),
        };
        let value = params
            .get(name)
            .and_then(|v| v.as_str())
            .filter(|s| {
                if is_path {
                    is_volume_path(s)
                } else {
                    is_slug(s)
                }
            })
            .ok_or(err(INVALID_PARAMS))?;
        out.push_str(value);
        used += 1;
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    let mut sep = '?';
    for key in route.query {
        if let Some(v) = params.get(*key) {
            let v = query_value(v).ok_or(err(INVALID_PARAMS))?;
            out.push(sep);
            out.push_str(key);
            out.push('=');
            out.push_str(&v);
            sep = '&';
            used += 1;
        }
    }
    if used != params.len() {
        return Err(err(INVALID_PARAMS));
    }
    Ok(out)
}
