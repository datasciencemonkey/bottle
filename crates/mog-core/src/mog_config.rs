//! MOG configs as local JSON (#25, MB-02, ADR 0033).
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
use crate::ipc::JsonObject;
use crate::ipc_gateway::GatewayComponents;
use crate::token_guard::{self, ConfigSecret};
use crate::types::{CaptureMode, TranscriptProvider};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

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

/// `mog.binding.backend`: the 4.7 `BackendBinding` union (G-01 kinds only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
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
    pub auto_send_ptt: bool,
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

/// True if `id` can name a config file (and a `mog-<id>` window label).
pub fn valid_id(id: &str) -> bool {
    let _ = id;
    todo!("#25")
}

/// Check and normalize a config for `config_write` (MB-02).
pub fn parse_config(json: &JsonObject) -> Result<MogConfig, ConfigError> {
    let _ = (json, token_guard::check_config_json);
    todo!("#25")
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

/// The MOG configs in one directory.
#[derive(Debug)]
pub struct MogConfigStore {
    dir: PathBuf,
}

impl MogConfigStore {
    /// A store in `dir`. The directory is made on the first write.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// Every config, sorted by ID.
    pub fn list(&self) -> Result<ConfigList, ConfigError> {
        let _ = &self.dir;
        todo!("#25")
    }

    /// One config.
    pub fn read(&self, id: &str) -> Result<MogConfig, ConfigError> {
        let _ = id;
        todo!("#25")
    }

    /// Check, normalize and save a config. Returns the saved config.
    pub fn write(&self, id: &str, json: &JsonObject) -> Result<MogConfig, ConfigError> {
        let _ = (id, json);
        todo!("#25")
    }

    /// Delete a config on a user action. `false` if there was none.
    pub fn delete(&self, id: &str) -> Result<bool, ConfigError> {
        let _ = id;
        todo!("#25")
    }
}

// ---------- gateway seam ----------

/// Serves the `config_*` commands on the R12 gateway (`ipc_gateway::hardened_gateway_with`).
/// Every other command gets `not_implemented`.
#[derive(Debug, Clone)]
pub struct MogConfigComponent {
    store: Arc<MogConfigStore>,
}

impl MogConfigComponent {
    pub fn new(store: Arc<MogConfigStore>) -> Self {
        Self { store }
    }
}

impl GatewayComponents for MogConfigComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        let _ = (command, args, &self.store);
        todo!("#25")
    }
}

#[cfg(test)]
mod tests;
