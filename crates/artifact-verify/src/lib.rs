//! Bottle artefact verification and third-party notice checks.
//!
//! * [`verify`]: streaming SHA-256 check of a downloaded artefact against a pinned
//!   digest, with a constant-time comparison. A mismatch deletes the file (SEC-20b, W-17d).
//! * [`sums`]: parser for the Databricks CLI release `databricks_cli_<ver>_SHA256SUMS` file.
//! * [`notices`]: checks for `THIRD_PARTY_NOTICES.md` (SEC-21b) and for the
//!   antigravity-bricks-plugin MIT header in ported files (SEC-21a).

pub mod digest;
pub mod notices;
pub mod sums;
pub mod verify;

pub use digest::{DigestParseError, Sha256Digest};
pub use sums::{Sha256Sums, SumsError};
pub use verify::{
    sha256_file, sha256_reader, verify_against_sums, verify_file, verify_pinned, PinnedArtifact,
    VerifyError,
};
