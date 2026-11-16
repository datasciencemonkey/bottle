//! The Genie One MCP endpoint (G-02) and the rejected Genie backends (G-03), #28.

/// The path of the Genie One MCP server (G-02).
pub const GENIE_ONE_PATH: &str = "/ai-gateway/mcp-services/system.ai.genie_one_mcp";

/// Why an MCP URL is not the Genie One endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointError {
    BetaGenieMcp,
    PerSpaceGenieMcp,
    GenieOneApiSse,
    NotHttps,
    BadHost,
    WrongPath,
    MissingWorkspaceId,
}

impl EndpointError {
    pub fn code(&self) -> &'static str {
        "todo"
    }
    pub fn message(&self) -> &'static str {
        ""
    }
}

impl std::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for EndpointError {}

/// The Genie One MCP URL of one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenieOneEndpoint {
    url: String,
    host: String,
    workspace_id: String,
}

impl GenieOneEndpoint {
    pub fn new(_host: &str, _workspace_id: &str) -> Result<Self, EndpointError> {
        Err(EndpointError::WrongPath)
    }
    pub fn parse(_url: &str) -> Result<Self, EndpointError> {
        Err(EndpointError::WrongPath)
    }
    pub fn url(&self) -> &str {
        &self.url
    }
    pub fn host(&self) -> &str {
        &self.host
    }
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
}

pub fn rejected_backend(_url: &str) -> Option<EndpointError> {
    None
}
