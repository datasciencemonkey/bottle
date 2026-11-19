// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).
//! Resolve `databricks` to an absolute path (W-02a).
//!
//! After the PATH entries, [`resolve`] looks in the usual install folders ([`fallback_dirs`], the
//! list of the reference `cli_candidates`). A macOS app that starts from Finder, the Dock or at
//! login gets the launchd PATH `/usr/bin:/bin:/usr/sbin:/sbin`, which has neither Homebrew nor
//! the install-script folder. The launch check runs the W-02b location check on the CLI found,
//! wherever it was found. The MDM `CliPath` still wins: then no other folder is searched.
//!
//! W-17f (#161): the Bottle bin folder ([`crate::bottle_bin_dir`]) comes first, before PATH. So
//! after the guided install R5 uses the Bottle copy, and an older CLI on PATH stays unchanged.

use std::path::{Path, PathBuf};

use crate::error::CliError;
use crate::sys::{Env, Fs};
use crate::version::{Version, MIN_VERSION};

/// Base name of the CLI binary.
pub const CLI_NAME: &str = "databricks";

/// Policy inputs (Appendix C MDM keys `CliPath`, `CliMinVersion`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliPolicy {
    /// MDM `CliPath`: when set, only this absolute path is used.
    pub cli_path: Option<PathBuf>,
    /// MDM `CliMinVersion` (default 1.17.0).
    pub min_version: Version,
}

impl Default for CliPolicy {
    fn default() -> Self {
        Self {
            cli_path: None,
            min_version: MIN_VERSION,
        }
    }
}

/// File name of the CLI binary on the given OS family.
pub fn binary_name(windows: bool) -> &'static str {
    if windows {
        "databricks.exe"
    } else {
        CLI_NAME
    }
}

/// A resolved CLI: where it was found and its canonical absolute path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub found: PathBuf,
    pub canonical: PathBuf,
}

/// The usual install folders, searched after PATH: Homebrew on Apple silicon, `/usr/local/bin`
/// (Homebrew on Intel and the official install script), `~/.local/bin` and `~/bin`. A home folder
/// is added only when `HOME` is absolute. Windows has none: a Windows app gets the user PATH.
pub fn fallback_dirs<E: Env + ?Sized>(env: &E) -> Vec<PathBuf> {
    if cfg!(windows) {
        return Vec::new();
    }
    let mut dirs = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ];
    if let Some(home) = env
        .var("HOME")
        .map(PathBuf::from)
        .filter(|h| h.is_absolute())
    {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join("bin"));
    }
    dirs
}

/// Finds the CLI: the policy path if set, else the Bottle bin folder, then the first absolute
/// PATH entry, then the first [`fallback_dirs`] folder, that holds an executable `databricks`.
pub fn resolve<F: Fs + ?Sized, E: Env + ?Sized>(
    fs: &F,
    env: &E,
    policy: &CliPolicy,
) -> Result<Resolved, CliError> {
    if let Some(path) = &policy.cli_path {
        if !path.is_absolute() {
            return Err(CliError::NotAbsolute { path: path.clone() });
        }
        return found_at(fs, path).ok_or(CliError::Missing);
    }
    let name = binary_name(cfg!(windows));
    let on_path: Vec<PathBuf> = env
        .var("PATH")
        .map(|v| std::env::split_paths(&v).collect())
        .unwrap_or_default();
    crate::install::bottle_bin_dir(env)
        .into_iter()
        .chain(on_path)
        .chain(fallback_dirs(env))
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| found_at(fs, &dir.join(name)))
        .ok_or(CliError::Missing)
}

/// `path` as a resolved CLI if it is an executable file.
fn found_at<F: Fs + ?Sized>(fs: &F, path: &Path) -> Option<Resolved> {
    if !fs.is_file(path) || fs.unix_mode(path).is_some_and(|m| m & 0o111 == 0) {
        return None;
    }
    let canonical = fs.canonicalize(path).ok()?;
    Some(Resolved {
        found: path.to_path_buf(),
        canonical,
    })
}
