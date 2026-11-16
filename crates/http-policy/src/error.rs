use std::fmt;

/// Every refusal of the policy layer. No variant carries a URL, header or body,
/// so `Display` and `Debug` can never leak upstream content or secrets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PolicyError {
    /// The URL could not be parsed.
    InvalidUrl,
    /// The scheme is not `https`.
    NotHttps,
    /// The port is not 443.
    NotPort443,
    /// The URL has a user name or password.
    UserInfo,
    /// The host is an IP literal.
    IpLiteral,
    /// The URL has no host.
    MissingHost,
    /// The host ends with a dot or is otherwise not in canonical form.
    NonCanonicalHost,
    /// The host is an `accounts.*` console host.
    AccountsHost,
    /// The host is not under a Databricks domain of Section 4.6.
    NotDatabricksDomain,
    /// The host is a Databricks host but not the exact profile host.
    HostMismatch,
    /// A redirect points to another host.
    CrossHostRedirect,
    /// A redirect drops HTTPS or port 443.
    RedirectDowngrade,
    /// A redirect has no usable `Location`.
    MissingLocation,
    /// More than [`crate::MAX_REDIRECTS`] redirects.
    TooManyRedirects,
    /// The upstream returned a non-JSON page (HTML login, proxy page...).
    ProxyPage {
        /// The HTTP status of the page. Only the number is kept.
        status: u16,
    },
}

impl PolicyError {
    /// The `BackendErrorCode` (Section 4.7) that the card shows.
    pub fn code(&self) -> &'static str {
        todo!("not implemented")
    }

    /// Whether the request may be sent again with the token. Never for `PROXY_PAGE`.
    pub fn may_retry(&self) -> bool {
        todo!("not implemented")
    }

    /// Safe, static card text for the error. Never contains upstream content.
    pub fn card_text(&self) -> &'static str {
        todo!("not implemented")
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let _ = f;
        todo!("not implemented")
    }
}

impl std::error::Error for PolicyError {}
