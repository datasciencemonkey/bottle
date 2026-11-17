//! The Genie One MCP endpoint (G-02) and the rejected Genie backends (G-03), #28.
//!
//! - [`GenieOneEndpoint::new`] builds the only MVP backend URL,
//!   `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`, so every
//!   MCP call carries `?o=` (G-02, AT-07). R8 McpTransport takes only this type.
//! - [`rejected_backend`] checks if the path of one URL is the Beta `/api/2.0/mcp/genie` (end of
//!   life 2026-11-30), the per-space Genie MCP `/api/2.0/mcp/genie/<space_id>` or the Genie One
//!   API SSE `/api/2.0/genie/one/...`. It compares the canonical path, and it rejects a path that
//!   it cannot make canonical (fail closed). The config loader uses it on URL fields (G-03).
//!
//! The rules come from the SP-09 spike (`spikes/sp-09/src/endpoint.rs`), without the `url`
//! crate: R8 builds its own URLs, so the parser only has to refuse what it does not build.
//! Errors are content-free: a code and a fixed message, never the host or the URL.

use std::fmt;

/// The path of the Genie One MCP server (G-02).
pub const GENIE_ONE_PATH: &str = "/ai-gateway/mcp-services/system.ai.genie_one_mcp";

/// The canonical segments of the Beta Genie MCP `/api/2.0/mcp/genie` (G-03).
const BETA_SEGMENTS: [&str; 4] = ["api", "2.0", "mcp", "genie"];
/// The canonical segments of the Genie One API SSE `/api/2.0/genie/one` (G-03).
const GENIE_ONE_API_SEGMENTS: [&str; 4] = ["api", "2.0", "genie", "one"];

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
    /// G-02: `?o=` is missing, repeated, not all digits or has a leading zero.
    MissingWorkspaceId,
    /// G-02: the query has a key other than the one `o=`.
    ExtraQuery,
    /// G-03: the URL path cannot be made canonical (bad or encoded `/` or `\\`, white space).
    NonCanonicalUrl,
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
            EndpointError::ExtraQuery => "mcp_url_extra_query",
            EndpointError::NonCanonicalUrl => "mcp_url_not_canonical",
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
            EndpointError::ExtraQuery => {
                "mcp_url_extra_query: the MCP URL query must hold only o=<workspace_id>."
            }
            EndpointError::NonCanonicalUrl => {
                "mcp_url_not_canonical: the URL path has an encoded / or \\, a bad percent \
                 escape or white space, so Bottle cannot check it."
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

/// Digits only, with no leading zero (so one workspace has one canonical URL).
pub(crate) fn valid_workspace_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id.bytes().all(|b| b.is_ascii_digit())
        && (id == "0" || !id.starts_with('0'))
}

/// A label that a URL parser reads as a number (decimal or `0x` hex), so the host is an IPv4
/// address (WHATWG "ends in a number").
fn numeric_label(label: &str) -> bool {
    let hex = label.get(..2).is_some_and(|p| p.eq_ignore_ascii_case("0x"))
        && label[2..].bytes().all(|b| b.is_ascii_hexdigit());
    hex || label.bytes().all(|b| b.is_ascii_digit())
}

/// A bare DNS host name: labels of ASCII letters, digits and `-`, joined by single dots. The
/// last label is not a number, so an IPv4 address or an all-numeric host is refused.
fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.rsplit('.').next().is_some_and(numeric_label)
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

/// The path of a URL or of path text: after `scheme:`, the `/` run and the authority, up to `?`
/// or `#`. Text with no scheme is a path.
fn path_of(url: &str) -> &str {
    let scheme_end = url.find(':').filter(|&i| {
        let scheme = &url[..i];
        scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    });
    let rest = match scheme_end {
        Some(i) => {
            let after = url[i + 1..].trim_start_matches('/');
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

/// The canonical segments of a path, in lower case. Each segment loses its `;params`, and its
/// percent-encoded unreserved characters are decoded. Empty and `.` segments are removed, and
/// `..` removes the segment before it (RFC 3986 5.2.4). Other escapes stay encoded.
/// A `\`, white space, a control character, a bad escape or an encoded `/` or `\` gives
/// [`EndpointError::NonCanonicalUrl`].
fn canonical_segments(path: &str) -> Result<Vec<String>, EndpointError> {
    if path
        .chars()
        .any(|c| c == '\\' || c.is_whitespace() || c.is_control())
    {
        return Err(EndpointError::NonCanonicalUrl);
    }
    let mut out: Vec<String> = Vec::new();
    for raw in path.split('/') {
        let raw = raw.split(';').next().unwrap_or("");
        let mut seg = String::with_capacity(raw.len());
        let mut bytes = raw.bytes();
        while let Some(b) = bytes.next() {
            if b != b'%' {
                seg.push(char::from(b.to_ascii_lowercase()));
                continue;
            }
            let hex = |b: Option<u8>| b.and_then(|b| char::from(b).to_digit(16));
            let (Some(hi), Some(lo)) = (hex(bytes.next()), hex(bytes.next())) else {
                return Err(EndpointError::NonCanonicalUrl);
            };
            let v = (hi * 16 + lo) as u8;
            if v.is_ascii_alphanumeric() || matches!(v, b'-' | b'.' | b'_' | b'~') {
                seg.push(char::from(v.to_ascii_lowercase()));
            } else if v == b'/' || v == b'\\' {
                return Err(EndpointError::NonCanonicalUrl);
            } else {
                seg.push_str(&format!("%{v:02x}"));
            }
        }
        match seg.as_str() {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(seg),
        }
    }
    Ok(out)
}

/// A canonical segment names `target` if it is `target`, or `target` and then escapes (such as
/// `genie%20`). The second form fails closed.
fn names(seg: &str, target: &str) -> bool {
    seg.strip_prefix(target)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('%'))
}

/// The G-03 rejection for one URL (or path text), if its canonical path is a rejected Genie
/// backend, or [`EndpointError::NonCanonicalUrl`] if the path cannot be made canonical.
/// A `\` anywhere in the URL is refused, because URL parsers read it as `/`.
pub fn rejected_backend(url: &str) -> Option<EndpointError> {
    let url = url.trim();
    if url.contains('\\') {
        return Some(EndpointError::NonCanonicalUrl);
    }
    let segs = match canonical_segments(path_of(url)) {
        Ok(segs) => segs,
        Err(e) => return Some(e),
    };
    let starts = |prefix: &[&str]| {
        segs.len() >= prefix.len() && segs.iter().zip(prefix).all(|(s, t)| names(s, t))
    };
    if starts(&BETA_SEGMENTS) {
        if segs.len() == BETA_SEGMENTS.len() {
            Some(EndpointError::BetaGenieMcp)
        } else {
            Some(EndpointError::PerSpaceGenieMcp)
        }
    } else if starts(&GENIE_ONE_API_SEGMENTS) {
        Some(EndpointError::GenieOneApiSse)
    } else {
        None
    }
}

impl GenieOneEndpoint {
    /// Build `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>`.
    /// `host` is a bare host or a workspace URL `https://<host>` (scheme in any case) with
    /// optional trailing `/`. The host is kept in lower case.
    pub fn new(host: &str, workspace_id: &str) -> Result<Self, EndpointError> {
        let trimmed = host.trim();
        let bare = match trimmed.get(..8) {
            Some(scheme) if scheme.eq_ignore_ascii_case("https://") => &trimmed[8..],
            _ => trimmed,
        };
        let bare = bare.trim_end_matches('/').to_ascii_lowercase();
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
        if !valid_host(host) || host.bytes().any(|b| b.is_ascii_uppercase()) {
            return Err(EndpointError::BadHost);
        }
        let (path, query) = match tail.split_once('?') {
            Some((p, q)) => (p, q),
            None => (tail, ""),
        };
        if path != GENIE_ONE_PATH {
            return Err(EndpointError::WrongPath);
        }
        let keys: Vec<&str> = query.split('&').collect();
        let mut ids = keys.iter().filter_map(|kv| kv.strip_prefix("o="));
        let id = match (ids.next(), ids.next()) {
            (Some(id), None) if valid_workspace_id(id) && !query.contains('#') => id,
            _ => return Err(EndpointError::MissingWorkspaceId),
        };
        if keys.len() != 1 {
            return Err(EndpointError::ExtraQuery);
        }
        let parsed = Self::new(host, id)?;
        if parsed.url != url {
            // Only the exact form that `new` builds is accepted.
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
        assert_eq!(
            GenieOneEndpoint::parse(&format!("{base}?o=1&x=2")),
            Err(EndpointError::ExtraQuery)
        );
        let upper = format!("https://H.cloud.databricks.com{GENIE_ONE_PATH}?o=1");
        assert_eq!(GenieOneEndpoint::parse(&upper), Err(EndpointError::BadHost));
    }
}
