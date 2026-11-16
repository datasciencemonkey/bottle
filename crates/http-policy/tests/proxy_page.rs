//! SEC-05a/b: a non-JSON upstream page is PROXY_PAGE, never shown, logged or retried.

use http_policy::{
    classify_response, execute, PolicyError, RawResponse, ResponseClass, TokenPolicy, MAX_ATTEMPTS,
    PROXY_PAGE_CARD_TEXT, PROXY_PAGE_CODE,
};

const LOGIN_PAGE: &[u8] =
    b"<!DOCTYPE html><html><head><title>Corp SSO</title></head><body>SECRET-MARKER-42 sign in</body></html>";

fn policy() -> TokenPolicy {
    TokenPolicy::new("https://dbc-1234.cloud.databricks.com").unwrap()
}

fn page(status: u16, ct: Option<&str>, body: &[u8]) -> RawResponse {
    RawResponse {
        status,
        content_type: ct.map(str::to_owned),
        location: None,
        body: body.to_vec(),
    }
}

/// [SEC-05a] accepts JSON content types with a JSON body
#[test]
fn sec_05a_json_is_json() {
    for ct in [
        "application/json",
        "application/json; charset=utf-8",
        "Application/JSON ; charset=UTF-8",
        "application/problem+json",
        "application/vnd.databricks.v1+json",
    ] {
        assert_eq!(
            classify_response(200, Some(ct), br#"{"a":1}"#),
            Ok(ResponseClass::Json),
            "{ct}"
        );
    }
    assert_eq!(
        classify_response(200, Some("application/json"), b"\xEF\xBB\xBF \n\t[1,2]"),
        Ok(ResponseClass::Json)
    );
    assert_eq!(
        classify_response(
            403,
            Some("application/json"),
            br#"{"error_code":"PERMISSION_DENIED"}"#
        ),
        Ok(ResponseClass::Json)
    );
}

/// [SEC-05a] an empty body is Empty, not a page
#[test]
fn sec_05a_empty_body_is_empty() {
    assert_eq!(classify_response(204, None, b""), Ok(ResponseClass::Empty));
    assert_eq!(
        classify_response(200, Some("application/json"), b"  \r\n"),
        Ok(ResponseClass::Empty)
    );
}

/// [SEC-05a] [AT-17] an HTML login page is PROXY_PAGE
#[test]
fn sec_05a_at_17_html_login_page_is_proxy_page() {
    assert_eq!(
        classify_response(200, Some("text/html; charset=utf-8"), LOGIN_PAGE),
        Err(PolicyError::ProxyPage { status: 200 })
    );
    assert_eq!(
        classify_response(407, Some("text/html"), LOGIN_PAGE),
        Err(PolicyError::ProxyPage { status: 407 })
    );
}

/// [SEC-05a] [AT-17] an HTML body behind a JSON content type is PROXY_PAGE (body sniff)
#[test]
fn sec_05a_at_17_html_with_json_content_type_is_proxy_page() {
    assert_eq!(
        classify_response(200, Some("application/json"), LOGIN_PAGE),
        Err(PolicyError::ProxyPage { status: 200 })
    );
    assert_eq!(
        classify_response(502, Some("application/json"), b"Bad Gateway"),
        Err(PolicyError::ProxyPage { status: 502 })
    );
}

/// [SEC-05a] a JSON body with a missing or non-JSON content type is PROXY_PAGE
#[test]
fn sec_05a_non_json_content_type_is_proxy_page() {
    for ct in [
        None,
        Some("text/plain"),
        Some("text/html"),
        Some("application/xml"),
        Some("application/jsonp"),
        Some(""),
    ] {
        assert_eq!(
            classify_response(200, ct, br#"{"a":1}"#),
            Err(PolicyError::ProxyPage { status: 200 }),
            "{ct:?}"
        );
    }
}

/// [SEC-05a] [AT-17] the typed error carries the PROXY_PAGE code and safe card text, never the body
#[test]
fn sec_05a_at_17_error_carries_code_and_safe_text_only() {
    let err = classify_response(200, Some("text/html"), LOGIN_PAGE).unwrap_err();
    assert_eq!(err.code(), PROXY_PAGE_CODE);
    assert_eq!(PROXY_PAGE_CODE, "PROXY_PAGE");
    assert_eq!(err.card_text(), PROXY_PAGE_CARD_TEXT);
    assert!(!PROXY_PAGE_CARD_TEXT.is_empty());
    for shown in [
        err.to_string(),
        format!("{err:?}"),
        err.card_text().to_owned(),
    ] {
        assert!(!shown.contains("SECRET-MARKER-42"), "{shown}");
        assert!(!shown.contains('<'), "{shown}");
        assert!(!shown.contains("Corp SSO"), "{shown}");
    }
    assert!(err.to_string().contains("PROXY_PAGE"));
}

/// [SEC-05a] other refusals have a code and text without URLs
#[test]
fn sec_05a_other_errors_have_codes() {
    for e in [
        PolicyError::InvalidUrl,
        PolicyError::NotHttps,
        PolicyError::NotPort443,
        PolicyError::UserInfo,
        PolicyError::IpLiteral,
        PolicyError::MissingHost,
        PolicyError::NonCanonicalHost,
        PolicyError::AccountsHost,
        PolicyError::NotDatabricksDomain,
        PolicyError::HostMismatch,
        PolicyError::CrossHostRedirect,
        PolicyError::RedirectDowngrade,
        PolicyError::MissingLocation,
        PolicyError::TooManyRedirects,
    ] {
        assert!(
            ["CLIENT_BUG", "NETWORK_BLOCKED"].contains(&e.code()),
            "{e:?}"
        );
        assert!(!e.card_text().is_empty());
        assert!(!e.to_string().contains("://"));
        assert!(!e.may_retry(), "{e:?}");
    }
}

/// [SEC-05a] [AT-17] the driver drops the body and returns only PROXY_PAGE
#[test]
fn sec_05a_at_17_execute_returns_proxy_page_without_body() {
    let out = execute(
        &policy(),
        "https://dbc-1234.cloud.databricks.com/api/2.0/x",
        |_| page(200, Some("text/html"), LOGIN_PAGE),
    );
    let err = out.result.unwrap_err();
    assert_eq!(err, PolicyError::ProxyPage { status: 200 });
    let dump = format!("{:?}", out.trace);
    assert!(!dump.contains("SECRET-MARKER-42"));
}

/// [SEC-05b] [AT-17] the trace shows no retry for a non-JSON page, also on 5xx
#[test]
fn sec_05b_at_17_execute_never_retries_proxy_page() {
    for status in [200, 401, 403, 502, 503, 504] {
        let mut sends = 0;
        let out = execute(
            &policy(),
            "https://dbc-1234.cloud.databricks.com/api/2.0/x",
            |_| {
                sends += 1;
                page(status, Some("text/html"), LOGIN_PAGE)
            },
        );
        assert_eq!(out.result.unwrap_err(), PolicyError::ProxyPage { status });
        assert_eq!(sends, 1, "status {status}");
        assert_eq!(out.trace.len(), 1);
        assert!(!PolicyError::ProxyPage { status }.may_retry());
    }
}

/// [SEC-05b] a transient JSON 503 is retried, at most MAX_ATTEMPTS times; a page on the retry stops it
#[test]
fn sec_05b_execute_retries_only_json_transient() {
    let mut sends = 0;
    let out = execute(
        &policy(),
        "https://dbc-1234.cloud.databricks.com/api/2.0/x",
        |_| {
            sends += 1;
            page(
                503,
                Some("application/json"),
                br#"{"error_code":"TEMPORARILY_UNAVAILABLE"}"#,
            )
        },
    );
    assert_eq!(sends, usize::from(MAX_ATTEMPTS));
    let (class, _) = out.result.unwrap();
    assert_eq!(class, ResponseClass::Json);

    let mut sends = 0;
    let out = execute(
        &policy(),
        "https://dbc-1234.cloud.databricks.com/api/2.0/x",
        |_| {
            sends += 1;
            if sends == 1 {
                page(
                    503,
                    Some("application/json"),
                    br#"{"error_code":"TEMPORARILY_UNAVAILABLE"}"#,
                )
            } else {
                page(503, Some("text/html"), LOGIN_PAGE)
            }
        },
    );
    assert_eq!(
        out.result.unwrap_err(),
        PolicyError::ProxyPage { status: 503 }
    );
    assert_eq!(sends, 2);
    assert!(out.trace.iter().all(|a| a.with_token));
}
