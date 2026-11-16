//! `mog-core`: the Bottle Rust core (ARC-1, section 4.3 and 4.4 of the requirements).
//!
//! The core owns everything inside the secret boundary: the CLI child process, tokens,
//! HTTP and MCP transport, audio, the local store and the IPC gateway. The Tauri app
//! links this crate; `mog-core-ffi` exposes the same command and event envelope through
//! a C ABI. The 17 component modules (R1-R17) land in #225.

pub mod agent_host;
pub mod bottle_mcp;
pub mod capture;
pub mod cli_runner;
pub mod error;
pub mod hotkey;
pub mod http_transport;
pub mod ipc_gateway;
#[cfg(feature = "lakebase")]
pub mod lakebase_pool;
pub mod local_store;
pub mod mcp_transport;
pub mod policy_store;
pub mod session_guard;
pub mod stt;
pub mod token_broker;
pub mod types;
pub mod window_host;
pub mod wipe;

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
