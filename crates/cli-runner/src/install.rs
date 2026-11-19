//! The guided install of the official Databricks CLI (#161; W-17b, W-17d, W-17e, W-17f,
//! SEC-20b, U16).
//!
//! Bottle never bundles the CLI (W-17a). After the user agrees, [`install`]:
//! 1. makes a new, empty download folder (`Bottle/downloads/<unique>`),
//! 2. downloads the release checksum file `databricks_cli_<v>_SHA256SUMS` and checks it against
//!    the SHA-256 pinned in this file ([`PINNED_SUMS_SHA256`], SEC-20b),
//! 3. downloads the OS and CPU archive from `github.com/databricks/cli/releases` and checks it
//!    against its line in the checksum file before it unpacks it (W-17d),
//! 4. unpacks only the top-level `databricks` (`databricks.exe`) entry and moves it into the
//!    Bottle bin folder: macOS `~/Library/Application Support/Bottle/bin`, Windows
//!    `%LOCALAPPDATA%\Bottle\bin` (W-17e). No admin rights, no PATH change.
//!
//! Any failure deletes the download folder, so a changed archive never stays on disk. An
//! existing CLI elsewhere is never touched; [`crate::resolve`] looks in the Bottle bin folder
//! first, so R5 uses the Bottle copy (W-17f). The network is behind [`Fetch`]: tests use fakes.
//! The downloaded files are data only: nothing in the download folder is run.

use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use artifact_verify::{verify_against_sums, verify_file, Sha256Digest, Sha256Sums, VerifyError};

use crate::resolve::binary_name;
use crate::sys::Env;
use crate::version::Version;

/// The CLI release that Bottle installs (U16). It is at or above the recommended 1.25.
pub const PINNED_CLI_VERSION: Version = Version::new(1, 26, 0);
/// SEC-20b: the SHA-256 of `databricks_cli_1.26.0_SHA256SUMS`, the release checksum file. Each
/// archive is checked against its line in that file, so this one pin covers every OS and CPU.
pub const PINNED_SUMS_SHA256: &str =
    "74acf2bbc7cd5c36fb77386b5980009bc17930f6af2b69953333e14ee69b08bc";
/// The official release downloads (W-17d). Only this prefix is asked for.
pub const RELEASE_DOWNLOAD_URL: &str = "https://github.com/databricks/cli/releases/download/";
/// The release page the install step shows as the source.
pub const RELEASES_PAGE: &str = "https://github.com/databricks/cli/releases";

/// The checksum file is a few lines of text.
const MAX_SUMS_BYTES: u64 = 64 * 1024;
/// The release archive is about 50 MB; anything far larger is refused.
const MAX_ARCHIVE_BYTES: u64 = 512 * 1024 * 1024;

/// The OS of a release asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
}

/// The CPU of a release asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X64,
    Arm64,
}

impl Os {
    /// The OS this build runs on, if the CLI has a release for it.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Os::MacOs)
        } else if cfg!(windows) {
            Some(Os::Windows)
        } else if cfg!(target_os = "linux") {
            Some(Os::Linux)
        } else {
            None
        }
    }

    fn asset(self) -> &'static str {
        match self {
            Os::MacOs => "darwin",
            Os::Windows => "windows",
            Os::Linux => "linux",
        }
    }
}

impl Arch {
    /// The CPU this build runs on, if the CLI has a release for it.
    pub fn current() -> Option<Self> {
        if cfg!(target_arch = "x86_64") {
            Some(Arch::X64)
        } else if cfg!(target_arch = "aarch64") {
            Some(Arch::Arm64)
        } else {
            None
        }
    }

    fn asset(self) -> &'static str {
        match self {
            Arch::X64 => "amd64",
            Arch::Arm64 => "arm64",
        }
    }
}

/// An absolute folder from an env variable; a relative or empty value is no folder.
fn abs_var<E: Env + ?Sized>(env: &E, name: &str) -> Option<PathBuf> {
    env.var(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

/// W-17e: the Bottle bin folder of `os`, a folder only the user can write. `None` when the
/// user folder variable is missing or relative (never the cwd).
pub fn bottle_bin_dir_for<E: Env + ?Sized>(os: Os, env: &E) -> Option<PathBuf> {
    let base = match os {
        Os::MacOs => abs_var(env, "HOME")?
            .join("Library")
            .join("Application Support"),
        Os::Windows => abs_var(env, "LOCALAPPDATA")?,
        Os::Linux => abs_var(env, "XDG_DATA_HOME")
            .or_else(|| abs_var(env, "HOME").map(|h| h.join(".local").join("share")))?,
    };
    Some(base.join("Bottle").join("bin"))
}

/// The Bottle bin folder of this OS ([`bottle_bin_dir_for`]).
pub fn bottle_bin_dir<E: Env + ?Sized>(env: &E) -> Option<PathBuf> {
    bottle_bin_dir_for(Os::current()?, env)
}

/// Downloads one URL into `out`. The real one is HTTPS to GitHub only, with no token.
pub trait Fetch {
    fn fetch(&self, url: &str, out: &mut dyn Write) -> io::Result<()>;
}

/// What to download, what to check it against, and where to put the CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub version: Version,
    pub os: Os,
    /// For example `databricks_cli_1.26.0_darwin_arm64.zip`.
    pub archive_name: String,
    pub archive_url: String,
    pub sums_name: String,
    pub sums_url: String,
    /// The pinned SHA-256 (hex) of the checksum file (SEC-20b).
    pub sums_sha256: String,
    /// The Bottle bin folder (W-17e).
    pub bin_dir: PathBuf,
}

impl InstallPlan {
    /// The pinned release ([`PINNED_CLI_VERSION`]) for `os` and `arch`, into `bin_dir`.
    pub fn pinned(os: Os, arch: Arch, bin_dir: PathBuf) -> Self {
        let v = PINNED_CLI_VERSION;
        let base = format!("{RELEASE_DOWNLOAD_URL}v{v}/");
        let archive_name = format!("databricks_cli_{v}_{}_{}.zip", os.asset(), arch.asset());
        let sums_name = format!("databricks_cli_{v}_SHA256SUMS");
        Self {
            version: v,
            os,
            archive_url: format!("{base}{archive_name}"),
            archive_name,
            sums_url: format!("{base}{sums_name}"),
            sums_name,
            sums_sha256: PINNED_SUMS_SHA256.to_owned(),
            bin_dir,
        }
    }

    /// The pinned release for this OS and CPU, into the Bottle bin folder of this user.
    pub fn for_this_machine<E: Env + ?Sized>(env: &E) -> Result<Self, InstallError> {
        let (os, arch) = Os::current()
            .zip(Arch::current())
            .ok_or(InstallError::Unsupported)?;
        let bin = bottle_bin_dir_for(os, env).ok_or(InstallError::NoUserFolder)?;
        Ok(Self::pinned(os, arch, bin))
    }

    /// The folder that holds the download folders: `Bottle/downloads`.
    pub fn download_root(&self) -> PathBuf {
        self.bin_dir
            .parent()
            .unwrap_or(&self.bin_dir)
            .join("downloads")
    }

    /// The CLI file name for the plan's OS.
    pub fn binary_name(&self) -> &'static str {
        binary_name(self.os == Os::Windows)
    }
}

/// Why the install stopped. [`InstallError::code`] is a stable, content-free code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallError {
    /// The user has not agreed (W-17b). Nothing was downloaded.
    ConsentRequired,
    /// The CLI has no release for this OS or CPU.
    Unsupported,
    /// No user folder for the Bottle bin folder.
    NoUserFolder,
    /// A download failed or was too large.
    Download,
    /// The checksum file or the archive did not match (SEC-20b). The download is deleted.
    ChecksumMismatch,
    /// The archive has no top-level CLI entry, or did not unpack.
    ArchiveInvalid,
    /// A local file operation failed.
    Io,
}

impl InstallError {
    pub fn code(self) -> &'static str {
        match self {
            InstallError::ConsentRequired => "cli_install_consent_required",
            InstallError::Unsupported => "cli_install_unsupported_platform",
            InstallError::NoUserFolder => "cli_install_no_user_folder",
            InstallError::Download => "cli_install_download_failed",
            InstallError::ChecksumMismatch => "cli_install_checksum_mismatch",
            InstallError::ArchiveInvalid => "cli_install_archive_invalid",
            InstallError::Io => "cli_install_io_failed",
        }
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for InstallError {}

/// Install the CLI of `plan` after consent (W-17b) and return its path. See the module doc.
pub fn install(
    plan: &InstallPlan,
    fetch: &dyn Fetch,
    consent: bool,
) -> Result<PathBuf, InstallError> {
    if !consent {
        return Err(InstallError::ConsentRequired);
    }
    let root = plan.download_root();
    let work = new_download_dir(&root)?;
    let result = install_in(plan, fetch, &work);
    // Every path deletes the download, so no unchecked or rejected file stays on disk.
    let _ = fs::remove_dir_all(&work);
    // Only removes the folder when no other download is in it.
    let _ = fs::remove_dir(&root);
    result
}

fn install_in(plan: &InstallPlan, fetch: &dyn Fetch, work: &Path) -> Result<PathBuf, InstallError> {
    // SEC-20b: the checksum file must match its pin before any line of it is trusted.
    let sums_path = work.join(&plan.sums_name);
    download(fetch, &plan.sums_url, &sums_path, MAX_SUMS_BYTES)?;
    let pin =
        Sha256Digest::from_hex(&plan.sums_sha256).map_err(|_| InstallError::ChecksumMismatch)?;
    verify_file(&sums_path, &pin).map_err(verify_error)?;
    let text = fs::read_to_string(&sums_path).map_err(|_| InstallError::ChecksumMismatch)?;
    let sums = Sha256Sums::parse(&text).map_err(|_| InstallError::ChecksumMismatch)?;

    // W-17d: the archive is checked against the checksum file before it is unpacked.
    let archive = work.join(&plan.archive_name);
    download(fetch, &plan.archive_url, &archive, MAX_ARCHIVE_BYTES)?;
    verify_against_sums(&archive, &sums, &plan.archive_name).map_err(verify_error)?;

    let name = plan.binary_name();
    let staged = work.join(format!("{name}.new"));
    unpack_cli(&archive, name, &staged)?;
    make_user_only(&staged)?;
    fs::create_dir_all(&plan.bin_dir).map_err(|_| InstallError::Io)?;
    make_user_only(&plan.bin_dir)?;
    let dest = plan.bin_dir.join(name);
    // Same volume (both under the Bottle folder): an atomic move over an older Bottle copy.
    fs::rename(&staged, &dest).map_err(|_| InstallError::Io)?;
    Ok(dest)
}

fn verify_error(e: VerifyError) -> InstallError {
    match e {
        VerifyError::Io { .. } => InstallError::Io,
        VerifyError::BadPin { .. }
        | VerifyError::Mismatch { .. }
        | VerifyError::NotListed { .. } => InstallError::ChecksumMismatch,
    }
}

/// A new, empty folder under `root` for one install.
fn new_download_dir(root: &Path) -> Result<PathBuf, InstallError> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(root).map_err(|_| InstallError::Io)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    for _ in 0..16 {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = root.join(format!("cli-{}-{nanos}-{n}", std::process::id()));
        // `create_dir` fails if the folder exists, so the folder is new and empty.
        match fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(InstallError::Io),
        }
    }
    Err(InstallError::Io)
}

/// A writer that refuses more than `left` bytes.
struct Capped<W> {
    inner: W,
    left: u64,
}

impl<W: Write> Write for Capped<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.left {
            return Err(io::Error::other("download too large"));
        }
        let n = self.inner.write(buf)?;
        self.left -= n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn download(fetch: &dyn Fetch, url: &str, dest: &Path, cap: u64) -> Result<(), InstallError> {
    if !url.starts_with(RELEASE_DOWNLOAD_URL) {
        return Err(InstallError::Download);
    }
    let file = fs::File::create_new(dest).map_err(|_| InstallError::Io)?;
    let mut out = Capped {
        inner: io::BufWriter::new(file),
        left: cap,
    };
    fetch
        .fetch(url, &mut out)
        .and_then(|()| out.flush())
        .map_err(|_| InstallError::Download)?;
    // Close the file before it is hashed or deleted (Windows).
    drop(out);
    Ok(())
}

/// Unpack only the top-level entry `name` of the zip `archive` into `dest`.
fn unpack_cli(archive: &Path, name: &str, dest: &Path) -> Result<(), InstallError> {
    let file = fs::File::open(archive).map_err(|_| InstallError::Io)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| InstallError::ArchiveInvalid)?;
    // `by_name` matches the exact entry name: `../databricks` or `bin/databricks` do not match.
    let mut entry = zip
        .by_name(name)
        .map_err(|_| InstallError::ArchiveInvalid)?;
    if !entry.is_file() || entry.size() > MAX_ARCHIVE_BYTES {
        return Err(InstallError::ArchiveInvalid);
    }
    let mut out = fs::File::create_new(dest).map_err(|_| InstallError::Io)?;
    io::copy(&mut (&mut entry).take(MAX_ARCHIVE_BYTES), &mut out)
        .map_err(|_| InstallError::ArchiveInvalid)?;
    out.sync_all().map_err(|_| InstallError::Io)
}

/// Unix: `rwxr-xr-x`, so only the user can write the CLI and its folder (W-02b, W-17e). Other
/// OSes: the user folder (`%LOCALAPPDATA%`) already gives only the user write access.
#[cfg(unix)]
fn make_user_only(path: &Path) -> Result<(), InstallError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).map_err(|_| InstallError::Io)
}

#[cfg(not(unix))]
fn make_user_only(_path: &Path) -> Result<(), InstallError> {
    Ok(())
}
