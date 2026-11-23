//! Slice 2a: the pinned Genie Code CLI (ADR to follow; plan Amendment 2 §A7).
//!
//! Bottle runs only this version, and only a binary whose SHA-256 matches. The digests were taken
//! from the owner's installed 0.5.0 copy on 2026-11-23. Any other version or digest is
//! `genie_code_version`. Re-run the lockdown canaries before changing this file.

/// The only Genie Code CLI version Bottle runs.
pub const GENIE_PINNED: &str = "0.5.0";

/// The SHA-256 of each pinned binary, by target triple. A target with no entry shows
/// "Genie Code is not available on this Mac yet".
pub const GENIE_DIGESTS: &[(&str, &str, &str)] = &[
    // (target triple, binary, SHA-256 hex)
    (
        "aarch64-apple-darwin",
        "genie",
        "d6faf47ecb8de61b91f52e38f63aee56d00b9bb05c1d607dfbff05ab060b97c2",
    ),
    (
        "aarch64-apple-darwin",
        "genie-code-mode-host",
        "de2c0bee5c412f345335b0b6048bf56e8d3070af404a1f0af3062fa66426557a",
    ),
];

/// The pinned digest of `binary` on `target`, if any.
pub fn pinned_digest(target: &str, binary: &str) -> Option<&'static str> {
    GENIE_DIGESTS
        .iter()
        .find(|(t, b, _)| *t == target && *b == binary)
        .map(|(_, _, d)| *d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_digest_is_64_lowercase_hex() {
        for (_, _, d) in GENIE_DIGESTS {
            assert_eq!(d.len(), 64);
            assert!(d
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c)));
        }
    }

    #[test]
    fn only_apple_silicon_is_pinned() {
        assert!(pinned_digest("aarch64-apple-darwin", "genie").is_some());
        assert!(pinned_digest("x86_64-apple-darwin", "genie").is_none());
        assert!(pinned_digest("aarch64-apple-darwin", "other").is_none());
    }
}
