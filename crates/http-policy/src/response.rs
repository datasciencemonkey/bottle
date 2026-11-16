use crate::PolicyError;

/// The `BackendErrorCode` for a non-JSON upstream page.
pub const PROXY_PAGE_CODE: &str = "PROXY_PAGE";

/// The only text the card shows for a non-JSON upstream page (SEC-05a). The
/// failure action map offers [Retry]; the retry is a new user action, not an
/// automatic resend.
pub const PROXY_PAGE_CARD_TEXT: &str = "Bottle got a web page instead of a Databricks answer. \
     A proxy, VPN or sign-in page may be in the way. Check your network, then select Retry.";

/// How a response body may be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseClass {
    /// A JSON API answer. The caller may parse it.
    Json,
    /// No body (for example 204). Nothing to parse or show.
    Empty,
}

/// Classifies a response from its status, `Content-Type` and body bytes.
///
/// A body is JSON only when the media type is `application/json` or
/// `application/*+json` AND the first byte after an optional UTF-8 BOM and
/// whitespace is `{` or `[`. Anything else is a non-JSON page and is returned as
/// [`PolicyError::ProxyPage`], which keeps only the status, never the body.
pub fn classify_response(
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<ResponseClass, PolicyError> {
    let body = body.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(body);
    let first = body.iter().find(|b| !b.is_ascii_whitespace());
    let Some(first) = first else {
        return Ok(ResponseClass::Empty);
    };
    if is_json_media_type(content_type) && matches!(first, b'{' | b'[') {
        Ok(ResponseClass::Json)
    } else {
        Err(PolicyError::ProxyPage { status })
    }
}

fn is_json_media_type(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else {
        return false;
    };
    let media = ct
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    media == "application/json" || (media.starts_with("application/") && media.ends_with("+json"))
}
