//! The host's one T7 ProfileService (#17, ADR 0094). Fake config text and a fake R5 only: no
//! test reads the real `~/.databrickscfg` or runs a real CLI.
//!
//! Requirement IDs: W-03a, W-03b, W-10a.

use std::path::PathBuf;
use std::sync::Arc;

use bottle_lib::{mcp_host, profiles_host};
use mog_core::cli_runner::{refuse_token_command, CliRunner};
use mog_core::error::CoreResult;
use mog_core::http_transport::rate_bucket::RateBucket;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::profile_service::{parse_config, ConfigProfile, ConfigSource};
use serde_json::{json, Value};

struct FakeConfig;

impl ConfigSource for FakeConfig {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        Ok(parse_config(
            b"[fake-u2m]\nhost = https://example-a.cloud.databricks.com\nauth_type = databricks-cli\n\
              [fake-pat]\nhost = https://example-b.cloud.databricks.com\n",
        ))
    }
}

/// A fake R5 whose `auth describe` always reports the OS key store.
struct SecureCli;

impl CliRunner for SecureCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/fake/databricks"))
    }
    fn run_json(&self, args: &[&str]) -> CoreResult<Value> {
        refuse_token_command(args)?;
        Ok(json!({ "details": { "token_storage": { "mode": "secure" } } }))
    }
}

struct NoWait;

impl RateBucket for NoWait {
    fn reserve(&self, _profile: &str) -> std::time::Duration {
        std::time::Duration::ZERO
    }
    fn on_rate_limited(&self, _profile: &str, _retry_after: Option<std::time::Duration>) {}
}

#[test]
fn w_03a_host_t7_answers_profiles_list_and_never_picks() {
    let host = profiles_host::build(Arc::new(FakeConfig), Arc::new(SecureCli));
    let reply = host.call("profiles_list", json!({})).unwrap();
    assert_eq!(
        reply,
        json!([
            { "name": "fake-u2m", "host": "https://example-a.cloud.databricks.com",
              "authType": "databricks-cli", "supported": true },
            { "name": "fake-pat", "host": "https://example-b.cloud.databricks.com",
              "authType": "", "supported": false }
        ])
    );
}

#[test]
fn w_03b_r8_builds_on_the_host_t7() {
    let host = profiles_host::build(Arc::new(FakeConfig), Arc::new(SecureCli));
    assert!(mcp_host::build_with(Arc::new(NoWait), host.0.clone()).is_ok());
}

fn src(rel: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn w_03b_the_app_installs_t7_before_r8_and_r8_uses_it() {
    let lib = src("src/lib.rs");
    let t7 = lib
        .find("profiles_host::install(handle)")
        .expect("T7 installed");
    let r8 = lib.find("mcp_host::install(handle)").expect("R8 installed");
    assert!(t7 < r8, "T7 is managed before R8 reads it");
    let mcp = src("src/mcp_host.rs");
    assert!(mcp.contains("try_state::<ProfilesHost>()"));
    let t7_src = src("src/profiles_host.rs");
    assert!(t7_src.contains("ConfigFile::of_user()"));
}
