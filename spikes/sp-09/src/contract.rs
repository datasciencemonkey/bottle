//! Contract between the client and the Genie One `tools/list` result
//! (fixtures/genie_tools.json). The request builders here are the only place
//! that shapes tool arguments, so the contract test covers every call the bridge makes.
use serde::Serialize;
use serde_json::{Map, Value, json};

/// Tool names the report may show by name: the required tools plus the known extra.
pub const KNOWN_TOOLS: [&str; 5] = [
    "genie_ask",
    "genie_poll_response",
    "genie_get_query_result",
    "genie_cancel_response",
    "view_ask",
];

/// Argument names the client sends; only these appear by name in a violation.
pub const ARG_NAMES: [&str; 4] = ["question", "conversation_id", "response_id", "item_id"];

const JSON_TYPES: [&str; 7] = [
    "string", "integer", "number", "boolean", "object", "array", "null",
];

/// A server-given name as a fixed word, or its length.
fn safe_name(name: &str, list: &[&'static str]) -> String {
    match list.iter().find(|n| **n == name) {
        Some(n) => (*n).to_string(),
        None => format!("<name len={}>", name.chars().count()),
    }
}

/// G-06a: the bridge calls only these tools (plus `tools/list`).
pub const REQUIRED_TOOLS: [&str; 4] = [
    "genie_ask",
    "genie_poll_response",
    "genie_get_query_result",
    "genie_cancel_response",
];

pub fn ask_args(question: &str, conversation_id: Option<&str>) -> Value {
    let mut m = Map::new();
    m.insert("question".into(), json!(question));
    if let Some(c) = conversation_id {
        m.insert("conversation_id".into(), json!(c));
    }
    Value::Object(m)
}

/// G-06b: `warehouse_id` goes in `_meta`. G-07a: no `progressToken`.
pub fn ask_meta(warehouse_id: Option<&str>) -> Option<Value> {
    warehouse_id.map(|w| json!({ "warehouse_id": w }))
}

pub fn poll_args(conversation_id: &str, response_id: &str) -> Value {
    json!({ "conversation_id": conversation_id, "response_id": response_id })
}

pub fn query_result_args(conversation_id: &str, response_id: &str, item_id: &str) -> Value {
    json!({ "conversation_id": conversation_id, "response_id": response_id, "item_id": item_id })
}

pub fn cancel_args(conversation_id: &str) -> Value {
    json!({ "conversation_id": conversation_id })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub tool: String,
    pub problem: String,
}

fn type_ok(want: &str, v: &Value) -> bool {
    match want {
        "string" => v.is_string(),
        "integer" => v.is_i64() || v.is_u64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        "object" => v.is_object(),
        "array" => v.is_array(),
        "null" => v.is_null(),
        _ => true,
    }
}

/// A small JSON Schema check: `required`, property `type` and `additionalProperties: false`.
pub fn check_args(schema: &Value, args: &Value) -> Vec<String> {
    let mut problems = Vec::new();
    let Some(obj) = args.as_object() else {
        return vec!["arguments are not an object".into()];
    };
    let props = schema.get("properties").and_then(Value::as_object);
    for r in schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(name) = r.as_str().filter(|n| !obj.contains_key(*n)) {
            problems.push(format!(
                "the client does not send required argument {}",
                safe_name(name, &ARG_NAMES)
            ));
        }
    }
    for (k, v) in obj {
        match props.and_then(|p| p.get(k)) {
            Some(p) => {
                let types: Vec<&str> = match p.get("type") {
                    Some(Value::String(t)) => vec![t.as_str()],
                    Some(Value::Array(ts)) => ts.iter().filter_map(Value::as_str).collect(),
                    _ => vec![],
                };
                if !types.is_empty() && !types.iter().any(|t| type_ok(t, v)) {
                    let types: Vec<String> =
                        types.iter().map(|t| safe_name(t, &JSON_TYPES)).collect();
                    problems.push(format!("argument {k} does not match type {types:?}"));
                }
            }
            None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                problems.push(format!("the schema does not allow argument {k}"));
            }
            None if props.is_some() => {
                problems.push(format!("the schema does not list argument {k}"));
            }
            None => {}
        }
    }
    problems
}

/// Sample calls for each tool, with every optional argument the client can send.
fn samples(tool: &str) -> Vec<Value> {
    match tool {
        "genie_ask" => vec![ask_args("q", None), ask_args("q", Some("c"))],
        "genie_poll_response" => vec![poll_args("c", "r")],
        "genie_get_query_result" => vec![query_result_args("c", "r", "pub_1_output")],
        "genie_cancel_response" => vec![cancel_args("c")],
        _ => vec![],
    }
}

/// Check a `tools/list` result (or the fixture, which has the same `tools` array).
pub fn check_tools(tools_result: &Value) -> Vec<Violation> {
    let mut out = Vec::new();
    let tools = tools_result.get("tools").and_then(Value::as_array);
    let Some(tools) = tools else {
        return vec![Violation {
            tool: "*".into(),
            problem: "no tools array".into(),
        }];
    };
    for name in REQUIRED_TOOLS {
        let Some(tool) = tools
            .iter()
            .find(|t| t.get("name").and_then(Value::as_str) == Some(name))
        else {
            out.push(Violation {
                tool: name.into(),
                problem: "missing from tools/list".into(),
            });
            continue;
        };
        let schema = tool.get("inputSchema").cloned().unwrap_or(Value::Null);
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            out.push(Violation {
                tool: name.into(),
                problem: "inputSchema.type is not object".into(),
            });
            continue;
        }
        for args in samples(name) {
            for p in check_args(&schema, &args) {
                out.push(Violation {
                    tool: name.into(),
                    problem: p,
                });
            }
        }
    }
    out.dedup();
    out
}

pub fn is_placeholder(fixture: &Value) -> bool {
    fixture.get("_placeholder").and_then(Value::as_bool) == Some(true)
}
