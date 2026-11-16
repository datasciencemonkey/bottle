use url::{Host, Url};

use crate::PolicyError;

/// Databricks domains of Section 4.6 rule 2. A profile host must be a subdomain of one.
pub const DATABRICKS_DOMAINS: [&str; 5] = [
    "cloud.databricks.com",
    "gcp.databricks.com",
    "azuredatabricks.net",
    "cloud.databricks.us",
    "databricks.azure.us",
];

/// The Databricks Apps domain (Section 4.6 rule 1). The AWS, Azure and GCP forms
/// (`*.aws.`, `*.<n>.azure.`, `*.gcp.databricksapps.com`) are all subdomains of it.
const APPS_DOMAIN: &str = "databricksapps.com";

/// The highest number of redirects followed for one request.
pub const MAX_REDIRECTS: u8 = 5;

/// Where a token-bearing request goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TokenTarget {
    /// The exact host of the profile.
    ProfileHost,
    /// An allowlisted Databricks App URL (`custom_agent`).
    AppUrl,
}

/// The result of [`TokenPolicy::check_redirect`]. Refusals are returned as `Err`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedirectDecision {
    /// Follow the redirect to this URL with the token.
    Follow(Url),
}

/// What kind of allowed host a URL names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum HostKind {
    Workspace,
    App,
}

/// Token-isolation policy for one CLI profile.
#[derive(Clone, Debug)]
pub struct TokenPolicy {
    profile_host: String,
}

impl TokenPolicy {
    /// Builds the policy from the profile `host` (a URL such as
    /// `https://dbc-1234.cloud.databricks.com/`). The host must itself pass the
    /// rules of Section 4.6 rule 2.
    pub fn new(profile_host: &str) -> Result<Self, PolicyError> {
        let url = parse(profile_host)?;
        let host = canonical_host(&url)?;
        match host_kind(&host)? {
            HostKind::Workspace => Ok(Self { profile_host: host }),
            HostKind::App => Err(PolicyError::NotDatabricksDomain),
        }
    }

    /// The normalised profile host name (lowercase, punycode, no port).
    pub fn profile_host(&self) -> &str {
        &self.profile_host
    }

    /// Decides whether the token may go to `url` (SEC-04a).
    pub fn check_request(&self, url: &str) -> Result<TokenTarget, PolicyError> {
        self.check_url(&parse(url)?)
    }

    /// Decides whether a 3xx from `current` to `location` may be followed with the
    /// token (SEC-04b). `hops` is the number of redirects already followed.
    ///
    /// Only a redirect to the same host, still over https on 443, that still passes
    /// [`TokenPolicy::check_request`] is followed. Every other redirect is refused,
    /// so the request fails and the token never leaves the host.
    pub fn check_redirect(
        &self,
        current: &Url,
        location: Option<&str>,
        hops: u8,
    ) -> Result<RedirectDecision, PolicyError> {
        if hops >= MAX_REDIRECTS {
            return Err(PolicyError::TooManyRedirects);
        }
        let location = location
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .ok_or(PolicyError::MissingLocation)?;
        let target = current
            .join(location)
            .map_err(|_| PolicyError::InvalidUrl)?;
        let same_host = match (current.host(), target.host()) {
            (Some(Host::Domain(a)), Some(Host::Domain(b))) => {
                a.trim_end_matches('.') == b.trim_end_matches('.')
            }
            _ => false,
        };
        if !same_host {
            return Err(PolicyError::CrossHostRedirect);
        }
        if target.scheme() != "https" || target.port_or_known_default() != Some(443) {
            return Err(PolicyError::RedirectDowngrade);
        }
        self.check_url(&target)?;
        Ok(RedirectDecision::Follow(target))
    }

    fn check_url(&self, url: &Url) -> Result<TokenTarget, PolicyError> {
        let host = canonical_host(url)?;
        match host_kind(&host)? {
            HostKind::Workspace if host == self.profile_host => Ok(TokenTarget::ProfileHost),
            HostKind::Workspace => Err(PolicyError::HostMismatch),
            HostKind::App => Ok(TokenTarget::AppUrl),
        }
    }
}

fn parse(raw: &str) -> Result<Url, PolicyError> {
    Url::parse(raw).map_err(|_| PolicyError::InvalidUrl)
}

/// Checks scheme, userinfo, host type and port, and returns the host name.
/// The `url` crate has already lowercased it, mapped IDN to punycode, decoded
/// percent-escapes and parsed numeric IPv4 forms.
fn canonical_host(url: &Url) -> Result<String, PolicyError> {
    if url.scheme() != "https" {
        return Err(PolicyError::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(PolicyError::UserInfo);
    }
    let host = match url.host() {
        None => return Err(PolicyError::MissingHost),
        Some(Host::Ipv4(_) | Host::Ipv6(_)) => return Err(PolicyError::IpLiteral),
        Some(Host::Domain(d)) => d,
    };
    if url.port_or_known_default() != Some(443) {
        return Err(PolicyError::NotPort443);
    }
    if host.is_empty() || host.ends_with('.') || host.split('.').any(str::is_empty) {
        return Err(PolicyError::NonCanonicalHost);
    }
    Ok(host.to_owned())
}

fn host_kind(host: &str) -> Result<HostKind, PolicyError> {
    if host.split('.').next() == Some("accounts") {
        return Err(PolicyError::AccountsHost);
    }
    if DATABRICKS_DOMAINS.iter().any(|d| is_subdomain(host, d)) {
        Ok(HostKind::Workspace)
    } else if is_subdomain(host, APPS_DOMAIN) {
        Ok(HostKind::App)
    } else {
        Err(PolicyError::NotDatabricksDomain)
    }
}

/// True when `host` is a strict subdomain of `domain` (label boundary).
fn is_subdomain(host: &str, domain: &str) -> bool {
    host.len() > domain.len() + 1
        && host.ends_with(domain)
        && host.as_bytes()[host.len() - domain.len() - 1] == b'.'
}
