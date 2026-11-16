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

fn valid_workspace_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit())
}

impl GenieOneEndpoint {
    /// Build `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`.
    /// `host` may carry an `https://` prefix and a trailing `/`, nothing else.
    pub fn new(host: &str, workspace_id: &str) -> Result<Self, EndpointError> {
        let bare = host
            .trim()
            .trim_start_matches("https://")
            .trim_end_matches('/');
        if bare.is_empty() || bare.contains(['/', '?', '#', '@', ' ']) || bare.contains("://") {
            return Err(EndpointError::BadHost);
        }
        if !valid_workspace_id(workspace_id) {
            return Err(EndpointError::MissingWorkspaceId);
        }
        Self::parse(&format!("https://{bare}{GENIE_ONE_PATH}?o={workspace_id}"))
    }

    /// Check a full URL. This is the config-loader rule of G-02 and G-03.
    pub fn parse(url: &str) -> Result<Self, EndpointError> {
        Self::check(url, false)
    }

    /// Same rules, but plain `http` is allowed for a loopback host (test servers only).
    pub fn parse_loopback_for_tests(url: &str) -> Result<Self, EndpointError> {
        Self::check(url, true)
    }

    fn check(raw: &str, allow_loopback_http: bool) -> Result<Self, EndpointError> {
        let url = Url::parse(raw).map_err(|_| EndpointError::BadUrl)?;
        let path = url.path().trim_end_matches('/');
        // G-03 first, so the error names the rejected backend.
        if path == "/api/2.0/mcp/genie" {
            return Err(EndpointError::BetaGenieEndpoint);
        }
        if path.starts_with("/api/2.0/mcp/genie/") {
            return Err(EndpointError::PerSpaceGenieMcp);
        }
        if path.starts_with("/api/2.0/genie/one") {
            return Err(EndpointError::GenieOneApiSse);
        }
        let host = url.host_str().ok_or(EndpointError::BadHost)?;
        let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
        match url.scheme() {
            "https" => {}
            "http" if allow_loopback_http && loopback => {}
            _ => return Err(EndpointError::NotHttps),
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(EndpointError::BadHost);
        }
        if path != GENIE_ONE_PATH {
            return Err(EndpointError::WrongPath);
        }
        let o: Vec<String> = url
            .query_pairs()
            .filter(|(k, _)| k == "o")
            .map(|(_, v)| v.into_owned())
            .collect();
        let workspace_id = match o.as_slice() {
            [one] if valid_workspace_id(one) => one.clone(),
            _ => return Err(EndpointError::MissingWorkspaceId),
        };
        Ok(GenieOneEndpoint { url, workspace_id })
    }

    pub fn url(&self) -> &Url {
        &self.url
    }

    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
}
