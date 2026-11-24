//! The orchestrator model's call (ADR 0122).
//!
//! Every orchestrator call goes to the home profile's Unity AI Gateway **Responses** route,
//! `POST /ai-gateway/mlflow/v1/responses` (NFR-21a; never `/serving-endpoints`, NFR-21b). Model ids
//! are `system.ai.*` only. The token comes from the CLI token broker and never leaves the
//! implementation.
//!
//! These are live facts from DEFAULT on 2026-11-24, using synthetic prompts:
//! - `system.ai.claude-haiku-5-5` returns `output[]` items: `message`/`output_text` and
//!   `function_call {name, arguments: <JSON string>, call_id}`.
//! - `text.format` json_schema is refused for Anthropic models ("structured output … requires
//!   forced tool use"), and `json_object` is refused too. Callers must not depend on structured
//!   output. The ask loop ends with a terminal `submit_answer` tool. The router asks for JSON in
//!   its instructions and validates the reply itself.
//! - One short call takes about 1.3 s to 2.4 s end to end.
//!
//! The trait is the frozen seam between the two Rust lanes of ADR 0122:
//! - the gateway lane implements it over `http_transport` (R7), and the `model_invoke` host uses it;
//! - the ask loop (`agent_run::orchestrator`) calls it with its fixed tool list.
//!
//! The IPC `model_invoke` command still refuses any request that has tool fields (RTR-20a,
//! [`crate::model_request_guard`]). Only the Rust ask loop sends tools, and it builds its tool
//! list itself.

use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;

use crate::error::CoreResult;

pub mod client;
pub mod invoke;
pub mod models;
pub mod responses;

/// One Responses-API request to the orchestrator model.
#[derive(Debug, Clone)]
pub struct ChatRequest<'a> {
    /// The CLI profile whose AI Gateway runs the model: the home profile. Bottle never picks it.
    pub profile: &'a str,
    /// The model id on that gateway, for example `system.ai.claude-haiku-5-5`.
    pub model: &'a str,
    /// The Responses body without `model`: `instructions`, `input`, `tools`, `max_output_tokens`
    /// and so on. The implementation adds `model` and `"store": false`.
    pub body: Value,
    /// The deadline of this one call. The implementation returns `timeout` when it passes.
    pub deadline: Duration,
    /// Set by the caller to abandon the call (Stop, Wipe, session end). The implementation must
    /// return `cancelled` promptly and drop the request. It must not wait for the full deadline.
    pub cancel: Option<&'a AtomicBool>,
}

/// The orchestrator model's call. Implementations must be content-free in logs: no prompt,
/// no reply and no token may appear in a log line, an error message or a trace.
pub trait ChatClient: Send + Sync {
    /// Send one request and return the parsed Responses JSON reply.
    fn chat(&self, req: ChatRequest<'_>) -> CoreResult<Value>;
}

impl<T: ChatClient + ?Sized> ChatClient for std::sync::Arc<T> {
    fn chat(&self, req: ChatRequest<'_>) -> CoreResult<Value> {
        (**self).chat(req)
    }
}

/// The default orchestrator model (owner decision, 2026-11-24).
pub const DEFAULT_ORCHESTRATOR_MODEL: &str = "system.ai.claude-haiku-5-5";
/// The fallback when the default is not served or has not passed its eval.
pub const FALLBACK_ORCHESTRATOR_MODEL: &str = "system.ai.gpt-oss-20b";
