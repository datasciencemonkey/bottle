//! SEC-04a: the token goes only to the exact profile host over HTTPS 443 or to an
//! allowlisted Databricks App URL.

use http_policy::{PolicyError, TokenPolicy, TokenTarget, DATABRICKS_DOMAINS};

const PROFILE: &str = "https://dbc-1234.cloud.databricks.com";

fn policy() -> TokenPolicy {
    TokenPolicy::new(PROFILE).expect("valid profile host")
}

/// [SEC-04a] accepts the exact profile host over https on 443
#[test]
fn accepts_exact_profile_host() {
    let p = policy();
    assert_eq!(p.profile_host(), "dbc-1234.cloud.databricks.com");
    assert_eq!(
        p.check_request("https://dbc-1234.cloud.databricks.com/api/2.0/preview/scim/v2/Me"),
        Ok(TokenTarget::ProfileHost)
    );
}

/// [SEC-04a] accepts every Databricks domain of Section 4.6 as a profile host
#[test]
fn accepts_every_databricks_domain() {
    for d in DATABRICKS_DOMAINS {
        let host = format!("https://ws-1.{d}/");
        let p = TokenPolicy::new(&host).unwrap_or_else(|e| panic!("{d}: {e:?}"));
        assert_eq!(
            p.check_request(&format!("{host}api/2.0/x")),
            Ok(TokenTarget::ProfileHost)
        );
    }
    let azure = TokenPolicy::new("https://adb-123.4.azuredatabricks.net").unwrap();
    assert_eq!(azure.profile_host(), "adb-123.4.azuredatabricks.net");
}

/// [SEC-04a] [AT-17] rejects accounts.* hosts as profile hosts and request targets
#[test]
fn rejects_accounts_hosts() {
    for h in [
        "https://accounts.cloud.databricks.com",
        "https://accounts.azuredatabricks.net",
        "https://accounts.gcp.databricks.com",
        "https://ACCOUNTS.cloud.databricks.us",
        "https://accounts.databricks.azure.us",
    ] {
        assert_eq!(
            TokenPolicy::new(h).unwrap_err(),
            PolicyError::AccountsHost,
            "{h}"
        );
    }
    assert_eq!(
        policy().check_request("https://accounts.cloud.databricks.com/api/2.0/accounts"),
        Err(PolicyError::AccountsHost)
    );
    assert_eq!(
        policy().check_request("https://accounts.aws.databricksapps.com/api/invocations"),
        Err(PolicyError::AccountsHost)
    );
}

/// [SEC-04a] [AT-17] rejects another Databricks workspace host
#[test]
fn rejects_other_workspace_host() {
    assert_eq!(
        policy().check_request("https://dbc-9999.cloud.databricks.com/api/2.0/x"),
        Err(PolicyError::HostMismatch)
    );
}

/// [SEC-04a] [AT-17] rejects non-Databricks hosts and suffix tricks
#[test]
fn rejects_non_databricks_and_suffix_tricks() {
    let p = policy();
    for u in [
        "https://evil.example.com/",
        "https://dbc-1234.cloud.databricks.com.evil.com/",
        "https://evilcloud.databricks.com/",
        "https://dbc-1234.databricks.com/",
        "https://dbc-1234-cloud.databricks.com/",
        "https://cloud.databricks.com/",
    ] {
        let err = p.check_request(u).unwrap_err();
        assert!(
            matches!(
                err,
                PolicyError::NotDatabricksDomain | PolicyError::HostMismatch
            ),
            "{u}: {err:?}"
        );
    }
    for h in [
        "https://evil.example.com",
        "https://cloud.databricks.com",
        "https://x.databricks.com",
    ] {
        assert_eq!(
            TokenPolicy::new(h).unwrap_err(),
            PolicyError::NotDatabricksDomain,
            "{h}"
        );
    }
}

/// [SEC-04a] [AT-17] the userinfo and fragment tricks do not move the token
#[test]
fn rejects_userinfo_and_at_tricks() {
    let p = policy();
    assert_eq!(
        p.check_request("https://dbc-1234.cloud.databricks.com@evil.example.com/"),
        Err(PolicyError::UserInfo)
    );
    assert_eq!(
        p.check_request("https://user:pw@dbc-1234.cloud.databricks.com/"),
        Err(PolicyError::UserInfo)
    );
    assert_eq!(
        p.check_request("https://evil.example.com#@dbc-1234.cloud.databricks.com/"),
        Err(PolicyError::NotDatabricksDomain)
    );
    assert_eq!(
        p.check_request("https://evil.example.com\\@dbc-1234.cloud.databricks.com/"),
        Err(PolicyError::NotDatabricksDomain)
    );
    assert_eq!(
        TokenPolicy::new("https://u@dbc-1234.cloud.databricks.com").unwrap_err(),
        PolicyError::UserInfo
    );
}

/// [SEC-04a] normalises uppercase hosts before the exact match
#[test]
fn uppercase_hosts_are_normalised() {
    let p = TokenPolicy::new("https://DBC-1234.Cloud.Databricks.COM/").unwrap();
    assert_eq!(p.profile_host(), "dbc-1234.cloud.databricks.com");
    assert_eq!(
        p.check_request("HTTPS://dbc-1234.CLOUD.databricks.com/api"),
        Ok(TokenTarget::ProfileHost)
    );
}

/// [SEC-04a] refuses trailing-dot hosts (non-canonical)
#[test]
fn trailing_dot_is_refused() {
    assert_eq!(
        TokenPolicy::new("https://dbc-1234.cloud.databricks.com.").unwrap_err(),
        PolicyError::NonCanonicalHost
    );
    assert_eq!(
        policy().check_request("https://dbc-1234.cloud.databricks.com./api"),
        Err(PolicyError::NonCanonicalHost)
    );
}

/// [SEC-04a] accepts an explicit :443 and refuses any other port
#[test]
fn port_rules() {
    let p = policy();
    assert_eq!(
        p.check_request("https://dbc-1234.cloud.databricks.com:443/api"),
        Ok(TokenTarget::ProfileHost)
    );
    for u in [
        "https://dbc-1234.cloud.databricks.com:8443/api",
        "https://dbc-1234.cloud.databricks.com:80/api",
    ] {
        assert_eq!(p.check_request(u), Err(PolicyError::NotPort443), "{u}");
    }
    assert_eq!(
        TokenPolicy::new("https://dbc-1234.cloud.databricks.com:8443").unwrap_err(),
        PolicyError::NotPort443
    );
}

/// [SEC-04a] [AT-17] refuses http and other schemes (downgrade)
#[test]
fn refuses_http_downgrade() {
    let p = policy();
    for u in [
        "http://dbc-1234.cloud.databricks.com/api",
        "http://dbc-1234.cloud.databricks.com:443/api",
        "ws://dbc-1234.cloud.databricks.com/api",
        "wss://dbc-1234.cloud.databricks.com/api",
        "ftp://dbc-1234.cloud.databricks.com/api",
    ] {
        assert_eq!(p.check_request(u), Err(PolicyError::NotHttps), "{u}");
    }
    assert_eq!(
        TokenPolicy::new("http://dbc-1234.cloud.databricks.com").unwrap_err(),
        PolicyError::NotHttps
    );
}

/// [SEC-04a] [AT-17] refuses IPv4 and IPv6 literals, also in hex and decimal forms
#[test]
fn refuses_ip_literals() {
    let p = policy();
    for u in [
        "https://127.0.0.1/api",
        "https://169.254.169.254/latest/meta-data",
        "https://0x7f000001/",
        "https://2130706433/",
        "https://[::1]/",
        "https://[fe80::1]/",
    ] {
        assert_eq!(p.check_request(u), Err(PolicyError::IpLiteral), "{u}");
    }
    assert_eq!(
        TokenPolicy::new("https://10.0.0.1").unwrap_err(),
        PolicyError::IpLiteral
    );
}

/// [SEC-04a] [AT-17] IDN homographs become punycode and never match
#[test]
fn idn_homographs_do_not_match() {
    let p = policy();
    // Cyrillic small letter i (U+0456) in "databr\u{456}cks".
    let err = p
        .check_request("https://dbc-1234.cloud.databr\u{456}cks.com/api")
        .unwrap_err();
    assert_eq!(err, PolicyError::NotDatabricksDomain);
    let puny = url::Url::parse("https://dbc-1234.cloud.databr\u{456}cks.com/").unwrap();
    let puny = format!("https://{}/api", puny.host_str().unwrap());
    assert!(puny.contains("xn--"), "{puny}");
    assert_eq!(
        p.check_request(&puny),
        Err(PolicyError::NotDatabricksDomain)
    );
    // A Unicode label in the workspace part is punycoded and compared exactly.
    let idn = TokenPolicy::new("https://d\u{e9}mo.cloud.databricks.com").unwrap();
    assert!(
        idn.profile_host().starts_with("xn--"),
        "{}",
        idn.profile_host()
    );
    assert_eq!(
        idn.check_request("https://D\u{c9}MO.cloud.databricks.com/api"),
        Ok(TokenTarget::ProfileHost)
    );
    assert_eq!(
        idn.check_request("https://demo.cloud.databricks.com/api"),
        Err(PolicyError::HostMismatch)
    );
}

/// [SEC-04a] refuses unparsable or host-less URLs
#[test]
fn refuses_invalid_urls() {
    let p = policy();
    assert_eq!(p.check_request("not a url"), Err(PolicyError::InvalidUrl));
    assert_eq!(p.check_request("/api/2.0/x"), Err(PolicyError::InvalidUrl));
    assert_eq!(p.check_request(""), Err(PolicyError::InvalidUrl));
    assert_eq!(TokenPolicy::new("").unwrap_err(), PolicyError::InvalidUrl);
    assert!(p.check_request("file:///etc/passwd").is_err());
}

/// [SEC-04a] accepts the allowlisted Databricks App URL forms (AWS, Azure, GCP)
#[test]
fn accepts_app_urls() {
    let p = policy();
    for u in [
        "https://my-agent-1234567890.aws.databricksapps.com/api/invocations",
        "https://my-agent-1234567890.11.azure.databricksapps.com/api/responses",
        "https://my-agent-1234567890.gcp.databricksapps.com/api/invocations",
        "https://my-agent-1234567890.databricksapps.com/api/invocations",
        "https://MY-AGENT.aws.DatabricksApps.com:443/api/invocations",
    ] {
        assert_eq!(p.check_request(u), Ok(TokenTarget::AppUrl), "{u}");
    }
}

/// [SEC-04a] [AT-17] refuses look-alike or downgraded app URLs
#[test]
fn refuses_bad_app_urls() {
    let p = policy();
    for (u, want) in [
        (
            "http://my-agent.aws.databricksapps.com/api",
            PolicyError::NotHttps,
        ),
        (
            "https://my-agent.aws.databricksapps.com:8443/api",
            PolicyError::NotPort443,
        ),
        (
            "https://databricksapps.com/api",
            PolicyError::NotDatabricksDomain,
        ),
        (
            "https://evildatabricksapps.com/api",
            PolicyError::NotDatabricksDomain,
        ),
        (
            "https://my-agent.databricksapps.com.evil.com/api",
            PolicyError::NotDatabricksDomain,
        ),
        (
            "https://u:p@my-agent.aws.databricksapps.com/api",
            PolicyError::UserInfo,
        ),
        (
            "https://my-agent.aws.databricksapps.com./api",
            PolicyError::NonCanonicalHost,
        ),
    ] {
        assert_eq!(p.check_request(u), Err(want), "{u}");
    }
    // An app URL is never a profile host.
    assert_eq!(
        TokenPolicy::new("https://my-agent.aws.databricksapps.com").unwrap_err(),
        PolicyError::NotDatabricksDomain
    );
}

/// [SEC-04a] a profile host with a path or trailing slash is accepted; only the host counts
#[test]
fn profile_host_with_path() {
    let p = TokenPolicy::new("https://dbc-1234.cloud.databricks.com/?o=123").unwrap();
    assert_eq!(p.profile_host(), "dbc-1234.cloud.databricks.com");
}
