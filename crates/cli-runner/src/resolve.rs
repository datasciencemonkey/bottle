//! Resolve `databricks` to an absolute path (W-02a).

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

/// Finds the CLI: the policy path if set, else the first absolute PATH entry
/// that holds an executable `databricks`.
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
    let path_var = env.var("PATH").ok_or(CliError::Missing)?;
    let name = binary_name(cfg!(windows));
    std::env::split_paths(&path_var)
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
