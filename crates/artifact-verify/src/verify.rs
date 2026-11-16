//! Streaming SHA-256 verification of downloaded artefacts (SEC-20b, W-17d).

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::digest::Sha256Digest;
use crate::sums::{Sha256Sums, SumsError};

/// Read buffer size for streaming hashing.
const CHUNK: usize = 64 * 1024;

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
pub fn sha256_reader<R: Read>(mut reader: R) -> io::Result<Sha256Digest> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(Sha256Digest::from_bytes(hasher.finalize().into()))
}

/// Hashes a file without loading it into memory. The file is closed on return,
/// so it can be deleted afterwards on Windows too.
pub fn sha256_file(path: &Path) -> io::Result<Sha256Digest> {
    let file = File::open(path)?;
    sha256_reader(file)
}

/// Deletes a rejected download; true if the file is gone afterwards.
fn discard(path: &Path) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => true,
        Err(_) => !path.exists(),
    }
}

/// Checks `path` against `expected`. On a mismatch the file is deleted and
/// [`VerifyError::Mismatch`] is returned; the caller must stop the install.
pub fn verify_file(path: &Path, expected: &Sha256Digest) -> Result<(), VerifyError> {
    let actual = sha256_file(path).map_err(|source| VerifyError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if actual.ct_eq(expected) {
        return Ok(());
    }
    let deleted = discard(path);
    Err(VerifyError::Mismatch {
        path: path.to_path_buf(),
        expected: *expected,
        actual,
        deleted,
    })
}

/// Checks `path` against a pinned artefact hash. A malformed pin deletes the
/// download too, because it cannot be verified.
pub fn verify_pinned(path: &Path, pin: &PinnedArtifact) -> Result<(), VerifyError> {
    match Sha256Digest::from_hex(pin.sha256_hex) {
        Ok(expected) => verify_file(path, &expected),
        Err(_) => {
            discard(path);
            Err(VerifyError::BadPin {
                file_name: pin.file_name.to_string(),
            })
        }
    }
}

/// Checks `path` against the entry for `file_name` in a release `SHA256SUMS` file.
/// A missing entry deletes the download and returns [`VerifyError::NotListed`].
pub fn verify_against_sums(
    path: &Path,
    sums: &Sha256Sums,
    file_name: &str,
) -> Result<(), VerifyError> {
    match sums.expected_for(file_name) {
        Ok(expected) => verify_file(path, expected),
        Err(source) => {
            let deleted = discard(path);
            Err(VerifyError::NotListed {
                path: path.to_path_buf(),
                source,
                deleted,
            })
        }
    }
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
