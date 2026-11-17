//! #379 (ADR 0040 deviation, #25): the Tauri host sends `config_*` to
//! `mog_config::MogConfigComponent` through the hardened gateway. Its content-free codes cross
//! as themselves; a missing store answers `config_storage_failed`.

use bottle_lib::ipc::{route, BrainLink, HostComponents};
use mog_core::ipc::ChannelRef;
use mog_core::ipc_gateway::{GatewayComponents, ERROR_CODES};
use mog_core::mog_config::{MogConfigComponent, MogConfigStore};
use serde_json::{json, Value};
use std::sync::Arc;

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

/// A fresh store folder under the OS temp dir, removed on drop.
struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let p = std::env::temp_dir().join(format!(
            "bottle-config-routing-{tag}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        Self(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn call(
    from: &str,
    config: Option<&dyn GatewayComponents>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let components = HostComponents {
        from,
        brain: &NoBrain,
        windows: None,
        hotkeys: None,
        permissions: None,
        consent: None,
        config,
    };
    route(components, command, args)
}

fn minimal() -> Value {
    json!({"schemaVersion": 1, "name": "MOG 1", "binding": {"home": "dev"}})
}

#[test]
fn mb_02_config_commands_reach_the_config_store_from_brain_and_hub() {
    let dir = TempDir::new("rw");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    let c: Option<&dyn GatewayComponents> = Some(&comp);
    let wrote = call(
        "hub",
        c,
        "config_write",
        json!({"id": "m1", "json": minimal()}),
    )
    .unwrap();
    assert_eq!(wrote["id"], json!("m1"));
    let read = call("brain", c, "config_read", json!({"id": "m1"})).unwrap();
    assert_eq!(read["json"]["name"], json!("MOG 1"));
    let list = call("brain", c, "config_list", json!({})).unwrap();
    assert!(list.to_string().contains("m1"), "{list}");
    let deleted = call("hub", c, "config_delete", json!({"id": "m1"})).unwrap();
    assert_eq!(deleted, json!({"deleted": true}));
}

#[test]
fn mb_02_a_secret_in_a_config_is_refused_with_its_own_code() {
    let dir = TempDir::new("secret");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    let mut cfg = minimal();
    cfg["password"] = json!("x");
    let err = call(
        "hub",
        Some(&comp),
        "config_write",
        json!({"id": "m1", "json": cfg}),
    )
    .unwrap_err();
    assert_eq!(err, json!({"error": "secret_named_key"}));
    // Nothing was saved.
    let read = call("hub", Some(&comp), "config_read", json!({"id": "m1"})).unwrap_err();
    assert_eq!(read, json!({"error": "config_not_found"}));
}

#[test]
fn mb_02_a_token_like_value_is_refused_end_to_end_with_its_own_code() {
    let dir = TempDir::new("tokenvalue");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    let fake = "password=test-not-real";
    let mut cfg = minimal();
    cfg["persona"] = json!(format!("use {fake} please"));
    let err = call(
        "brain",
        Some(&comp),
        "config_write",
        json!({"id": "m1", "json": cfg}),
    )
    .unwrap_err();
    assert_eq!(err, json!({"error": "token_like_value"}));
    assert!(!err.to_string().contains(fake));
    let read = call("hub", Some(&comp), "config_read", json!({"id": "m1"})).unwrap_err();
    assert_eq!(read, json!({"error": "config_not_found"}));
}

#[test]
fn sec_03_unknown_config_args_are_refused_by_the_gateway_schema_check() {
    let dir = TempDir::new("extraargs");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    for (command, args) in [
        ("config_list", json!({"extra": 1})),
        ("config_read", json!({"id": "m1", "extra": 1})),
        (
            "config_write",
            json!({"id": "m1", "json": minimal(), "extra": 1}),
        ),
        ("config_delete", json!({"id": "m1", "extra": 1})),
    ] {
        assert_eq!(
            call("hub", Some(&comp), command, args),
            Err(json!({"error": "invalid_args"})),
            "{command}"
        );
    }
    // A missing store does not mask the schema check: the gateway runs it first.
    assert_eq!(
        call("hub", None, "config_read", json!({"id": "m1", "extra": 1})),
        Err(json!({"error": "invalid_args"}))
    );
}

#[test]
fn sec_03_config_store_codes_cross_as_themselves_and_are_in_error_codes() {
    let dir = TempDir::new("codes");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    for (args, code) in [
        (json!({"id": "bad id"}), "invalid_config_id"),
        (json!({"id": "nope"}), "config_not_found"),
    ] {
        let err = call("brain", Some(&comp), "config_read", args).unwrap_err();
        assert_eq!(err, json!({"error": code}));
        assert!(ERROR_CODES.contains(&code));
    }
    for code in [
        "config_too_large",
        "unsupported_schema_version",
        "invalid_config",
        "config_limit_reached",
        "config_storage_failed",
        "secret_named_key",
        "token_like_value",
    ] {
        assert!(ERROR_CODES.contains(&code), "{code}");
    }
}

#[test]
fn arc_4_config_commands_stay_brain_and_hub_only() {
    let dir = TempDir::new("acl");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    for from in ["mog-a1", "card-a1"] {
        let err = call(from, Some(&comp), "config_list", json!({})).unwrap_err();
        assert_eq!(err, json!({"error": "window_not_allowed"}));
    }
}

#[test]
fn mb_02_a_missing_config_store_answers_config_storage_failed() {
    for (command, args) in [
        ("config_list", json!({})),
        ("config_read", json!({"id": "m1"})),
        ("config_write", json!({"id": "m1", "json": minimal()})),
        ("config_delete", json!({"id": "m1"})),
    ] {
        assert_eq!(
            call("hub", None, command, args),
            Err(json!({"error": "config_storage_failed"})),
            "{command}"
        );
    }
}

#[test]
fn mb_02_the_host_reads_the_store_with_try_state() {
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ipc.rs"),
    )
    .unwrap();
    assert!(src.contains("try_state::<std::sync::Arc<mog_core::mog_config::MogConfigStore>>()"));
    assert!(!src.contains("state::<std::sync::Arc<mog_core::mog_config::MogConfigStore>>().inner"));
}

#[test]
fn g_03_a_rejected_genie_backend_url_crosses_with_its_own_code() {
    let dir = TempDir::new("g03");
    let comp = MogConfigComponent::new(Arc::new(MogConfigStore::new(&dir.0)));
    for (url, code) in [
        (
            "https://h.cloud.databricks.com/api/2.0/mcp/genie",
            "genie_beta_mcp_rejected",
        ),
        (
            "https://h.cloud.databricks.com/api/2.0/mcp/genie/01ef",
            "genie_space_mcp_rejected",
        ),
        (
            "https://h.cloud.databricks.com/api/2.0/genie/one/x",
            "genie_one_sse_rejected",
        ),
        (
            "https://h.cloud.databricks.com/api/2.0/mcp/%2Fgenie",
            "mcp_url_not_canonical",
        ),
    ] {
        let mut cfg = minimal();
        cfg["binding"]["backend"] = json!({"kind": "custom_agent", "profile": "dev",
            "protocol": "invocations", "appUrl": url});
        let err = call(
            "hub",
            Some(&comp),
            "config_write",
            json!({"id": "m1", "json": cfg}),
        )
        .unwrap_err();
        assert_eq!(err, json!({"error": code}), "{url}");
    }
}

/// Every code the config store can give is in `ERROR_CODES`, so none becomes `host_failed`.
#[test]
fn sec_03_every_config_error_code_is_in_error_codes() {
    use mog_core::genie_endpoint::EndpointError;
    use mog_core::mog_config::ConfigError;
    use mog_core::token_guard::ConfigSecret;
    // No wildcard: a new `ConfigError` variant must be added to `all` below.
    fn listed(e: ConfigError) {
        match e {
            ConfigError::InvalidId
            | ConfigError::TooLarge
            | ConfigError::Secret(_)
            | ConfigError::UnsupportedSchemaVersion
            | ConfigError::InvalidConfig
            | ConfigError::NotFound
            | ConfigError::LimitReached
            | ConfigError::StorageFailed
            | ConfigError::RejectedBackend(_) => {}
        }
    }
    let mut all = vec![
        ConfigError::InvalidId,
        ConfigError::TooLarge,
        ConfigError::Secret(ConfigSecret::SecretNamedKey),
        ConfigError::Secret(ConfigSecret::TokenLikeValue),
        ConfigError::UnsupportedSchemaVersion,
        ConfigError::InvalidConfig,
        ConfigError::NotFound,
        ConfigError::LimitReached,
        ConfigError::StorageFailed,
    ];
    // The endpoint errors that `parse_config` can give (`genie_endpoint::rejected_backend`).
    for e in [
        EndpointError::BetaGenieMcp,
        EndpointError::PerSpaceGenieMcp,
        EndpointError::GenieOneApiSse,
        EndpointError::NonCanonicalUrl,
    ] {
        all.push(ConfigError::RejectedBackend(e));
    }
    for e in all {
        listed(e);
        assert!(ERROR_CODES.contains(&e.code()), "{}", e.code());
    }
}
