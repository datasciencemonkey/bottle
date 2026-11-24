//! The orchestrator model's chat call (ADR 0122).
//!
//! Every orchestrator call goes to the home profile's Unity AI Gateway route
//! `POST /ai-gateway/mlflow/v1/chat/completions` (NFR-21a; never `/serving-endpoints`, NFR-21b).
//! This is the same route that the recording notes call uses (`recording_cloud::model`). The token
//! comes from the CLI token broker and never leaves the implementation.
//!
//! The trait is the frozen seam between the two Rust lanes of ADR 0122:
//! - the gateway lane implements it over `http_transport` (R7), and the `model_invoke` host uses it;
//! - the ask loop (`agent_run::orchestrator`) calls it with one tool schema, `ask_genie`.
//!
//! The IPC `model_invoke` command still refuses any request that has tool fields (RTR-20a,
//! [`crate::model_request_guard`]). Only the Rust ask loop sends tools, and it builds its tool
//! list itself.

use std::time::Duration;

use serde_json::Value;

use crate::error::CoreResult;

/// One chat-completions request to the orchestrator model.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest<'a> {
    /// The CLI profile whose AI Gateway runs the model: the home profile. Bottle never picks it.
    pub profile: &'a str,
    /// The model name on that gateway, for example `system.ai.claude-haiku-5-5`.
    pub model: &'a str,
    /// The chat-completions body without `model`: `messages`, plus `tools`, `response_format`,
    /// `max_tokens` and so on. The implementation adds `model`.
    pub body: Value,
    /// The deadline of this one call. The implementation cancels the request and returns an error
    /// when it passes.
    pub deadline: Duration,
}

/// The orchestrator model's chat call. Implementations must be content-free in logs: no prompt,
/// no reply and no token may appear in a log line, an error message or a trace.
pub trait ChatClient: Send + Sync {
    /// Send one request and return the parsed chat-completions JSON reply.
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
