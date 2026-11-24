//! Recording Settings persistence (ADR 0103): the contract [`RecordingSettings`] in a small
//! JSON file in the app config dir, behind the contract [`RecordingSettingsStore`]. It holds no
//! secret, token or transcript. AppSettings is still in memory (#77), so this file is the store
//! until it lands.
//!
//! The component is built before the app knows its config dir, so the path is set once later
//! ([`JsonSettingsStore::set_path`]). A missing or unreadable file reads as the defaults (knob
//! off). A write goes to a temp file and is renamed (no torn file).

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, CoreResult};
use crate::recording::RecordingSettingsStore;
use crate::types::{
    EndpointName, RecordingSettings, RecordingSettingsPatch, RECORDING_DEFAULT_ENDPOINT,
    RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT, RECORDING_LEGACY_NOTES_ENDPOINT,
    RECORDING_LEGACY_TRANSCRIBE_ENDPOINT,
};

use super::layout::is_uc_name;

/// The file name in the app config dir.
pub const SETTINGS_FILE: &str = "recording-settings.json";
/// The longest instructions text (bytes) the pipeline sends.
pub const MAX_INSTRUCTIONS_BYTES: usize = 64 * 1024;

/// Where the pipeline looks for a volume root (the probe input).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Storage {
    /// My Files of the user (probed; the default).
    #[default]
    MyFiles,
    /// An existing volume path `/Volumes/c/s/v`.
    Volume {
        /// The volume root path.
        path: String,
    },
    /// A `catalog.schema` where Bottle creates `bottle_audio` after the user confirms.
    CatalogSchema {
        /// The catalog.
        catalog: String,
        /// The schema.
        schema: String,
    },
}

/// A model endpoint name: `[A-Za-z0-9_.-]{1,128}` (same rule as `types::EndpointName`).
pub fn is_endpoint_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
}

/// The model id to call (ADR 0123): the two old default ids (`databricks-*`) map to the
/// `system.ai.*` defaults. Any other value stays as it is.
pub fn current_endpoint(s: &str) -> &str {
    match s {
        RECORDING_LEGACY_TRANSCRIBE_ENDPOINT => RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT,
        RECORDING_LEGACY_NOTES_ENDPOINT => RECORDING_DEFAULT_ENDPOINT,
        other => other,
    }
}

/// Instructions the pipeline accepts: not blank, at most [`MAX_INSTRUCTIONS_BYTES`].
pub fn valid_instructions(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= MAX_INSTRUCTIONS_BYTES
}

/// A catalog and schema pair Bottle accepts for `bottle_audio`.
pub fn valid_catalog_schema(catalog: &str, schema: &str) -> bool {
    is_uc_name(catalog) && is_uc_name(schema)
}

/// Write `bytes` to `path`, readable by the user only (0600 on unix; the ACL of the user's
/// app data folder on Windows).
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    f.write_all(bytes)?;
    f.sync_all()
}

/// The new default as an endpoint, when `saved` is an old default; else `None` (keep it).
fn legacy_to_current(saved: &str) -> Option<EndpointName> {
    let now = current_endpoint(saved);
    (now != saved)
        .then(|| EndpointName::try_from(now.to_owned()).ok())
        .flatten()
}

fn failed() -> CoreError {
    CoreError::IpcRefused {
        code: "host_failed",
    }
}

/// The JSON file store of the recording Settings.
#[derive(Debug, Default)]
pub struct JsonSettingsStore {
    path: OnceLock<PathBuf>,
    /// Writes are serialized, so two saves never interleave.
    lock: Mutex<()>,
}

impl JsonSettingsStore {
    /// A store whose path is set later.
    pub fn deferred() -> Self {
        Self::default()
    }
    /// A store on `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let s = Self::default();
        s.set_path(path);
        s
    }
    /// Set the file path once (later calls are ignored).
    pub fn set_path(&self, path: impl Into<PathBuf>) {
        let _ = self.path.set(path.into());
    }
}

impl RecordingSettingsStore for JsonSettingsStore {
    fn load(&self) -> CoreResult<RecordingSettings> {
        let Some(p) = self.path.get() else {
            return Ok(RecordingSettings::default());
        };
        let mut s = std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice::<RecordingSettings>(&b).ok())
            .unwrap_or_default();
        // ADR 0123: a saved old default id reads as the new default.
        let patch = RecordingSettingsPatch {
            transcribe_endpoint: legacy_to_current(s.transcribe_endpoint.as_str()),
            notes_endpoint: legacy_to_current(s.notes_endpoint.as_str()),
            ..Default::default()
        };
        s = s.apply(patch);
        Ok(s)
    }

    fn save(&self, settings: &RecordingSettings) -> CoreResult<()> {
        let p = self.path.get().ok_or_else(failed)?;
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let text = serde_json::to_vec_pretty(settings).map_err(|_| failed())?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|_| failed())?;
        }
        let tmp = p.with_extension("json.tmp");
        write_private(&tmp, &text).map_err(|_| failed())?;
        std::fs::rename(&tmp, p).map_err(|_| failed())
    }
}
