//! The token egress seam for 4.6 rules 2, 3 and 6 (#224, ARC-6).
//!
//! This module holds no host allowlist, no redirect policy and no page classifier. Those are
//! #207 (`crates/http-policy`, PR #256). It holds the trait that R7 HttpTransport and R8
//! McpTransport call, and a contract suite that any implementation must pass.
//!
//! [`HttpPolicyEgress`] is the shipped implementation: an adapter over `http_policy::TokenPolicy`
//! and `http_policy::classify_response` (#207). `tests/token_isolation.rs` runs
//! [`contract_violations`] against it.

use http_policy::{classify_response, ResponseClass, TokenPolicy};
use serde_json::Value;

/// The 4.6 rules that [`contract_violations`] checks.
pub const CONTRACT_RULES: [u8; 3] = [2, 3, 6];

/// The decisions R7 and R8 need before a token leaves the core, and before a reply reaches TS.
pub trait TokenEgressPolicy {
    /// Rule 2: may the token of the profile whose host is `profile_host` go to `url`?
    fn may_send_token(&self, profile_host: &str, url: &str) -> bool;
    /// Rule 3: may a 3xx from `from_url` to `location_url` be followed with `Authorization`?
    fn may_follow_redirect_with_authorization(
        &self,
        profile_host: &str,
        from_url: &str,
        location_url: &str,
    ) -> bool;
    /// Rule 6: the JSON that may reach TS for an upstream reply, or `None` for a non-JSON page.
    fn reply_for_ts(&self, content_type: Option<&str>, body: &[u8]) -> Option<Value>;
    /// Rule 6: may a call that got a non-JSON page be retried with the token?
    fn may_retry_with_token_after_non_json(&self) -> bool;
}

/// One failed contract case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The 4.6 rule number.
    pub rule: u8,
    /// What the case checks (fake hosts only).
    pub case: String,
}

/// A fake workspace host in the Databricks domain list.
const PROFILE_HOST: &str = "dbc-fake.cloud.databricks.com";

/// Run every contract case against `policy` and return the cases it fails.
pub fn contract_violations(policy: &dyn TokenEgressPolicy) -> Vec<Violation> {
    let mut out = Vec::new();
    let mut expect = |rule: u8, ok: bool, case: String| {
        if !ok {
            out.push(Violation { rule, case });
        }
    };
    let p = PROFILE_HOST;

    // Rule 2: the exact profile host, HTTPS, port 443, Databricks domains, no accounts.*.
    let send_cases: [(&str, String, bool); 10] = [
        (p, format!("https://{p}/api/2.0/genie/spaces"), true),
        (
            "adb-1.2.azuredatabricks.net",
            "https://adb-1.2.azuredatabricks.net/api/2.0/x".into(),
            true,
        ),
        (p, format!("http://{p}/api/2.0/x"), false),
        (p, format!("https://{p}:8443/api/2.0/x"), false),
        (
            "accounts.cloud.databricks.com",
            "https://accounts.cloud.databricks.com/api/2.0/x".into(),
            false,
        ),
        (
            p,
            "https://other.cloud.databricks.com/api/2.0/x".into(),
            false,
        ),
        (p, "https://attacker.example/api/2.0/x".into(), false),
        (p, format!("https://{p}.attacker.io/api/2.0/x"), false),
        (p, format!("https://{p}@attacker.example/x"), false),
        (
            "dbc.attacker.example",
            "https://dbc.attacker.example/api/2.0/x".into(),
            false,
        ),
    ];
    for (host, url, want) in send_cases {
        let got = policy.may_send_token(host, &url);
        expect(
            2,
            got == want,
            format!("profile {host}: token to {url} must be {want}"),
        );
    }

    // Rule 3: no cross-host redirect with Authorization; same host stays possible.
    let from = format!("https://{p}/api/2.0/x");
    let redirect_cases: [(String, bool); 4] = [
        (format!("https://{p}/api/2.0/y"), true),
        ("https://attacker.example/y".into(), false),
        ("https://other.cloud.databricks.com/y".into(), false),
        (format!("http://{p}/y"), false),
    ];
    for (to, want) in redirect_cases {
        let got = policy.may_follow_redirect_with_authorization(p, &from, &to);
        expect(
            3,
            got == want,
            format!("redirect {from} -> {to} with Authorization must be {want}"),
        );
    }

    // Rule 6: no non-JSON page reaches TS, and no retry with the token after one.
    let page = b"<html><body>Sign in</body></html>".as_slice();
    for (ct, body, label) in [
        (Some("text/html; charset=utf-8"), page, "an HTML page"),
        (Some("application/json"), page, "HTML labelled as JSON"),
        (None, page, "a page with no content type"),
    ] {
        expect(
            6,
            policy.reply_for_ts(ct, body).is_none(),
            format!("{label} must not reach TS"),
        );
    }
    let json_body = br#"{"conversation_id":"c1"}"#.as_slice();
    expect(
        6,
        policy.reply_for_ts(Some("application/json"), json_body)
            == Some(serde_json::json!({"conversation_id": "c1"})),
        "a JSON reply must reach TS as JSON".into(),
    );
    expect(
        6,
        !policy.may_retry_with_token_after_non_json(),
        "a non-JSON page must not be retried with the token".into(),
    );
    out
}

/// The shipped [`TokenEgressPolicy`]: `http_policy` decides every case (#207).
///
/// A profile host that `TokenPolicy::new` refuses (not a Databricks domain, `accounts.*`, an IP
/// literal) gets no token anywhere.
#[derive(Debug, Default, Clone, Copy)]
pub struct HttpPolicyEgress;

impl HttpPolicyEgress {
    fn policy(profile_host: &str) -> Option<TokenPolicy> {
        TokenPolicy::new(&format!("https://{profile_host}/")).ok()
    }
}

impl TokenEgressPolicy for HttpPolicyEgress {
    fn may_send_token(&self, profile_host: &str, url: &str) -> bool {
        Self::policy(profile_host).is_some_and(|p| p.check_request(url).is_ok())
    }

    fn may_follow_redirect_with_authorization(
        &self,
        profile_host: &str,
        from_url: &str,
        location_url: &str,
    ) -> bool {
        let Some(policy) = Self::policy(profile_host) else {
            return false;
        };
        if policy.check_request(from_url).is_err() {
            return false;
        }
        let Ok(from) = http_policy::Url::parse(from_url) else {
            return false;
        };
        policy.check_redirect(&from, Some(location_url), 0).is_ok()
    }

    fn reply_for_ts(&self, content_type: Option<&str>, body: &[u8]) -> Option<Value> {
        match classify_response(200, content_type, body) {
            Ok(ResponseClass::Json) => serde_json::from_slice(body).ok(),
            Ok(ResponseClass::Empty) | Err(_) => None,
        }
    }

    fn may_retry_with_token_after_non_json(&self) -> bool {
        http_policy::PolicyError::ProxyPage { status: 200 }.may_retry()
    }
}
