//! The Genie One MCP endpoint (G-02) and the rejected Genie backends (G-03), #28.
//!
//! - [`GenieOneEndpoint::new`] builds the only MVP backend URL,
//!   `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`, so every
//!   MCP call carries `?o=` (G-02, AT-07). R8 McpTransport takes only this type.
//! - [`rejected_backend`] finds the Beta `/api/2.0/mcp/genie` (end of life 2026-11-30), the
//!   per-space Genie MCP `/api/2.0/mcp/genie/<space_id>` and the Genie One API SSE
//!   `/api/2.0/genie/one/...` in any URL text. The config loader uses it (G-03).
//!
//! The rules come from the SP-09 spike (`spikes/sp-09/src/endpoint.rs`), without the `url`
//! crate: R8 builds its own URLs, so the parser only has to refuse what it does not build.
//! Errors are content-free: a code and a fixed message, never the host or the URL.

use std::fmt;

/// The path of the Genie One MCP server (G-02).
pub const GENIE_ONE_PATH: &str = "/ai-gateway/mcp-services/system.ai.genie_one_mcp";

const BETA_PATH: &str = "/api/2.0/mcp/genie";
const GENIE_ONE_API_PATH: &str = "/api/2.0/genie/one";

/// Why an MCP URL is not the Genie One endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndpointError {
    /// G-03: the Beta Genie MCP `/api/2.0/mcp/genie`.
    BetaGenieMcp,
    /// G-03: the per-space Genie MCP `/api/2.0/mcp/genie/<space_id>`.
    PerSpaceGenieMcp,
    /// G-03: the Genie One API SSE `/api/2.0/genie/one/...`.
    GenieOneApiSse,
    /// The URL is not `https`.
    NotHttps,
    /// The host is empty, has a port, user info, a path or a character a host cannot have.
    BadHost,
    /// The path is not [`GENIE_ONE_PATH`].
    WrongPath,
    /// G-02: `?o=` is missing, repeated or not all digits.
    MissingWorkspaceId,
}

impl EndpointError {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            EndpointError::BetaGenieMcp => "genie_beta_mcp_rejected",
            EndpointError::PerSpaceGenieMcp => "genie_space_mcp_rejected",
            EndpointError::GenieOneApiSse => "genie_one_sse_rejected",
            EndpointError::NotHttps => "mcp_url_not_https",
            EndpointError::BadHost => "mcp_url_bad_host",
            EndpointError::WrongPath => "mcp_url_wrong_path",
            EndpointError::MissingWorkspaceId => "mcp_url_missing_workspace_id",
        }
    }

    /// A clear, fixed message for a person (G-03). It names no host and no URL.
    pub fn message(&self) -> &'static str {
        match self {
            EndpointError::BetaGenieMcp => {
                "genie_beta_mcp_rejected: the Beta Genie MCP endpoint /api/2.0/mcp/genie is not \
                 supported (end of life 2026-11-30). Use Genie One MCP at \
                 https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>."
            }
            EndpointError::PerSpaceGenieMcp => {
                "genie_space_mcp_rejected: the per-space Genie MCP /api/2.0/mcp/genie/<space_id> \
                 is not a Bottle backend. Use Genie One MCP at \
                 https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>."
            }
            EndpointError::GenieOneApiSse => {
                "genie_one_sse_rejected: the Genie One API SSE /api/2.0/genie/one/... is not a \
                 Bottle backend. Use Genie One MCP at \
                 https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>."
            }
            EndpointError::NotHttps => "mcp_url_not_https: the MCP URL must use https.",
            EndpointError::BadHost => {
                "mcp_url_bad_host: the workspace host must be a bare host name with no port, \
                 user or path."
            }
            EndpointError::WrongPath => {
                "mcp_url_wrong_path: the MCP URL path must be \
                 /ai-gateway/mcp-services/system.ai.genie_one_mcp."
            }
            EndpointError::MissingWorkspaceId => {
                "mcp_url_missing_workspace_id: the MCP URL must carry ?o=<workspace_id> with \
                 digits only."
            }
        }
    }
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for EndpointError {}

/// The Genie One MCP URL of one workspace (G-02). It always carries `?o=<workspace_id>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenieOneEndpoint {
    url: String,
    host: String,
    workspace_id: String,
}

fn valid_workspace_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

/// A bare DNS host name: labels of ASCII letters, digits and `-`, joined by single dots.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// The path of a URL or of path text: after `scheme://authority`, up to `?` or `#`.
fn path_of(url: &str) -> &str {
    let rest = match url.find("://") {
        Some(i) => {
            let after = &url[i + 3..];
            match after.find(['/', '?', '#']) {
                Some(j) => &after[j..],
                None => "",
            }
        }
        None => url,
    };
    match rest.find(['?', '#']) {
        Some(j) => &rest[..j],
        None => rest,
    }
}

/// The G-03 rejection for a URL (or path text), if it names a rejected Genie backend.
/// The match is case-insensitive and ignores a trailing `/`.
pub fn rejected_backend(url: &str) -> Option<EndpointError> {
    let path = path_of(url.trim()).to_ascii_lowercase();
    let path = path.trim_end_matches('/');
    if path == BETA_PATH {
        Some(EndpointError::BetaGenieMcp)
    } else if path.starts_with(&format!("{BETA_PATH}/")) {
        Some(EndpointError::PerSpaceGenieMcp)
    } else if path == GENIE_ONE_API_PATH || path.starts_with(&format!("{GENIE_ONE_API_PATH}/")) {
        Some(EndpointError::GenieOneApiSse)
    } else {
        None
    }
}

impl GenieOneEndpoint {
    /// Build `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`.
    /// `host` is a bare host or a workspace URL `https://<host>` with an optional trailing `/`.
    /// The host is kept in lower case.
    pub fn new(host: &str, workspace_id: &str) -> Result<Self, EndpointError> {
        let trimmed = host.trim();
        let bare = trimmed.strip_prefix("https://").unwrap_or(trimmed);
        let bare = bare.strip_suffix('/').unwrap_or(bare).to_ascii_lowercase();
        if !valid_host(&bare) {
            return Err(EndpointError::BadHost);
        }
        if !valid_workspace_id(workspace_id) {
            return Err(EndpointError::MissingWorkspaceId);
        }
        Ok(Self {
            url: format!("https://{bare}{GENIE_ONE_PATH}?o={workspace_id}"),
            host: bare,
            workspace_id: workspace_id.to_owned(),
        })
    }

    /// Check a full URL with the G-02 and G-03 rules. A G-03 URL names its own error first.
    pub fn parse(url: &str) -> Result<Self, EndpointError> {
        if let Some(e) = rejected_backend(url) {
            return Err(e);
        }
        let rest = url
            .strip_prefix("https://")
            .ok_or(EndpointError::NotHttps)?;
        let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (host, tail) = rest.split_at(split);
        if !valid_host(host) {
            return Err(EndpointError::BadHost);
        }
        let (path, query) = match tail.split_once('?') {
            Some((p, q)) => (p, q),
            None => (tail, ""),
        };
        if path != GENIE_ONE_PATH {
            return Err(EndpointError::WrongPath);
        }
        let mut ids = query.split('&').filter_map(|kv| kv.strip_prefix("o="));
        let id = match (ids.next(), ids.next()) {
            (Some(id), None) if valid_workspace_id(id) && !query.contains('#') => id,
            _ => return Err(EndpointError::MissingWorkspaceId),
        };
        let parsed = Self::new(host, id)?;
        if parsed.url != url {
            // Only the exact form that `new` builds is accepted (no extra query keys).
            return Err(EndpointError::WrongPath);
        }
        Ok(parsed)
    }

    /// The full URL, with `?o=`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The workspace host, in lower case.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The `o=` value.
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_02_path_of_handles_urls_and_bare_paths() {
        assert_eq!(path_of("https://h/a/b?x#y"), "/a/b");
        assert_eq!(path_of("https://h"), "");
        assert_eq!(path_of("https://h?o=1"), "");
        assert_eq!(path_of("/api/2.0/mcp/genie?o=1"), "/api/2.0/mcp/genie");
    }

    #[test]
    fn g_02_parse_refuses_extra_query_keys_and_upper_case_hosts() {
        let base = format!("https://h.cloud.databricks.com{GENIE_ONE_PATH}");
        assert!(GenieOneEndpoint::parse(&format!("{base}?o=1&x=2")).is_err());
        let upper = format!("https://H.cloud.databricks.com{GENIE_ONE_PATH}?o=1");
        assert!(GenieOneEndpoint::parse(&upper).is_err());
    }
}
