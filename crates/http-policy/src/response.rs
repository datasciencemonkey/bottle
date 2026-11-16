use crate::PolicyError;

/// The `BackendErrorCode` for a non-JSON upstream page.
pub const PROXY_PAGE_CODE: &str = "PROXY_PAGE";

/// The only text the card shows for a non-JSON upstream page.
pub const PROXY_PAGE_CARD_TEXT: &str = "";

/// How a response body may be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResponseClass {
    /// A JSON API answer. The caller may parse it.
    Json,
    /// No body (for example 204). Nothing to parse or show.
    Empty,
}

/// Classifies a response from its status, `Content-Type` and body bytes.
/// A non-JSON page is returned as [`PolicyError::ProxyPage`]; the body is not kept.
pub fn classify_response(
    status: u16,
    content_type: Option<&str>,
    body: &[u8],
) -> Result<ResponseClass, PolicyError> {
    let _ = (status, content_type, body);
    todo!("not implemented")
}
