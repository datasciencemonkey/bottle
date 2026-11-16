//! Error envelopes: a JSON-RPC `error` versus a `result` with `isError: true`,
//! plus the data-driven busy and clarify signals.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub raw: Value,
}

impl ToolResult {
    pub fn is_error(&self) -> bool {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn text(&self) -> String {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn structured(&self) -> Option<Value> {
        unimplemented!("TODO(#29): green commit")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToolCallOutcome {
    Ok(ToolResult),
    ToolError(ToolResult),
    RpcError(RpcError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvelopeKind {
    Result,
    IsError,
    JsonRpcError,
}

impl ToolCallOutcome {
    pub fn from_message(_msg: &Value) -> Result<Self, String> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn kind(&self) -> EnvelopeKind {
        unimplemented!("TODO(#29): green commit")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextSource {
    JsonRpcError,
    IsError,
    HttpBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TextRule {
    pub id: String,
    pub contains: String,
    pub sources: Vec<TextSource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClarifyRule {
    pub status_field: String,
    pub completed_value: String,
    pub answer_field: String,
    pub query_items_field: String,
    pub max_chars: usize,
    pub ends_with: String,
    #[serde(default)]
    pub explicit_fields: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SignalRules {
    pub busy: Vec<TextRule>,
    pub clarify: ClarifyRule,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "signal", rename_all = "snake_case")]
pub enum Signal {
    ConversationBusy { rule_id: String, source: TextSource },
    Clarify { question_chars: usize, via: String },
}

impl SignalRules {
    pub fn from_json(_text: &str) -> Result<Self, serde_json::Error> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn builtin() -> Self {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn classify_call(&self, _outcome: &ToolCallOutcome) -> Option<Signal> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn classify_http_body(&self, _body: &str) -> Option<Signal> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn clarify(&self, _poll: &Value) -> Option<Signal> {
        unimplemented!("TODO(#29): green commit")
    }
}
