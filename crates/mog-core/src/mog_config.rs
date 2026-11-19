//! MOG configs as local JSON (#25, MB-02, ADR 0035).
//!
//! - [`MogConfig`] is the schema of one MOG config: the `mog.*` fields of Appendix F, with
//!   `deny_unknown_fields` on every level and a `schemaVersion`.
//! - [`MogConfigStore`] keeps one `<id>.json` file per MOG in a directory that the host picks
//!   (the app config dir). Writes are atomic: a temp file in the same directory, then a rename.
//! - `write` rejects a token-like value or a key that names a secret with
//!   [`token_guard::check_config_json`] (#224). This module adds no second detector.
//! - [`MogConfigComponent`] serves `config_list`, `config_read`, `config_write` and
//!   `config_delete` on the R12 gateway seam ([`GatewayComponents`]).
//!
//! Errors are content-free codes: never a path, a key or a value. Nothing here logs. A bad
//! file is reported, never deleted (no automatic deletion of user content).

use crate::error::{CoreError, CoreResult};
use crate::genie_endpoint::{self, EndpointError};
use crate::ipc::{ConfigIdArgs, ConfigWriteArgs, JsonObject};
use crate::ipc_gateway::GatewayComponents;
use crate::token_guard::{self, ConfigSecret};
use crate::types::{CaptureMode, TranscriptProvider};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// The component name in [`CoreError::Unavailable`].
pub const COMPONENT: &str = "MogConfigStore";

/// The only schema version that this build reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

/// The largest config, as compact JSON, that `write` and `read` accept.
pub const MAX_CONFIG_BYTES: usize = 32 * 1024;

/// The most configs that the store keeps.
pub const MAX_CONFIGS: usize = 256;

/// The longest MOG ID. The ID rule is the window-label rule of `src-tauri/src/windows.rs`.
pub const MAX_ID_LEN: usize = 64;

/// `mog.persona`: at most 2,000 characters (Appendix F).
pub const MAX_PERSONA_CHARS: usize = 2_000;

/// Why a config call failed. It carries no key, value or path, so it can cross IPC (ARC-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    /// The ID is empty, too long, or has a character other than ASCII letters, digits and `-`.
    InvalidId,
    /// The config is larger than [`MAX_CONFIG_BYTES`].
    TooLarge,
    /// MB-02: a key names a secret, or a key or value looks like a token.
    Secret(ConfigSecret),
    /// `schemaVersion` is not [`SCHEMA_VERSION`].
    UnsupportedSchemaVersion,
    /// The config does not match the schema or its bounds.
    InvalidConfig,
    /// No config has this ID.
    NotFound,
    /// The store already holds [`MAX_CONFIGS`] configs.
    LimitReached,
    /// The file system call failed.
    StorageFailed,
    /// G-03: the backend names a rejected Genie endpoint.
    RejectedBackend(EndpointError),
}

impl ConfigError {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            ConfigError::InvalidId => "invalid_config_id",
            ConfigError::TooLarge => "config_too_large",
            ConfigError::Secret(s) => s.code(),
            ConfigError::UnsupportedSchemaVersion => "unsupported_schema_version",
            ConfigError::InvalidConfig => "invalid_config",
            ConfigError::NotFound => "config_not_found",
            ConfigError::LimitReached => "config_limit_reached",
            ConfigError::StorageFailed => "config_storage_failed",
            ConfigError::RejectedBackend(e) => e.code(),
        }
    }
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for ConfigError {}

impl From<ConfigError> for CoreError {
    fn from(e: ConfigError) -> Self {
        CoreError::Unavailable {
            component: COMPONENT,
            reason: e.code(),
        }
    }
}

// ---------- schema (Appendix F, `mog.*`) ----------

/// One MOG config. Field names are the Appendix F keys without the `mog.` prefix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MogConfig {
    pub schema_version: u32,
    /// `mog.name`.
    pub name: String,
    #[serde(default)]
    pub ui: UiConfig,
    pub binding: BindingConfig,
    #[serde(default)]
    pub capture: CaptureConfig,
    #[serde(default)]
    pub hotkeys: MogHotkeys,
    #[serde(default)]
    pub genie: GenieConfig,
    #[serde(default)]
    pub agent: AgentConfig,
    #[serde(default)]
    pub pages: PagesConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    /// `mog.persona`, at most [`MAX_PERSONA_CHARS`] characters.
    #[serde(default)]
    pub persona: String,
    #[serde(default)]
    pub stt: SttConfig,
    /// `mog.autoStart`.
    #[serde(default)]
    pub auto_start: bool,
    #[serde(default)]
    pub card: CardConfig,
}

/// `mog.ui.size`: S 88 px, M 112 px, L 144 px.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MogSize {
    S,
    #[default]
    M,
    L,
}

/// `mog.ui.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct UiConfig {
    /// `mog.ui.variant`: `auto` or a colour variant name.
    pub variant: String,
    pub size: MogSize,
    /// `mog.ui.dozeAfterMin`: 0 means never.
    pub doze_after_min: u32,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            variant: "auto".to_owned(),
            size: MogSize::M,
            doze_after_min: 20,
        }
    }
}

/// `genie_agent` mode (4.7 `BackendBinding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenieAgentMode {
    Chat,
    AgentMode,
}

/// `custom_agent` protocol (4.7 `BackendBinding`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CustomAgentProtocol {
    Invocations,
    Responses,
}

/// The 4.7 `BackendKind` enum (G-01, D28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackendKind {
    GenieOneMcp,
    GenieAgent,
    CustomAgent,
}

/// Every backend kind, in the 4.7 order.
pub const BACKEND_KINDS: [BackendKind; 3] = [
    BackendKind::GenieOneMcp,
    BackendKind::GenieAgent,
    BackendKind::CustomAgent,
];

impl BackendKind {
    /// The wire name (`genie_one_mcp`, `genie_agent`, `custom_agent`).
    pub fn as_str(&self) -> &'static str {
        match self {
            BackendKind::GenieOneMcp => "genie_one_mcp",
            BackendKind::GenieAgent => "genie_agent",
            BackendKind::CustomAgent => "custom_agent",
        }
    }

    /// The kind for an exact wire name; any other value is `None` (G-01).
    pub fn parse(s: &str) -> Option<Self> {
        BACKEND_KINDS.into_iter().find(|k| k.as_str() == s)
    }
}

/// `mog.binding.backend`: the 4.7 `BackendBinding` union (G-01 kinds only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum BackendConfig {
    #[serde(rename_all = "camelCase")]
    GenieOneMcp {
        profile: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        warehouse_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    GenieAgent {
        profile: String,
        space_id: String,
        mode: GenieAgentMode,
    },
    #[serde(rename_all = "camelCase")]
    CustomAgent {
        profile: String,
        app_url: String,
        protocol: CustomAgentProtocol,
    },
}

impl BackendConfig {
    /// The G-01 kind of this backend.
    pub fn kind(&self) -> BackendKind {
        match self {
            BackendConfig::GenieOneMcp { .. } => BackendKind::GenieOneMcp,
            BackendConfig::GenieAgent { .. } => BackendKind::GenieAgent,
            BackendConfig::CustomAgent { .. } => BackendKind::CustomAgent,
        }
    }
}

/// `mog.binding.*` (D62).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BindingConfig {
    /// `mog.binding.home`: the home profile name (never a token).
    pub home: String,
    /// `mog.binding.backend`. A missing backend is `genie_one_mcp` on the home profile, so a
    /// saved config always has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<BackendConfig>,
    /// `mog.binding.genieCompanion`.
    #[serde(default = "yes")]
    pub genie_companion: bool,
    /// `mog.binding.profileAliases`: spoken word to profile name.
    #[serde(default)]
    pub profile_aliases: BTreeMap<String, String>,
}

fn yes() -> bool {
    true
}

/// `mog.capture.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct CaptureConfig {
    pub modes: Vec<CaptureMode>,
    pub click_toggles_listening: bool,
    /// 0 turns the VAD end off (CAP-14c).
    pub vad_end_ms: u32,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            modes: vec![CaptureMode::Ptt, CaptureMode::HandsFree],
            click_toggles_listening: true,
            vad_end_ms: 1_500,
        }
    }
}

/// `mog.hotkeys.*` (v1).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct MogHotkeys {
    /// `mog.hotkeys.chord`: a digit 0-9; `None` means the lowest free digit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chord: Option<u8>,
}

/// `mog.genie.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct GenieConfig {
    /// No reader in the MVP: ⌥G sends at once (ADR 0098). Kept for the v1 preview (RTR-04a):
    /// with `deny_unknown_fields`, a removal would reject the configs that are already saved.
    pub auto_send_ptt: bool,
    /// `mog.genie.previewMs`: kept for the v1 preview, as `auto_send_ptt` is.
    pub preview_ms: u32,
    pub share_context: bool,
    pub share_context_cross_workspace: bool,
}

impl Default for GenieConfig {
    fn default() -> Self {
        Self {
            auto_send_ptt: true,
            preview_ms: 1_500,
            share_context: false,
            share_context_cross_workspace: false,
        }
    }
}

/// `mog.agent.harness` (B4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    #[default]
    Claude,
    Codex,
}

/// `mog.agent.*`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct AgentConfig {
    pub harness: Harness,
    /// `None` is the AI Gateway default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// `mog.pages.*`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PagesConfig {
    /// `None` is the admin `PageDomainId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain_id: Option<String>,
}

/// `mog.memory.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct MemoryConfig {
    pub enabled: bool,
    pub lakebase_sync: bool,
    pub transcript_sync: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            lakebase_sync: false,
            transcript_sync: false,
        }
    }
}

/// `mog.stt.*`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct SttConfig {
    /// `None` means "per probe".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<TranscriptProvider>,
}

/// `mog.card.*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct CardConfig {
    pub auto_hide_sec: u32,
    pub max_blurbs: u32,
}

impl Default for CardConfig {
    fn default() -> Self {
        Self {
            auto_hide_sec: 60,
            max_blurbs: 3,
        }
    }
}

/// Windows device names. `con.json` and the like name a device, not a file.
const DEVICE_NAMES: [&str; 22] = [
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

/// True if `id` can name a config file: 1 to [`MAX_ID_LEN`] lower-case ASCII letters, digits and
/// `-`, and not a Windows device name. So an ID never holds a path separator or a dot, two IDs
/// never share a file on a case-insensitive file system, and each ID also names a `mog-<id>`
/// window (the window-label rule is wider: it allows upper case).
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !DEVICE_NAMES.contains(&id)
}

const MAX_NAME_CHARS: usize = 64;
const MAX_SHORT_CHARS: usize = 256;
const MAX_VARIANT_CHARS: usize = 32;
const MAX_URL_CHARS: usize = 2_048;
const MAX_ALIASES: usize = 32;
const MAX_ALIAS_WORD_CHARS: usize = 64;
const MAX_DOZE_MIN: u32 = 24 * 60;
const MAX_VAD_END_MS: u32 = 60_000;
const MAX_PREVIEW_MS: u32 = 60_000;
const MAX_AUTO_HIDE_SEC: u32 = 3_600;
const MAX_BLURBS: u32 = 10;

fn chars_in(s: &str, min: usize, max: usize) -> bool {
    let n = s.chars().count();
    (min..=max).contains(&n)
}

/// A short single-line value: a profile name, an ID or a model name.
fn short(s: &str) -> bool {
    chars_in(s, 1, MAX_SHORT_CHARS) && !s.chars().any(char::is_control)
}

fn opt_short(s: &Option<String>) -> bool {
    s.as_deref().is_none_or(short)
}

fn backend_ok(b: &BackendConfig) -> bool {
    match b {
        BackendConfig::GenieOneMcp {
            profile,
            warehouse_id,
        } => short(profile) && opt_short(warehouse_id),
        BackendConfig::GenieAgent {
            profile, space_id, ..
        } => short(profile) && short(space_id),
        // The host and workspace checks of 4.6 rule 1 run when the app is called (v1).
        BackendConfig::CustomAgent {
            profile, app_url, ..
        } => {
            short(profile)
                && chars_in(app_url, 1, MAX_URL_CHARS)
                && app_url.starts_with("https://")
                && !app_url.chars().any(|c| c.is_control() || c.is_whitespace())
        }
    }
}

impl MogConfig {
    /// The bounds that serde does not check.
    fn in_bounds(&self) -> bool {
        let b = &self.binding;
        let modes = &self.capture.modes;
        let unique_modes = modes
            .iter()
            .enumerate()
            .all(|(i, m)| !modes[..i].contains(m));
        chars_in(&self.name, 1, MAX_NAME_CHARS)
            && !self.name.chars().any(char::is_control)
            && chars_in(&self.ui.variant, 1, MAX_VARIANT_CHARS)
            && !self.ui.variant.chars().any(char::is_control)
            && self.ui.doze_after_min <= MAX_DOZE_MIN
            && short(&b.home)
            && b.backend.as_ref().is_none_or(backend_ok)
            && b.profile_aliases.len() <= MAX_ALIASES
            && b.profile_aliases.iter().all(|(word, profile)| {
                chars_in(word, 1, MAX_ALIAS_WORD_CHARS)
                    && !word.chars().any(char::is_control)
                    && short(profile)
            })
            && unique_modes
            && self.capture.vad_end_ms <= MAX_VAD_END_MS
            && self.hotkeys.chord.is_none_or(|d| d <= 9)
            && self.genie.preview_ms <= MAX_PREVIEW_MS
            && opt_short(&self.agent.model)
            && opt_short(&self.pages.domain_id)
            && self.persona.chars().count() <= MAX_PERSONA_CHARS
            && self.card.auto_hide_sec <= MAX_AUTO_HIDE_SEC
            && (1..=MAX_BLURBS).contains(&self.card.max_blurbs)
    }
}

/// G-03: the first URL field under `mog.binding` (a key that ends in `url`, in any case, such as
/// `appUrl`) that names a rejected Genie backend (the Beta Genie MCP, the per-space Genie MCP or
/// the Genie One API SSE) or cannot be made canonical. Loading it fails with a clear code
/// instead of a generic `invalid_config`. Other strings (`home`, `profile`, `spaceId`) are not
/// URLs, so they are not checked here.
fn rejected_backend_in(v: &Value) -> Option<EndpointError> {
    match v {
        Value::Array(items) => items.iter().find_map(rejected_backend_in),
        Value::Object(m) => m.iter().find_map(|(key, v)| match v {
            Value::String(s) if key.to_ascii_lowercase().ends_with("url") => {
                genie_endpoint::rejected_backend(s)
            }
            _ => rejected_backend_in(v),
        }),
        _ => None,
    }
}

/// Check and normalize a config for `config_write` (MB-02).
///
/// The order is fixed: size, then the MB-02 secret check (so a secret under an unknown key is
/// reported as a secret), then `schemaVersion`, the G-03 backend check, the schema and the bounds. The result has every
/// Appendix F default filled in, and a backend.
pub fn parse_config(json: &JsonObject) -> Result<MogConfig, ConfigError> {
    let size = serde_json::to_vec(json).map_err(|_| ConfigError::InvalidConfig)?;
    if size.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::TooLarge);
    }
    token_guard::check_config_json(json).map_err(ConfigError::Secret)?;
    match json.get("schemaVersion") {
        Some(v) if v.as_u64() == Some(u64::from(SCHEMA_VERSION)) => {}
        Some(Value::Number(_)) => return Err(ConfigError::UnsupportedSchemaVersion),
        _ => return Err(ConfigError::InvalidConfig),
    }
    if let Some(e) = json.get("binding").and_then(rejected_backend_in) {
        return Err(ConfigError::RejectedBackend(e));
    }
    // serde messages quote keys and values, so they are dropped (4.6 rule 7).
    let value = serde_json::to_value(json).map_err(|_| ConfigError::InvalidConfig)?;
    let mut cfg: MogConfig =
        serde_json::from_value(value).map_err(|_| ConfigError::InvalidConfig)?;
    if !cfg.in_bounds() {
        return Err(ConfigError::InvalidConfig);
    }
    if cfg.binding.backend.is_none() {
        cfg.binding.backend = Some(BackendConfig::GenieOneMcp {
            profile: cfg.binding.home.clone(),
            warehouse_id: None,
        });
    }
    Ok(cfg)
}

// ---------- store ----------

/// One entry of `config_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigSummary {
    pub id: String,
    pub name: String,
}

/// The reply of `config_list`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigList {
    /// Readable configs, by ID.
    pub items: Vec<ConfigSummary>,
    /// IDs of files that do not pass the checks. They stay on disk.
    pub unreadable: Vec<String>,
}

const EXT: &str = ".json";

/// The MOG configs in one directory.
#[derive(Debug)]
pub struct MogConfigStore {
    dir: PathBuf,
    /// One write or delete at a time, so the config cap and the rename hold.
    lock: Mutex<()>,
}

fn storage<T>(_: T) -> ConfigError {
    ConfigError::StorageFailed
}

impl MogConfigStore {
    /// A store in `dir`. The directory is made on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            lock: Mutex::new(()),
        }
    }

    /// The directory of the config files.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, id: &str) -> Result<PathBuf, ConfigError> {
        if valid_id(id) {
            Ok(self.dir.join(format!("{id}{EXT}")))
        } else {
            Err(ConfigError::InvalidId)
        }
    }

    /// The IDs of the config files, sorted. A missing directory has none.
    fn ids(&self) -> Result<Vec<String>, ConfigError> {
        let entries = match fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(storage(e)),
        };
        let mut ids = Vec::new();
        for entry in entries {
            let entry = entry.map_err(storage)?;
            let name = entry.file_name();
            let Some(id) = name.to_str().and_then(|n| n.strip_suffix(EXT)) else {
                continue;
            };
            if valid_id(id) && entry.file_type().map_err(storage)?.is_file() {
                ids.push(id.to_owned());
            }
        }
        ids.sort();
        Ok(ids)
    }

    /// Every config, sorted by ID. A file that fails the checks is listed in `unreadable`.
    pub fn list(&self) -> Result<ConfigList, ConfigError> {
        let mut out = ConfigList::default();
        for id in self.ids()? {
            match self.read(&id) {
                Ok(cfg) => out.items.push(ConfigSummary { id, name: cfg.name }),
                Err(ConfigError::StorageFailed) => return Err(ConfigError::StorageFailed),
                // Removed between the directory read and the file read.
                Err(ConfigError::NotFound) => {}
                Err(_) => out.unreadable.push(id),
            }
        }
        Ok(out)
    }

    /// One config. The file gets the same checks as a write; a bad file stays on disk.
    pub fn read(&self, id: &str) -> Result<MogConfig, ConfigError> {
        let path = self.path(id)?;
        let file = match fs::File::open(&path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(ConfigError::NotFound),
            Err(e) => return Err(storage(e)),
        };
        let mut bytes = Vec::new();
        file.take(MAX_CONFIG_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(storage)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let json: JsonObject =
            serde_json::from_slice(&bytes).map_err(|_| ConfigError::InvalidConfig)?;
        parse_config(&json)
    }

    /// Check, normalize and save a config. Returns the saved config. A rejected config
    /// changes nothing on disk.
    pub fn write(&self, id: &str, json: &JsonObject) -> Result<MogConfig, ConfigError> {
        let path = self.path(id)?;
        let cfg = parse_config(json)?;
        // The saved file has the defaults filled in, so it can be larger than the request. It
        // gets the same cap that `read` applies, so every saved config can be read back.
        let bytes = serde_json::to_vec(&cfg).map_err(|_| ConfigError::InvalidConfig)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        if !path.exists() && self.ids()?.len() >= MAX_CONFIGS {
            return Err(ConfigError::LimitReached);
        }
        make_private_dir(&self.dir)?;
        atomic_write(&self.dir, id, &path, &bytes)?;
        Ok(cfg)
    }

    /// Delete a config on a user action (the hub). `false` if there was none.
    pub fn delete(&self, id: &str) -> Result<bool, ConfigError> {
        let path = self.path(id)?;
        let _guard = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        match fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(storage(e)),
        }
    }
}

fn make_private_dir(dir: &Path) -> Result<(), ConfigError> {
    let mut b = fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).map_err(storage)
}

/// Write `bytes` to a new temp file in `dir`, flush it to disk, then rename it over `path`.
/// A reader sees the old file or the new file, never a part of one.
fn atomic_write(dir: &Path, id: &str, path: &Path, bytes: &[u8]) -> Result<(), ConfigError> {
    let mut nonce = [0u8; 8];
    getrandom::fill(&mut nonce).map_err(storage)?;
    let suffix: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
    // A leading dot and the `.tmp-` suffix keep the temp file out of `ids`.
    let tmp = dir.join(format!(".{id}{EXT}.tmp-{suffix}"));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let result = opts
        .open(&tmp)
        .and_then(|mut f| {
            f.write_all(bytes)?;
            f.sync_all()
        })
        .and_then(|()| fs::rename(&tmp, path));
    if let Err(e) = result {
        // Only the temp file that this call made; never a user file.
        let _ = fs::remove_file(&tmp);
        return Err(storage(e));
    }
    // Make the rename durable. Not every platform can open a directory, so this is best effort.
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    Ok(())
}

// ---------- gateway seam ----------

/// Serves the `config_*` commands on the R12 gateway (`ipc_gateway::hardened_gateway_with`).
/// The gateway has checked the window and the argument schema already. Every other command
/// gets `not_implemented`.
#[derive(Debug, Clone)]
pub struct MogConfigComponent {
    store: Arc<MogConfigStore>,
}

impl MogConfigComponent {
    pub fn new(store: Arc<MogConfigStore>) -> Self {
        Self { store }
    }
}

fn args<T: for<'de> Deserialize<'de>>(args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| CoreError::IpcRefused {
        code: "invalid_args",
    })
}

impl GatewayComponents for MogConfigComponent {
    fn call(&self, command: &'static str, a: Value) -> CoreResult<Value> {
        let reply = match command {
            "config_list" => serde_json::to_value(self.store.list()?),
            "config_read" => {
                let a: ConfigIdArgs = args(a)?;
                let cfg = self.store.read(&a.id)?;
                serde_json::to_value(cfg).map(|json| json!({ "id": a.id, "json": json }))
            }
            "config_write" => {
                let a: ConfigWriteArgs = args(a)?;
                let cfg = self.store.write(&a.id, &a.json)?;
                serde_json::to_value(cfg).map(|json| json!({ "id": a.id, "json": json }))
            }
            "config_delete" => {
                let a: ConfigIdArgs = args(a)?;
                Ok(json!({ "deleted": self.store.delete(&a.id)? }))
            }
            _ => {
                return Err(CoreError::IpcRefused {
                    code: "not_implemented",
                })
            }
        };
        reply.map_err(|_| ConfigError::InvalidConfig.into())
    }
}

#[cfg(test)]
mod tests;
