//! Redacted shapes of JSON values for the live report.
//!
//! `shape` keeps key names, value types, string lengths and array lengths, and
//! no value at all: every string becomes `<string len=N>`, every number
//! `<number>` and every bool `<bool>`. Protocol facts (the envelope kind, the
//! `isError` flag, the content item types, the poll status and the JSON-RPC
//! error code) come only from `envelope_facts` and `known_status`, which read
//! fixed paths and keep only values from fixed lists.
use serde_json::{Map, Value, json};

pub fn shape(value: &Value) -> Value {
    match value {
        Value::Null => Value::Null,
        Value::Bool(_) => json!("<bool>"),
        Value::Number(_) => json!("<number>"),
        Value::String(s) => json!(format!("<string len={}>", s.chars().count())),
        Value::Array(items) => {
            let mut m = Map::new();
            m.insert("array_len".into(), json!(items.len()));
            if let Some(first) = items.first() {
                m.insert("first".into(), shape(first));
            }
            Value::Object(m)
        }
        Value::Object(obj) => {
            Value::Object(obj.iter().map(|(k, v)| (k.clone(), shape(v))).collect())
        }
    }
}

/// MCP content item types (2025-06-18).
pub const CONTENT_TYPES: [&str; 5] = ["text", "image", "audio", "resource", "resource_link"];

/// Genie One poll status values (docs and the 2026-11-14 probe).
pub const STATUS_VALUES: [&str; 6] = [
    "in_progress",
    "completed",
    "incomplete",
    "failed",
    "unknown",
    "cancelled",
];

/// A status value only if it is on the fixed list; else `<other>`.
pub fn known_status(v: Option<&Value>) -> Value {
    match v {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(s)) if STATUS_VALUES.contains(&s.as_str()) => json!(s),
        Some(_) => json!("<other>"),
    }
}

/// Facts from a `CallToolResult` at fixed paths only.
pub fn envelope_facts(result: &Value) -> Value {
    let types: Vec<Value> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|i| match i.get("type").and_then(Value::as_str) {
                    Some(t) if CONTENT_TYPES.contains(&t) => json!(t),
                    _ => json!("<other>"),
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "isError": result.get("isError").and_then(Value::as_bool),
        "content_types": types,
        "has_structured_content": result.get("structuredContent").is_some_and(Value::is_object),
        "structured_status": known_status(result.pointer("/structuredContent/status")),
    })
}
