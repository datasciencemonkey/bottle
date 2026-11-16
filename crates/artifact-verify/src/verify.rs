//! Streaming SHA-256 verification of downloaded artefacts (SEC-20b, W-17d).

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use crate::digest::Sha256Digest;
use crate::sums::{Sha256Sums, SumsError};

/// A downloadable artefact with its pinned SHA-256 (hex).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinnedArtifact {
    /// File name of the artefact, for messages.
    pub file_name: &'static str,
    /// Pinned lower- or upper-case hex SHA-256.
    pub sha256_hex: &'static str,
}

/// Why a downloaded artefact was rejected. In every case except `Io`, the
/// verifier has tried to delete the download; see `deleted`.
#[derive(Debug)]
pub enum VerifyError {
    /// The file could not be read.
    Io { path: PathBuf, source: io::Error },
    /// The pinned digest itself is malformed (a build error, not a download error).
    BadPin { file_name: String },
    /// The file hash does not match the pinned hash.
    Mismatch {
        path: PathBuf,
        expected: Sha256Digest,
        actual: Sha256Digest,
        deleted: bool,
    },
    /// The checksum file does not list the artefact, so it cannot be trusted.
    NotListed {
        path: PathBuf,
        source: SumsError,
        deleted: bool,
    },
}

/// Hashes everything readable from `reader` in fixed-size chunks.
pub fn sha256_reader<R: Read>(_reader: R) -> io::Result<Sha256Digest> {
    unimplemented!()
}

/// Hashes a file without loading it into memory.
pub fn sha256_file(_path: &Path) -> io::Result<Sha256Digest> {
    unimplemented!()
}

/// Checks `path` against `expected`. On a mismatch the file is deleted and
/// [`VerifyError::Mismatch`] is returned; the caller must stop the install.
pub fn verify_file(_path: &Path, _expected: &Sha256Digest) -> Result<(), VerifyError> {
    unimplemented!()
}

/// Checks `path` against a pinned artefact hash.
pub fn verify_pinned(_path: &Path, _pin: &PinnedArtifact) -> Result<(), VerifyError> {
    unimplemented!()
}

/// Checks `path` against the entry for `file_name` in a release `SHA256SUMS` file.
/// A missing entry deletes the download and returns [`VerifyError::NotListed`].
pub fn verify_against_sums(
    _path: &Path,
    _sums: &Sha256Sums,
    _file_name: &str,
) -> Result<(), VerifyError> {
    unimplemented!()
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "cannot read {}: {source}", path.display()),
            Self::BadPin { file_name } => write!(f, "pinned SHA-256 for {file_name} is malformed"),
            Self::Mismatch {
                path,
                expected,
                actual,
                deleted,
            } => write!(
                f,
                "SHA-256 mismatch for {}: expected {expected}, got {actual} (download deleted: {deleted})",
                path.display()
            ),
            Self::NotListed {
                path,
                source,
                deleted,
            } => write!(
                f,
                "{source}; refusing {} (download deleted: {deleted})",
                path.display()
            ),
        }
    }
}

impl std::error::Error for VerifyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::NotListed { source, .. } => Some(source),
            _ => None,
        }
    }
}
