//! CliRunner: launch checks (W-02a..d) and clean-env child processes (W-09).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::env::clean_env;
use crate::error::CliError;
use crate::location::check_location;
use crate::resolve::{resolve, CliPolicy};
use crate::sys::{Env, Fs, ProcOutput, Process, RealEnv, RealFs, RealProcess};
use crate::version::{Version, RECOMMENDED_VERSION};

/// A verified CLI. The app stores it and passes it back at the next launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliRecord {
    pub path: PathBuf,
    pub version: Version,
}

/// Non-blocking findings from a launch check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliWarning {
    /// The CLI path is different from the stored one.
    PathChanged { previous: PathBuf, current: PathBuf },
    /// The CLI version is different from the stored one.
    VersionChanged { previous: Version, current: Version },
    /// Supported, but older than the recommended version.
    BelowRecommended {
        found: Version,
        recommended: Version,
    },
}

impl CliWarning {
    pub fn code(&self) -> &'static str {
        match self {
            CliWarning::PathChanged { .. } => "CLI_PATH_CHANGED",
            CliWarning::VersionChanged { .. } => "CLI_VERSION_CHANGED",
            CliWarning::BelowRecommended { .. } => "CLI_BELOW_RECOMMENDED",
        }
    }
}

/// Result of a successful launch check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchReport {
    pub record: CliRecord,
    pub warnings: Vec<CliWarning>,
}

/// Data for the `cli_status` IPC (path, version, found). `plaintextStorage`
/// is owned by the profile/auth work, not this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliStatus {
    pub found: bool,
    pub path: Option<PathBuf>,
    pub version: Option<Version>,
}

/// Runs the Databricks CLI.
pub struct CliRunner<F, E, P> {
    fs: F,
    env: E,
    process: P,
    policy: CliPolicy,
    verified: Option<CliRecord>,
    timeout: Option<Duration>,
}

impl CliRunner<RealFs, RealEnv, RealProcess> {
    /// A runner on the real filesystem, env and processes.
    pub fn system(policy: CliPolicy) -> Self {
        Self::new(RealFs, RealEnv, RealProcess, policy)
    }
}

impl<F: Fs, E: Env, P: Process> CliRunner<F, E, P> {
    pub fn new(fs: F, env: E, process: P, policy: CliPolicy) -> Self {
        Self {
            fs,
            env,
            process,
            policy,
            verified: None,
            timeout: None,
        }
    }

    /// Kill every CLI child that runs longer than `timeout` ([`CliError::TimedOut`]).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }

    /// The child timeout, if any.
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }

    pub fn policy(&self) -> &CliPolicy {
        &self.policy
    }

    /// The CLI verified by the last successful `launch_check`.
    pub fn verified(&self) -> Option<&CliRecord> {
        self.verified.as_ref()
    }

    /// Resolve, check the location, read and check the version. Compares with
    /// `previous` (the stored record) and reports changes as warnings.
    pub fn launch_check(&mut self, previous: Option<&CliRecord>) -> Result<LaunchReport, CliError> {
        self.verified = None;
        let resolved = resolve(&self.fs, &self.env, &self.policy)?;
        check_location(&self.fs, &self.env, &resolved.found, &resolved.canonical)?;
        let path = resolved.canonical;
        let out = self.spawn(&path, &[OsString::from("--version")])?;
        if !out.success() {
            return Err(CliError::VersionUnreadable);
        }
        let version = Version::parse_cli_output(&String::from_utf8_lossy(&out.stdout))
            .ok_or(CliError::VersionUnreadable)?;
        if version < self.policy.min_version {
            return Err(CliError::Unsupported {
                found: version,
                min: self.policy.min_version,
            });
        }
        let record = CliRecord { path, version };
        let mut warnings = Vec::new();
        if let Some(prev) = previous {
            if prev.path != record.path {
                warnings.push(CliWarning::PathChanged {
                    previous: prev.path.clone(),
                    current: record.path.clone(),
                });
            }
            if prev.version != record.version {
                warnings.push(CliWarning::VersionChanged {
                    previous: prev.version,
                    current: record.version,
                });
            }
        }
        if version < RECOMMENDED_VERSION {
            warnings.push(CliWarning::BelowRecommended {
                found: version,
                recommended: RECOMMENDED_VERSION,
            });
        }
        self.verified = Some(record.clone());
        Ok(LaunchReport { record, warnings })
    }

    /// Status for Settings and the `cli_status` IPC.
    pub fn status(&self) -> CliStatus {
        CliStatus {
            found: self.verified.is_some(),
            path: self.verified.as_ref().map(|r| r.path.clone()),
            version: self.verified.as_ref().map(|r| r.version),
        }
    }

    /// The env every CLI child gets.
    pub fn child_env(&self) -> Vec<(OsString, OsString)> {
        clean_env(self.env.vars())
    }

    /// Runs the verified CLI with `args` and the clean env.
    pub fn run<I, S>(&self, args: I) -> Result<ProcOutput, CliError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let record = self.verified.as_ref().ok_or(CliError::NotVerified)?;
        // Cheap re-check: the verified file must still exist in a safe place.
        if !self.fs.is_file(&record.path) {
            return Err(CliError::Missing);
        }
        check_location(&self.fs, &self.env, &record.path, &record.path)?;
        let args: Vec<OsString> = args
            .into_iter()
            .map(|a| a.as_ref().to_os_string())
            .collect();
        self.spawn(&record.path, &args)
    }

    /// Every child goes through here, so every child gets the clean env.
    fn spawn(&self, program: &Path, args: &[OsString]) -> Result<ProcOutput, CliError> {
        self.process
            .run_with_timeout(program, args, &self.child_env(), self.timeout)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::TimedOut => CliError::TimedOut,
                kind => CliError::Spawn {
                    message: kind.to_string(),
                },
            })
    }
}
