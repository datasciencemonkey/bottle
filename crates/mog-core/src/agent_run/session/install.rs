//! `genie_code_install` (plan Amendment 2, A7; v1 2.2).
//!
//! The official install source is an open owner question (`pin.rs` holds no URL), so
//! [`INSTALL_SOURCE`] is `None` and `genie_code_install` answers `install_failed`. The checked
//! placement is built and tested: the bytes must match the pinned SHA-256 before they reach the
//! final path, and the file is user-only.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The pinned download URL. `None` until the owner names the official source.
pub const INSTALL_SOURCE: Option<&str> = None;
/// The most bytes that one download may have.
pub const MAX_DOWNLOAD_BYTES: usize = 512 * 1024 * 1024;

/// The download seam. The real one is added with the install source.
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &str, max_bytes: usize) -> io::Result<Vec<u8>>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum InstallError {
    /// The download or the placement failed. IPC code `install_failed`.
    Failed,
    /// The SHA-256 is not the pinned one. IPC code `checksum_mismatch`.
    ChecksumMismatch,
}

impl InstallError {
    pub fn code(&self) -> &'static str {
        match self {
            InstallError::Failed => "install_failed",
            InstallError::ChecksumMismatch => "checksum_mismatch",
        }
    }
}

/// Check `bytes` against `want_hex`, then write them to `<dir>/<name>` (0700) through a temp
/// file and a rename. Nothing is left at the final path on a mismatch.
pub fn place_checked(
    bytes: &[u8],
    want_hex: &str,
    dir: &Path,
    name: &str,
) -> Result<PathBuf, InstallError> {
    let have: String = Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if have != want_hex {
        return Err(InstallError::ChecksumMismatch);
    }
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).map_err(|_| InstallError::Failed)?;
    let tmp = dir.join(format!(".{name}.part"));
    let _ = fs::remove_file(&tmp);
    let mut o = fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o700);
    }
    let r = o
        .open(&tmp)
        .and_then(|mut f| f.write_all(bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, dir.join(name)));
    if r.is_err() {
        let _ = fs::remove_file(&tmp);
        return Err(InstallError::Failed);
    }
    Ok(dir.join(name))
}

/// Download the pinned binary and place it. With no install source this is `Failed`.
pub fn install(
    fetcher: &dyn Fetcher,
    source: Option<&str>,
    want_hex: &str,
    dir: &Path,
) -> Result<PathBuf, InstallError> {
    let url = source.ok_or(InstallError::Failed)?;
    let bytes = fetcher
        .fetch(url, MAX_DOWNLOAD_BYTES)
        .map_err(|_| InstallError::Failed)?;
    place_checked(&bytes, want_hex, dir, "genie")
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<u8>);
    impl Fetcher for Fixed {
        fn fetch(&self, _: &str, _: usize) -> io::Result<Vec<u8>> {
            Ok(self.0.clone())
        }
    }

    fn sha(b: &[u8]) -> String {
        Sha256::digest(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    }

    fn dir() -> PathBuf {
        let mut n = [0u8; 6];
        getrandom::fill(&mut n).unwrap();
        std::env::temp_dir().join(format!(
            "r1-inst-{}",
            n.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))
    }

    #[test]
    fn a_matching_digest_places_a_user_only_file() {
        let d = dir();
        let p = install(
            &Fixed(b"genie".to_vec()),
            Some("https://x/y"),
            &sha(b"genie"),
            &d,
        )
        .unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"genie");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&p).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_digest_mismatch_leaves_nothing_at_the_final_path() {
        let d = dir();
        let e = install(
            &Fixed(b"evil".to_vec()),
            Some("https://x/y"),
            &sha(b"genie"),
            &d,
        );
        assert_eq!(e, Err(InstallError::ChecksumMismatch));
        assert!(!d.join("genie").exists());
        assert_eq!(e.unwrap_err().code(), "checksum_mismatch");
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn with_no_install_source_the_install_fails() {
        assert!(INSTALL_SOURCE.is_none());
        let e = install(&Fixed(vec![]), INSTALL_SOURCE, &sha(b""), &dir());
        assert_eq!(e, Err(InstallError::Failed));
        assert_eq!(InstallError::Failed.code(), "install_failed");
    }
}
