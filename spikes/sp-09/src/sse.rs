//! Minimal `text/event-stream` parser for MCP Streamable HTTP replies.

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub data: String,
}

pub fn parse_events(_body: &str) -> Vec<SseEvent> {
    unimplemented!("TODO(#29): green commit")
}
