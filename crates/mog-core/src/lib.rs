//! `mog-core`: the Bottle Rust core (ARC-1, section 4.3 and 4.4 of the requirements).
//!
//! The core owns everything inside the secret boundary: the CLI child process, tokens,
//! HTTP and MCP transport, audio, the local store and the IPC gateway. The Tauri app
//! links this crate; `mog-core-ffi` exposes the same command and event envelope through
//! a C ABI. The 17 component modules (R1-R17) land in #225.

pub mod types;

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
