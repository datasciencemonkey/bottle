//! Bottle request identity: the User-Agent and the AI Gateway request tags (#197).
//!
//! - SEC-06b, B0: every outbound call carries `User-Agent: bottle/<ver> (<os>; mog)`.
//! - SEC-06a, D57: every Unity Gateway call carries `Databricks-Ai-Gateway-Request-Tags`, a
//!   JSON string map with the keys `app`, `mog` and `op` only.
//!
//! This module builds the header values and holds the [`RequestDecorator`] seam. R7
//! HttpTransport and R8 McpTransport call the decorator for every request.
//! TODO(#207): the real R7 transport (PR #256) calls [`RequestDecorator::decorate`].
//! TODO(#136): SP-18 confirms if `MCP_SERVICE` usage rows keep the tags.
//!
//! The tag value holds no PII, transcript, token or URL: the MOG ID must be a short slug and
//! the value must not match a rule-7 secret pattern. A bad value fails closed (ADR 0023).

use std::fmt;

use crate::token_guard::contains_secret;
use crate::types::{TagApp, TagOp, Tags};

/// The AI Gateway request tags header (SEC-06a).
pub const TAGS_HEADER: &str = "Databricks-Ai-Gateway-Request-Tags";
/// The User-Agent header (SEC-06b).
pub const USER_AGENT_HEADER: &str = "User-Agent";
/// The longest MOG ID that may go in the tags header.
pub const MAX_MOG_ID_LEN: usize = 64;
/// The longest tags header value.
pub const MAX_TAGS_HEADER_LEN: usize = 256;

/// Why a header could not be built. Each variant has a stable, content-free code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityError {
    /// The MOG ID is empty.
    MogIdEmpty,
    /// The MOG ID is longer than [`MAX_MOG_ID_LEN`].
    MogIdTooLong,
    /// The MOG ID has a character other than ASCII letters, digits, `_` or `-`.
    MogIdChars,
    /// The tags value matches a rule-7 secret pattern.
    SecretInTags,
    /// The tags value is longer than [`MAX_TAGS_HEADER_LEN`].
    TagsTooLong,
    /// A Unity Gateway call has no tags (SEC-06a).
    MissingTags,
}

impl IdentityError {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            IdentityError::MogIdEmpty => "tags_mog_empty",
            IdentityError::MogIdTooLong => "tags_mog_too_long",
            IdentityError::MogIdChars => "tags_mog_chars",
            IdentityError::SecretInTags => "tags_secret",
            IdentityError::TagsTooLong => "tags_too_long",
            IdentityError::MissingTags => "tags_missing",
        }
    }
}

impl fmt::Display for IdentityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for IdentityError {}

/// The kind of an outbound call, which decides the headers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallKind {
    /// A Unity Gateway call (`/ai-gateway/…`: model and ASR calls). User-Agent and tags.
    UnityGateway,
    /// An MCP call (Genie One MCP). User-Agent and tags (TODO(#136)).
    Mcp,
    /// Any other workspace REST call (Genie, SCIM, Knowledge, Apps). User-Agent only.
    Workspace,
}

/// The `<os>` token of the User-Agent for an OS name of `std::env::consts::OS`.
pub fn os_token(os: &str) -> &'static str {
    match os {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        _ => "other",
    }
}

/// `bottle/<version> (<os>; mog)` for a given version and OS name.
pub fn user_agent_for(version: &str, os: &str) -> String {
    format!("bottle/{version} ({}; mog)", os_token(os))
}

/// The User-Agent of this build: the crate version (the app version) and this OS.
pub fn user_agent() -> String {
    user_agent_for(env!("CARGO_PKG_VERSION"), std::env::consts::OS)
}

fn check_mog_id(mog: &str) -> Result<(), IdentityError> {
    if mog.is_empty() {
        return Err(IdentityError::MogIdEmpty);
    }
    if mog.len() > MAX_MOG_ID_LEN {
        return Err(IdentityError::MogIdTooLong);
    }
    if !mog
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(IdentityError::MogIdChars);
    }
    Ok(())
}

/// The tags header value for `tags`: `{"app":"bottle","mog":"<id>","op":"<op>"}`.
pub fn tags_header_value(tags: &Tags) -> Result<String, IdentityError> {
    check_mog_id(&tags.mog)?;
    // The MOG ID is a slug, so it needs no JSON escape.
    let value = format!(
        r#"{{"app":"{}","mog":"{}","op":"{}"}}"#,
        tag_app(tags.app),
        tags.mog,
        tag_op(tags.op)
    );
    if contains_secret(&value) {
        return Err(IdentityError::SecretInTags);
    }
    if value.len() > MAX_TAGS_HEADER_LEN {
        return Err(IdentityError::TagsTooLong);
    }
    Ok(value)
}

/// Adds the Bottle identity headers to every outbound request.
pub trait RequestDecorator: Send + Sync {
    /// The headers for one call of `kind`. `tags` comes from the IPC `tags` argument.
    fn headers(
        &self,
        kind: CallKind,
        tags: Option<&Tags>,
    ) -> Result<Vec<(&'static str, String)>, IdentityError>;

    /// Remove every caller-set User-Agent and tags header (any ASCII case) from `headers`,
    /// then add [`RequestDecorator::headers`]. On error `headers` is unchanged.
    fn decorate(
        &self,
        headers: &mut Vec<(String, String)>,
        kind: CallKind,
        tags: Option<&Tags>,
    ) -> Result<(), IdentityError> {
        let add = self.headers(kind, tags)?;
        headers.retain(|(n, _)| {
            !n.eq_ignore_ascii_case(USER_AGENT_HEADER) && !n.eq_ignore_ascii_case(TAGS_HEADER)
        });
        headers.extend(add.into_iter().map(|(n, v)| (n.to_owned(), v)));
        Ok(())
    }
}

/// The Bottle decorator: [`user_agent`] on every call, tags on Unity Gateway and MCP calls.
#[derive(Debug, Clone)]
pub struct BottleIdentity {
    user_agent: String,
}

impl BottleIdentity {
    /// The decorator of this build.
    pub fn new() -> Self {
        Self {
            user_agent: user_agent(),
        }
    }
}

impl Default for BottleIdentity {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestDecorator for BottleIdentity {
    fn headers(
        &self,
        kind: CallKind,
        tags: Option<&Tags>,
    ) -> Result<Vec<(&'static str, String)>, IdentityError> {
        let mut out = vec![(USER_AGENT_HEADER, self.user_agent.clone())];
        match kind {
            CallKind::UnityGateway | CallKind::Mcp => {
                let tags = tags.ok_or(IdentityError::MissingTags)?;
                out.push((TAGS_HEADER, tags_header_value(tags)?));
            }
            // Data minimisation: the MOG ID goes only where usage rows record it.
            CallKind::Workspace => {}
        }
        Ok(out)
    }
}

fn tag_app(app: TagApp) -> &'static str {
    match app {
        TagApp::Bottle => "bottle",
    }
}

fn tag_op(op: TagOp) -> &'static str {
    match op {
        TagOp::Asr => "asr",
        TagOp::Cleanup => "cleanup",
        TagOp::Router => "router",
        TagOp::Writer => "writer",
        TagOp::Judge => "judge",
        TagOp::Embed => "embed",
        TagOp::Genie => "genie",
        TagOp::Merge => "merge",
        TagOp::Agent => "agent",
        TagOp::Page => "page",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(mog: &str, op: TagOp) -> Tags {
        Tags {
            app: TagApp::Bottle,
            mog: mog.to_owned(),
            op,
        }
    }

    fn header<'a>(h: &'a [(&'static str, String)], name: &str) -> Option<&'a str> {
        h.iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn sec_06b_user_agent_format() {
        assert_eq!(
            user_agent_for("0.1.0", "macos"),
            "bottle/0.1.0 (macos; mog)"
        );
        assert_eq!(
            user_agent_for("1.2.3", "windows"),
            "bottle/1.2.3 (windows; mog)"
        );
        assert_eq!(
            user_agent_for("1.2.3", "linux"),
            "bottle/1.2.3 (linux; mog)"
        );
    }

    #[test]
    fn sec_06b_user_agent_os_token_is_fixed_set() {
        assert_eq!(os_token("macos"), "macos");
        assert_eq!(os_token("windows"), "windows");
        assert_eq!(os_token("linux"), "linux");
        // An unknown OS name never reaches the header as free text.
        assert_eq!(os_token("my laptop; x"), "other");
        assert_eq!(os_token("freebsd"), "other");
    }

    #[test]
    fn sec_06b_b0_user_agent_uses_app_version_and_this_os() {
        let ua = user_agent();
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            ua,
            format!("bottle/{version} ({}; mog)", os_token(std::env::consts::OS))
        );
        // B0: the product token is `bottle`, and the version is the app version.
        let conf = include_str!("../../../src-tauri/tauri.conf.json");
        let conf: serde_json::Value = serde_json::from_str(conf).unwrap();
        assert_eq!(conf["version"].as_str(), Some(version));
        assert!(ua.starts_with("bottle/"));
    }

    #[test]
    fn sec_06b_user_agent_on_every_call_kind() {
        let d = BottleIdentity::new();
        let t = tags("mog-1", TagOp::Genie);
        for kind in [CallKind::UnityGateway, CallKind::Mcp, CallKind::Workspace] {
            let h = d.headers(kind, Some(&t)).unwrap();
            assert_eq!(header(&h, USER_AGENT_HEADER), Some(user_agent().as_str()));
        }
        // A workspace call without tags still gets the User-Agent.
        let h = d.headers(CallKind::Workspace, None).unwrap();
        assert_eq!(header(&h, USER_AGENT_HEADER), Some(user_agent().as_str()));
    }

    #[test]
    fn sec_06a_tags_header_is_json_string_map() {
        let v = tags_header_value(&tags("mog_7-a", TagOp::Asr)).unwrap();
        assert_eq!(v, r#"{"app":"bottle","mog":"mog_7-a","op":"asr"}"#);
        let parsed: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&v).unwrap();
        assert!(parsed.values().all(|x| x.is_string()));
    }

    #[test]
    fn sec_06a_tags_only_app_mog_op_for_every_op() {
        let ops = [
            (TagOp::Asr, "asr"),
            (TagOp::Cleanup, "cleanup"),
            (TagOp::Router, "router"),
            (TagOp::Writer, "writer"),
            (TagOp::Judge, "judge"),
            (TagOp::Embed, "embed"),
            (TagOp::Genie, "genie"),
            (TagOp::Merge, "merge"),
            (TagOp::Agent, "agent"),
            (TagOp::Page, "page"),
        ];
        for (op, name) in ops {
            let v = tags_header_value(&tags("m1", op)).unwrap();
            let parsed: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(&v).unwrap();
            let keys: Vec<&str> = parsed.keys().map(String::as_str).collect();
            assert_eq!(keys, ["app", "mog", "op"]);
            assert_eq!(parsed["app"], "bottle");
            assert_eq!(parsed["op"], name);
            // The wire name matches the IPC serde name of the op.
            assert_eq!(serde_json::to_value(op).unwrap(), name);
        }
    }

    #[test]
    fn sec_06a_rejects_mog_ids_that_could_carry_pii_or_urls() {
        let bad = [
            ("", IdentityError::MogIdEmpty),
            ("https://host.example/x", IdentityError::MogIdChars),
            ("jane.doe@example.com", IdentityError::MogIdChars),
            ("what is revenue", IdentityError::MogIdChars),
            ("a\"b", IdentityError::MogIdChars),
            ("a\r\nX-Evil: 1", IdentityError::MogIdChars),
            ("caf\u{e9}", IdentityError::MogIdChars),
        ];
        for (mog, err) in bad {
            assert_eq!(
                tags_header_value(&tags(mog, TagOp::Asr)),
                Err(err),
                "{mog:?}"
            );
        }
        let long = "a".repeat(MAX_MOG_ID_LEN + 1);
        assert_eq!(
            tags_header_value(&tags(&long, TagOp::Asr)),
            Err(IdentityError::MogIdTooLong)
        );
        let max = "a".repeat(MAX_MOG_ID_LEN);
        let v = tags_header_value(&tags(&max, TagOp::Cleanup)).unwrap();
        assert!(v.len() <= MAX_TAGS_HEADER_LEN);
    }

    #[test]
    fn sec_06a_rejects_secret_in_tags() {
        // A PAT-shaped MOG ID passes the slug rule but must not leave the core.
        let pat = format!("dapi{}", "0123456789abcdef".repeat(2));
        assert_eq!(
            tags_header_value(&tags(&pat, TagOp::Asr)),
            Err(IdentityError::SecretInTags)
        );
        assert_eq!(
            tags_header_value(&tags("eyJhbGciOi-x", TagOp::Asr)).map(|_| ()),
            Ok(()),
            "no dot, so not a JWT"
        );
        let akia = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
        assert_eq!(
            tags_header_value(&tags(&akia, TagOp::Asr)),
            Err(IdentityError::SecretInTags)
        );
    }

    #[test]
    fn sec_06a_error_codes_are_content_free() {
        let all = [
            IdentityError::MogIdEmpty,
            IdentityError::MogIdTooLong,
            IdentityError::MogIdChars,
            IdentityError::SecretInTags,
            IdentityError::TagsTooLong,
            IdentityError::MissingTags,
        ];
        let codes: Vec<&str> = all.iter().map(IdentityError::code).collect();
        assert_eq!(
            codes,
            [
                "tags_mog_empty",
                "tags_mog_too_long",
                "tags_mog_chars",
                "tags_secret",
                "tags_too_long",
                "tags_missing",
            ]
        );
        let err = tags_header_value(&tags("jane.doe@example.com", TagOp::Asr)).unwrap_err();
        assert!(!err.to_string().contains("jane"));
    }

    #[test]
    fn sec_06a_unity_gateway_call_carries_tags() {
        let d = BottleIdentity::new();
        let t = tags("mog-1", TagOp::Cleanup);
        let h = d.headers(CallKind::UnityGateway, Some(&t)).unwrap();
        assert_eq!(
            header(&h, TAGS_HEADER),
            Some(r#"{"app":"bottle","mog":"mog-1","op":"cleanup"}"#)
        );
        assert_eq!(h.len(), 2);
    }

    #[test]
    fn sec_06a_unity_gateway_call_without_tags_fails_closed() {
        let d = BottleIdentity::new();
        assert_eq!(
            d.headers(CallKind::UnityGateway, None),
            Err(IdentityError::MissingTags)
        );
        let bad = tags("a b", TagOp::Asr);
        assert_eq!(
            d.headers(CallKind::UnityGateway, Some(&bad)),
            Err(IdentityError::MogIdChars)
        );
    }

    #[test]
    fn sec_06a_workspace_rest_call_has_no_tags_header() {
        let d = BottleIdentity::new();
        let t = tags("mog-1", TagOp::Genie);
        let h = d.headers(CallKind::Workspace, Some(&t)).unwrap();
        assert_eq!(header(&h, TAGS_HEADER), None);
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn sp_18_sec_06a_mcp_call_carries_tags() {
        let d = BottleIdentity::new();
        let t = tags("mog-1", TagOp::Genie);
        let h = d.headers(CallKind::Mcp, Some(&t)).unwrap();
        assert_eq!(
            header(&h, TAGS_HEADER),
            Some(r#"{"app":"bottle","mog":"mog-1","op":"genie"}"#)
        );
        assert_eq!(
            d.headers(CallKind::Mcp, None),
            Err(IdentityError::MissingTags)
        );
    }

    #[test]
    fn sec_06a_sec_06b_decorate_replaces_caller_set_headers() {
        let d = BottleIdentity::new();
        let t = tags("mog-1", TagOp::Agent);
        let mut h = vec![
            ("user-agent".to_owned(), "claude-cli/9".to_owned()),
            (
                "DATABRICKS-AI-GATEWAY-REQUEST-TAGS".to_owned(),
                r#"{"app":"spoof"}"#.to_owned(),
            ),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ];
        d.decorate(&mut h, CallKind::UnityGateway, Some(&t))
            .unwrap();
        let ua: Vec<_> = h
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(USER_AGENT_HEADER))
            .collect();
        assert_eq!(ua.len(), 1);
        assert_eq!(ua[0].1, user_agent());
        let tg: Vec<_> = h
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case(TAGS_HEADER))
            .collect();
        assert_eq!(tg.len(), 1);
        assert_eq!(tg[0].1, r#"{"app":"bottle","mog":"mog-1","op":"agent"}"#);
        assert!(h.iter().any(|(n, _)| n == "Content-Type"));

        // A workspace call drops a caller-set tags header and adds none.
        let mut h = vec![(TAGS_HEADER.to_owned(), "x".to_owned())];
        d.decorate(&mut h, CallKind::Workspace, None).unwrap();
        assert_eq!(h, vec![(USER_AGENT_HEADER.to_owned(), user_agent())]);
    }

    #[test]
    fn sec_06a_decorate_error_leaves_headers_unchanged() {
        let d = BottleIdentity::new();
        let mut h = vec![("user-agent".to_owned(), "x".to_owned())];
        let before = h.clone();
        assert_eq!(
            d.decorate(&mut h, CallKind::UnityGateway, None),
            Err(IdentityError::MissingTags)
        );
        assert_eq!(h, before);
    }

    #[test]
    fn sec_06b_header_values_are_visible_ascii() {
        let d = BottleIdentity::new();
        let t = tags("mog-1", TagOp::Asr);
        for (_, v) in d.headers(CallKind::UnityGateway, Some(&t)).unwrap() {
            assert!(v.bytes().all(|b| (0x20..0x7f).contains(&b)), "{v:?}");
        }
    }
}
