//! SEC-04b: never send the token across a redirect to another host.

use http_policy::{
    execute, OutboundRequest, PolicyError, RawResponse, RedirectDecision, TokenPolicy,
    MAX_REDIRECTS,
};
use url::Url;

fn policy() -> TokenPolicy {
    TokenPolicy::new("https://dbc-1234.cloud.databricks.com").unwrap()
}

fn here() -> Url {
    Url::parse("https://dbc-1234.cloud.databricks.com/api/2.0/genie/x").unwrap()
}

fn redirect(to: &str) -> RawResponse {
    RawResponse {
        status: 302,
        content_type: Some("text/html".into()),
        location: Some(to.into()),
        body: b"<html>moved</html>".to_vec(),
    }
}

fn json_ok() -> RawResponse {
    RawResponse {
        status: 200,
        content_type: Some("application/json".into()),
        location: None,
        body: br#"{"ok":true}"#.to_vec(),
    }
}

/// [SEC-04b] follows a same-host https redirect (absolute and relative)
#[test]
fn sec_04b_follows_same_host_redirect() {
    let p = policy();
    let want = Url::parse("https://dbc-1234.cloud.databricks.com/api/2.0/genie/y").unwrap();
    assert_eq!(
        p.check_redirect(
            &here(),
            Some("https://DBC-1234.cloud.databricks.com:443/api/2.0/genie/y"),
            0
        ),
        Ok(RedirectDecision::Follow(want.clone()))
    );
    assert_eq!(
        p.check_redirect(&here(), Some("y"), 0),
        Ok(RedirectDecision::Follow(want.clone()))
    );
    assert_eq!(
        p.check_redirect(&here(), Some("/api/2.0/genie/y"), 0),
        Ok(RedirectDecision::Follow(want))
    );
}

/// [SEC-04b] [AT-17] refuses a redirect to another host
#[test]
fn sec_04b_at_17_refuses_cross_host_redirect() {
    let p = policy();
    for loc in [
        "https://evil.example.com/steal",
        "https://dbc-9999.cloud.databricks.com/api",
        "https://accounts.cloud.databricks.com/login",
        "//evil.example.com/steal",
        "https://dbc-1234.cloud.databricks.com.evil.com/",
        "https://127.0.0.1/",
        "https://my-agent.aws.databricksapps.com/api/invocations",
    ] {
        assert_eq!(
            p.check_redirect(&here(), Some(loc), 0),
            Err(PolicyError::CrossHostRedirect),
            "{loc}"
        );
    }
}

/// [SEC-04b] [AT-17] refuses a same-host redirect that drops https, port 443 or adds userinfo
#[test]
fn sec_04b_at_17_refuses_downgrade_redirect() {
    let p = policy();
    for loc in [
        "http://dbc-1234.cloud.databricks.com/api",
        "https://dbc-1234.cloud.databricks.com:8443/api",
    ] {
        assert_eq!(
            p.check_redirect(&here(), Some(loc), 0),
            Err(PolicyError::RedirectDowngrade),
            "{loc}"
        );
    }
    assert_eq!(
        p.check_redirect(
            &here(),
            Some("https://u:p@dbc-1234.cloud.databricks.com/api"),
            0
        ),
        Err(PolicyError::UserInfo)
    );
    assert_eq!(
        p.check_redirect(
            &here(),
            Some("https://dbc-1234.cloud.databricks.com./api"),
            0
        ),
        Err(PolicyError::NonCanonicalHost)
    );
}

/// [SEC-04b] refuses a redirect with no Location and stops after MAX_REDIRECTS hops
#[test]
fn sec_04b_missing_location_and_hop_limit() {
    let p = policy();
    assert_eq!(
        p.check_redirect(&here(), None, 0),
        Err(PolicyError::MissingLocation)
    );
    assert_eq!(
        p.check_redirect(&here(), Some(""), 0),
        Err(PolicyError::MissingLocation)
    );
    assert_eq!(
        p.check_redirect(&here(), Some("/api/y"), MAX_REDIRECTS),
        Err(PolicyError::TooManyRedirects)
    );
    assert!(p
        .check_redirect(&here(), Some("/api/y"), MAX_REDIRECTS - 1)
        .is_ok());
}

/// [SEC-04b] an app-URL request may only redirect within the same app host
#[test]
fn sec_04b_app_url_redirect_same_host_only() {
    let p = policy();
    let app = Url::parse("https://my-agent.aws.databricksapps.com/api/invocations").unwrap();
    assert!(p.check_redirect(&app, Some("/api/responses"), 0).is_ok());
    assert_eq!(
        p.check_redirect(
            &app,
            Some("https://other-agent.aws.databricksapps.com/api"),
            0
        ),
        Err(PolicyError::CrossHostRedirect)
    );
}

/// [SEC-04b] [AT-17] malicious redirect fails in the driver: the token never leaves the host
#[test]
fn sec_04b_at_17_execute_malicious_redirect_fails() {
    let p = policy();
    let mut sent: Vec<OutboundRequest> = Vec::new();
    let out = execute(
        &p,
        "https://dbc-1234.cloud.databricks.com/api/2.0/x",
        |req| {
            sent.push(req.clone());
            redirect("https://evil.example.com/collect")
        },
    );
    assert_eq!(out.result.unwrap_err(), PolicyError::CrossHostRedirect);
    assert_eq!(sent.len(), 1, "no request to the redirect target");
    assert!(sent
        .iter()
        .all(|r| r.url.host_str() == Some("dbc-1234.cloud.databricks.com")));
    assert_eq!(out.trace.len(), 1);
    assert_eq!(out.trace[0].status, 302);
}

/// [SEC-04b] the driver follows a same-host redirect with the token
#[test]
fn sec_04b_execute_follows_same_host_redirect() {
    let p = policy();
    let mut n = 0;
    let out = execute(
        &p,
        "https://dbc-1234.cloud.databricks.com/api/2.0/x",
        |req| {
            n += 1;
            assert!(req.with_token);
            assert_eq!(req.url.host_str(), Some("dbc-1234.cloud.databricks.com"));
            if n == 1 {
                redirect("/api/2.0/y")
            } else {
                json_ok()
            }
        },
    );
    let (_, body) = out.result.unwrap();
    assert_eq!(body, br#"{"ok":true}"#);
    assert_eq!(out.trace.len(), 2);
}

/// [SEC-04b] the driver stops a same-host redirect loop
#[test]
fn sec_04b_execute_stops_redirect_loop() {
    let p = policy();
    let out = execute(&p, "https://dbc-1234.cloud.databricks.com/a", |_| {
        redirect("/a")
    });
    assert_eq!(out.result.unwrap_err(), PolicyError::TooManyRedirects);
    assert_eq!(out.trace.len(), usize::from(MAX_REDIRECTS) + 1);
}

/// [SEC-04a] the driver sends nothing to a refused host
#[test]
fn sec_04a_execute_refuses_bad_host_before_send() {
    let p = policy();
    let out = execute(&p, "https://accounts.cloud.databricks.com/api", |_| {
        panic!("must not send")
    });
    assert_eq!(out.result.unwrap_err(), PolicyError::AccountsHost);
    assert!(out.trace.is_empty());
}
