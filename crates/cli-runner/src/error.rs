//! Typed CliRunner errors.

use std::fmt;
use std::path::PathBuf;

use crate::version::Version;

/// Why a CLI location is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsafeReason {
    /// The CLI is inside this temporary directory.
    TempDir { root: PathBuf },
    /// This path (the binary or its folder) is writable by every user.
    WorldWritable { path: PathBuf },
}

/// Errors from CliRunner. [`CliError::code`] is a stable machine code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliError {
    /// No `databricks` binary was found.
    Missing,
    /// The configured CLI path (MDM `CliPath`) is not absolute.
    NotAbsolute { path: PathBuf },
    /// The CLI is in a world-writable or temporary directory.
    UnsafeLocation { path: PathBuf, reason: UnsafeReason },
    /// The CLI is older than the minimum version.
    Unsupported { found: Version, min: Version },
    /// `databricks --version` failed or printed no version.
    VersionUnreadable,
    /// `run` was called before a successful `launch_check`.
    NotVerified,
    /// The CLI process could not be started.
    Spawn { message: String },
    /// The CLI child ran longer than the runner timeout and was killed.
    TimedOut,
    /// The caller cancelled the CLI child and it was killed (#162, W-07h).
    Cancelled,
}

impl CliError {
    /// Stable machine code for this error.
    pub fn code(&self) -> &'static str {
        match self {
            CliError::Missing | CliError::NotAbsolute { .. } => "CLI_MISSING",
            CliError::UnsafeLocation { .. } => "CLI_UNSAFE_PATH",
            CliError::Unsupported { .. } => "CLI_UNSUPPORTED",
            CliError::VersionUnreadable => "CLI_VERSION_UNREADABLE",
            CliError::NotVerified => "CLI_NOT_VERIFIED",
            CliError::Spawn { .. } => "CLI_SPAWN_FAILED",
            CliError::TimedOut => "CLI_TIMED_OUT",
            CliError::Cancelled => "CLI_CANCELLED",
        }
    }

    /// The `BackendErrorCode` (section 4.7) for this error: the CLI is unusable.
    pub fn backend_code(&self) -> &'static str {
        "CLI_MISSING"
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Missing => write!(f, "the Databricks CLI was not found"),
            CliError::NotAbsolute { path } => {
                write!(f, "the CLI path is not absolute: {}", path.display())
            }
            CliError::UnsafeLocation { path, reason } => match reason {
                UnsafeReason::TempDir { root } => write!(
                    f,
                    "the CLI at {} is in a temporary folder ({})",
                    path.display(),
                    root.display()
                ),
                UnsafeReason::WorldWritable { path: w } => write!(
                    f,
                    "the CLI at {} is unsafe: {} is writable by all users",
                    path.display(),
                    w.display()
                ),
            },
            CliError::Unsupported { found, min } => {
                write!(
                    f,
                    "CLI version {found} is too old; {min} or later is needed"
                )
            }
            CliError::VersionUnreadable => write!(f, "the CLI version could not be read"),
            CliError::NotVerified => write!(f, "the CLI was not checked at launch"),
            CliError::Spawn { message } => write!(f, "the CLI could not start: {message}"),
            CliError::TimedOut => write!(f, "the CLI did not answer in time"),
            CliError::Cancelled => write!(f, "the CLI was stopped"),
        }
    }
}

impl std::error::Error for CliError {}
