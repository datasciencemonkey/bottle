//! Tests for streaming SHA-256 verification of downloaded artefacts.

use std::fs;
use std::io::{self, Read};
use std::path::Path;

use artifact_verify::{
    sha256_file, sha256_reader, verify_against_sums, verify_file, verify_pinned, PinnedArtifact,
    Sha256Digest, Sha256Sums, VerifyError,
};
use sha2::{Digest, Sha256};

const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn write(dir: &Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(name);
    fs::write(&p, bytes).unwrap();
    p
}

/// A reader that hands out at most 7 bytes per call and is interrupted once.
struct Dribble<'a> {
    data: &'a [u8],
    interrupted: bool,
}

impl Read for Dribble<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::Error::new(io::ErrorKind::Interrupted, "retry"));
        }
        let n = self.data.len().min(buf.len()).min(7);
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

/// [SEC-20b] digest hex parsing accepts 64 hex chars in either case and rejects others
#[test]
fn digest_hex_round_trip_and_rejects_bad_input() {
    let d = Sha256Digest::from_hex(ABC_SHA256).unwrap();
    assert_eq!(d.to_hex(), ABC_SHA256);
    assert_eq!(Sha256Digest::from_hex(&ABC_SHA256.to_uppercase()).unwrap(), d);
    assert!(Sha256Digest::from_hex(&ABC_SHA256[..63]).is_err());
    assert!(Sha256Digest::from_hex(&format!("{ABC_SHA256}0")).is_err());
    let bad = format!("{}g", &ABC_SHA256[..63]);
    assert!(Sha256Digest::from_hex(&bad).is_err());
    assert!(Sha256Digest::from_hex("").is_err());
}

/// [SEC-20b] equality is a constant-time compare of all 32 bytes
#[test]
fn digest_constant_time_equality() {
    let a = Sha256Digest::from_hex(ABC_SHA256).unwrap();
    let mut bytes = *a.as_bytes();
    assert!(a.ct_eq(&Sha256Digest::from_bytes(bytes)));
    bytes[31] ^= 1;
    assert!(!a.ct_eq(&Sha256Digest::from_bytes(bytes)));
    bytes[31] ^= 1;
    bytes[0] ^= 0x80;
    assert_ne!(a, Sha256Digest::from_bytes(bytes));
}

/// [SEC-20b] streaming hash matches known vectors and a one-shot hash, across chunk sizes
#[test]
fn streaming_hash_matches_one_shot() {
    assert_eq!(sha256_reader(&b""[..]).unwrap().to_hex(), EMPTY_SHA256);
    assert_eq!(sha256_reader(&b"abc"[..]).unwrap().to_hex(), ABC_SHA256);

    let data: Vec<u8> = (0..(3 * 1024 * 1024 + 17)).map(|i| (i % 251) as u8).collect();
    let one_shot: [u8; 32] = Sha256::digest(&data).into();
    assert_eq!(
        sha256_reader(&data[..]).unwrap(),
        Sha256Digest::from_bytes(one_shot)
    );
    let dribble = Dribble {
        data: &data[..1000],
        interrupted: false,
    };
    let small: [u8; 32] = Sha256::digest(&data[..1000]).into();
    assert_eq!(
        sha256_reader(dribble).unwrap(),
        Sha256Digest::from_bytes(small)
    );

    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "big.bin", &data);
    assert_eq!(sha256_file(&p).unwrap(), Sha256Digest::from_bytes(one_shot));
}

/// [SEC-20b] [AT-37] a matching artefact passes and is kept
#[test]
fn matching_artifact_passes_and_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "model.onnx", b"abc");
    verify_file(&p, &Sha256Digest::from_hex(ABC_SHA256).unwrap()).unwrap();
    assert!(p.exists());
}

/// [SEC-20b] [AT-37] a changed artefact stops the install: typed mismatch error, file deleted
#[test]
fn changed_artifact_is_rejected_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let p = write(dir.path(), "model.onnx", b"abd");
    let err = verify_file(&p, &Sha256Digest::from_hex(ABC_SHA256).unwrap()).unwrap_err();
    match &err {
        VerifyError::Mismatch {
            path,
            expected,
            actual,
            deleted,
        } => {
            assert_eq!(path, &p);
            assert_eq!(expected.to_hex(), ABC_SHA256);
            assert_ne!(actual.to_hex(), ABC_SHA256);
            assert!(*deleted);
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
    assert!(!p.exists(), "the download must be deleted");
    assert!(err.to_string().contains(ABC_SHA256));
}

/// [SEC-20b] a pinned-artefact entry verifies; a bad pin is a typed error and keeps nothing
#[test]
fn pinned_artifact_verification() {
    let dir = tempfile::tempdir().unwrap();
    let ok = PinnedArtifact {
        file_name: "silero_vad.onnx",
        sha256_hex: ABC_SHA256,
    };
    let p = write(dir.path(), "silero_vad.onnx", b"abc");
    verify_pinned(&p, &ok).unwrap();

    let wrong = PinnedArtifact {
        file_name: "silero_vad.onnx",
        sha256_hex: EMPTY_SHA256,
    };
    assert!(matches!(
        verify_pinned(&p, &wrong),
        Err(VerifyError::Mismatch { deleted: true, .. })
    ));
    assert!(!p.exists());

    let p = write(dir.path(), "x.onnx", b"abc");
    let bad = PinnedArtifact {
        file_name: "x.onnx",
        sha256_hex: "not-hex",
    };
    assert!(matches!(
        verify_pinned(&p, &bad),
        Err(VerifyError::BadPin { .. })
    ));
    assert!(!p.exists(), "an unverifiable download must not be kept");
}

/// [SEC-20b] a missing download is an Io error, not a pass
#[test]
fn missing_file_is_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("absent.bin");
    assert!(matches!(
        verify_file(&p, &Sha256Digest::from_hex(ABC_SHA256).unwrap()),
        Err(VerifyError::Io { .. })
    ));
}

/// [W-17d] [SEC-20b] [AT-37] the CLI archive is checked against the release SHA256SUMS
#[test]
fn cli_archive_checked_against_release_sums() {
    let dir = tempfile::tempdir().unwrap();
    let name = "databricks_cli_0.250.0_linux_amd64.zip";
    let sums = Sha256Sums::parse(&format!(
        "{EMPTY_SHA256}  databricks_cli_0.250.0_darwin_arm64.zip\n{ABC_SHA256}  {name}\n"
    ))
    .unwrap();

    let good = write(dir.path(), name, b"abc");
    verify_against_sums(&good, &sums, name).unwrap();
    assert!(good.exists());

    let changed = write(dir.path(), "changed.zip", b"abc!");
    assert!(matches!(
        verify_against_sums(&changed, &sums, name),
        Err(VerifyError::Mismatch { deleted: true, .. })
    ));
    assert!(!changed.exists(), "a changed archive deletes the download");
}

/// [W-17d] [SEC-20b] an archive the SHA256SUMS file does not list stops the install and is deleted
#[test]
fn unlisted_cli_archive_is_rejected_and_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let sums = Sha256Sums::parse(&format!(
        "{ABC_SHA256}  databricks_cli_0.250.0_linux_amd64.zip\n"
    ))
    .unwrap();
    let p = write(dir.path(), "databricks_cli_0.250.0_windows_arm64.zip", b"abc");
    let err =
        verify_against_sums(&p, &sums, "databricks_cli_0.250.0_windows_arm64.zip").unwrap_err();
    assert!(matches!(err, VerifyError::NotListed { deleted: true, .. }));
    assert!(!p.exists());
}

/// Makes the file at `p` impossible to delete; returns a guard closure to undo it.
#[cfg(unix)]
fn make_undeletable(p: &Path) -> impl FnOnce() {
    use std::os::unix::fs::PermissionsExt;
    let dir = p.parent().unwrap().to_path_buf();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    move || fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap()
}

/// Makes the file at `p` impossible to delete; returns a guard closure to undo it.
#[cfg(windows)]
fn make_undeletable(p: &Path) -> impl FnOnce() {
    let mut perm = fs::metadata(p).unwrap().permissions();
    perm.set_readonly(true);
    fs::set_permissions(p, perm).unwrap();
    let p = p.to_path_buf();
    move || {
        let mut perm = fs::metadata(&p).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        perm.set_readonly(false);
        fs::set_permissions(&p, perm).unwrap();
    }
}

/// [SEC-20b] if the bad download cannot be deleted, the error still stops the install and says so
#[cfg(any(unix, windows))]
#[test]
fn mismatch_reports_failed_delete() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("dl");
    fs::create_dir(&sub).unwrap();
    let p = write(&sub, "model.onnx", b"abd");
    let undo = make_undeletable(&p);
    let res = verify_file(&p, &Sha256Digest::from_hex(ABC_SHA256).unwrap());
    let still_there = p.exists();
    undo();
    match res {
        Err(VerifyError::Mismatch { deleted, .. }) => {
            // Some CI users (root) can delete anyway; the flag must match reality.
            assert_eq!(deleted, !still_there);
        }
        other => panic!("expected Mismatch, got {other:?}"),
    }
}
