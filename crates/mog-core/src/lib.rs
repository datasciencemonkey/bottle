//! `mog-core`: the Bottle Rust core (ARC-1, section 4.3 and 4.4 of the requirements).
//!
//! The core owns everything inside the secret boundary: the CLI child process, tokens,
//! HTTP and MCP transport, audio, the local store and the IPC gateway. The Tauri app
//! links this crate; `mog-core-ffi` exposes the same command and event envelope through
//! a C ABI. There is one module per component (R1-R14, R16, R17); R15 is `mog-core-ffi`.

pub mod action_ticket;
pub mod agent_guard;
pub mod agent_host;
pub mod agent_run;
pub mod app_settings;
pub mod audio_device;
pub mod audit;
pub mod bottle_mcp;
pub mod capture;
pub mod cli_runner;
pub mod cli_status;
pub mod clock;
pub mod cloud_dictation;
pub mod consent;
pub mod consent_ipc;
pub mod egress_policy;
pub mod error;
pub mod genie_endpoint;
pub mod hotkey;
pub mod hotkey_bindings;
pub mod hotkey_core;
pub mod hotkey_macos;
pub mod hotkey_windows;
pub mod http_transport;
pub mod insertion;
pub mod ipc;
pub mod ipc_gateway;
pub mod keyboard_env;
#[cfg(feature = "lakebase")]
pub mod lakebase_pool;
pub mod latency_bench;
pub mod link_open;
pub mod local_store;
pub mod loopback;
pub mod mcp_transport;
pub mod model_gateway;
pub mod model_request_guard;
pub mod mog_config;
pub mod mog_look;
pub mod policy_store;
pub mod profile_login;
pub mod profile_service;
pub mod recording;
pub mod recording_capture;
pub mod recording_cloud;
pub mod request_identity;
pub mod resource_budget;
pub mod session_guard;
pub mod stt;
pub mod stt_bench;
pub mod stt_engine;
#[cfg(feature = "local-stt")]
pub mod stt_local;
pub mod stt_model;
pub mod stt_setup;
pub mod telemetry;
pub mod testkit;
pub mod token_broker;
pub mod token_events;
pub mod token_guard;
pub mod types;
pub mod window_host;
pub mod wipe;
pub mod workspaces;

/// The crate version, so hosts can check which core they link.
pub fn core_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_1_core_reports_a_version() {
        assert!(!core_version().is_empty());
    }
}
