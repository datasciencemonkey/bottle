//! Contract between the client and the Genie One `tools/list` (fixtures/genie_tools.json).
use serde::Serialize;
use serde_json::Value;

/// G-06a: the bridge calls only these tools (plus `tools/list`).
pub const REQUIRED_TOOLS: [&str; 4] = [
    "genie_ask",
    "genie_poll_response",
    "genie_get_query_result",
    "genie_cancel_response",
];

pub fn ask_args(_question: &str, _conversation_id: Option<&str>) -> Value {
    unimplemented!("TODO(#29): green commit")
}
pub fn ask_meta(_warehouse_id: Option<&str>) -> Option<Value> {
    unimplemented!("TODO(#29): green commit")
}
pub fn poll_args(_conversation_id: &str, _response_id: &str) -> Value {
    unimplemented!("TODO(#29): green commit")
}
pub fn query_result_args(_conversation_id: &str, _response_id: &str, _item_id: &str) -> Value {
    unimplemented!("TODO(#29): green commit")
}
pub fn cancel_args(_conversation_id: &str) -> Value {
    unimplemented!("TODO(#29): green commit")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Violation {
    pub tool: String,
    pub problem: String,
}

pub fn check_args(_schema: &Value, _args: &Value) -> Vec<String> {
    unimplemented!("TODO(#29): green commit")
}
pub fn check_tools(_tools_result: &Value) -> Vec<Violation> {
    unimplemented!("TODO(#29): green commit")
}
pub fn is_placeholder(_fixture: &Value) -> bool {
    unimplemented!("TODO(#29): green commit")
}
