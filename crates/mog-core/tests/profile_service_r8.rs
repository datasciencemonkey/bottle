//! W-03b, W-10a (#17, ADR 0094): R8 on T7 as its profile hosts never sends, and never lends a
//! token, for a profile that T7 does not support. A fake Genie One MCP server on loopback, a fake
//! R6 broker, fake config text and a fake R5. No real `~/.databrickscfg`, no live CLI.

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::path::PathBuf;
use std::sync::Arc;

use fake_mcp::*;
use mog_core::cli_runner::{refuse_token_command, CliRunner};
use mog_core::error::CoreResult;
use mog_core::mcp_transport::{McpCall, McpLimits, McpSeams, McpTransport, ReqwestMcpTransport};
use mog_core::profile_service::{parse_config, ConfigProfile, ConfigSource, ProfileService};
use mog_core::request_identity::BottleIdentity;
use serde_json::{json, Value};

/// One profile named [`PROFILE`] at `origin`, with `auth_type` `auth`.
struct OneProfile {
    origin: String,
    auth: &'static str,
}

impl ConfigSource for OneProfile {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        let text = format!(
            "[{PROFILE}]\nhost = {}\nauth_type = {}\n",
            self.origin, self.auth
        );
        Ok(parse_config(text.as_bytes()))
    }
}

/// A fake R5 whose `auth describe` reports `mode`.
struct Describe(&'static str);

impl CliRunner for Describe {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/fake/databricks"))
    }
    fn run_json(&self, args: &[&str]) -> CoreResult<Value> {
        refuse_token_command(args)?;
        Ok(json!({ "details": { "token_storage": { "mode": self.0 } } }))
    }
}

/// Ask once through R8 on T7; return the error code (or "ok"), the sends and the token lends.
fn ask_on_t7(auth: &'static str, mode: &'static str) -> (String, usize, usize) {
    let s = genie_one(|_| None);
    let t7 = Arc::new(ProfileService::new(
        Arc::new(OneProfile {
            origin: s.origin(),
            auth,
        }),
        Arc::new(Describe(mode)),
    ));
    let broker = Arc::new(FakeBroker::default());
    let t = ReqwestMcpTransport::with_seams(McpSeams {
        hosts: t7,
        endpoints: Endpoints::one(&s.url()),
        tokens: broker.clone(),
        policy: Arc::new(LoopbackPolicy),
        decorator: Arc::new(BottleIdentity::new()),
        log: Arc::new(Lines::default()),
        limits: McpLimits::default(),
    })
    .unwrap();
    let tags = tags(MOG);
    let r = t.call_tool(McpCall::new(
        PROFILE,
        "genie_ask",
        obj(json!({ "question": QUESTION })),
        &tags,
    ));
    let code = match r {
        Ok(_) => "ok".to_owned(),
        Err(e) => e.code().to_owned(),
    };
    (code, s.seen().len(), broker.lend_count())
}

#[test]
fn w_03b_a_pat_profile_never_calls_genie() {
    // A PAT profile has no auth_type key; a service principal has oauth-m2m.
    for auth in ["\"\"", "oauth-m2m", "pat"] {
        let (code, sent, lent) = ask_on_t7(auth, "secure");
        assert_eq!(code, "profile_not_supported", "{auth}");
        assert_eq!(sent, 0, "{auth}: nothing reaches Genie");
        assert_eq!(lent, 0, "{auth}: no token is lent");
    }
}

#[test]
fn w_10a_a_plaintext_storage_profile_never_calls_genie() {
    let (code, sent, lent) = ask_on_t7("databricks-cli", "plaintext");
    assert_eq!(code, "profile_not_supported");
    assert_eq!((sent, lent), (0, 0));
}

#[test]
fn w_03b_a_supported_profile_reaches_genie() {
    // The control: the same rig with a databricks-cli profile and secure storage sends.
    let (_code, sent, lent) = ask_on_t7("databricks-cli", "secure");
    assert!(sent > 0, "a supported profile reaches the fake Genie One");
    assert!(lent > 0);
}
