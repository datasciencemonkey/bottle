//! The G-06a tool allowlist of R8, with the argument contract of each tool (G-06a, G-06b, G-07a).
//!
//! The argument names come from the Genie One tool contracts in the reference plugin
//! (`plugins/databricks-genie/scripts/genie_tools.json`, also in `src/test-support/genie-tools.json`).
//! A contract test checks this table against that file.
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_tools.json @ a31fd2c (MIT).

use serde_json::{Map, Value};

use crate::ipc::JsonObject;

/// One allowlisted tool: its name and its string arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolContract {
    /// The MCP tool name.
    pub name: &'static str,
    /// Arguments that must be there.
    pub required: &'static [&'static str],
    /// Arguments that may be there.
    pub optional: &'static [&'static str],
}

/// The only tools that R8 calls (G-06a). `view_ask` (for MCP Apps clients) is not on it.
pub const ALLOWED_TOOLS: &[ToolContract] = &[
    ToolContract {
        name: "genie_ask",
        required: &["question"],
        optional: &["conversation_id"],
    },
    ToolContract {
        name: "genie_poll_response",
        required: &["conversation_id", "response_id"],
        optional: &[],
    },
    ToolContract {
        name: "genie_get_query_result",
        required: &["conversation_id", "response_id", "item_id"],
        optional: &[],
    },
    ToolContract {
        name: "genie_cancel_response",
        required: &["conversation_id"],
        optional: &[],
    },
];

/// The contract of `name`, if it is on the allowlist (exact match only).
pub fn allowed(name: &str) -> Option<&'static ToolContract> {
    ALLOWED_TOOLS.iter().find(|t| t.name == name)
}

/// True if `args` has every required argument, no other argument, and only string values.
pub fn args_match(contract: &ToolContract, args: &JsonObject) -> bool {
    contract.required.iter().all(|k| args.contains_key(*k))
        && args.iter().all(|(k, v)| {
            (contract.required.contains(&k.as_str()) || contract.optional.contains(&k.as_str()))
                && v.is_string()
        })
}

/// The longest warehouse ID that R8 sends.
const MAX_WAREHOUSE_ID: usize = 64;

/// A SQL warehouse ID: 1 to 64 ASCII letters or digits.
fn valid_warehouse_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_WAREHOUSE_ID && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// The `_meta` that R8 sends for `tool` (G-06b, G-07a): `{"warehouse_id": ..}` for
/// `genie_ask` when `meta` has one, else nothing. Every other key (`progressToken` too) is
/// dropped. `Err(())` if the warehouse ID of a `genie_ask` is not a warehouse ID.
#[allow(clippy::result_unit_err)]
pub fn outbound_meta(tool: &str, meta: Option<&JsonObject>) -> Result<Option<Value>, ()> {
    if tool != "genie_ask" {
        return Ok(None);
    }
    let Some(w) = meta.and_then(|m| m.get("warehouse_id")) else {
        return Ok(None);
    };
    match w.as_str() {
        Some(id) if valid_warehouse_id(id) => {
            let mut m = Map::new();
            m.insert("warehouse_id".to_owned(), Value::String(id.to_owned()));
            Ok(Some(Value::Object(m)))
        }
        _ => Err(()),
    }
}

/// Keep only the listed tools that are on the allowlist, each once, in allowlist order.
pub fn keep_allowed(listed: &[Value]) -> Vec<Value> {
    ALLOWED_TOOLS
        .iter()
        .filter_map(|c| {
            listed
                .iter()
                .find(|t| t.get("name").and_then(Value::as_str) == Some(c.name))
                .cloned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn obj(v: Value) -> JsonObject {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn g_06a_allowed_is_an_exact_match() {
        assert!(allowed("genie_ask").is_some());
        for n in ["Genie_Ask", "genie_ask\0", "view_ask", "genie", ""] {
            assert!(allowed(n).is_none(), "{n:?}");
        }
    }

    #[test]
    fn g_06a_args_match_the_contract() {
        let ask = allowed("genie_ask").unwrap();
        assert!(args_match(ask, &obj(json!({"question": "q"}))));
        assert!(args_match(
            ask,
            &obj(json!({"question": "q", "conversation_id": "c"}))
        ));
        assert!(!args_match(ask, &obj(json!({"conversation_id": "c"}))));
        assert!(!args_match(ask, &obj(json!({"question": ["q"]}))));
        assert!(!args_match(ask, &obj(json!({"question": "q", "x": "y"}))));
    }

    #[test]
    fn g_06b_g_07a_meta_is_only_the_warehouse_on_genie_ask() {
        let m = obj(json!({"warehouse_id": "abc", "progressToken": 1}));
        assert_eq!(
            outbound_meta("genie_ask", Some(&m)),
            Ok(Some(json!({"warehouse_id": "abc"})))
        );
        assert_eq!(outbound_meta("genie_poll_response", Some(&m)), Ok(None));
        assert_eq!(
            outbound_meta("genie_ask", Some(&obj(json!({"progressToken": 1})))),
            Ok(None)
        );
        assert_eq!(outbound_meta("genie_ask", None), Ok(None));
        assert_eq!(
            outbound_meta("genie_ask", Some(&obj(json!({"warehouse_id": "a-b"})))),
            Err(())
        );
    }

    #[test]
    fn g_06a_keep_allowed_drops_other_and_duplicate_tools() {
        let listed = vec![
            json!({"name": "genie_cancel_response"}),
            json!({"name": "view_ask"}),
            json!({"name": "genie_ask", "n": 1}),
            json!({"name": "genie_ask", "n": 2}),
            json!({"name": 5}),
        ];
        let kept = keep_allowed(&listed);
        assert_eq!(
            kept,
            vec![
                json!({"name": "genie_ask", "n": 1}),
                json!({"name": "genie_cancel_response"})
            ]
        );
    }
}
