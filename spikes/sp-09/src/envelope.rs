//! Error envelopes: a JSON-RPC `error` versus a `result` with `isError: true`,
//! plus the data-driven busy and clarify signals (rules in fixtures/signals.json).
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// The `result` of a `tools/call` (MCP `CallToolResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub raw: Value,
}

impl ToolResult {
    pub fn is_error(&self) -> bool {
        self.raw
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// All `text` content items, joined with a new line.
    pub fn text(&self) -> String {
        self.raw
            .get("content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter(|i| i.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|i| i.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// `structuredContent`, else the text content if it is one JSON object.
    pub fn structured(&self) -> Option<Value> {
        if let Some(sc) = self.raw.get("structuredContent").filter(|v| v.is_object()) {
            return Some(sc.clone());
        }
        serde_json::from_str::<Value>(self.text().trim())
            .ok()
            .filter(Value::is_object)
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
    /// Read one JSON-RPC response message.
    pub fn from_message(msg: &Value) -> Result<Self, String> {
        if let Some(err) = msg.get("error") {
            let e: RpcError = serde_json::from_value(err.clone())
                .map_err(|_| "the JSON-RPC error has no code or message".to_string())?;
            return Ok(ToolCallOutcome::RpcError(e));
        }
        match msg.get("result") {
            Some(r) if r.is_object() => {
                let tr = ToolResult { raw: r.clone() };
                Ok(if tr.is_error() {
                    ToolCallOutcome::ToolError(tr)
                } else {
                    ToolCallOutcome::Ok(tr)
                })
            }
            _ => Err("the message has no result object and no error".into()),
        }
    }

    pub fn kind(&self) -> EnvelopeKind {
        match self {
            ToolCallOutcome::Ok(_) => EnvelopeKind::Result,
            ToolCallOutcome::ToolError(_) => EnvelopeKind::IsError,
            ToolCallOutcome::RpcError(_) => EnvelopeKind::JsonRpcError,
        }
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

const BUILTIN: &str = include_str!("../fixtures/signals.json");

impl SignalRules {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn builtin() -> Self {
        Self::from_json(BUILTIN).expect("fixtures/signals.json is valid")
    }

    fn busy_match(&self, source: TextSource, text: &str) -> Option<Signal> {
        let lower = text.to_lowercase();
        self.busy
            .iter()
            .find(|r| r.sources.contains(&source) && lower.contains(&r.contains.to_lowercase()))
            .map(|r| Signal::ConversationBusy {
                rule_id: r.id.clone(),
                source,
            })
    }

    /// G-14/G-15a: busy text in a JSON-RPC error (message and data) or an isError result.
    pub fn classify_call(&self, outcome: &ToolCallOutcome) -> Option<Signal> {
        match outcome {
            ToolCallOutcome::RpcError(e) => {
                let data = e.data.as_ref().map(Value::to_string).unwrap_or_default();
                self.busy_match(TextSource::JsonRpcError, &format!("{} {}", e.message, data))
            }
            ToolCallOutcome::ToolError(r) => self.busy_match(TextSource::IsError, &r.text()),
            ToolCallOutcome::Ok(_) => None,
        }
    }

    pub fn classify_http_body(&self, body: &str) -> Option<Signal> {
        self.busy_match(TextSource::HttpBody, body)
    }

    /// G-22 on a poll result (`structuredContent`). An explicit field from the
    /// data wins; else: completed, short (< max_chars), ends with "?", no query items.
    pub fn clarify(&self, poll: &Value) -> Option<Signal> {
        let c = &self.clarify;
        for f in &c.explicit_fields {
            if let Some(q) = poll
                .get(f)
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            {
                return Some(Signal::Clarify {
                    question_chars: q.trim().chars().count(),
                    via: format!("explicit:{f}"),
                });
            }
        }
        if poll.get(&c.status_field).and_then(Value::as_str) != Some(c.completed_value.as_str()) {
            return None;
        }
        let answer = poll.get(&c.answer_field).and_then(Value::as_str)?.trim();
        let n = answer.chars().count();
        let has_items = poll
            .get(&c.query_items_field)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        (n > 0 && n < c.max_chars && answer.ends_with(c.ends_with.as_str()) && !has_items).then(
            || Signal::Clarify {
                question_chars: n,
                via: "heuristic".into(),
            },
        )
    }
}
