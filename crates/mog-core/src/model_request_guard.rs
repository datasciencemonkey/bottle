//! Small-model calls never carry tools (#117; RTR-20a, ADR 0056).
//!
//! The router, cleanup and writer calls (and judge, embed and merge) go through `model_invoke`.
//! [`crate::ipc_gateway::CheckedGateway`] calls [`tool_fields`] on every `model_invoke` request
//! and refuses one with a tool field (`invalid_args`) before any component runs. The brain checks
//! the same list first (`src/brain/components/model-guard.ts`).

use serde_json::Value;

/// The chat-completions fields that turn on tools or function calls.
pub const TOOL_FIELDS: &[&str] = &[
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "parallel_tool_calls",
];

/// The tool fields at the top level of a chat-completions body, where the API reads them. The
/// names are compared without case (`Tools` counts). A value that is not an object has none.
pub fn tool_fields(request: &Value) -> Vec<&'static str> {
    match request.as_object() {
        Some(obj) => TOOL_FIELDS
            .iter()
            .copied()
            .filter(|f| obj.keys().any(|k| k.eq_ignore_ascii_case(f)))
            .collect(),
        None => Vec::new(),
    }
}

/// Whether the `model_invoke` arguments `args` hold a request with a tool field.
pub fn model_invoke_has_tools(args: &Value) -> bool {
    args.get("request")
        .is_some_and(|r| !tool_fields(r).is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rtr_20a_finds_each_field_and_ignores_nested_names() {
        for f in TOOL_FIELDS {
            let mut req = json!({"messages": []});
            req[*f] = json!(true);
            assert_eq!(tool_fields(&req), vec![*f]);
        }
        // A schema property named `tools` is data, not a tool field.
        let req =
            json!({"response_format": {"json_schema": {"schema": {"properties": {"tools": {}}}}}});
        assert!(tool_fields(&req).is_empty());
    }

    #[test]
    fn rtr_20a_tool_field_names_are_compared_without_case() {
        for f in [
            "Tools",
            "TOOL_CHOICE",
            "Functions",
            "Function_Call",
            "Parallel_Tool_Calls",
        ] {
            let req = json!({ f: [] });
            assert_eq!(tool_fields(&req).len(), 1, "{f}");
        }
    }

    #[test]
    fn rtr_20a_model_invoke_has_tools_reads_the_request() {
        assert!(model_invoke_has_tools(&json!({"request": {"tools": []}})));
        assert!(!model_invoke_has_tools(
            &json!({"request": {"messages": []}})
        ));
        assert!(!model_invoke_has_tools(&json!({})));
    }
}
