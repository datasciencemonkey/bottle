//! ADR 0117: the Tauri host sends each `workspace_*` command to the managed `WorkspacesHost`
//! through the hardened gateway, and `profile_reauth` to the login component. Only the brain may
//! call them (ARC-4). Fake T7, fake R5 and a temp dir only: no CLI, no token, no real profile.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use bottle_lib::ipc::{route, BrainLink, HostComponents};
use bottle_lib::profiles_host::ProfileLoginHost;
use bottle_lib::workspaces_host::WorkspacesHost;
use mog_core::cli_runner::{refuse_token_command, CliRunner};
use mog_core::error::CoreResult;
use mog_core::ipc::ChannelRef;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mog_config::MogConfigStore;
use mog_core::profile_login::{LoginVerifier, ProfileLogin};
use mog_core::profile_service::{parse_config, ConfigProfile, ConfigSource};
use mog_core::types::Profile;
use mog_core::workspaces::{ProfileSource, WorkspaceStore};
use serde_json::{json, Value};

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

struct FakeT7;

impl ProfileSource for FakeT7 {
    fn profiles(&self, _refresh: bool) -> CoreResult<Vec<Profile>> {
        Ok(["home", "other"]
            .iter()
            .map(|n| Profile {
                name: (*n).to_owned(),
                host: format!("https://{n}.example.test"),
                auth_type: "databricks-cli".to_owned(),
                supported: true,
                workspace_id: None,
            })
            .collect())
    }
}

struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new() -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        Self(std::env::temp_dir().join(format!(
            "bottle-ws-routing-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A host over a temp dir with one old MOG `a` on `home`. Config `n` (on `other`) is made on
/// first use, so `other` starts as a new connection.
fn host(s: &Scratch) -> WorkspacesHost {
    let configs = Arc::new(MogConfigStore::new(s.0.join("mogs")));
    let put = |id: &str, home: &str| {
        let cfg = json!({"schemaVersion": 1, "name": id, "binding": {"home": home}});
        let Value::Object(m) = cfg else {
            unreachable!()
        };
        configs.write(id, &m.into_iter().collect()).unwrap();
    };
    put("a", "home");
    let h = WorkspacesHost::with_store(Arc::new(WorkspaceStore::new(
        &s.0,
        configs.clone(),
        Arc::new(FakeT7),
    )));
    // The migration runs now, so `n` is not part of it.
    call("brain", Some(&h), "workspace_list", json!({})).unwrap();
    put("n", "other");
    h
}

fn call(
    from: &str,
    ws: Option<&WorkspacesHost>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let c = HostComponents {
        workspaces: ws.map(|w| w as &dyn GatewayComponents),
        ..HostComponents::none(from, &NoBrain)
    };
    route(c, command, args)
}

const COMMANDS: [&str; 4] = [
    "workspace_list",
    "workspace_set_available",
    "workspace_make_home",
    "workspace_settings_write",
];

fn args_of(command: &str) -> Value {
    match command {
        "workspace_set_available" => json!({"profile": "other", "available": true, "mogId": "n"}),
        "workspace_make_home" => json!({"profile": "other"}),
        "workspace_settings_write" => json!({"genieCallBudget": 20}),
        _ => json!({}),
    }
}

#[test]
fn the_brain_reaches_each_workspace_command_and_gets_a_workspace_list() {
    let s = Scratch::new();
    let h = host(&s);
    for command in COMMANDS {
        let out = call("brain", Some(&h), command, args_of(command))
            .unwrap_or_else(|e| panic!("{command}: {e}"));
        assert!(out["workspaces"].is_array(), "{command}");
        assert!(out["agent"]["genieCallBudget"].is_number(), "{command}");
    }
    let l = call(
        "brain",
        Some(&h),
        "workspace_list",
        json!({"refresh": true}),
    )
    .unwrap();
    assert_eq!(l["home"], "other", "the last call made `other` the home");
    assert_eq!(l["agent"]["genieCallBudget"], 20);
}

#[test]
fn a_refusal_of_the_store_reaches_the_brain_as_its_code() {
    let s = Scratch::new();
    let h = host(&s);
    let out = call(
        "brain",
        Some(&h),
        "workspace_set_available",
        json!({"profile": "home", "available": false}),
    );
    assert_eq!(out, Err(json!({"error": "home_always_available"})));
}

#[test]
fn only_the_brain_may_call_a_workspace_command() {
    let s = Scratch::new();
    let h = host(&s);
    for from in ["hub", "mog-a1", "card-a1"] {
        for command in COMMANDS {
            assert_eq!(
                call(from, Some(&h), command, args_of(command)),
                Err(json!({"error": "window_not_allowed"})),
                "{from} {command}"
            );
        }
    }
}

#[test]
fn no_managed_workspace_host_or_an_empty_one_means_host_failed() {
    let empty = WorkspacesHost::default();
    for command in COMMANDS {
        assert_eq!(
            call("brain", None, command, args_of(command)),
            Err(json!({"error": "host_failed"})),
            "{command} unmanaged"
        );
        assert_eq!(
            call("brain", Some(&empty), command, args_of(command)),
            Err(json!({"error": "host_failed"})),
            "{command} empty"
        );
    }
    // An empty host allows no `genie_ask` (fail closed).
    assert!(!empty.gate().allows("home"));
}

#[test]
fn the_gate_of_the_host_follows_the_store() {
    let s = Scratch::new();
    let h = host(&s);
    let gate = h.gate();
    assert!(gate.allows("home"));
    assert!(!gate.allows("other"));
    call(
        "brain",
        Some(&h),
        "workspace_set_available",
        args_of("workspace_set_available"),
    )
    .unwrap();
    assert!(gate.allows("other"));
}

#[test]
fn route_from_reads_the_managed_workspaces_host() {
    let ipc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ipc.rs"),
    )
    .unwrap();
    assert!(ipc.contains("try_state::<crate::workspaces_host::WorkspacesHost>()"));
}

// ---------- profile_reauth ----------

struct EmptyConfig;

impl ConfigSource for EmptyConfig {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        Ok(parse_config(
            b"[home]\nhost = https://home.example.test\nauth_type = databricks-cli\n",
        ))
    }
}

struct NoCli;

impl CliRunner for NoCli {
    fn resolve(&self) -> CoreResult<std::path::PathBuf> {
        Ok("/fake/databricks".into())
    }
    fn run_json(&self, args: &[&str]) -> CoreResult<Value> {
        refuse_token_command(args)?;
        Ok(json!({}))
    }
}

struct OkVerifier;

impl LoginVerifier for OkVerifier {
    fn verify(&self, _profile: &str) -> CoreResult<()> {
        Ok(())
    }
}

#[test]
fn profile_reauth_routes_to_the_login_component() {
    // The login component signs a U2M profile in again. `NoCli` reports no secure token
    // storage, so the call is refused before any CLI login runs. The other paths are in
    // `crates/mog-core/tests/profile_login.rs`.
    let login = ProfileLoginHost(Arc::new(ProfileLogin::new(
        Arc::new(EmptyConfig),
        Arc::new(NoCli),
        Arc::new(OkVerifier),
    )));
    let c = |from| HostComponents {
        login: Some(&login as &dyn GatewayComponents),
        ..HostComponents::none(from, &NoBrain)
    };
    assert_eq!(
        route(c("brain"), "profile_reauth", json!({"profile": "home"})),
        Err(json!({"error": "profile_invalid"}))
    );
    assert_eq!(
        route(c("hub"), "profile_reauth", json!({"profile": "home"})),
        Err(json!({"error": "window_not_allowed"}))
    );
    // Without the login host the command answers `host_failed`.
    assert_eq!(
        route(
            HostComponents::none("brain", &NoBrain),
            "profile_reauth",
            json!({"profile": "home"})
        ),
        Err(json!({"error": "host_failed"}))
    );
}
