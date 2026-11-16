//! Bottle artefact verification (SEC-20b).
//!
//! * [`verify`]: streaming SHA-256 check of a downloaded artefact against a pinned
//!   digest, with a constant-time comparison. A mismatch deletes the file (SEC-20b, W-17d).
//! * [`sums`]: parser for the Databricks CLI release `databricks_cli_<ver>_SHA256SUMS` file.
//!
//! The licence notices (SEC-21a, SEC-21b) are checked by `tests/third-party-notices.test.ts` (#228).

pub mod digest;
pub mod sums;
pub mod verify;

pub use digest::{DigestParseError, Sha256Digest};
pub use sums::{Sha256Sums, SumsError};
pub use verify::{
    sha256_file, sha256_reader, verify_against_sums, verify_file, verify_pinned, PinnedArtifact,
    VerifyError,
};
