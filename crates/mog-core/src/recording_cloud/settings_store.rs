//! Recording Settings persistence (ADR 0103): a small JSON file in the app config dir. It holds
//! no secret, token or transcript: the knob, the consent acknowledgement, the storage choice,
//! the two model endpoints, the preset prompt and the length preset. AppSettings is
//! still in memory (#77), so this file is the store until it lands.
//!
//! A missing or unreadable file reads as the defaults; a field the file lacks takes its default.
//! A write is validated first, then written to a temp file and renamed (no torn file).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::errors::ErrorKind;
use super::layout::{is_uc_name, is_volume_path, VolumeRoot};

/// The file name in the app config dir.
pub const SETTINGS_FILE: &str = "recording-settings.json";
/// The default notes endpoint (it exists on the DEFAULT workspace).
pub const DEFAULT_ENDPOINT: &str = "databricks-claude-sonnet-5-5";
/// The default transcribe endpoint (audio in, verified live on DEFAULT).
pub const DEFAULT_TRANSCRIBE_ENDPOINT: &str = "databricks-gemini-3-8-flash";
/// The default recording length in minutes.
pub const DEFAULT_LENGTH_MIN: u32 = 60;
/// The shortest custom length.
pub const MIN_LENGTH_MIN: u32 = 5;
/// The longest custom length.
pub const MAX_LENGTH_MIN: u32 = 120;
/// The longest preset prompt (bytes).
pub const MAX_INSTRUCTIONS_BYTES: usize = 16 * 1024;

/// The default preset prompt (the brief's text).
pub const DEFAULT_PRESET: &str = "You get the transcript of a call. Speakers are labeled Speaker 1, Speaker 2, and so on.
Write short notes in Markdown with these sections:
## Summary
Three to five sentences.
## Decisions
A bullet list. Write \"None\" if there are no decisions.
## Action items
One sub-heading per speaker. Under each, a bullet list of that speaker's action items, with an owner and a due date if the call names one.
Use only facts from the transcript.
";

/// Where recordings go.
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

/// The recording Settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    /// The global knob. OFF by default; when OFF no capture path can start.
    pub enabled: bool,
    /// The first-use consent notice was acknowledged.
    pub consent_ack: bool,
    /// Where recordings go.
    pub storage: Storage,
    /// The model endpoint for the transcript ("Advanced").
    pub transcribe_endpoint: String,
    /// The model endpoint for the notes ("Advanced").
    pub notes_endpoint: String,
    /// The preset prompt.
    pub preset_instructions: String,
    /// The length preset in minutes (5..=120).
    pub length_min: u32,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            consent_ack: false,
            storage: Storage::MyFiles,
            transcribe_endpoint: DEFAULT_TRANSCRIBE_ENDPOINT.to_owned(),
            notes_endpoint: DEFAULT_ENDPOINT.to_owned(),
            preset_instructions: DEFAULT_PRESET.to_owned(),
            length_min: DEFAULT_LENGTH_MIN,
        }
    }
}

/// A model endpoint name: `[A-Za-z0-9_-]{1,128}`.
pub fn is_endpoint_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Instructions Bottle accepts: not blank, at most [`MAX_INSTRUCTIONS_BYTES`].
pub fn valid_instructions(s: &str) -> bool {
    !s.trim().is_empty() && s.len() <= MAX_INSTRUCTIONS_BYTES
}

impl RecordingSettings {
    /// Check every field. Any bad value is [`ErrorKind::BadSettings`].
    pub fn validate(&self) -> Result<(), ErrorKind> {
        let storage_ok = match &self.storage {
            Storage::MyFiles => true,
            Storage::Volume { path } => is_volume_path(path) && VolumeRoot::parse(path).is_ok(),
            Storage::CatalogSchema { catalog, schema } => is_uc_name(catalog) && is_uc_name(schema),
        };
        let ok = storage_ok
            && is_endpoint_name(&self.transcribe_endpoint)
            && is_endpoint_name(&self.notes_endpoint)
            && valid_instructions(&self.preset_instructions)
            && (MIN_LENGTH_MIN..=MAX_LENGTH_MIN).contains(&self.length_min);
        ok.then_some(()).ok_or(ErrorKind::BadSettings)
    }
}

/// The Settings file. `None` path (no app config dir): reads give defaults, writes fail.
#[derive(Debug, Clone)]
pub struct SettingsFile {
    path: Option<PathBuf>,
}

impl SettingsFile {
    /// The file at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }
    /// A store with no file (no app config dir).
    pub fn none() -> Self {
        Self { path: None }
    }
    /// The stored Settings, or the defaults when the file is missing, unreadable or not valid.
    pub fn read(&self) -> RecordingSettings {
        let Some(p) = &self.path else {
            return RecordingSettings::default();
        };
        std::fs::read(p)
            .ok()
            .and_then(|b| serde_json::from_slice::<RecordingSettings>(&b).ok())
            .filter(|s| s.validate().is_ok())
            .unwrap_or_default()
    }
    /// Validate and store `s`.
    pub fn write(&self, s: &RecordingSettings) -> Result<(), ErrorKind> {
        s.validate()?;
        let p = self.path.as_ref().ok_or(ErrorKind::BadSettings)?;
        let text = serde_json::to_vec_pretty(s).map_err(|_| ErrorKind::BadSettings)?;
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|_| ErrorKind::BadSettings)?;
        }
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|_| ErrorKind::BadSettings)?;
        std::fs::rename(&tmp, p).map_err(|_| ErrorKind::BadSettings)
    }
}
