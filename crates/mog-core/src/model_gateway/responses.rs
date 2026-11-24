//! Helpers for the Responses API body of the orchestrator model (ADR 0122).
//!
//! - [`output_text`], [`function_calls`] and [`usage`] read a reply of
//!   `POST /ai-gateway/mlflow/v1/responses`.
//! - [`from_chat_request`] turns a chat-completions request of the router into a Responses body
//!   (the brain's router request is in chat shape): `messages` become `input`,
//!   `response_format` becomes a line of `instructions`, and `max_tokens` becomes `max_output_tokens`.
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

/// The text of a chat `content`: a string, or a list of `text` parts and nothing else. `None`
/// for any other part (an image, an audio part, a tool part).
fn chat_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => parts
            .iter()
            .map(|p| {
                let is_text = p.get("type").and_then(Value::as_str) == Some("text");
                is_text
                    .then(|| p.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()
            .map(|t| t.join("")),
        _ => None,
    }
}

/// The instruction line of a chat `response_format`, or `None` for a plain text one. Structured
/// output (`text.format`) is refused by newer Anthropic models, so the schema goes into the
/// instructions and the brain's router step validates the reply.
fn format_instruction(rf: &Value) -> Option<String> {
    match rf.get("type").and_then(Value::as_str)? {
        "json_object" => Some("Return only a JSON object.".to_owned()),
        "json_schema" => {
            let schema = rf.get("json_schema")?.get("schema")?;
            Some(format!(
                "Return only a JSON object that matches this schema: {schema}"
            ))
        }
        _ => None,
    }
}

/// The chat-completions shape of a Responses reply: `{choices:[{message:{content:<text>}}]}`.
/// The brain reads `model_invoke` replies in this shape (`messageContent`). `None` when
/// [`checked_text`] refuses the reply.
pub fn to_chat_reply(reply: &Value) -> Option<Value> {
    let content = checked_text(reply)?;
    Some(json!({"choices": [{"message": {"role": "assistant", "content": content}}]}))
}

/// The top-level fields of a chat request that Bottle sends. Any other field is refused: the
/// request is an allowlist, never a passthrough (RTR-20a, ADR 0122). `model` is accepted and
/// ignored; `stream` only as `false`.
const CHAT_FIELDS: &[&str] = &[
    "model",
    "messages",
    "temperature",
    "top_p",
    "max_tokens",
    "max_completion_tokens",
    "response_format",
    "stream",
];

/// The Responses body of a chat-shaped request, or `None` if the request is not a plain chat
/// request: an unknown field, a role other than system, developer, user or assistant, a message
/// with a field other than `role` and `content`, or a content part that is not text. Every
/// content part of the result is `input_text` or `output_text`, and every `input` item is a
/// message.
pub fn from_chat_request(request: &Map<String, Value>) -> Option<Map<String, Value>> {
    if request.keys().any(|k| !CHAT_FIELDS.contains(&k.as_str()))
        || request.get("stream").is_some_and(|s| s != &json!(false))
    {
        return None;
    }
    let mut out = Map::new();
    let mut input = Vec::new();
    let mut instructions = Vec::new();
    for m in request.get("messages")?.as_array()? {
        let m = m.as_object()?;
        if m.keys().any(|k| k != "role" && k != "content") {
            return None;
        }
        let text = chat_text(m.get("content")?)?;
        match m.get("role")?.as_str()? {
            "system" | "developer" => instructions.push(text),
            "assistant" => input.push(json!({
                "role": "assistant",
                "content": [{"type": "output_text", "text": text}],
            })),
            "user" => input.push(json!({
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            })),
            _ => return None,
        }
    }
    out.insert("input".to_owned(), Value::Array(input));
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
    if let Some(line) = request.get("response_format").and_then(format_instruction) {
        instructions.push(line);
    }
    if !instructions.is_empty() {
        out.insert("instructions".to_owned(), json!(instructions.join("\n\n")));
    }
    Some(out)
}

/// The one text of a finished reply, or `None` when the reply is not a plain answer: the status
/// is not `completed` (or absent), there is no or more than one `message` item, or a content part
/// is not `output_text` (a `refusal` part, for example). The brain then falls back.
pub fn checked_text(reply: &Value) -> Option<String> {
    if !matches!(
        reply.get("status").and_then(Value::as_str),
        None | Some("completed")
    ) {
        return None;
    }
    let mut messages = reply
        .get("output")?
        .as_array()?
        .iter()
        .filter(|i| i.get("type").and_then(Value::as_str) == Some("message"));
    let msg = messages.next()?;
    if messages.next().is_some() {
        return None;
    }
    let mut out = String::new();
    for p in msg.get("content")?.as_array()? {
        if p.get("type").and_then(Value::as_str) != Some("output_text") {
            return None;
        }
        out.push_str(p.get("text")?.as_str()?);
    }
    Some(out)
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
            "stream": false,
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
        }));
        let r = from_chat_request(&chat).unwrap();
        assert_eq!(
            Value::Object(r),
            json!({
                "instructions": "be brief\n\nReturn only a JSON object that matches this schema: {\"type\":\"object\"}",
                "input": [
                    {"role": "user", "content": [{"type": "input_text", "text": "hi"}]},
                    {"role": "assistant", "content": [{"type": "output_text", "text": "hello"}]},
                    {"role": "user", "content": [{"type": "input_text", "text": "bye"}]},
                ],
                "max_output_tokens": 120,
                "temperature": 0,
            })
        );
    }

    #[test]
    fn adr_0122_responses_json_object_instruction() {
        let r = from_chat_request(&obj(json!({
            "messages": [{"role": "user", "content": "x"}],
            "response_format": {"type": "json_object"},
            "max_completion_tokens": 5,
        })))
        .unwrap();
        assert_eq!(r["instructions"], "Return only a JSON object.");
        assert!(r.get("text").is_none());
        assert_eq!(r["max_output_tokens"], 5);
    }

    #[test]
    fn rtr_20a_responses_translation_is_an_allowlist() {
        let user = json!({"role": "user", "content": "x"});
        let bad = [
            json!({"messages": [user], "prompt": {"id": "p"}}),
            json!({"messages": [user], "previous_response_id": "r"}),
            json!({"messages": [user], "stream": true}),
            json!({"input": [user]}),
            json!({"messages": [{"role": "tool", "content": "x"}]}),
            json!({"messages": [{"role": "user", "content": "x", "tool_calls": []}]}),
            json!({"messages": [{"role": "user", "content": [
                {"type": "image_url", "image_url": {"url": "u"}}]}]}),
            json!({"messages": [{"role": "user", "content": [{"type": "input_image"}]}]}),
            json!({"messages": [{"role": "user", "content": [{"type": "mcp_call"}]}]}),
            json!({"messages": "x"}),
            json!({}),
        ];
        for b in bad {
            assert!(from_chat_request(&obj(b.clone())).is_none(), "{b}");
        }
    }

    #[test]
    fn adr_0122_responses_to_chat_reply_wraps_the_text() {
        let r = json!({"output": [{"type": "message", "content": [
            {"type": "output_text", "text": "{\"a\":1}"}]}]});
        assert_eq!(
            to_chat_reply(&r).unwrap()["choices"][0]["message"]["content"],
            "{\"a\":1}"
        );
    }

    #[test]
    fn adr_0122_responses_checked_text_refuses_what_is_not_one_plain_answer() {
        let msg =
            |t: &str| json!({"type": "message", "content": [{"type": "output_text", "text": t}]});
        let refusal = json!({"type": "message", "content": [{"type": "refusal", "refusal": "no"}]});
        assert!(to_chat_reply(&json!({"output": []})).is_none());
        assert!(to_chat_reply(&json!({"output": [msg("a"), msg("b")]})).is_none());
        assert!(to_chat_reply(&json!({"output": [refusal]})).is_none());
        assert!(to_chat_reply(&json!({"status": "incomplete", "output": [msg("a")]})).is_none());
        assert!(to_chat_reply(&json!({"status": "completed", "output": [msg("a")]})).is_some());
        // Reasoning items beside one message are fine.
        assert!(to_chat_reply(&json!({"output": [{"type": "reasoning"}, msg("a")]})).is_some());
    }
}
