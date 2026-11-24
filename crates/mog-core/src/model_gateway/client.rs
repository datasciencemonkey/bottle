//! `GatewayChatClient` (ADR 0122): the [`ChatClient`] over the existing R7 transport.
//!
//! One call is one `POST /ai-gateway/mlflow/v1/responses` (route `mlflow.responses`, the Responses API,
//! NFR-21a) through [`RecordingHttp`]. The real one is `ReqwestHttpTransport`, so the host rules
//! (workspace host policy, no cross-host redirect), the token broker (R6), the request tags and
//! the one shared rate bucket all apply. This module builds no URL and holds no token.
//!
//! Errors are `CoreError::Unavailable` with one of [`CODES`]. They hold no prompt, reply, token
//! or upstream message: the code comes from the HTTP status, the Databricks `error_code` or the
//! transport's own content-free code.

use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use super::{ChatClient, ChatRequest};
use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;
use crate::recording_cloud::http::{RecordingHttp, ReqBody};
use crate::types::{TagApp, TagOp, Tags};

const ID: &str = "model_gateway";
/// How often a waiting call looks at the cancel flag.
const CANCEL_POLL: Duration = Duration::from_millis(20);
/// The route ID of the Unity AI Gateway Responses call.
pub const RESPONSES_ROUTE: &str = "mlflow.responses";

/// The sign-in is missing or expired.
pub const SIGNED_OUT: &str = "signed_out";
/// The gateway does not serve the model, or the user cannot use it.
pub const MODEL_NOT_FOUND: &str = "model_not_found";
/// The call was refused: a host rule, a permission or a bad request.
pub const REFUSED: &str = "refused";
/// The gateway or the shared bucket asked Bottle to wait.
pub const RATE_LIMITED: &str = "rate_limited";
/// The deadline passed.
pub const TIMEOUT: &str = "timeout";
/// The gateway is not reachable.
pub const OFFLINE: &str = "offline";
/// The caller set the cancel flag (Stop, Wipe, session end). Stays inside Rust.
pub const CANCELLED: &str = "cancelled";
/// The reply is not a Responses reply.
pub const BAD_REPLY: &str = "bad_reply";

/// The complete set of error codes of this client.
pub const CODES: &[&str] = &[
    SIGNED_OUT,
    MODEL_NOT_FOUND,
    REFUSED,
    RATE_LIMITED,
    TIMEOUT,
    OFFLINE,
    BAD_REPLY,
    CANCELLED,
];

fn err(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// A transport fault (a content-free R7 code) as one of [`CODES`].
fn from_transport(e: &CoreError) -> CoreError {
    err(match e.code() {
        "auth_expired" | "token_unusable" | "login_required" => SIGNED_OUT,
        "rate_limited" => RATE_LIMITED,
        "timeout" => TIMEOUT,
        "offline" | "transport_failed" => OFFLINE,
        "non_json_reply" | "reply_too_large" => BAD_REPLY,
        _ => REFUSED,
    })
}

/// A non-2xx reply as one of [`CODES`]. Only the status and the `error_code` are read.
fn from_status(status: u16, error_code: &str) -> CoreError {
    err(match status {
        401 => SIGNED_OUT,
        404 => MODEL_NOT_FOUND,
        400 if error_code == "ENDPOINT_NOT_FOUND" => MODEL_NOT_FOUND,
        429 => RATE_LIMITED,
        408 | 504 => TIMEOUT,
        s if s >= 500 => OFFLINE,
        _ => REFUSED,
    })
}

/// The chat call of the orchestrator model over the gateway.
#[derive(Clone)]
pub struct GatewayChatClient {
    http: Arc<dyn RecordingHttp>,
    mog: String,
}

impl std::fmt::Debug for GatewayChatClient {
    /// Shows nothing: no profile, host or MOG ID.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayChatClient").finish_non_exhaustive()
    }
}

impl GatewayChatClient {
    /// A client over `http`. `mog` is the MOG ID of the request tags that [`ChatClient::chat`]
    /// sends (op `agent`); `model_invoke` sends the tags of the IPC call instead.
    pub fn new(http: Arc<dyn RecordingHttp>, mog: impl Into<String>) -> Self {
        Self {
            http,
            mog: mog.into(),
        }
    }

    /// As [`ChatClient::chat`], with the request `tags` of the IPC call (SEC-06a).
    pub fn chat_tagged(&self, req: ChatRequest<'_>, tags: Tags) -> CoreResult<Value> {
        let Value::Object(mut body) = req.body else {
            return Err(err(REFUSED));
        };
        body.insert("model".to_owned(), Value::String(req.model.to_owned()));
        body.insert("store".to_owned(), Value::Bool(false));
        let body = Value::Object(body);
        let profile = req.profile.to_owned();
        let http = Arc::clone(&self.http);
        // The deadline is the caller's, not the transport's 10 minute model timeout: the call runs
        // on its own thread, and the caller stops waiting when the deadline passes. The thread
        // ends with the transport's own timeout; its reply is dropped.
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("model-gateway-chat".to_owned())
            .spawn(move || {
                let r = http.send(
                    &profile,
                    RESPONSES_ROUTE,
                    &JsonObject::new(),
                    ReqBody::Json(&body),
                    Some(&tags),
                );
                let _ = tx.send(r);
            })
            .map_err(|_| err(OFFLINE))?;
        // Wait in short slices so a cancel is seen promptly.
        let end = Instant::now() + req.deadline;
        let reply = loop {
            if req.cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
                return Err(err(CANCELLED));
            }
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(err(TIMEOUT));
            }
            match rx.recv_timeout(left.min(CANCEL_POLL)) {
                Ok(r) => break r.map_err(|e| from_transport(&e))?,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(err(OFFLINE)),
            }
        };
        if !reply.ok() {
            return Err(from_status(reply.status, reply.error_code()));
        }
        let json = reply.json();
        // A Responses reply has an `output` array.
        match json.get("output").and_then(Value::as_array) {
            Some(_) => Ok(json.clone()),
            _ => Err(err(BAD_REPLY)),
        }
    }
}

impl ChatClient for GatewayChatClient {
    fn chat(&self, req: ChatRequest<'_>) -> CoreResult<Value> {
        let tags = Tags {
            app: TagApp::Bottle,
            mog: self.mog.clone(),
            op: TagOp::Agent,
        };
        self.chat_tagged(req, tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording_cloud::http::{Reply, ReplyBody, ScriptedHttp};
    use serde_json::json;
    use std::sync::atomic::AtomicBool;

    fn client(h: &Arc<ScriptedHttp>) -> GatewayChatClient {
        GatewayChatClient::new(h.clone(), "orchestrator")
    }

    fn req(body: Value, ms: u64) -> ChatRequest<'static> {
        ChatRequest {
            profile: "home",
            model: "system.ai.claude-haiku-5-5",
            body,
            deadline: Duration::from_millis(ms),
            cancel: None,
        }
    }

    fn ok_reply() -> Value {
        json!({"output": [{"type": "message", "content": [{"type": "output_text", "text": "hi"}]}]})
    }

    #[test]
    fn adr_0122_gateway_chat_sends_model_in_body_on_the_gateway_route() {
        let h = Arc::new(ScriptedHttp::new());
        h.json(RESPONSES_ROUTE, 200, ok_reply());
        let out = client(&h)
            .chat(req(
                json!({"messages": [{"role": "user", "content": "x"}]}),
                1000,
            ))
            .unwrap();
        assert_eq!(out, ok_reply());
        let seen = h.seen();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].route, "mlflow.responses");
        let body = seen[0].json.clone().unwrap();
        assert_eq!(body["model"], "system.ai.claude-haiku-5-5");
        assert_eq!(body["store"], false);
        assert!(body["messages"].is_array());
        let tags = seen[0].tags.clone().unwrap();
        assert_eq!((tags.mog.as_str(), tags.op), ("orchestrator", TagOp::Agent));
    }

    #[test]
    fn adr_0122_gateway_chat_maps_status_to_content_free_codes() {
        let cases: [(u16, &str, &str); 8] = [
            (401, "", SIGNED_OUT),
            (403, "PERMISSION_DENIED", REFUSED),
            (404, "NOT_FOUND", MODEL_NOT_FOUND),
            (400, "ENDPOINT_NOT_FOUND", MODEL_NOT_FOUND),
            (400, "BAD_REQUEST", REFUSED),
            (429, "", RATE_LIMITED),
            (504, "", TIMEOUT),
            (503, "", OFFLINE),
        ];
        for (status, code, want) in cases {
            let h = Arc::new(ScriptedHttp::new());
            h.json(
                RESPONSES_ROUTE,
                status,
                json!({"error_code": code, "message": "SECRET PROMPT TEXT"}),
            );
            let e = client(&h).chat(req(json!({}), 1000)).unwrap_err();
            assert_eq!(e.code(), want, "status {status}");
            let shown = format!("{e} {e:?}");
            assert!(!shown.contains("SECRET"));
        }
    }

    #[test]
    fn adr_0122_gateway_chat_maps_transport_faults() {
        let cases = [
            ("auth_expired", SIGNED_OUT),
            ("token_unusable", SIGNED_OUT),
            ("login_required", SIGNED_OUT),
            ("rate_limited", RATE_LIMITED),
            ("timeout", TIMEOUT),
            ("offline", OFFLINE),
            ("non_json_reply", BAD_REPLY),
            ("host_refused", REFUSED),
            ("redirect_refused", REFUSED),
        ];
        for (reason, want) in cases {
            let h = Arc::new(ScriptedHttp::new());
            h.on(
                RESPONSES_ROUTE,
                Err(CoreError::Unavailable {
                    component: "R7",
                    reason,
                }),
            );
            let e = client(&h).chat(req(json!({}), 1000)).unwrap_err();
            assert_eq!(e.code(), want, "{reason}");
            assert!(CODES.contains(&e.code()));
        }
    }

    #[test]
    fn adr_0122_gateway_chat_refuses_a_reply_that_is_not_a_chat_completion() {
        for body in [json!({"choices": []}), json!({"x": 1}), json!("text")] {
            let h = Arc::new(ScriptedHttp::new());
            h.json(RESPONSES_ROUTE, 200, body);
            let e = client(&h).chat(req(json!({}), 1000)).unwrap_err();
            assert_eq!(e.code(), BAD_REPLY);
        }
        let h = Arc::new(ScriptedHttp::new());
        h.on(
            RESPONSES_ROUTE,
            Ok(Reply {
                status: 200,
                body: ReplyBody::Empty,
            }),
        );
        assert_eq!(
            client(&h).chat(req(json!({}), 1000)).unwrap_err().code(),
            BAD_REPLY
        );
    }

    #[test]
    fn adr_0122_gateway_chat_refuses_a_body_that_is_not_an_object() {
        let h = Arc::new(ScriptedHttp::new());
        let e = client(&h).chat(req(json!([1]), 1000)).unwrap_err();
        assert_eq!(e.code(), REFUSED);
        assert!(h.seen().is_empty());
    }

    /// A transport that does not answer until the test lets it go.
    struct Stuck(std::sync::Mutex<mpsc::Receiver<()>>);
    impl RecordingHttp for Stuck {
        fn send(
            &self,
            _: &str,
            _: &str,
            _: &JsonObject,
            _: ReqBody<'_>,
            _: Option<&Tags>,
        ) -> CoreResult<Reply> {
            let _ = self.0.lock().unwrap().recv();
            Err(err(OFFLINE))
        }
    }

    #[test]
    fn adr_0122_gateway_chat_enforces_the_deadline() {
        let (release, wait) = mpsc::channel();
        let c = GatewayChatClient::new(Arc::new(Stuck(std::sync::Mutex::new(wait))), "m");
        let e = c.chat(req(json!({}), 30)).unwrap_err();
        assert_eq!(e.code(), TIMEOUT);
        drop(release);
    }

    #[test]
    fn adr_0122_gateway_chat_cancel_returns_promptly() {
        let (release, wait) = mpsc::channel();
        let c = GatewayChatClient::new(Arc::new(Stuck(std::sync::Mutex::new(wait))), "m");
        let flag = AtomicBool::new(false);
        let started = Instant::now();
        let e = std::thread::scope(|s| {
            s.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                flag.store(true, Ordering::SeqCst);
            });
            let mut r = req(json!({}), 60_000);
            r.cancel = Some(&flag);
            c.chat(r).unwrap_err()
        });
        assert_eq!(e.code(), CANCELLED);
        assert!(started.elapsed() < Duration::from_secs(5));
        drop(release);
    }
}
