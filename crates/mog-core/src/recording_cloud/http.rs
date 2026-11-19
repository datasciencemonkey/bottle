//! The HTTP seam of the recording pipeline (ADR 0103): one call on a fixed recording route
//! (`http_transport::routes::RECORDING_ROUTE_TABLE`), with a JSON or raw body, and the HTTP
//! status kept so the pipeline can map 403/404 and the `error_code` to plain errors.
//!
//! The real one is `ReqwestHttpTransport` (R7): same host rules, token broker, rate bucket,
//! redirect and page rules as every other R7 call. Transport faults (offline, timeout, a 401
//! after one retry, a 429, an HTML page) stay [`CoreError`]s with content-free codes.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde_json::Value;

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::types::Tags;

/// The body of one recording call.
#[derive(Debug, Clone, Copy)]
pub enum ReqBody<'a> {
    /// No body.
    None,
    /// A JSON body (`application/json`).
    Json(&'a Value),
    /// Raw bytes (`application/octet-stream`): a Files API upload.
    Bytes(&'a [u8]),
}

/// The body of one reply.
#[derive(Debug, Clone, PartialEq)]
pub enum ReplyBody {
    /// No body (for example a `204` upload reply).
    Empty,
    /// A JSON body (scrubbed by R7).
    Json(Value),
    /// Raw bytes: a `files.download` reply with `application/octet-stream`.
    Bytes(Vec<u8>),
}

/// One reply: any status except 3xx, 401 and 429 (R7 handles those).
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    /// The HTTP status.
    pub status: u16,
    /// The body.
    pub body: ReplyBody,
}

impl Reply {
    /// A 2xx status.
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
    /// The JSON body, or `Value::Null`.
    pub fn json(&self) -> &Value {
        static NULL: Value = Value::Null;
        match &self.body {
            ReplyBody::Json(v) => v,
            _ => &NULL,
        }
    }
    /// The `error_code` of a Databricks error body, or `""`.
    pub fn error_code(&self) -> &str {
        self.json()
            .get("error_code")
            .and_then(Value::as_str)
            .unwrap_or("")
    }
    /// The `message` of a Databricks error body, or `""`. Never log it.
    pub fn message(&self) -> &str {
        self.json()
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
    }
}

/// One call on a fixed recording route for `profile`. `params` fill the route's slots and
/// query keys (a `{*path}` slot takes a validated volume path). `tags` go on the Unity Gateway
/// model route only (SEC-06a); the workspace routes take `None`.
pub trait RecordingHttp: Send + Sync {
    /// Send one call and return its reply.
    fn send(
        &self,
        profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: ReqBody<'_>,
        tags: Option<&Tags>,
    ) -> CoreResult<Reply>;
}

/// One call that [`ScriptedHttp`] saw.
#[derive(Debug, Clone, PartialEq)]
pub struct SeenCall {
    /// The route ID.
    pub route: String,
    /// The params.
    pub params: JsonObject,
    /// The JSON body, if any.
    pub json: Option<Value>,
    /// The raw body, if any.
    pub bytes: Option<Vec<u8>>,
    /// The tags, if any.
    pub tags: Option<Tags>,
}

/// A test double: answers each call with the next scripted reply for its route, in order.
/// A route with no reply left answers `CoreError::Unavailable { reason: "no_script" }`.
#[derive(Debug, Default)]
pub struct ScriptedHttp {
    replies: Mutex<Vec<(String, VecDeque<CoreResult<Reply>>)>>,
    seen: Mutex<Vec<SeenCall>>,
}

impl ScriptedHttp {
    /// An empty script.
    pub fn new() -> Self {
        Self::default()
    }
    /// Add a reply for `route`.
    pub fn on(&self, route: &str, reply: CoreResult<Reply>) -> &Self {
        let mut r = self.replies.lock().unwrap_or_else(|e| e.into_inner());
        match r.iter_mut().find(|(k, _)| k == route) {
            Some((_, q)) => q.push_back(reply),
            None => r.push((route.to_owned(), VecDeque::from([reply]))),
        }
        self
    }
    /// Add a JSON reply.
    pub fn json(&self, route: &str, status: u16, v: Value) -> &Self {
        self.on(
            route,
            Ok(Reply {
                status,
                body: ReplyBody::Json(v),
            }),
        )
    }
    /// Add an empty reply.
    pub fn empty(&self, route: &str, status: u16) -> &Self {
        self.on(
            route,
            Ok(Reply {
                status,
                body: ReplyBody::Empty,
            }),
        )
    }
    /// Add a raw bytes reply.
    pub fn bytes(&self, route: &str, b: &[u8]) -> &Self {
        self.on(
            route,
            Ok(Reply {
                status: 200,
                body: ReplyBody::Bytes(b.to_vec()),
            }),
        )
    }
    /// Every call seen, in order.
    pub fn seen(&self) -> Vec<SeenCall> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    /// The calls seen on `route`.
    pub fn seen_on(&self, route: &str) -> Vec<SeenCall> {
        self.seen()
            .into_iter()
            .filter(|c| c.route == route)
            .collect()
    }
}

impl RecordingHttp for ScriptedHttp {
    fn send(
        &self,
        _profile: &str,
        route_id: &str,
        params: &JsonObject,
        body: ReqBody<'_>,
        tags: Option<&Tags>,
    ) -> CoreResult<Reply> {
        let (json, bytes) = match body {
            ReqBody::None => (None, None),
            ReqBody::Json(v) => (Some(v.clone()), None),
            ReqBody::Bytes(b) => (None, Some(b.to_vec())),
        };
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SeenCall {
                route: route_id.to_owned(),
                params: params.clone(),
                json,
                bytes,
                tags: tags.cloned(),
            });
        let mut r = self.replies.lock().unwrap_or_else(|e| e.into_inner());
        r.iter_mut()
            .find(|(k, _)| k == route_id)
            .and_then(|(_, q)| q.pop_front())
            .unwrap_or(Err(CoreError::Unavailable {
                component: "test",
                reason: "no_script",
            }))
    }
}
