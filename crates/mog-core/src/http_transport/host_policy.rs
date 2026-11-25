//! The production token egress policy of R7 (#364, ADR 0046): 4.6 rules 2, 3 and 6.
//!
//! TODO(#207): this narrow policy stands in until `crates/http-policy` (PR #256) merges. Then
//! an adapter over `http_policy::TokenPolicy` replaces it behind the same
//! [`TokenEgressPolicy`] seam, and the R7 tests keep passing.
//!
//! The host check follows the workspace URL rule of
//! datasciencemonkey/antigravity-bricks-plugin `genie_bridge.py` (`workspace_url`): https only,
//! port 443, no user info, no `accounts` or `accounts-*` first label, the Databricks domain list,
//! DNS labels only. No code is copied.
//!
//! There is no loopback or plain-HTTP exception here. Tests that use a loopback server bring
//! their own test-double policy.

use reqwest::Url;
use serde_json::Value;

use crate::egress_policy::TokenEgressPolicy;

/// The Databricks domain list of 4.6 rule 2. `staging.databricks.com` and `dev.databricks.com`
/// cover internal staging/dev workspaces (e.g. `dogfood.staging.databricks.com`), owner review
/// 2026-11-24: Connect and `profile_reauth` refused those hosts before this list named them.
pub const DATABRICKS_DOMAINS: [&str; 7] = [
    "cloud.databricks.com",
    "gcp.databricks.com",
    "azuredatabricks.net",
    "cloud.databricks.us",
    "databricks.azure.us",
    "staging.databricks.com",
    "dev.databricks.com",
];

/// The production [`TokenEgressPolicy`]. TODO(#207): replace with the http-policy adapter.
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowlistEgressPolicy;

fn is_dns_label(l: &str) -> bool {
    let b = l.as_bytes();
    !b.is_empty()
        && b.len() <= 63
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
}

/// True if `host` is a workspace host that may get a token (rule 2): Databricks domain,
/// DNS labels only, not an `accounts` host.
pub fn is_workspace_host(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    let first = labels[0];
    DATABRICKS_DOMAINS
        .iter()
        .any(|d| host.len() > d.len() + 1 && host.ends_with(&format!(".{d}")))
        && labels.iter().all(|l| is_dns_label(l))
        && first != "accounts"
        && !first.starts_with("accounts-")
}

/// The host of `url` if the token of `profile_host` may go there (rule 2).
fn token_host(profile_host: &str, url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    if u.port().is_some_and(|p| p != 443) {
        return None;
    }
    // `domain()` is `None` for an IP address host.
    let host = u.domain()?.to_owned();
    (host == profile_host && is_workspace_host(&host)).then_some(host)
}

impl TokenEgressPolicy for AllowlistEgressPolicy {
    fn may_send_token(&self, profile_host: &str, url: &str) -> bool {
        token_host(profile_host, url).is_some()
    }

    fn may_follow_redirect_with_authorization(
        &self,
        profile_host: &str,
        from_url: &str,
        location_url: &str,
    ) -> bool {
        token_host(profile_host, from_url).is_some()
            && token_host(profile_host, location_url).is_some()
    }

    fn reply_for_ts(&self, content_type: Option<&str>, body: &[u8]) -> Option<Value> {
        let essence = content_type?.split(';').next()?.trim().to_ascii_lowercase();
        let json_type = essence == "application/json"
            || (essence.starts_with("application/") && essence.ends_with("+json"));
        if !json_type {
            return None;
        }
        serde_json::from_slice(body).ok()
    }

    fn may_retry_with_token_after_non_json(&self) -> bool {
        false
    }
}
