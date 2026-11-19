//! R5 at app start and the `cli_status` IPC (#16; W-02a, W-02c, W-02d, W-10a; Appendix C).
//!
//! - [`cli_policy`] reads the MDM keys `CliPath` and `CliMinVersion` from R13. A bad or missing
//!   key keeps the default, and no key lowers the 1.17 floor.
//! - [`CliStatusComponent::launch`] runs the launch check once at app start, compares with the
//!   record of the previous launch ([`CliRecordFile`], in the app config dir), stores the new
//!   record and keeps the warnings (W-02d). A missing or refused CLI never fails the start: the
//!   reply then says `found: false`, and the last verified record stays for the next compare.
//! - `cli_status` answers `{path, version, found, plaintextStorage}` (4.5).
//!
//! `plaintextStorage` is not an R5 fact. It comes from the T7 profile check (W-10a,
//! [`PlaintextStorage`]): true when the last `profiles_list` found a `databricks-cli` profile whose
//! `auth describe` shows plaintext token storage (or no mode, an older CLI). It is false before
//! the first profile check and when no T7 is given. A failed `auth describe` blocks the profile
//! but is not reported as plaintext. `cli_status` runs no CLI child of its own.
//!
//! The record holds only the CLI path and version: no secret. TODO(#570): the
//! `cli_status` reply has no field for the launch warnings yet, so they wait in
//! [`CliStatusComponent::warnings`] for the contract change named in the host (`cli_host.rs`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::cli_runner::{
    CliPolicy, CliRecord, CliStatus, CliWarning, DatabricksCliRunner, LaunchReport,
};
use crate::error::{CoreError, CoreResult};
use crate::ipc_gateway::GatewayComponents;
use crate::policy_store::PolicyStore;
use ::cli_runner::{Env, Fs, Process, Version, MIN_VERSION};

/// Appendix C: the absolute CLI path. When set, R5 uses only this path.
pub const CLI_PATH_KEY: &str = "CliPath";
/// Appendix C: the lowest CLI version R5 accepts. It can only raise the 1.17 floor.
pub const CLI_MIN_VERSION_KEY: &str = "CliMinVersion";
/// The file of the last verified CLI in the app config dir.
pub const RECORD_FILE: &str = "cli-record.json";

/// The R5 policy from the MDM keys. A read error, a bad value or an unset key keeps the default.
pub fn cli_policy<P: PolicyStore + ?Sized>(policy: &P) -> CliPolicy {
    let text = |key: &str| {
        policy
            .get(key)
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(str::to_owned))
            .filter(|s| !s.trim().is_empty())
    };
    let min_version = text(CLI_MIN_VERSION_KEY)
        .and_then(|s| Version::parse(s.trim()))
        .map_or(MIN_VERSION, |v| v.max(MIN_VERSION));
    CliPolicy {
        cli_path: text(CLI_PATH_KEY).map(PathBuf::from),
        min_version,
    }
}

/// The record of the last verified CLI: `{"path", "version"}` in one JSON file.
#[derive(Debug, Clone)]
pub struct CliRecordFile {
    path: Option<PathBuf>,
}

impl CliRecordFile {
    /// The file at `path`; `None` (no app config dir) stores nothing.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// [`RECORD_FILE`] in `dir`.
    pub fn in_dir(dir: &Path) -> Self {
        Self::new(Some(dir.join(RECORD_FILE)))
    }

    /// The stored record. A missing, unreadable or bad file is no record.
    pub fn load(&self) -> Option<CliRecord> {
        let text = std::fs::read(self.path.as_ref()?).ok()?;
        let v: Value = serde_json::from_slice(&text).ok()?;
        let path = v.get("path")?.as_str().filter(|p| !p.is_empty())?;
        let version = Version::parse(v.get("version")?.as_str()?)?;
        Some(CliRecord {
            path: PathBuf::from(path),
            version,
        })
    }

    /// Store `record` (write a temporary file, then rename it). A path that is not UTF-8 is not
    /// stored, so it is compared as new at the next launch.
    pub fn save(&self, record: &CliRecord) -> std::io::Result<()> {
        let Some(file) = &self.path else {
            return Ok(());
        };
        let Some(path) = record.path.to_str() else {
            return Ok(());
        };
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = json!({ "path": path, "version": record.version.to_string() });
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, body.to_string())?;
        std::fs::rename(&tmp, file)
    }
}

/// The launch check and status of R5, as the `cli_status` component needs them.
pub trait LaunchCheck: Send + Sync {
    fn launch_check(&self, previous: Option<&CliRecord>) -> CoreResult<LaunchReport>;
    fn status(&self) -> CliStatus;
}

impl<F: Fs + Send + Sync, E: Env + Send + Sync, P: Process + Send + Sync> LaunchCheck
    for DatabricksCliRunner<F, E, P>
{
    fn launch_check(&self, previous: Option<&CliRecord>) -> CoreResult<LaunchReport> {
        DatabricksCliRunner::launch_check(self, previous)
    }
    fn status(&self) -> CliStatus {
        DatabricksCliRunner::status(self)
    }
}

/// W-10a: whether the last profile check found plaintext token storage (T7 answers it).
pub trait PlaintextStorage: Send + Sync {
    fn plaintext_storage(&self) -> bool;
}

/// The `cli_status` component over the one R5 of the process.
pub struct CliStatusComponent {
    cli: Arc<dyn LaunchCheck>,
    record: CliRecordFile,
    plaintext: Option<Arc<dyn PlaintextStorage>>,
    warnings: Mutex<Vec<CliWarning>>,
}

impl std::fmt::Debug for CliStatusComponent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliStatusComponent").finish_non_exhaustive()
    }
}

impl CliStatusComponent {
    pub fn new(
        cli: Arc<dyn LaunchCheck>,
        record: CliRecordFile,
        plaintext: Option<Arc<dyn PlaintextStorage>>,
    ) -> Self {
        Self {
            cli,
            record,
            plaintext,
            warnings: Mutex::new(Vec::new()),
        }
    }

    /// The launch check (W-02d). Returns the warnings and keeps them. Never fails: a missing or
    /// refused CLI gives no warning and `found: false`.
    pub fn launch(&self) -> Vec<CliWarning> {
        let previous = self.record.load();
        let warnings = match self.cli.launch_check(previous.as_ref()) {
            Ok(report) => {
                // A failed write only means the next launch compares with the older record.
                let _ = self.record.save(&report.record);
                report.warnings
            }
            Err(_) => Vec::new(),
        };
        *self.warnings.lock().unwrap_or_else(|p| p.into_inner()) = warnings.clone();
        warnings
    }

    /// The warnings of the last [`CliStatusComponent::launch`].
    pub fn warnings(&self) -> Vec<CliWarning> {
        self.warnings
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    /// The `cli_status` reply. An unverified CLI has empty `path` and `version`.
    pub fn reply(&self) -> Value {
        let s = self.cli.status();
        json!({
            "path": s.path.as_deref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            "version": s.version.map(|v| v.to_string()).unwrap_or_default(),
            "found": s.found,
            "plaintextStorage": self.plaintext.as_ref().is_some_and(|p| p.plaintext_storage()),
        })
    }
}

impl GatewayComponents for CliStatusComponent {
    fn call(&self, command: &'static str, _args: Value) -> CoreResult<Value> {
        if command != "cli_status" {
            return Err(CoreError::IpcRefused {
                code: "not_implemented",
            });
        }
        Ok(self.reply())
    }
}
