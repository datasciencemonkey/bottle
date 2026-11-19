//! #555 (#17, ADR 0094): the Tauri host sends `profiles_list` to the managed T7
//! `ProfilesHost` through the hardened gateway. Only the brain and the hub may call it
//! (SEC-02a). Fake config text and a fake R5 only: no test reads the real `~/.databrickscfg`,
//! runs a real CLI or picks a profile.
//!
//! Requirement IDs: W-03a, SEC-02a.

use std::path::PathBuf;
use std::sync::Arc;

use bottle_lib::ipc::{route, BrainLink, HostComponents};
use bottle_lib::profiles_host::{self, ProfilesHost};
use mog_core::cli_runner::{refuse_token_command, CliRunner};
use mog_core::error::CoreResult;
use mog_core::ipc::ChannelRef;
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

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
    fn notify_hub(&self, _notice: Value) -> bool {
        false
    }
    fn subscribe_hub(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

fn t7() -> ProfilesHost {
    profiles_host::build(Arc::new(FakeConfig), Arc::new(SecureCli))
}

fn call(from: &str, profiles: Option<&ProfilesHost>) -> Result<Value, Value> {
    let c = HostComponents {
        profiles: profiles.map(|p| p as &dyn mog_core::ipc_gateway::GatewayComponents),
        ..HostComponents::none(from, &NoBrain)
    };
    route(c, "profiles_list", json!({}))
}

#[test]
fn w_03a_sec_02a_the_brain_and_the_hub_reach_t7_through_profiles_list() {
    let host = t7();
    let want = json!([
        { "name": "fake-u2m", "host": "https://example-a.cloud.databricks.com",
          "authType": "databricks-cli", "supported": true },
        { "name": "fake-pat", "host": "https://example-b.cloud.databricks.com",
          "authType": "", "supported": false }
    ]);
    assert_eq!(call("brain", Some(&host)), Ok(want.clone()));
    assert_eq!(call("hub", Some(&host)), Ok(want));
}

#[test]
fn sec_02a_a_mog_window_cannot_call_profiles_list() {
    let host = t7();
    for from in ["mog-a1", "card-a1"] {
        assert_eq!(
            call(from, Some(&host)),
            Err(json!({"error": "window_not_allowed"})),
            "{from}"
        );
    }
}

#[test]
fn w_03a_no_managed_profiles_host_means_host_failed() {
    for from in ["brain", "hub"] {
        assert_eq!(
            call(from, None),
            Err(json!({"error": "host_failed"})),
            "{from}"
        );
    }
}

#[test]
fn w_03a_route_from_reads_the_managed_profiles_host() {
    // `try_state` turns a missing T7 into a silent `host_failed`, so `route_from` must read the
    // one `ProfilesHost` that `profiles_host::install` manages (#17).
    let ipc = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/ipc.rs"))
        .unwrap();
    assert!(ipc.contains("try_state::<crate::profiles_host::ProfilesHost>()"));
}
