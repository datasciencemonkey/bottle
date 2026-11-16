//! `mog-core-ffi`: the C ABI over `mog-core` (ARC-1, R15).
//!
//! The ABI functions `mog_init`, `mog_call`, `mog_set_event_callback` and
//! `mog_abi_version` land in #225 (stubs) and #176 (implementation).
//! TODO(#225): add the four ABI stubs.

/// Re-export the core version so the FFI crate proves it links `mog-core`.
pub fn linked_core_version() -> &'static str {
    mog_core::core_version()
}

#[cfg(test)]
mod tests {
    #[test]
    fn arc_1_ffi_crate_links_mog_core() {
        assert_eq!(super::linked_core_version(), mog_core::core_version());
    }
}
