use url::Url;

use crate::PolicyError;

/// Databricks domains of Section 4.6 rule 2. A profile host must be a subdomain of one.
pub const DATABRICKS_DOMAINS: [&str; 5] = [
    "cloud.databricks.com",
    "gcp.databricks.com",
    "azuredatabricks.net",
    "cloud.databricks.us",
    "databricks.azure.us",
];

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

/// The result of [`TokenPolicy::check_redirect`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RedirectDecision {
    /// Follow the redirect to this URL with the token.
    Follow(Url),
}

/// Token-isolation policy for one CLI profile.
#[derive(Clone, Debug)]
pub struct TokenPolicy {
    profile_host: String,
}

impl TokenPolicy {
    /// Builds the policy from the profile `host` (a URL such as
    /// `https://dbc-1234.cloud.databricks.com/`).
    pub fn new(profile_host: &str) -> Result<Self, PolicyError> {
        let _ = profile_host;
        todo!("not implemented")
    }

    /// The normalised profile host name.
    pub fn profile_host(&self) -> &str {
        &self.profile_host
    }

    /// Decides whether the token may go to `url`.
    pub fn check_request(&self, url: &str) -> Result<TokenTarget, PolicyError> {
        let _ = url;
        todo!("not implemented")
    }

    /// Decides whether a 3xx from `current` to `location` may be followed with the
    /// token. `hops` is the number of redirects already followed.
    pub fn check_redirect(
        &self,
        current: &Url,
        location: Option<&str>,
        hops: u8,
    ) -> Result<RedirectDecision, PolicyError> {
        let _ = (current, location, hops);
        todo!("not implemented")
    }
}
