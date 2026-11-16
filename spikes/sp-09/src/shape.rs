//! Redacted, typed shapes for the live report (default deny).
//!
//! The report never holds a `serde_json::Value` from the server. It holds
//! `Shape` values: types, string lengths, array lengths and key counts. A key
//! name shows only if it is on a fixed list for that exact protocol path (the
//! `CallToolResult` keys and the top-level Genie One poll keys); the name then
//! comes from the list (`&'static str`), not from the server. Every other key
//! shows as its length only. Protocol words (the content types, the poll
//! status, `isError`) come only from fixed lists in `envelope_facts` and
//! `known_status`.
use serde::Serialize;
use serde_json::{Value, json};

/// Keys of an MCP `CallToolResult` (2025-06-18).
pub const RESULT_KEYS: [&str; 4] = ["content", "structuredContent", "isError", "_meta"];

/// Top-level keys of a Genie One poll or ask `structuredContent` (docs and the 2026-11-14 probe).
pub const POLL_KEYS: [&str; 9] = [
    "status",
    "final_answer",
    "progress_steps",
    "narration_instruction",
    "query_items",
    "deep_link",
    "conversation_id",
    "response_id",
    "truncated",
];

/// At most this many fields of one object are listed (the count is always exact).
pub const MAX_FIELDS: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Shape {
    Null,
    Bool,
    Number,
    String {
        len: usize,
    },
    Array {
        len: usize,
        #[serde(skip_serializing_if = "Option::is_none")]
        first: Option<Box<Shape>>,
    },
    Object {
        keys: usize,
        fields: Vec<Field>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Field {
    pub key: Key,
    pub value: Shape,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Key {
    /// A name from a fixed list in this file.
    Known(&'static str),
    /// Any other key: its length only.
    Redacted { key_len: usize },
}

fn known(list: &'static [&'static str], k: &str) -> Option<&'static str> {
    list.iter().copied().find(|x| *x == k)
}

fn shape_with(
    value: &Value,
    keys: &'static [&'static str],
    child: &dyn Fn(&'static str, &Value) -> Shape,
) -> Shape {
    match value {
        Value::Object(obj) => Shape::Object {
            keys: obj.len(),
            fields: obj
                .iter()
                .take(MAX_FIELDS)
                .map(|(k, v)| match known(keys, k) {
                    Some(name) => Field {
                        key: Key::Known(name),
                        value: child(name, v),
                    },
                    None => Field {
                        key: Key::Redacted {
                            key_len: k.chars().count(),
                        },
                        value: shape(v),
                    },
                })
                .collect(),
        },
        other => shape(other),
    }
}

/// Generic shape: no key name and no value at any depth.
pub fn shape(value: &Value) -> Shape {
    match value {
        Value::Null => Shape::Null,
        Value::Bool(_) => Shape::Bool,
        Value::Number(_) => Shape::Number,
        Value::String(s) => Shape::String {
            len: s.chars().count(),
        },
        Value::Array(items) => Shape::Array {
            len: items.len(),
            first: items.first().map(|f| Box::new(shape(f))),
        },
        Value::Object(_) => shape_with(value, &[], &|_, v| shape(v)),
    }
}

/// A poll or ask `structuredContent`: top-level `POLL_KEYS` by name, the rest generic.
pub fn poll_shape(sc: &Value) -> Shape {
    shape_with(sc, &POLL_KEYS, &|_, v| shape(v))
}

/// A `CallToolResult`: `RESULT_KEYS` by name; `structuredContent` as a poll shape.
pub fn result_shape(result: &Value) -> Shape {
    shape_with(result, &RESULT_KEYS, &|name, v| {
        if name == "structuredContent" {
            poll_shape(v)
        } else {
            shape(v)
        }
    })
}

/// Which `POLL_KEYS` are present, and how many other keys there are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct KeyFacts {
    pub known: Vec<&'static str>,
    pub other_keys: usize,
}

pub fn poll_keys(sc: &Value) -> KeyFacts {
    let obj = sc.as_object();
    let known_keys: Vec<&'static str> = POLL_KEYS
        .iter()
        .copied()
        .filter(|k| obj.is_some_and(|o| o.contains_key(*k)))
        .collect();
    let other_keys = obj.map_or(0, |o| o.len()) - known_keys.len();
    KeyFacts {
        known: known_keys,
        other_keys,
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
        Some(Value::String(s)) => match known(&STATUS_VALUES, s) {
            Some(k) => json!(k),
            None => json!("<other>"),
        },
        Some(_) => json!("<other>"),
    }
}

/// Facts from a `CallToolResult` at fixed paths only.
pub fn envelope_facts(result: &Value) -> Value {
    let types: Vec<&'static str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|i| {
                    i.get("type")
                        .and_then(Value::as_str)
                        .and_then(|t| known(&CONTENT_TYPES, t))
                        .unwrap_or("<other>")
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

/// Tool names from a `tools/list`: names on `known_names` by name, the rest as a count and lengths.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolNames {
    pub known: Vec<&'static str>,
    pub other_count: usize,
    pub other_name_lens: Vec<usize>,
}

pub fn tool_names(tools_result: &Value, known_names: &'static [&'static str]) -> ToolNames {
    let mut out = ToolNames {
        known: Vec::new(),
        other_count: 0,
        other_name_lens: Vec::new(),
    };
    for t in tools_result
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let name = t.get("name").and_then(Value::as_str).unwrap_or("");
        match known(known_names, name) {
            Some(k) => out.known.push(k),
            None => {
                out.other_count += 1;
                out.other_name_lens.push(name.chars().count());
            }
        }
    }
    out
}
