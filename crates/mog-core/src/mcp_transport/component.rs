//! `mcp_call` (4.5) as an R12 gateway component: it parses [`McpCallArgs`] and calls R8.
//!
//! The gateway runs the window and schema checks first (SEC-02a, SEC-02b), so only the brain
//! reaches this. `tool: "tools/list"` lists the allowlisted tools; any other tool goes to
//! [`McpTransport::call_tool`], which refuses every tool not on the G-06a allowlist.
//! The Tauri host routes `mcp_call` here (#467, `src-tauri/src/mcp_host.rs`).

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use super::{McpCall, McpTransport};
use crate::error::{CoreError, CoreResult};
use crate::ipc::McpCallArgs;
use crate::ipc_gateway::GatewayComponents;
use crate::workspaces::WorkspaceGate;

/// The most `(profile, conversation)` pairs that [`ConversationPairs`] remembers.
const MAX_PAIRS: usize = 1024;

/// The tools that continue a conversation. With a gate, each needs a recorded pair or a profile
/// that the gate allows (ADR 0117 follow-up).
const CONTINUE_TOOLS: [&str; 3] = [
    "genie_poll_response",
    "genie_get_query_result",
    "genie_cancel_response",
];

/// The `(profile, conversation)` pairs of the successful `genie_ask` calls (ADR 0117 follow-up),
/// in memory. A poll, query result or cancel of a recorded pair is allowed even after its
/// workspace switched off, so a call in flight can end. It keeps no text, and `Debug` shows the
/// count only.
#[derive(Default)]
pub struct ConversationPairs {
    seen: Mutex<VecDeque<(String, String)>>,
}

impl fmt::Debug for ConversationPairs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConversationPairs")
            .field("pairs", &self.len())
            .finish()
    }
}

impl ConversationPairs {
    /// An empty record.
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<(String, String)>> {
        self.seen.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Record the pair of a successful `genie_ask`.
    pub fn record(&self, profile: &str, conversation: &str) {
        let mut seen = self.lock();
        if seen.iter().any(|(p, c)| p == profile && c == conversation) {
            return;
        }
        seen.push_back((profile.to_owned(), conversation.to_owned()));
        if seen.len() > MAX_PAIRS {
            seen.pop_front();
        }
    }

    /// True if `genie_ask` of `profile` made `conversation`.
    pub fn contains(&self, profile: &str, conversation: &str) -> bool {
        self.lock()
            .iter()
            .any(|(p, c)| p == profile && c == conversation)
    }

    /// The count of recorded pairs.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    /// True if no pair is recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// True if a call of `tool` on `conversation` of `profile` may go on: a recorded pair, or a
/// profile that `gate` allows. A tool that does not continue a conversation always may.
pub fn pair_allows(
    gate: Option<&Arc<dyn WorkspaceGate>>,
    pairs: &ConversationPairs,
    tool: &str,
    profile: &str,
    conversation: Option<&str>,
) -> bool {
    if !CONTINUE_TOOLS.contains(&tool) {
        return true;
    }
    conversation.is_some_and(|c| pairs.contains(profile, c))
        || gate.is_some_and(|g| g.allows(profile))
}

/// The `conversation_id` of a successful `genie_ask` reply.
pub fn reply_conversation(structured_content: &Value) -> Option<&str> {
    structured_content
        .get("conversation_id")
        .and_then(Value::as_str)
        .filter(|c| !c.is_empty())
}

/// The `mcp_call` component over one R8.
#[derive(Debug, Clone)]
pub struct McpCallComponent<T> {
    transport: T,
    /// ADR 0117 (FLAG 7): the allow-list for `genie_ask`. `None` allows every profile.
    gate: Option<Arc<dyn WorkspaceGate>>,
    /// The pairs of the successful asks, for the gate on polls, query results and cancels.
    pairs: Arc<ConversationPairs>,
}

impl<T: McpTransport> McpCallComponent<T> {
    /// The component over `transport`.
    pub fn new(transport: T) -> Self {
        Self::with_gate(transport, None)
    }

    /// The component with the workspace allow-list `gate` (ADR 0117, FLAG 7). With a gate,
    /// `genie_ask` for a profile that is neither home nor available is refused with
    /// `workspace_not_available` before any send, on every path. A poll, cancel or query
    /// result needs a recorded `(profile, conversation)` pair of a successful ask, or a profile
    /// that the gate allows (so a journal resume of an allowed profile works).
    pub fn with_gate(transport: T, gate: Option<Arc<dyn WorkspaceGate>>) -> Self {
        Self {
            transport,
            gate,
            pairs: Arc::new(ConversationPairs::new()),
        }
    }

    /// The recorded pairs.
    pub fn pairs(&self) -> &Arc<ConversationPairs> {
        &self.pairs
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

impl<T: McpTransport> GatewayComponents for McpCallComponent<T> {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        if command != "mcp_call" {
            return Err(refused("not_implemented"));
        }
        let a: McpCallArgs = serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        if a.tool == "tools/list" {
            let tools = self.transport.list_tools(&a.profile, &a.tags)?;
            return Ok(json!({
                "structuredContent": {"tools": tools},
                "text": "",
                "isError": false,
                "httpStatus": 200,
            }));
        }
        if a.tool == "genie_ask" && self.gate.as_ref().is_some_and(|g| !g.allows(&a.profile)) {
            return Err(refused("workspace_not_available"));
        }
        if self.gate.is_some() {
            let conversation = a.args.get("conversation_id").and_then(Value::as_str);
            if !pair_allows(
                self.gate.as_ref(),
                &self.pairs,
                &a.tool,
                &a.profile,
                conversation,
            ) {
                return Err(refused("workspace_not_available"));
            }
        }
        let mut call = McpCall::new(&a.profile, &a.tool, a.args, &a.tags);
        if let Some(m) = &a.meta {
            call = call.meta(m);
        }
        if let Some(ms) = a.timeout_ms {
            call = call.timeout(Duration::from_millis(u64::from(ms)));
        }
        let reply = self.transport.call_tool(call)?;
        if a.tool == "genie_ask" && !reply.is_error {
            if let Some(c) = reply_conversation(&reply.structured_content) {
                self.pairs.record(&a.profile, c);
            }
        }
        Ok(reply.to_ipc_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreResult;
    use crate::mcp_transport::McpReply;
    use crate::types::Tags;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Answers every call with a conversation ID, and counts the calls.
    #[derive(Default)]
    struct Conv {
        sent: AtomicUsize,
    }

    impl McpTransport for Conv {
        fn call_tool(&self, _call: McpCall<'_>) -> CoreResult<McpReply> {
            self.sent.fetch_add(1, Ordering::SeqCst);
            Ok(McpReply {
                structured_content: json!({"status": "in_progress", "conversation_id": "c-1"}),
                text: String::new(),
                is_error: false,
                http_status: 200,
                rpc_error: None,
            })
        }
        fn list_tools(&self, _: &str, _: &Tags) -> CoreResult<Vec<Value>> {
            Ok(Vec::new())
        }
    }

    #[derive(Debug)]
    struct Allow(&'static [&'static str]);

    impl WorkspaceGate for Allow {
        fn allows(&self, profile: &str) -> bool {
            self.0.contains(&profile)
        }
    }

    fn call_args(profile: &str, tool: &str, args: Value) -> Value {
        json!({
            "profile": profile, "server": "genie_one", "tool": tool, "args": args,
            "opId": "00000000-0000-4000-8000-000000000141",
            "tags": {"app": "bottle", "mog": "mog-1", "op": "genie"},
        })
    }

    fn poll(profile: &str, conversation: &str) -> Value {
        call_args(
            profile,
            "genie_poll_response",
            json!({"conversation_id": conversation, "response_id": "r-1"}),
        )
    }

    fn code(r: CoreResult<Value>) -> String {
        match r {
            Ok(_) => "ok".into(),
            Err(e) => e.code().to_owned(),
        }
    }

    #[test]
    fn adr_0117_a_recorded_pair_may_poll_even_when_its_workspace_is_off() {
        let t = Arc::new(Conv::default());
        let allow: Arc<dyn WorkspaceGate> = Arc::new(Allow(&["on"]));
        let c = McpCallComponent::with_gate(t.clone(), Some(allow));
        let ask = call_args("on", "genie_ask", json!({"question": "q"}));
        assert_eq!(code(c.call("mcp_call", ask)), "ok");
        assert!(c.pairs().contains("on", "c-1"));
        // The same conversation under a profile that the gate refuses and that has no pair.
        assert_eq!(
            code(c.call("mcp_call", poll("off", "c-1"))),
            "workspace_not_available"
        );
        assert_eq!(t.sent.load(Ordering::SeqCst), 1, "nothing was sent");
    }

    #[test]
    fn adr_0117_a_pair_that_the_ask_recorded_polls_after_the_gate_closes() {
        let t = Arc::new(Conv::default());
        let c = McpCallComponent::with_gate(t.clone(), Some(Arc::new(Allow(&[]))));
        c.pairs().record("gone", "c-1");
        assert_eq!(code(c.call("mcp_call", poll("gone", "c-1"))), "ok");
        assert_eq!(t.sent.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn adr_0117_an_unrecorded_pair_of_a_disallowed_profile_is_refused_for_each_tool() {
        let t = Arc::new(Conv::default());
        let c = McpCallComponent::with_gate(t.clone(), Some(Arc::new(Allow(&[]))));
        for (tool, args) in [
            (
                "genie_poll_response",
                json!({"conversation_id": "c-9", "response_id": "r"}),
            ),
            ("genie_cancel_response", json!({"conversation_id": "c-9"})),
            (
                "genie_get_query_result",
                json!({"conversation_id": "c-9", "response_id": "r", "item_id": "pub_1_output"}),
            ),
        ] {
            assert_eq!(
                code(c.call("mcp_call", call_args("off", tool, args))),
                "workspace_not_available",
                "{tool}"
            );
        }
        assert_eq!(t.sent.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn adr_0117_a_journal_resume_pair_of_an_allowed_profile_is_allowed() {
        let t = Arc::new(Conv::default());
        let c = McpCallComponent::with_gate(t.clone(), Some(Arc::new(Allow(&["on"]))));
        // No ask in this process (a restart): the profile is allowed, so the poll goes.
        assert_eq!(code(c.call("mcp_call", poll("on", "c-77"))), "ok");
    }

    #[test]
    fn adr_0117_pairs_debug_shows_the_count_only() {
        let p = ConversationPairs::new();
        p.record("secret-profile", "secret-conversation");
        let text = format!("{p:?}");
        assert!(!text.contains("secret"), "{text}");
        assert!(text.contains("pairs: 1"), "{text}");
    }
}
