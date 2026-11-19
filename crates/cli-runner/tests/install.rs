//! The guided CLI install (#161; W-17a, W-17b, W-17d, W-17e, W-17f, SEC-20b, U16).
//!
//! No test reaches the network: [`FakeFetch`] serves fake archives and fake checksum files from
//! memory and records every URL. Every value is fake.

mod common;

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use cli_runner::install::{
    bottle_bin_dir_for, install, Arch, Fetch, InstallError, InstallPlan, Os, PINNED_CLI_VERSION,
    PINNED_SUMS_SHA256, RELEASE_DOWNLOAD_URL,
};
use cli_runner::{binary_name, resolve, CliPolicy, Version};
use common::{abs, cli_in, FakeEnv, FakeFs};
use sha2::{Digest, Sha256};

/// Serves fixed bodies by URL and records every URL asked for.
#[derive(Default)]
struct FakeFetch {
    bodies: HashMap<String, Vec<u8>>,
    calls: RefCell<Vec<String>>,
}

impl Fetch for FakeFetch {
    fn fetch(&self, url: &str, out: &mut dyn Write) -> io::Result<()> {
        self.calls.borrow_mut().push(url.to_owned());
        match self.bodies.get(url) {
            Some(b) => out.write_all(b),
            None => Err(io::Error::new(io::ErrorKind::NotFound, "fake: 404")),
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A zip archive with these entries (stored, no compression).
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buf = io::Cursor::new(Vec::new());
    {
        let mut w = zip::ZipWriter::new(&mut buf);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, body) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap();
    }
    buf.into_inner()
}

const FAKE_CLI: &[u8] = b"#!/bin/sh\necho fake databricks cli, not real\n";

/// A plan for this machine's OS name, in `root`, with a fake checksum file and archive. The
/// pinned hash of the plan is the hash of the fake checksum file.
struct Fixture {
    _dir: tempfile::TempDir,
    plan: InstallPlan,
    fetch: FakeFetch,
}

fn fixture_with(archive: Vec<u8>, listed: Option<Vec<u8>>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let os = if cfg!(windows) {
        Os::Windows
    } else {
        Os::MacOs
    };
    let mut plan = InstallPlan::pinned(os, Arch::Arm64, dir.path().join("Bottle").join("bin"));
    // The checksum file lists the hash of `listed` (default: the archive itself).
    let listed_hash = hex(listed.as_deref().unwrap_or(&archive));
    let sums = format!(
        "{}  databricks_cli_1.26.0_linux_amd64.zip\n{listed_hash}  {}\n",
        "0".repeat(64),
        plan.archive_name
    )
    .into_bytes();
    plan.sums_sha256 = hex(&sums);
    let mut fetch = FakeFetch::default();
    fetch.bodies.insert(plan.sums_url.clone(), sums);
    fetch.bodies.insert(plan.archive_url.clone(), archive);
    Fixture {
        _dir: dir,
        plan,
        fetch,
    }
}

fn good_archive() -> Vec<u8> {
    let name = binary_name(cfg!(windows));
    zip_of(&[
        ("LICENSE", b"fake licence"),
        ("README.md", b"fake readme"),
        (name, FAKE_CLI),
    ])
}

fn fixture() -> Fixture {
    fixture_with(good_archive(), None)
}

/// No file is left in the download folders of the plan.
fn downloads_empty(plan: &InstallPlan) -> bool {
    let root = plan.download_root();
    !root.exists() || std::fs::read_dir(&root).unwrap().next().is_none()
}

/// [W-17b] [U16] without consent no download starts and nothing is installed
#[test]
fn w_17b_no_download_without_consent() {
    let f = fixture();
    let err = install(&f.plan, &f.fetch, false).unwrap_err();
    assert_eq!(err, InstallError::ConsentRequired);
    assert_eq!(err.code(), "cli_install_consent_required");
    assert!(f.fetch.calls.borrow().is_empty(), "no fetch before consent");
    assert!(!f.plan.bin_dir.exists());
    assert!(downloads_empty(&f.plan));
}

/// [W-17d] the official release of the OS and CPU comes from github.com/databricks/cli/releases
#[test]
fn w_17d_release_asset_for_each_os_and_cpu() {
    assert_eq!(PINNED_CLI_VERSION, Version::new(1, 26, 0));
    assert!(PINNED_CLI_VERSION >= cli_runner::MIN_VERSION);
    assert_eq!(
        RELEASE_DOWNLOAD_URL,
        "https://github.com/databricks/cli/releases/download/"
    );
    let bin = abs("x/Bottle/bin");
    let cases = [
        (
            Os::MacOs,
            Arch::Arm64,
            "databricks_cli_1.26.0_darwin_arm64.zip",
        ),
        (
            Os::MacOs,
            Arch::X64,
            "databricks_cli_1.26.0_darwin_amd64.zip",
        ),
        (
            Os::Windows,
            Arch::X64,
            "databricks_cli_1.26.0_windows_amd64.zip",
        ),
        (
            Os::Windows,
            Arch::Arm64,
            "databricks_cli_1.26.0_windows_arm64.zip",
        ),
        (
            Os::Linux,
            Arch::X64,
            "databricks_cli_1.26.0_linux_amd64.zip",
        ),
    ];
    for (os, arch, name) in cases {
        let p = InstallPlan::pinned(os, arch, bin.clone());
        assert_eq!(p.archive_name, name);
        assert_eq!(
            p.archive_url,
            format!("https://github.com/databricks/cli/releases/download/v1.26.0/{name}")
        );
        assert_eq!(
            p.sums_url,
            "https://github.com/databricks/cli/releases/download/v1.26.0/databricks_cli_1.26.0_SHA256SUMS"
        );
        assert_eq!(p.sums_sha256, PINNED_SUMS_SHA256);
        assert_eq!(p.version, PINNED_CLI_VERSION);
    }
}

/// [SEC-20b] the pinned SHA-256 of the release checksum file is a well-formed digest
#[test]
fn sec_20b_pinned_checksum_file_hash_is_well_formed() {
    assert_eq!(PINNED_SUMS_SHA256.len(), 64);
    assert!(PINNED_SUMS_SHA256
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
}

/// [W-17d] [W-17e] after consent the checked archive installs the CLI in the Bottle bin folder
#[test]
fn w_17d_w_17e_consent_installs_the_checked_cli() {
    let f = fixture();
    let path = install(&f.plan, &f.fetch, true).unwrap();
    assert_eq!(path, f.plan.bin_dir.join(binary_name(cfg!(windows))));
    assert_eq!(std::fs::read(&path).unwrap(), FAKE_CLI);
    // The checksum file first, then the archive: nothing else.
    assert_eq!(
        *f.fetch.calls.borrow(),
        vec![f.plan.sums_url.clone(), f.plan.archive_url.clone()]
    );
    // Only the CLI is unpacked; the download folder is gone.
    let names: Vec<_> = std::fs::read_dir(&f.plan.bin_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        vec![std::ffi::OsString::from(binary_name(cfg!(windows)))]
    );
    assert!(downloads_empty(&f.plan));
}

/// [W-17e] the installed CLI is executable and only the user can write it or its folder
#[cfg(unix)]
#[test]
fn w_17e_installed_cli_is_user_only_writable() {
    use std::os::unix::fs::PermissionsExt;
    let f = fixture();
    let path = install(&f.plan, &f.fetch, true).unwrap();
    for p in [path.as_path(), f.plan.bin_dir.as_path()] {
        let mode = std::fs::metadata(p).unwrap().permissions().mode();
        assert_eq!(mode & 0o022, 0, "{} is writable by others", p.display());
        assert_eq!(mode & 0o100, 0o100, "{} is not executable", p.display());
    }
}

/// [W-17d] [SEC-20b] a changed archive stops the install and deletes the download
#[test]
fn w_17d_sec_20b_changed_archive_stops_and_deletes_the_download() {
    let listed = good_archive();
    let mut changed = listed.clone();
    let last = changed.len() - 30;
    changed[last] ^= 0xff;
    let f = fixture_with(changed, Some(listed));
    let err = install(&f.plan, &f.fetch, true).unwrap_err();
    assert_eq!(err, InstallError::ChecksumMismatch);
    assert_eq!(err.code(), "cli_install_checksum_mismatch");
    assert!(downloads_empty(&f.plan), "the download is deleted");
    assert!(!f.plan.bin_dir.join(binary_name(cfg!(windows))).exists());
}

/// [SEC-20b] a checksum file that does not match its pinned SHA-256 stops before the archive
#[test]
fn sec_20b_changed_checksum_file_stops_before_the_archive() {
    let mut f = fixture();
    f.plan.sums_sha256 = "ab".repeat(32);
    let err = install(&f.plan, &f.fetch, true).unwrap_err();
    assert_eq!(err, InstallError::ChecksumMismatch);
    assert_eq!(*f.fetch.calls.borrow(), vec![f.plan.sums_url.clone()]);
    assert!(downloads_empty(&f.plan));
    assert!(!f.plan.bin_dir.join(binary_name(cfg!(windows))).exists());
}

/// [W-17d] [SEC-20b] an archive that the checksum file does not list is refused and deleted
#[test]
fn w_17d_unlisted_archive_is_refused() {
    let mut f = fixture();
    let sums = format!("{}  some_other_file.zip\n", "1".repeat(64)).into_bytes();
    f.plan.sums_sha256 = hex(&sums);
    f.fetch.bodies.insert(f.plan.sums_url.clone(), sums);
    let err = install(&f.plan, &f.fetch, true).unwrap_err();
    assert_eq!(err, InstallError::ChecksumMismatch);
    assert!(downloads_empty(&f.plan));
}

/// [W-17d] an archive without a top-level CLI (or with a path that leaves the folder) is refused
#[test]
fn w_17d_archive_without_a_top_level_cli_is_refused() {
    let name = binary_name(cfg!(windows));
    let evil = zip_of(&[
        (&format!("../{name}") as &str, FAKE_CLI),
        (&format!("sub/{name}") as &str, FAKE_CLI),
    ]);
    let f = fixture_with(evil, None);
    let err = install(&f.plan, &f.fetch, true).unwrap_err();
    assert_eq!(err, InstallError::ArchiveInvalid);
    assert!(downloads_empty(&f.plan));
    assert!(!f.plan.bin_dir.join(name).exists());
    assert!(!f.plan.bin_dir.parent().unwrap().join(name).exists());
}

/// [W-17d] a failed download stops the install and leaves no file
#[test]
fn w_17d_failed_download_leaves_nothing() {
    let mut f = fixture();
    f.fetch.bodies.remove(&f.plan.archive_url);
    let err = install(&f.plan, &f.fetch, true).unwrap_err();
    assert_eq!(err, InstallError::Download);
    assert!(downloads_empty(&f.plan));
}

/// [W-17e] the install folder is user-only: macOS Application Support, Windows LOCALAPPDATA
#[test]
fn w_17e_bottle_bin_folder_per_os() {
    let env = FakeEnv::new()
        .set("HOME", abs("Users/someone"))
        .set("LOCALAPPDATA", abs("Users/someone/AppData/Local"));
    assert_eq!(
        bottle_bin_dir_for(Os::MacOs, &env),
        Some(abs("Users/someone/Library/Application Support/Bottle/bin"))
    );
    assert_eq!(
        bottle_bin_dir_for(Os::Windows, &env),
        Some(abs("Users/someone/AppData/Local/Bottle/bin"))
    );
    // A relative or missing folder variable gives no folder (never the cwd).
    let rel = FakeEnv::new()
        .set("HOME", "someone")
        .set("LOCALAPPDATA", "x");
    assert_eq!(bottle_bin_dir_for(Os::MacOs, &rel), None);
    assert_eq!(bottle_bin_dir_for(Os::Windows, &rel), None);
}

/// [W-17f] the Bottle copy wins over an old CLI on PATH, which is not changed
#[test]
fn w_17f_resolve_uses_the_bottle_copy_first() {
    let env = FakeEnv::new()
        .set("HOME", abs("Users/someone"))
        .set("LOCALAPPDATA", abs("Users/someone/AppData/Local"))
        .with_path(&[abs("usr/local/bin")]);
    let bottle = cli_in(&cli_runner::bottle_bin_dir(&env).expect("a Bottle bin folder"));
    let old = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&old, 0o755).file(&bottle, 0o755);
    assert_eq!(
        resolve(&fs, &env, &CliPolicy::default()).unwrap().found,
        bottle
    );
    // Without the Bottle copy the old CLI is found as before.
    let fs = FakeFs::new().file(&old, 0o755);
    assert_eq!(
        resolve(&fs, &env, &CliPolicy::default()).unwrap().found,
        old
    );
    // The MDM CliPath still wins over the Bottle copy.
    let pinned = cli_in(&abs("opt/corp/bin"));
    let fs = FakeFs::new().file(&pinned, 0o755).file(&bottle, 0o755);
    let policy = CliPolicy {
        cli_path: Some(pinned.clone()),
        ..CliPolicy::default()
    };
    assert_eq!(resolve(&fs, &env, &policy).unwrap().found, pinned);
}

/// [W-17f] the install never changes an existing CLI outside the Bottle folder
#[test]
fn w_17f_install_leaves_an_old_cli_unchanged() {
    let f = fixture();
    let old_dir = f
        .plan
        .bin_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("old");
    std::fs::create_dir_all(&old_dir).unwrap();
    let old = old_dir.join(binary_name(cfg!(windows)));
    std::fs::write(&old, b"old fake cli 1.10").unwrap();
    install(&f.plan, &f.fetch, true).unwrap();
    assert_eq!(std::fs::read(&old).unwrap(), b"old fake cli 1.10");
}

/// [W-17a] the installer bundles no CLI: tauri.conf.json names no CLI binary or resource
#[test]
fn w_17a_installer_bundles_no_cli() {
    let conf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri/tauri.conf.json");
    let text = std::fs::read_to_string(&conf).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let bundle = &v["bundle"];
    for key in ["externalBin", "resources"] {
        let listed = bundle[key].to_string().to_ascii_lowercase();
        assert!(
            !listed.contains("databricks"),
            "bundle.{key} lists a Databricks CLI: {listed}"
        );
    }
    // No CLI binary sits in the bundle inputs either.
    let tauri_dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../src-tauri");
    for name in ["databricks", "databricks.exe"] {
        assert!(!tauri_dir.join(name).exists());
        assert!(!tauri_dir.join("binaries").join(name).exists());
    }
}
