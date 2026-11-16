//! Redacted shapes of JSON values for the live report. Strings become
//! `<string len=N>` and numbers `<number>`, except for a short list of envelope
//! keys whose values are protocol words, not user content.
use serde_json::{Map, Value, json};

/// Keys whose short string or number values are kept as they are.
pub const KEEP_KEYS: [&str; 7] = [
    "jsonrpc",
    "status",
    "type",
    "protocolVersion",
    "code",
    "isError",
    "name",
];

pub fn shape(value: &Value) -> Value {
    shape_inner(value, None)
}

fn shape_inner(value: &Value, key: Option<&str>) -> Value {
    let keep = key.is_some_and(|k| KEEP_KEYS.contains(&k));
    match value {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(*b),
        Value::Number(n) if keep => Value::Number(n.clone()),
        Value::Number(_) => json!("<number>"),
        Value::String(s) if keep && s.chars().count() <= 64 => Value::String(s.clone()),
        Value::String(s) => json!(format!("<string len={}>", s.chars().count())),
        Value::Array(items) => {
            let mut m = Map::new();
            m.insert("array_len".into(), json!(items.len()));
            if let Some(first) = items.first() {
                m.insert("first".into(), shape_inner(first, None));
            }
            Value::Object(m)
        }
        Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), shape_inner(v, Some(k))))
                .collect(),
        ),
    }
}
