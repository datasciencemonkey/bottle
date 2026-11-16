//! R17 BottleMcpServer (section 4.3).
//!
//! Responsibilities: The `bottle-mcp` server on loopback: `transcript.search`, `memory.recall`, `memory.write` (through R10), and the UI tools `show_blurb`, `set_mood`, `propose_page`, `ask_user` (relayed to the brain as `agent.tool_call`). No tool starts capture, approves context, publishes or switches the workspace. Model proxy to `/ai-gateway/…` with the token.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R17";

/// The tools of `bottle-mcp`, as the 4.3 row lists them.
pub const TOOLS: [&str; 7] = [
    "transcript.search",
    "memory.recall",
    "memory.write",
    "show_blurb",
    "set_mood",
    "propose_page",
    "ask_user",
];

/// The `bottle-mcp` loopback server and the model proxy for the agent harness.
pub trait BottleMcpServer: Send + Sync {
    /// Start the loopback server. Returns the port.
    fn start(&self) -> CoreResult<u16>;
    /// Stop the loopback server.
    fn stop(&self) -> CoreResult<()>;
}

/// The R17 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubBottleMcpServer;

impl BottleMcpServer for StubBottleMcpServer {
    fn start(&self) -> CoreResult<u16> {
        // TODO(#113): serve bottle-mcp with exactly 7 tools.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn stop(&self) -> CoreResult<()> {
        // TODO(#113): stop the loopback server.
        Err(CoreError::NotImplemented { component: ID })
    }
}
