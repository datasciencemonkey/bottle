//! Genie One MCP endpoint (G-02) and the rejected backends (G-03).
use url::Url;

pub const GENIE_ONE_PATH: &str = "/ai-gateway/mcp-services/system.ai.genie_one_mcp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenieOneEndpoint {
    url: Url,
    workspace_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EndpointError {
    #[error(
        "G-03: the Beta Genie MCP endpoint /api/2.0/mcp/genie is not allowed (end of life 2026-11-30); use https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>"
    )]
    BetaGenieEndpoint,
    #[error(
        "G-03: the per-space Genie MCP /api/2.0/mcp/genie/<space_id> is not allowed as a backend"
    )]
    PerSpaceGenieMcp,
    #[error("G-03: the Genie One API SSE /api/2.0/genie/one/... is not allowed as a backend")]
    GenieOneApiSse,
    #[error("the MCP URL must use https")]
    NotHttps,
    #[error("the MCP URL path must be /ai-gateway/mcp-services/system.ai.genie_one_mcp")]
    WrongPath,
    #[error("G-02: the MCP URL must carry ?o=<workspace_id> with digits only")]
    MissingWorkspaceId,
    #[error("the host is not valid")]
    BadHost,
    #[error("the URL is not valid")]
    BadUrl,
}

impl GenieOneEndpoint {
    pub fn new(_host: &str, _workspace_id: &str) -> Result<Self, EndpointError> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn parse(_url: &str) -> Result<Self, EndpointError> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn parse_loopback_for_tests(_url: &str) -> Result<Self, EndpointError> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn url(&self) -> &Url {
        &self.url
    }
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
}
