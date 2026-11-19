//! `link_open` (#523): open a link from a Genie answer in the default browser.
//!
//! A view never opens a URL. It sends a click to the brain, the brain checks the URL against
//! the workspace of its window, and this component checks it again, because Rust trusts no
//! caller (SEC-25). [`validate_link`] accepts only an `https` URL with no credentials, no
//! control characters and no non-default port, whose host is the host of the profile's
//! workspace or one of the allowed app hosts. The opener then gets the normalised URL as one
//! argument; no shell runs.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::error::{CoreError, CoreResult};
use crate::http_transport::ProfileHosts;
use crate::ipc::LinkOpenArgs;
use crate::ipc_gateway::GatewayComponents;
use crate::recording_cloud::opener::UrlOpener;

const ID: &str = "link_open";

/// The longest URL Rust opens.
pub const MAX_URL_BYTES: usize = 2048;

fn refused() -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason: "link_refused",
    }
}

/// The lower-case host of a bare `https://host` origin or of a bare host name.
fn bare_host(origin_or_host: &str) -> Option<String> {
    let t = origin_or_host.trim().trim_end_matches('/');
    let host = t.strip_prefix("https://").unwrap_or(t);
    (!host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    .then(|| host.to_ascii_lowercase())
}

/// Check `url` and return it, ready to open. `workspace` is the profile's workspace URL
/// (`https://<host>`); `allowed_hosts` are extra app hosts that the user's policy trusts.
pub fn validate_link(url: &str, workspace: &str, allowed_hosts: &[&str]) -> CoreResult<String> {
    if url.len() > MAX_URL_BYTES || !url.is_ascii() {
        return Err(refused());
    }
    // Only `https://`, compared without case; this also rejects `javascript:`, `file:`, `http:`,
    // scheme-relative and relative forms before any parse.
    let rest = match url.get(..8) {
        Some(p) if p.eq_ignore_ascii_case("https://") => &url[8..],
        _ => return Err(refused()),
    };
    if rest.bytes().any(|b| {
        b.is_ascii_control() || b == b' ' || b == b'\\' || b == b'<' || b == b'>' || b == b'"'
    }) {
        return Err(refused());
    }
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    // No `user:pass@host` (`https://evil.com/@host` has its `@` after the path, so it is a path
    // on evil.com and fails the host check below), and no port: the default port only.
    if authority.is_empty() || authority.contains(['@', ':', '[', ']']) {
        return Err(refused());
    }
    let host = bare_host(authority).ok_or_else(refused)?;
    if host.starts_with('.') || host.ends_with('.') || host.contains("..") {
        return Err(refused());
    }
    let trusted = bare_host(workspace).is_some_and(|w| w == host)
        || allowed_hosts
            .iter()
            .filter_map(|a| bare_host(a))
            .any(|a| a == host);
    if !trusted {
        return Err(refused());
    }
    Ok(format!("https://{host}{}", &rest[authority_end..]))
}

/// The `link_open` command over T7 (profile hosts) and the OS opener.
pub struct LinkOpenComponent {
    hosts: Arc<dyn ProfileHosts>,
    opener: Arc<dyn UrlOpener>,
    allowed_hosts: Vec<String>,
}

impl LinkOpenComponent {
    /// `allowed_hosts` is the `AllowedAppUrls` host list (empty until a policy source exists).
    pub fn new(
        hosts: Arc<dyn ProfileHosts>,
        opener: Arc<dyn UrlOpener>,
        allowed_hosts: Vec<String>,
    ) -> Self {
        Self {
            hosts,
            opener,
            allowed_hosts,
        }
    }
}

impl GatewayComponents for LinkOpenComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        if command != ID {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "not_implemented",
            });
        }
        let a: LinkOpenArgs = serde_json::from_value(args).map_err(|_| CoreError::Unavailable {
            component: ID,
            reason: "invalid_args",
        })?;
        let workspace = self.hosts.workspace_url(&a.profile)?;
        let allowed: Vec<&str> = self.allowed_hosts.iter().map(String::as_str).collect();
        let url = validate_link(&a.url, &workspace, &allowed)?;
        self.opener.open(&url)?;
        Ok(json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const WS: &str = "https://acme.cloud.databricks.com";

    fn ok(url: &str) -> String {
        validate_link(url, WS, &[]).unwrap_or_else(|_| panic!("should allow {url}"))
    }
    fn no(url: &str) {
        assert!(validate_link(url, WS, &[]).is_err(), "should refuse {url}");
    }

    #[test]
    fn allows_the_workspace_host() {
        assert_eq!(
            ok("https://acme.cloud.databricks.com/one/chat/threads/c1?o=1&utm_source=x"),
            "https://acme.cloud.databricks.com/one/chat/threads/c1?o=1&utm_source=x"
        );
        assert_eq!(
            ok("HTTPS://ACME.cloud.databricks.com/a"),
            "https://acme.cloud.databricks.com/a"
        );
        ok("https://acme.cloud.databricks.com");
    }

    #[test]
    fn allows_an_allowed_app_host_only_when_listed() {
        let u = "https://app.example.com/x";
        assert!(validate_link(u, WS, &[]).is_err());
        assert!(validate_link(u, WS, &["https://app.example.com/base"]).is_err());
        assert!(validate_link(u, WS, &["app.example.com"]).is_ok());
        assert!(validate_link(u, WS, &["https://app.example.com"]).is_ok());
    }

    #[test]
    fn refuses_other_schemes() {
        for u in [
            "http://acme.cloud.databricks.com/",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,x",
            "ftp://acme.cloud.databricks.com/",
            "//acme.cloud.databricks.com/",
            "acme.cloud.databricks.com/",
            "mailto:a@acme.cloud.databricks.com",
            "https:acme.cloud.databricks.com",
            "https:/acme.cloud.databricks.com",
            "",
        ] {
            no(u);
        }
    }

    #[test]
    fn refuses_host_spoofing() {
        for u in [
            "https://acme.cloud.databricks.com.evil.com/",
            "https://evil.com/@acme.cloud.databricks.com",
            "https://evil.com/acme.cloud.databricks.com",
            "https://evil.com?acme.cloud.databricks.com",
            "https://acme.cloud.databricks.com@evil.com/",
            "https://user:pw@acme.cloud.databricks.com/",
            "https://acme.cloud.databricks.com:8443/",
            "https://acme.cloud.databricks.com:443/",
            "https://acme.cloud.databricks.com\\@evil.com/",
            "https://evilacme.cloud.databricks.com/",
            "https://cloud.databricks.com/",
            "https://acme.cloud.databricks.com../",
            "https://[::1]/",
            "https://acme.cloud.databricks.com%2f@evil.com/",
            "https://аcme.cloud.databricks.com/",
        ] {
            no(u);
        }
    }

    #[test]
    fn refuses_control_space_and_long_input() {
        no("https://acme.cloud.databricks.com/a b");
        no("https://acme.cloud.databricks.com/a\nb");
        no("https://acme.cloud.databricks.com/a\"b");
        no("https://acme.cloud.databricks.com/<x>");
        no(&format!(
            "https://acme.cloud.databricks.com/{}",
            "a".repeat(MAX_URL_BYTES)
        ));
    }

    struct Hosts;
    impl ProfileHosts for Hosts {
        fn workspace_url(&self, profile: &str) -> CoreResult<String> {
            if profile == "p" {
                Ok(WS.to_owned())
            } else {
                Err(refused())
            }
        }
    }
    #[derive(Default)]
    struct Spy(Mutex<Vec<String>>);
    impl UrlOpener for Spy {
        fn open(&self, url: &str) -> CoreResult<()> {
            self.0.lock().unwrap().push(url.to_owned());
            Ok(())
        }
    }

    fn component(spy: &Arc<Spy>) -> LinkOpenComponent {
        LinkOpenComponent::new(Arc::new(Hosts), spy.clone(), vec![])
    }

    #[test]
    fn opens_a_trusted_link_once() {
        let spy = Arc::new(Spy::default());
        let c = component(&spy);
        let r = c
            .call(
                "link_open",
                json!({"profile": "p", "url": "https://acme.cloud.databricks.com/one/chat/threads/c1?o=1"}),
            )
            .unwrap();
        assert_eq!(r, json!({}));
        assert_eq!(
            *spy.0.lock().unwrap(),
            ["https://acme.cloud.databricks.com/one/chat/threads/c1?o=1"]
        );
    }

    #[test]
    fn never_opens_an_untrusted_link_or_an_unknown_profile() {
        let spy = Arc::new(Spy::default());
        let c = component(&spy);
        for args in [
            json!({"profile": "p", "url": "https://evil.com/"}),
            json!({"profile": "p", "url": "http://acme.cloud.databricks.com/"}),
            json!({"profile": "other", "url": "https://acme.cloud.databricks.com/"}),
            json!({"profile": "p"}),
            json!({"profile": "p", "url": "https://acme.cloud.databricks.com/", "x": 1}),
        ] {
            assert!(c.call("link_open", args.clone()).is_err(), "{args}");
        }
        assert!(spy.0.lock().unwrap().is_empty());
    }
}
