//! Helpers for the Responses API body of the orchestrator model (ADR 0122).
//!
//! - [`output_text`], [`function_calls`] and [`usage`] read a reply of
//!   `POST /ai-gateway/mlflow/v1/responses`.
//! - [`from_chat_request`] turns a chat-completions request of the router into a Responses body
//!   (the brain's router request is in chat shape): `messages` become `input`,
//!   `response_format` becomes `text.format`, and `max_tokens` becomes `max_output_tokens`.
//!   Tool fields are never translated: the caller refuses them first.

use serde_json::{json, Map, Value};

/// One `function_call` item of a Responses reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    /// The `call_id` to send back with the tool result.
    pub call_id: String,
    /// The tool name.
    pub name: String,
    /// The arguments: a JSON text, as the API sends it.
    pub arguments: String,
}

/// The token counts of a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    /// Tokens of the input.
    pub input_tokens: u64,
    /// Tokens of the output.
    pub output_tokens: u64,
}

/// The text of a reply: every `output_text` part of every `message` item, in order. `None` when
/// there is no text (a reply that has only function calls). A top-level `output_text` string, when
/// the API gives one, is used if the items give none.
pub fn output_text(reply: &Value) -> Option<String> {
    let mut out = String::new();
    for item in reply.get("output").and_then(Value::as_array)? {
        if item.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let parts = item.get("content").and_then(Value::as_array);
        for p in parts.into_iter().flatten() {
            if p.get("type").and_then(Value::as_str) == Some("output_text") {
                if let Some(t) = p.get("text").and_then(Value::as_str) {
                    out.push_str(t);
                }
            }
        }
    }
    if out.is_empty() {
        return reply
            .get("output_text")
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned);
    }
    Some(out)
}

/// The `function_call` items of a reply, in order. An item without a name or a call ID is skipped.
pub fn function_calls(reply: &Value) -> Vec<FunctionCall> {
    reply
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("function_call"))
        .filter_map(|i| {
            let s = |k: &str| i.get(k).and_then(Value::as_str).map(str::to_owned);
            Some(FunctionCall {
                call_id: s("call_id").or_else(|| s("id"))?,
                name: s("name")?,
                arguments: s("arguments").unwrap_or_else(|| "{}".to_owned()),
            })
        })
        .collect()
}

/// The token counts of a reply (zero when it gives none).
pub fn usage(reply: &Value) -> Usage {
    let n = |k: &str| {
        reply
            .get("usage")
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Usage {
        input_tokens: n("input_tokens"),
        output_tokens: n("output_tokens"),
    }
}

/// The text of a chat `content`: a string, or the `text` parts of a part list.
fn chat_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// The `text.format` of a chat `response_format`, or `None` for one that Bottle does not send.
fn text_format(rf: &Value) -> Option<Value> {
    match rf.get("type").and_then(Value::as_str)? {
        "text" => Some(json!({"type": "text"})),
        "json_object" => Some(json!({"type": "json_object"})),
        "json_schema" => {
            let js = rf.get("json_schema")?;
            let mut f = Map::new();
            f.insert("type".to_owned(), json!("json_schema"));
            for k in ["name", "schema", "strict"] {
                if let Some(v) = js.get(k) {
                    f.insert(k.to_owned(), v.clone());
                }
            }
            Some(Value::Object(f))
        }
        _ => None,
    }
}

/// The Responses body of a router request. A request that has `input` is already a Responses body
/// and goes through as it is (without `model`, which the client adds). Only the fields below are
/// kept; any other field is dropped.
pub fn from_chat_request(request: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    if request.contains_key("input") {
        out.extend(
            request
                .iter()
                .filter(|(k, _)| *k != "model")
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        return out;
    }
    let mut input = Vec::new();
    let mut instructions = Vec::new();
    for m in request
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = chat_text(m.get("content").unwrap_or(&Value::Null));
        match m.get("role").and_then(Value::as_str) {
            Some("system" | "developer") => instructions.push(text),
            Some("assistant") => input.push(json!({
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}],
            })),
            _ => input.push(json!({
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            })),
        }
    }
    out.insert("input".to_owned(), Value::Array(input));
    if !instructions.is_empty() {
        out.insert("instructions".to_owned(), json!(instructions.join("\n\n")));
    }
    if let Some(n) = request
        .get("max_completion_tokens")
        .or_else(|| request.get("max_tokens"))
    {
        out.insert("max_output_tokens".to_owned(), n.clone());
    }
    for k in ["temperature", "top_p"] {
        if let Some(v) = request.get(k) {
            out.insert(k.to_owned(), v.clone());
        }
    }
    if let Some(f) = request.get("response_format").and_then(text_format) {
        out.insert("text".to_owned(), json!({ "format": f }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn adr_0122_responses_reads_text_calls_and_usage() {
        let r = json!({
            "output": [
                {"type": "reasoning", "summary": []},
                {"type": "message", "content": [
                    {"type": "output_text", "text": "a"},
                    {"type": "output_text", "text": "b"}]},
                {"type": "function_call", "call_id": "c1", "name": "ask_genie",
                 "arguments": "{\"workspace\":\"w\"}"},
                {"type": "function_call", "name": "no_id"},
            ],
            "usage": {"input_tokens": 7, "output_tokens": 3},
        });
        assert_eq!(output_text(&r).as_deref(), Some("ab"));
        assert_eq!(
            function_calls(&r),
            vec![FunctionCall {
                call_id: "c1".into(),
                name: "ask_genie".into(),
                arguments: "{\"workspace\":\"w\"}".into(),
            }]
        );
        assert_eq!(
            usage(&r),
            Usage {
                input_tokens: 7,
                output_tokens: 3
            }
        );
    }

    #[test]
    fn adr_0122_responses_helpers_handle_a_reply_with_no_text_or_usage() {
        let r = json!({"output": [{"type": "function_call", "call_id": "c", "name": "n"}]});
        assert_eq!(output_text(&r), None);
        assert_eq!(function_calls(&r)[0].arguments, "{}");
        assert_eq!(usage(&r), Usage::default());
        assert_eq!(
            output_text(&json!({"output_text": "x", "output": []})).as_deref(),
            Some("x")
        );
        assert_eq!(output_text(&json!("x")), None);
    }

    #[test]
    fn adr_0122_responses_translates_a_chat_request() {
        let chat = obj(json!({
            "model": "evil",
            "messages": [
                {"role": "system", "content": "be brief"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "hello"},
                {"role": "user", "content": [{"type": "text", "text": "bye"}]},
            ],
            "response_format": {"type": "json_schema",
                "json_schema": {"name": "r", "schema": {"type": "object"}, "strict": true}},
            "max_tokens": 120,
            "temperature": 0,
            "logit_bias": {"1": 1},
        }));
        let r = from_chat_request(&chat);
        assert_eq!(
            Value::Object(r),
            json!({
                "instructions": "be brief",
                "input": [
                    {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                    {"role": "assistant", "content": [{"type": "output_text", "text": "hello"}]},
                    {"role": "user", "content": [{"type": "input_text", "text": "bye"}]},
                ],
                "max_output_tokens": 120,
                "temperature": 0,
                "text": {"format": {"type": "json_schema", "name": "r",
                    "schema": {"type": "object"}, "strict": true}},
            })
        );
    }

    #[test]
    fn adr_0122_responses_json_object_and_passthrough() {
        let r = from_chat_request(&obj(json!({
            "messages": [{"role": "user", "content": "x"}],
            "response_format": {"type": "json_object"},
            "max_completion_tokens": 5,
        })));
        assert_eq!(r["text"], json!({"format": {"type": "json_object"}}));
        assert_eq!(r["max_output_tokens"], 5);
        let own = from_chat_request(&obj(
            json!({"model": "m", "input": [], "max_output_tokens": 9}),
        ));
        assert_eq!(
            Value::Object(own),
            json!({"input": [], "max_output_tokens": 9})
        );
    }
}
