use std::fmt;

use crate::response::{PROXY_PAGE_CARD_TEXT, PROXY_PAGE_CODE};

/// Card text when Bottle refuses to send the token to a request URL.
const REFUSED_HOST_TEXT: &str =
    "Bottle did not send your sign-in because the address is not your workspace. Nothing was sent.";

/// Card text when the upstream redirects Bottle somewhere it may not follow.
const REFUSED_REDIRECT_TEXT: &str = "The workspace sent Bottle to another address. Bottle stopped \
     to protect your sign-in. Check your VPN or proxy, then retry.";

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
    ///
    /// - A refused request URL is `CLIENT_BUG`: Bottle built or was given an address
    ///   that the token may not go to.
    /// - A refused redirect is `NETWORK_BLOCKED`: something upstream (proxy, captive
    ///   portal, attacker) moved the request.
    /// - A non-JSON page is `PROXY_PAGE`.
    pub fn code(&self) -> &'static str {
        match self {
            Self::ProxyPage { .. } => PROXY_PAGE_CODE,
            Self::CrossHostRedirect
            | Self::RedirectDowngrade
            | Self::MissingLocation
            | Self::TooManyRedirects => "NETWORK_BLOCKED",
            _ => "CLIENT_BUG",
        }
    }

    /// Whether the request may be sent again with the token. Never for `PROXY_PAGE`
    /// (Section 4.6 rule 6), and never for a policy refusal.
    pub fn may_retry(&self) -> bool {
        false
    }

    /// Safe, static card text for the error. Never contains upstream content.
    pub fn card_text(&self) -> &'static str {
        match self.code() {
            PROXY_PAGE_CODE => PROXY_PAGE_CARD_TEXT,
            "NETWORK_BLOCKED" => REFUSED_REDIRECT_TEXT,
            _ => REFUSED_HOST_TEXT,
        }
    }

    fn reason(&self) -> &'static str {
        match self {
            Self::InvalidUrl => "the URL is not valid",
            Self::NotHttps => "the URL is not https",
            Self::NotPort443 => "the port is not 443",
            Self::UserInfo => "the URL has user info",
            Self::IpLiteral => "the host is an IP literal",
            Self::MissingHost => "the URL has no host",
            Self::NonCanonicalHost => "the host is not canonical",
            Self::AccountsHost => "accounts hosts are not allowed",
            Self::NotDatabricksDomain => "the host is not an allowed Databricks host",
            Self::HostMismatch => "the host is not the profile host",
            Self::CrossHostRedirect => "a redirect points to another host",
            Self::RedirectDowngrade => "a redirect drops https or port 443",
            Self::MissingLocation => "a redirect has no location",
            Self::TooManyRedirects => "too many redirects",
            Self::ProxyPage { .. } => "the upstream returned a non-JSON page",
        }
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.reason())?;
        if let Self::ProxyPage { status } = self {
            write!(f, " (status {status})")?;
        }
        Ok(())
    }
}

impl std::error::Error for PolicyError {}
