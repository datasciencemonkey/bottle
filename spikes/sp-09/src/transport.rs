//! `McpClient`: MCP Streamable HTTP over reqwest with a fresh token per request.
use crate::endpoint::GenieOneEndpoint;
use crate::envelope::{RpcError, Signal, ToolCallOutcome};
use crate::token::{TokenError, TokenSource};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum McpError {
    #[error("token: {0}")]
    Token(#[from] TokenError),
    #[error("network: {0}")]
    Network(String),
    #[error("AUTH_EXPIRED: 401 after one forced token refresh")]
    AuthExpired,
    #[error("GENIE_ONE_OFF: HTTP {status} without a session")]
    GenieOneOff { status: u16 },
    #[error("RATE_LIMITED: HTTP 429 (Retry-After {retry_after:?})")]
    RateLimited { retry_after: Option<String> },
    #[error("HTTP {status}")]
    Http {
        status: u16,
        body_signal: Option<Signal>,
        rpc_error: Option<RpcError>,
    },
    #[error("bad response: {0}")]
    BadResponse(String),
    #[error("the SSE stream ended before the JSON-RPC response")]
    StreamEndedWithoutResponse,
    #[error("the server named protocol {0}, not 2025-06-18")]
    UnsupportedProtocolVersion(String),
    #[error("call initialize first")]
    NotInitialized,
    #[error("G-06a: tool {0} is not on the allow list")]
    ToolNotAllowed(String),
}

/// One HTTP exchange, with no token, no body and no session ID value.
#[derive(Debug, Clone, Serialize)]
pub struct Exchange {
    pub method: String,
    pub attempt: u8,
    pub forced_refresh: bool,
    pub http_status: u16,
    pub content_type: String,
    pub session_id_sent: bool,
    pub protocol_header_sent: bool,
    pub o_param: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitializeInfo {
    pub protocol_version: String,
    pub session_id_present: bool,
    pub server_name: Option<String>,
    pub content_type: String,
    pub initialized_status: u16,
}

pub struct McpClient<T> {
    #[allow(dead_code)]
    endpoint: GenieOneEndpoint,
    tokens: T,
}

impl<T: TokenSource> McpClient<T> {
    pub fn new(_endpoint: GenieOneEndpoint, _tokens: T) -> Self {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn tokens(&self) -> &T {
        &self.tokens
    }
    pub fn session_id(&self) -> Option<String> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn exchanges(&self) -> Vec<Exchange> {
        unimplemented!("TODO(#29): green commit")
    }
    pub async fn initialize(&self) -> Result<InitializeInfo, McpError> {
        unimplemented!("TODO(#29): green commit")
    }
    pub async fn list_tools(&self) -> Result<Value, McpError> {
        unimplemented!("TODO(#29): green commit")
    }
    pub async fn call_tool(
        &self,
        _name: &str,
        _arguments: Value,
        _meta: Option<Value>,
    ) -> Result<ToolCallOutcome, McpError> {
        unimplemented!("TODO(#29): green commit")
    }
}
