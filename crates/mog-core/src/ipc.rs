//! The IPC command and event surface of requirements 4.5 (contract, #223).
//!
//! One table (`COMMANDS`) lists every command with the window classes that may call it.
//! `handle` is the shared entry for the Tauri host and the C ABI: it checks the window
//! label (ARC-4), parses the arguments with `deny_unknown_fields`, and then reaches the
//! component stub. Arguments never carry a token, a header or a request URL (ARC-6, 4.6).
//!
//! `COMMANDS` is 4.5 plus `brain_subscribe` (ADR 0003) and the hub notice pair (ADR 0091). Only the foundation lead changes this
//! file (brief, "Contracts").

use schemars::{JsonSchema, Schema};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub mod events;

// ---------- window classes ----------

/// The window classes of ARC-3, as the IPC table names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowClass {
    Brain,
    Hub,
    Mog,
    Card,
}

impl WindowClass {
    /// The class of a window label: `brain`, `hub`, `mog-<id>` or `card-<id>`.
    pub fn of_label(label: &str) -> Option<(WindowClass, Option<&str>)> {
        let id_ok = |id: &str| {
            !id.is_empty()
                && id.len() <= 64
                && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        };
        match label {
            "brain" => Some((WindowClass::Brain, None)),
            "hub" => Some((WindowClass::Hub, None)),
            _ => {
                if let Some(id) = label.strip_prefix("mog-") {
                    id_ok(id).then_some((WindowClass::Mog, Some(id)))
                } else if let Some(id) = label.strip_prefix("card-") {
                    id_ok(id).then_some((WindowClass::Card, Some(id)))
                } else {
                    None
                }
            }
        }
    }
}

/// The phase of a command (requirements phases).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Phase {
    #[serde(rename = "MVP-D")]
    MvpD,
    #[serde(rename = "v1")]
    V1,
}

/// One row of the 4.5 table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CommandSpec {
    pub name: &'static str,
    pub windows: &'static [WindowClass],
    pub phase: Phase,
}

const B: &[WindowClass] = &[WindowClass::Brain];
const H: &[WindowClass] = &[WindowClass::Hub];
const BH: &[WindowClass] = &[WindowClass::Brain, WindowClass::Hub];
const VIEWS: &[WindowClass] = &[WindowClass::Hub, WindowClass::Mog, WindowClass::Card];

const fn row(name: &'static str, windows: &'static [WindowClass]) -> CommandSpec {
    CommandSpec {
        name,
        windows,
        phase: Phase::MvpD,
    }
}

const fn v1(name: &'static str, windows: &'static [WindowClass]) -> CommandSpec {
    CommandSpec {
        name,
        windows,
        phase: Phase::V1,
    }
}

/// Every command of 4.5, in table order. Slash pairs are separate rows. `capture_resume` and
/// `permission_request` are new rows for the requirements owner (ADR 0039).
/// `capture_stop_all` is also reachable from the tray inside Rust; that path is not IPC.
pub const COMMANDS: &[CommandSpec] = &[
    row("cli_status", BH),
    row("cli_install", BH),
    row("profiles_list", BH),
    row("profile_status", BH),
    row("profile_login", BH),
    // Not in 4.5 (#162): Cancel and "Open the browser again" stop the `auth login` child (W-07h).
    row("profile_login_cancel", BH),
    row("profile_select", BH),
    row("capture_start", B),
    row("capture_stop", B),
    row("capture_pause", B),
    // #279 (ADR 0039): its own row, so a lost reply cannot flip the pause state.
    row("capture_resume", B),
    row("capture_status", B),
    row("capture_stop_all", B),
    row("capture_subscribe", B),
    row("session_attest", B),
    // #430 (ADR 0060): T12 in the brain writes and reads the consent records (SEC-32a).
    row("consent_notice_record", B),
    row("consent_records_list", B),
    row("stt_transcribe", B),
    row("insert_text", B),
    row("agent_start", B),
    row("agent_stop", B),
    row("agent_send", B),
    row("agent_tool_result", B),
    row("model_invoke", B),
    row("mcp_call", B),
    row("dbx_http", B),
    row("mem_local_exec", B),
    v1("mem_remote_exec", B),
    v1("mem_remote_status", BH),
    row("config_list", BH),
    row("config_read", BH),
    row("config_write", BH),
    row("config_delete", BH),
    row("policy_get", BH),
    row("hotkeys_set", BH),
    // #312 (ADR 0039): ask for an OS permission after a click. ADR 0101: the brain relays the
    // click of the hub or the pop-up (views call only `view_*`, MOG-08a).
    row("permission_request", BH),
    // #552 (ADR 0101): the state of an OS permission, with no prompt.
    row("permission_status", BH),
    // #453 (CAP-22): the app-wide settings (Appendix F, scope App), like the `config_*` rows.
    row("app_setting_read", BH),
    row("app_setting_write", BH),
    row("mog_set_active", B),
    row("window_mog_open", B),
    row("window_mog_close", B),
    row("window_card_show", B),
    row("window_card_hide", B),
    row("view_emit", VIEWS),
    row("audit_append", B),
    row("telemetry_span", B),
    // Not in 4.5 (ADR 0003): the brain's private Channel for core-to-brain traffic.
    row("brain_subscribe", B),
    // #509 (ADR 0091): an unshown dictation goes to the hub only, on the hub's private Channel.
    row("hub_notice_send", B),
    row("view_notice_subscribe", H),
    // #413, #45, #529 (ADR 0104): the hub's Local speech step. Hub-only, like
    // `view_notice_subscribe`; the Mog pop-up gets a plain text notice instead (no IPC of its
    // own), so these commands never need to resolve a Channel on a Mog webview.
    row("stt_model_status", H),
    row("stt_model_consent", H),
    row("stt_model_install", H),
    row("stt_model_verify", H),
    row("stt_model_cancel", H),
];

/// The row of a command, if it exists.
pub fn command(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|c| c.name == name)
}

// ---------- errors and results ----------

/// Why the gateway refused a call. No variant carries argument values, the received command
/// name or the window label (secret scrubbing, R12, SEC-03). `command` is always a name from
/// [`COMMANDS`]. Only the serde tag crosses IPC (`{ "error": "<code>" }`, ADR 0040).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum IpcError {
    /// The name is not in [`COMMANDS`]. The name is not kept: it is caller input (#333).
    UnknownCommand,
    /// The window class may not call this command. The window label is not kept (#333).
    WindowNotAllowed {
        command: String,
    },
    InvalidArgs {
        command: String,
        reason: String,
    },
    NotImplemented {
        command: String,
    },
    /// The host could not deliver the result (for example no brain channel yet).
    HostFailed {
        command: String,
    },
    /// CAP-02a: no live lease with this ID belongs to `mogId`. R2 `CaptureError::UnknownLease`
    /// maps here; the host that wires R2 (#4) also returns it when the lease owner is another
    /// MOG, so a caller cannot probe the leases of other MOGs (ADR 0007).
    UnknownLease {
        command: String,
    },
    /// 4.5 `capture_start`: another capture holds the mic (R2 `MicBusy`, CAP-01a).
    MicBusy {
        command: String,
    },
    /// 4.5 `capture_start`: a session needs a valid consent token first.
    ConsentRequired {
        command: String,
    },
    /// The command does not apply to the mode of this lease (R2 `InvalidMode`).
    InvalidMode {
        command: String,
    },
    /// R11 `window_mog_open`: the cap of active MOG windows is reached (ARC-3, MB-03b).
    MogCapReached {
        command: String,
    },
    /// R11: no MOG window (or no card) with this ID is open.
    UnknownMog {
        command: String,
    },
    /// R11: the old window with this label is still closing. Retry after it is gone (ADR 0022).
    WindowClosing {
        command: String,
    },
    /// `config_*` (#379): the config store is missing or could not read or write.
    ConfigStorageFailed {
        command: String,
    },
    /// `consent_*` (#430, ADR 0060): no consent records are managed, or the store failed.
    ConsentStorageFailed {
        command: String,
    },
    /// `config_*` (#379): the config ID is not valid. The `mog_config` code `invalid_config_id`.
    InvalidConfigId {
        command: String,
    },
    /// `config_*` (#379): the config is too large. The `mog_config` code `config_too_large`.
    ConfigTooLarge {
        command: String,
    },
    /// `config_*` (#379): the config schema version is not supported. The `mog_config` code `unsupported_schema_version`.
    UnsupportedSchemaVersion {
        command: String,
    },
    /// `config_*` (#379): the config does not match the schema. The `mog_config` code `invalid_config`.
    InvalidConfig {
        command: String,
    },
    /// `config_*` (#379): no config has this ID. The `mog_config` code `config_not_found`.
    ConfigNotFound {
        command: String,
    },
    /// `config_*` (#379): the config store is full. The `mog_config` code `config_limit_reached`.
    ConfigLimitReached {
        command: String,
    },
    /// `config_*` (#379): a config key names a secret (MB-02). The `mog_config` code `secret_named_key`.
    SecretNamedKey {
        command: String,
    },
    /// `config_*` (#379): a config key or value looks like a token (MB-02). The `mog_config` code `token_like_value`.
    TokenLikeValue {
        command: String,
    },
    /// `config_*` (#379): a config URL names the beta Genie MCP (G-03). The `mog_config` code `genie_beta_mcp_rejected`.
    GenieBetaMcpRejected {
        command: String,
    },
    /// `config_*` (#379): a config URL names a per-space Genie MCP (G-03). The `mog_config` code `genie_space_mcp_rejected`.
    GenieSpaceMcpRejected {
        command: String,
    },
    /// `config_*` (#379): a config URL names the Genie One API SSE (G-03). The `mog_config` code `genie_one_sse_rejected`.
    GenieOneSseRejected {
        command: String,
    },
    /// `config_*` (#379): a config URL path cannot be made canonical (G-03). The `mog_config` code `mcp_url_not_canonical`.
    McpUrlNotCanonical {
        command: String,
    },
    /// `mcp_call` (#479): R8 `genie_one_off`: Genie One is not on in the workspace: a 404 without a session (G-05b).
    GenieOneOff {
        command: String,
    },
    /// `mcp_call` (#479): R8 `auth_expired`: a 401 again after one token drop and resend (W-05d).
    AuthExpired {
        command: String,
    },
    /// `mcp_call` (#479): R8 `rate_limited`: the upstream answered 429.
    RateLimited {
        command: String,
    },
    /// `mcp_call` (#479): R8 `non_json_reply`: the upstream reply was not JSON (SEC-05a).
    NonJsonReply {
        command: String,
    },
    /// `mcp_call` (#479): R8 `timeout`: the call timed out.
    Timeout {
        command: String,
    },
    /// `mcp_call` (#479): R8 `offline`: the host did not accept a connection.
    Offline {
        command: String,
    },
    /// `mcp_call` (#479): R8 `host_refused`: the egress policy refused to send the token to the host (4.6 rule 2).
    HostRefused {
        command: String,
    },
    /// `mcp_call` (#479): R8 `redirect_refused`: the upstream answered with a redirect (4.6 rule 3).
    RedirectRefused {
        command: String,
    },
    /// `mcp_call` (#479): R8 `reply_too_large`: the reply is larger than the cap.
    ReplyTooLarge {
        command: String,
    },
    /// `mcp_call` (#479): R8 `bad_reply`: the reply holds no JSON-RPC response for the request.
    BadReply {
        command: String,
    },
    /// `mcp_call` (#479): R8 `protocol_unsupported`: the server named another MCP protocol version (G-04c).
    ProtocolUnsupported {
        command: String,
    },
    /// `mcp_call` (#479): R8 `tool_not_allowed`: the tool is not on the allowlist (G-06a).
    ToolNotAllowed {
        command: String,
    },
    /// `mcp_call` (#479): R8 `invalid_tool_args`: the tool arguments do not match the tool contract.
    InvalidToolArgs {
        command: String,
    },
    /// `mcp_call` (#479): R8 `invalid_meta`: the meta is not a warehouse ID (G-06b, G-07a).
    InvalidMeta {
        command: String,
    },
    /// `mcp_call` (#479): R8 `profile_invalid`: the profile name is empty.
    ProfileInvalid {
        command: String,
    },
    /// `mcp_call` (#479): R8 `poll_inflight`: a poll is already open for this conversation (G-07b, ARC-F1).
    PollInflight {
        command: String,
    },
    /// `agent_*` (#307): R16 `agent_unknown_mog`: the MOG has no agent session.
    AgentUnknownMog {
        command: String,
    },
    /// `agent_*` (#307): R16 `agent_unavailable`: the session failed after its one restart, or
    /// the runtime is gone (ARC-F14). The brain routes with the fallback classifier (RTR-15b).
    AgentUnavailable {
        command: String,
    },
    /// `agent_start` (#307): R16 `agent_launch_failed`: the runtime process did not start.
    AgentLaunchFailed {
        command: String,
    },
    /// `agent_start` (#307): R16 `agent_target_unavailable`: the MOG has no model target yet.
    AgentTargetUnavailable {
        command: String,
    },
    /// `agent_start` (#307): R16 `agent_invalid_target`: the model target is not allowed.
    AgentInvalidTarget {
        command: String,
    },
    /// `agent_start` (#307): R16 `agent_harness_mismatch`: the MOG has a live session of the
    /// other harness kind. Stop it first.
    AgentHarnessMismatch {
        command: String,
    },
    /// `profile_login` (#162, W-07g): a profile with this name exists, or its sign-in runs.
    ProfileExists {
        command: String,
    },
    /// `profile_login` (#162, W-07h): the user cancelled the sign-in.
    LoginCancelled {
        command: String,
    },
    /// `profile_login` (#162, W-07h): `databricks auth login` failed or could not start.
    CliFailed {
        command: String,
    },
    /// `profile_login` (#162, W-07h, Table W-ERR "403 grant"): the account has no grant.
    NoGrant {
        command: String,
    },
    /// `stt_transcribe` (#413, #45, ADR 0104): R4 has no installed, checked model.
    SttModelMissing {
        command: String,
    },
    /// `stt_model_install` (#413, ADR 0104): the user has not agreed to the download (CAP-49a).
    SttConsentRequired {
        command: String,
    },
    /// `stt_model_install` (#413, ADR 0104): a URL is not under a pinned source (CAP-49b).
    SttUrlNotAllowed {
        command: String,
    },
    /// `stt_model_install` (#413, ADR 0104): the download broke; the `.partial` file stays.
    SttDownloadFailed {
        command: String,
    },
    /// `stt_model_install` or `stt_model_verify` (#413, ADR 0104): a file does not match its
    /// pinned size or SHA-256 (CAP-49c). The file is deleted.
    SttHashMismatch {
        command: String,
    },
    /// `stt_model_install` (#413, ADR 0104): the release archive did not extract.
    SttExtractFailed {
        command: String,
    },
    /// `stt_model_*` (#413, ADR 0104): a local file operation failed.
    SttIoFailed {
        command: String,
    },
    /// `stt_model_install` (#413, ADR 0104): the user cancelled the download.
    SttDownloadCancelled {
        command: String,
    },
    /// `stt_model_install` or `stt_model_verify` (#413, ADR 0104): an install or a verify
    /// already runs.
    SttInstallBusy {
        command: String,
    },
    /// `stt_model_*` (#413, ADR 0104): the OS gives no user folder for the models (W-17e).
    SttModelsDirUnavailable {
        command: String,
    },
    /// `stt_model_install` (#413, #45, ADR 0104): this build has no `local-stt` feature, so it
    /// has no HTTP client for the model download (#534, #414).
    SttSetupUnavailable {
        command: String,
    },
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::UnknownCommand => write!(f, "unknown command"),
            IpcError::WindowNotAllowed { command } => {
                write!(f, "this window may not call {command}")
            }
            IpcError::InvalidArgs { command, reason } => {
                write!(f, "invalid arguments for {command}: {reason}")
            }
            IpcError::NotImplemented { command } => write!(f, "{command} is not implemented"),
            IpcError::HostFailed { command } => write!(f, "the host could not complete {command}"),
            IpcError::UnknownLease { command } => {
                write!(f, "{command}: no live capture lease with this ID")
            }
            IpcError::MicBusy { command } => write!(f, "{command}: the mic is busy"),
            IpcError::ConsentRequired { command } => write!(f, "{command}: consent is required"),
            IpcError::InvalidMode { command } => {
                write!(f, "{command} does not apply to this capture mode")
            }
            IpcError::MogCapReached { command } => {
                write!(f, "{command}: the cap of MOG windows is reached")
            }
            IpcError::UnknownMog { command } => write!(f, "{command}: no such MOG window"),
            IpcError::WindowClosing { command } => {
                write!(f, "{command}: the old window is still closing")
            }
            IpcError::ConfigStorageFailed { command } => {
                write!(f, "{command}: the config store failed")
            }
            IpcError::ConsentStorageFailed { command } => {
                write!(f, "{command}: the consent record store failed")
            }
            IpcError::InvalidConfigId { command } => {
                write!(f, "{command}: the config ID is not valid")
            }
            IpcError::ConfigTooLarge { command } => write!(f, "{command}: the config is too large"),
            IpcError::UnsupportedSchemaVersion { command } => {
                write!(f, "{command}: the config schema version is not supported")
            }
            IpcError::InvalidConfig { command } => {
                write!(f, "{command}: the config does not match the schema")
            }
            IpcError::ConfigNotFound { command } => write!(f, "{command}: no config has this ID"),
            IpcError::ConfigLimitReached { command } => {
                write!(f, "{command}: the config store is full")
            }
            IpcError::SecretNamedKey { command } => {
                write!(f, "{command}: a config key names a secret (MB-02)")
            }
            IpcError::TokenLikeValue { command } => write!(
                f,
                "{command}: a config key or value looks like a token (MB-02)"
            ),
            IpcError::GenieBetaMcpRejected { command } => {
                write!(f, "{command}: a config URL names the beta Genie MCP (G-03)")
            }
            IpcError::GenieSpaceMcpRejected { command } => write!(
                f,
                "{command}: a config URL names a per-space Genie MCP (G-03)"
            ),
            IpcError::GenieOneSseRejected { command } => write!(
                f,
                "{command}: a config URL names the Genie One API SSE (G-03)"
            ),
            IpcError::McpUrlNotCanonical { command } => write!(
                f,
                "{command}: a config URL path cannot be made canonical (G-03)"
            ),
            IpcError::GenieOneOff { command } => {
                write!(f, "{command}: Genie One is not on in the workspace: a 404 without a session (G-05b)")
            }
            IpcError::AuthExpired { command } => {
                write!(
                    f,
                    "{command}: a 401 again after one token drop and resend (W-05d)"
                )
            }
            IpcError::RateLimited { command } => {
                write!(f, "{command}: the upstream answered 429")
            }
            IpcError::NonJsonReply { command } => {
                write!(f, "{command}: the upstream reply was not JSON (SEC-05a)")
            }
            IpcError::Timeout { command } => {
                write!(f, "{command}: the call timed out")
            }
            IpcError::AgentUnknownMog { command } => {
                write!(f, "{command}: the MOG has no agent session")
            }
            IpcError::AgentUnavailable { command } => {
                write!(f, "{command}: the agent session is not available")
            }
            IpcError::AgentLaunchFailed { command } => {
                write!(f, "{command}: the agent runtime did not start")
            }
            IpcError::AgentTargetUnavailable { command } => {
                write!(f, "{command}: the MOG has no model target")
            }
            IpcError::AgentInvalidTarget { command } => {
                write!(f, "{command}: the model target is not allowed")
            }
            IpcError::AgentHarnessMismatch { command } => {
                write!(
                    f,
                    "{command}: the MOG has a session of the other harness kind"
                )
            }
            IpcError::ProfileExists { command } => write!(f, "{command}: the profile exists"),
            IpcError::LoginCancelled { command } => {
                write!(f, "{command}: the sign-in was cancelled")
            }
            IpcError::CliFailed { command } => write!(f, "{command}: the CLI sign-in failed"),
            IpcError::NoGrant { command } => write!(f, "{command}: the account has no access"),
            IpcError::SttModelMissing { command } => {
                write!(f, "{command}: R4 has no installed, checked model")
            }
            IpcError::SttConsentRequired { command } => {
                write!(f, "{command}: the user has not agreed to the download")
            }
            IpcError::SttUrlNotAllowed { command } => {
                write!(f, "{command}: a URL is not under a pinned source")
            }
            IpcError::SttDownloadFailed { command } => write!(f, "{command}: the download broke"),
            IpcError::SttHashMismatch { command } => {
                write!(f, "{command}: a file does not match its pin")
            }
            IpcError::SttExtractFailed { command } => {
                write!(f, "{command}: the release archive did not extract")
            }
            IpcError::SttIoFailed { command } => {
                write!(f, "{command}: a local file operation failed")
            }
            IpcError::SttDownloadCancelled { command } => {
                write!(f, "{command}: the user cancelled the download")
            }
            IpcError::SttInstallBusy { command } => {
                write!(f, "{command}: an install or a verify already runs")
            }
            IpcError::SttModelsDirUnavailable { command } => {
                write!(f, "{command}: the OS gives no user folder for the models")
            }
            IpcError::SttSetupUnavailable { command } => {
                write!(f, "{command}: this build has no local-stt feature")
            }
            IpcError::Offline { command } => {
                write!(f, "{command}: the host did not accept a connection")
            }
            IpcError::HostRefused { command } => {
                write!(f, "{command}: the egress policy refused to send the token to the host (4.6 rule 2)")
            }
            IpcError::RedirectRefused { command } => {
                write!(
                    f,
                    "{command}: the upstream answered with a redirect (4.6 rule 3)"
                )
            }
            IpcError::ReplyTooLarge { command } => {
                write!(f, "{command}: the reply is larger than the cap")
            }
            IpcError::BadReply { command } => {
                write!(
                    f,
                    "{command}: the reply holds no JSON-RPC response for the request"
                )
            }
            IpcError::ProtocolUnsupported { command } => {
                write!(
                    f,
                    "{command}: the server named another MCP protocol version (G-04c)"
                )
            }
            IpcError::ToolNotAllowed { command } => {
                write!(f, "{command}: the tool is not on the allowlist (G-06a)")
            }
            IpcError::InvalidToolArgs { command } => {
                write!(
                    f,
                    "{command}: the tool arguments do not match the tool contract"
                )
            }
            IpcError::InvalidMeta { command } => {
                write!(
                    f,
                    "{command}: the meta is not a warehouse ID (G-06b, G-07a)"
                )
            }
            IpcError::ProfileInvalid { command } => {
                write!(f, "{command}: the profile name is empty")
            }
            IpcError::PollInflight { command } => {
                write!(
                    f,
                    "{command}: a poll is already open for this conversation (G-07b, ARC-F1)"
                )
            }
        }
    }
}

impl std::error::Error for IpcError {}

/// What the host must do after the gateway accepted a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handled {
    /// `view_emit`: relay the arguments to the brain as `view:input` (never to a view).
    RelayToBrain { from: String },
    /// `brain_subscribe`: keep this Channel as the brain's private delivery path.
    SubscribeBrain { channel: ChannelRef },
    /// The call passed the window and argument checks, and the host sends it to its feature
    /// component (`mcp_call` to R8 `McpCallComponent`, #467; `agent_start`, `agent_stop` and
    /// `agent_send` to the R16 `SharedAgentHost`, #307).
    Component,
}

// ---------- shared argument types ----------

pub use crate::types::{CaptureMode, MogId, ProfileName, SessionPolicy, Tags, TranscriptProvider};

/// A lease ID argument (CAP-02a): 1-64 characters of `[A-Za-z0-9-]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LeaseRef(String);

/// The wire rule of a lease ID, also stated in the JSON Schema.
pub const LEASE_PATTERN: &str = "^[A-Za-z0-9-]{1,64}$";

impl JsonSchema for LeaseRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LeaseRef".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "string", "pattern": LEASE_PATTERN })
    }
}

impl LeaseRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for LeaseRef {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        if ok {
            Ok(LeaseRef(s))
        } else {
            Err("not a lease ID")
        }
    }
}

impl From<LeaseRef> for String {
    fn from(l: LeaseRef) -> String {
        l.0
    }
}

/// The wire prefix of a Tauri `Channel` argument (`__CHANNEL__:<id>`).
pub const CHANNEL_PREFIX: &str = "__CHANNEL__:";

/// A Tauri `Channel` argument. On the wire it is `__CHANNEL__:<numeric id>`; other strings fail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ChannelRef(String);

impl JsonSchema for ChannelRef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "ChannelRef".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> Schema {
        schemars::json_schema!({ "type": "string", "pattern": "^__CHANNEL__:[0-9]+$" })
    }
}

impl ChannelRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ChannelRef {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.strip_prefix(CHANNEL_PREFIX) {
            Some(id) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
                Ok(ChannelRef(s))
            }
            _ => Err("not a channel reference"),
        }
    }
}

impl From<ChannelRef> for String {
    fn from(c: ChannelRef) -> String {
        c.0
    }
}

/// A JSON object whose shape another contract owns (for example `AgentTurn` in 4.7).
pub type JsonObject = BTreeMap<String, Value>;

macro_rules! args {
    ($(#[$m:meta])* $name:ident { $($(#[$fm:meta])* $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
        #[serde(deny_unknown_fields, rename_all = "camelCase")]
        pub struct $name { $($(#[$fm])* pub $field: $ty),* }
    };
}

args!(
    /// No arguments.
    NoArgs {}
);
args!(CliInstallArgs {
    /// Must be `true`: the user gave consent (U16).
    consent: bool,
});
args!(ProfilesListArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh: Option<bool>,
});
args!(ProfileArgs {
    profile: ProfileName
});
args!(ProfileLoginArgs {
    /// The workspace that `databricks auth login --host` opens. Not a request target.
    workspace_url: String,
    profile: ProfileName,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SelectScope {
    Question,
    Session,
}
args!(ProfileSelectArgs {
    mog_id: MogId,
    profile: ProfileName,
    scope: SelectScope,
});

args!(CaptureStartArgs {
    mog_id: MogId,
    mode: CaptureMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    policy: Option<SessionPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    consent_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    others_track: Option<bool>,
});
args!(MogArgs { mog_id: MogId });
args!(
    /// CAP-02a: stop, pause and resume name the lease they act on. R2 rejects a stale one.
    CaptureLeaseArgs {
        mog_id: MogId,
        lease_id: LeaseRef,
    }
);
args!(CaptureSubscribeArgs {
    on_level: ChannelRef,
    on_partial: ChannelRef,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AttestKind {
    OnlyMe,
    OthersAgreed,
}
args!(SessionAttestArgs {
    mog_id: MogId,
    kind: AttestKind,
    notice_version: u32,
});

args!(
    /// SEC-32a: the brain names only the notice version. The Rust side fills the user, the
    /// device and the time (ADR 0060).
    ConsentNoticeRecordArgs {
        notice_version: u32
    }
);

args!(SttTranscribeArgs {
    segment_id: String,
    provider: TranscriptProvider,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<ProfileName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    tags: Tags,
});
args!(InsertTextArgs {
    mog_id: MogId,
    text: String,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
}
args!(AgentStartArgs {
    mog_id: MogId,
    harness: Harness,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
});
// `agent_send` (#307). The turn holds the user's question, so `Debug` shows only its size
// (SEC-16c). A plain comment, so the schema does not change. TODO(#505): the general fix.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct AgentSendArgs {
    pub mog_id: MogId,
    /// `AgentTurn` of 4.7 (owned by #222 and the agent issues).
    pub turn: JsonObject,
}

impl std::fmt::Debug for AgentSendArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSendArgs")
            .field("mog_id", &self.mog_id)
            .field("turn", &format_args!("<redacted {} keys>", self.turn.len()))
            .finish()
    }
}
args!(AgentToolResultArgs {
    mog_id: MogId,
    call_id: String,
    result: Value,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ModelRole {
    Cleanup,
    Router,
    Writer,
    Judge,
    Embed,
    Merge,
}
args!(ModelInvokeArgs {
    role: ModelRole,
    profile: ProfileName,
    request: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deadline_ms: Option<u32>,
    tags: Tags,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum McpServer {
    GenieOne,
}
args!(McpCallArgs {
    profile: ProfileName,
    server: McpServer,
    tool: String,
    /// Tool arguments. R8 strips any `profile` key before it forwards them (4.6 rule 5).
    args: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    meta: Option<JsonObject>,
    op_id: String,
    tags: Tags,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_ms: Option<u32>,
});

/// The fixed REST route IDs of 4.6. `dbx_http` takes only these, never a URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RouteId {
    #[serde(rename = "scim.me")]
    ScimMe,
    #[serde(rename = "genie.start_conversation")]
    GenieStartConversation,
    #[serde(rename = "genie.message")]
    GenieMessage,
    #[serde(rename = "genie.get_message")]
    GenieGetMessage,
    #[serde(rename = "genie.query_result")]
    GenieQueryResult,
    #[serde(rename = "genie.feedback")]
    GenieFeedback,
    #[serde(rename = "genie.download_viz")]
    GenieDownloadViz,
    #[serde(rename = "genie.delete_conversation")]
    GenieDeleteConversation,
    #[serde(rename = "genie_one.list_conversations")]
    GenieOneListConversations,
    #[serde(rename = "genie_one.delete_conversation")]
    GenieOneDeleteConversation,
    #[serde(rename = "apps.invocations")]
    AppsInvocations,
    #[serde(rename = "apps.events")]
    AppsEvents,
    #[serde(rename = "apps.responses")]
    AppsResponses,
    #[serde(rename = "knowledge.create_page")]
    KnowledgeCreatePage,
    #[serde(rename = "knowledge.get_page")]
    KnowledgeGetPage,
    #[serde(rename = "knowledge.list_domains")]
    KnowledgeListDomains,
    #[serde(rename = "knowledge.update_page")]
    KnowledgeUpdatePage,
    #[serde(rename = "knowledge.request_publish")]
    KnowledgeRequestPublish,
    #[serde(rename = "postgres.credentials")]
    PostgresCredentials,
}
args!(DbxHttpArgs {
    profile: ProfileName,
    route_id: RouteId,
    params: JsonObject,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stream: Option<ChannelRef>,
    tags: Tags,
});
args!(MemExecArgs {
    /// A statement ID of the fixed catalog (R9, R10). Never SQL text.
    stmt_id: String,
    params: JsonObject,
});
args!(ConfigIdArgs { id: String });
args!(ConfigWriteArgs {
    id: String,
    json: JsonObject,
});
args!(HotkeysSetArgs {
    bindings: JsonObject
});

/// The OS permission that `permission_request` asks for and `permission_status` reads
/// (CAP-36, CAP-37).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    /// Input Monitoring for the talk key (CAP-36).
    InputMonitoring,
    /// Accessibility for text insertion (CAP-37).
    Accessibility,
}
args!(
    /// `permission_request` and `permission_status` (ADR 0101). For `permission_request`, the
    /// brain or hub call is the user action (CAP-36, CAP-37).
    PermissionRequestArgs {
        permission: PermissionKind,
    }
);

args!(
    /// `app_setting_read` (#453): the key of one app-wide setting.
    AppSettingReadArgs {
        key: crate::types::AppSettingKey,
    }
);
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
args!(WindowMogOpenArgs {
    mog_id: MogId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    geometry: Option<Geometry>,
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ViewEmitKind {
    Poked,
    Hover,
    DragEnd,
    ChipClick,
    Cancel,
    Expand,
    Click,
    // #507 (ADR 0091): the view registered its listeners. No payload. The brain sends the
    // current state of the sending window (`from`) again. (A `//` comment: a doc comment
    // would turn the schema enum into `oneOf`.)
    Ready,
}
args!(ViewEmitArgs {
    mog_id: MogId,
    kind: ViewEmitKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    payload: Option<Value>,
});
args!(AuditAppendArgs {
    kind: String,
    mog_id: MogId,
    op_id: String,
    /// Content-free sizes, for example `{"chars": 120}`.
    sizes: BTreeMap<String, u64>,
});
args!(BrainSubscribeArgs {
    on_event: ChannelRef
});

/// The kind of a text-bearing hub notice (#509, ADR 0091).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HubNoticeKind {
    /// The dictated text found no MOG card to show on (#498, MOG-05b, MOG-06d).
    DictationUnshown,
}

/// `hub_notice_send` (#509, ADR 0091): the brain gives the host a notice for the hub window
/// only. `text` is user content: the `Debug` output redacts it (SEC-16c). #505 is the general
/// fix for every type that holds content.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HubNoticeSendArgs {
    pub kind: HubNoticeKind,
    pub mog_id: MogId,
    pub text: String,
}

impl std::fmt::Debug for HubNoticeSendArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubNoticeSendArgs")
            .field("kind", &self.kind)
            .field("mog_id", &self.mog_id)
            .field(
                "text",
                &format_args!("<redacted {} chars>", self.text.chars().count()),
            )
            .finish()
    }
}

args!(
    /// `view_notice_subscribe` (#509, ADR 0091): the hub's private Channel for its notices.
    ViewNoticeSubscribeArgs {
        on_notice: ChannelRef
    }
);
args!(TelemetrySpanArgs {
    name: String,
    start_ms: u64,
    end_ms: u64,
    /// Content-free attributes only.
    #[serde(default)]
    attrs: BTreeMap<String, serde_json::Number>,
});

/// The wire form of `mog_core::stt_model::ModelConsent` for `stt_model_consent` (#413, #45,
/// #529, ADR 0104). No `NotAsked` variant: the step only ever sends the user's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SttConsentWire {
    Granted,
    Declined,
}
args!(SttModelConsentArgs {
    consent: SttConsentWire
});

/// The wire form of `mog_core::stt_model::ModelSource` for `stt_model_install` (#413, ADR 0104).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SttSourceWire {
    Github,
    HfMirror,
}
args!(
    /// `stt_model_install` (#413, #45, ADR 0104): the user's chosen source and the hub's
    /// progress Channel. The gateway lets only the hub through, so the Channel resolves on the
    /// hub webview (like `view_notice_subscribe`, ADR 0091).
    SttModelInstallArgs {
        source: SttSourceWire,
        on_progress: ChannelRef,
    }
);

// ---------- dispatch ----------

/// Calls `$f::<ArgType>(name)` for the argument type of each command.
macro_rules! for_command {
    ($name:expr, $f:ident) => {
        match $name {
            "cli_status" | "capture_status" | "capture_stop_all" | "mem_remote_status"
            | "config_list" | "policy_get" => Some($f::<NoArgs>()),
            "consent_records_list" => Some($f::<NoArgs>()),
            "cli_install" => Some($f::<CliInstallArgs>()),
            "profiles_list" => Some($f::<ProfilesListArgs>()),
            "profile_status" | "profile_login_cancel" => Some($f::<ProfileArgs>()),
            "profile_login" => Some($f::<ProfileLoginArgs>()),
            "profile_select" => Some($f::<ProfileSelectArgs>()),
            "capture_start" => Some($f::<CaptureStartArgs>()),
            "capture_stop" | "capture_pause" | "capture_resume" => Some($f::<CaptureLeaseArgs>()),
            "agent_stop" | "mog_set_active" | "window_mog_close" | "window_card_show"
            | "window_card_hide" => Some($f::<MogArgs>()),
            "capture_subscribe" => Some($f::<CaptureSubscribeArgs>()),
            "session_attest" => Some($f::<SessionAttestArgs>()),
            "consent_notice_record" => Some($f::<ConsentNoticeRecordArgs>()),
            "stt_transcribe" => Some($f::<SttTranscribeArgs>()),
            "insert_text" => Some($f::<InsertTextArgs>()),
            "agent_start" => Some($f::<AgentStartArgs>()),
            "agent_send" => Some($f::<AgentSendArgs>()),
            "agent_tool_result" => Some($f::<AgentToolResultArgs>()),
            "model_invoke" => Some($f::<ModelInvokeArgs>()),
            "mcp_call" => Some($f::<McpCallArgs>()),
            "dbx_http" => Some($f::<DbxHttpArgs>()),
            "mem_local_exec" | "mem_remote_exec" => Some($f::<MemExecArgs>()),
            "config_read" | "config_delete" => Some($f::<ConfigIdArgs>()),
            "config_write" => Some($f::<ConfigWriteArgs>()),
            "hotkeys_set" => Some($f::<HotkeysSetArgs>()),
            "permission_request" | "permission_status" => Some($f::<PermissionRequestArgs>()),
            "app_setting_read" => Some($f::<AppSettingReadArgs>()),
            "app_setting_write" => Some($f::<crate::types::AppSetting>()),
            "window_mog_open" => Some($f::<WindowMogOpenArgs>()),
            "view_emit" => Some($f::<ViewEmitArgs>()),
            "audit_append" => Some($f::<AuditAppendArgs>()),
            "telemetry_span" => Some($f::<TelemetrySpanArgs>()),
            "brain_subscribe" => Some($f::<BrainSubscribeArgs>()),
            "hub_notice_send" => Some($f::<HubNoticeSendArgs>()),
            "view_notice_subscribe" => Some($f::<ViewNoticeSubscribeArgs>()),
            "stt_model_status" | "stt_model_verify" | "stt_model_cancel" => Some($f::<NoArgs>()),
            "stt_model_consent" => Some($f::<SttModelConsentArgs>()),
            "stt_model_install" => Some($f::<SttModelInstallArgs>()),
            _ => None,
        }
    };
}

fn schema_of<T: JsonSchema>() -> Schema {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<T>()
}

/// The JSON Schema of the arguments of every command.
pub fn schemas() -> BTreeMap<&'static str, Value> {
    COMMANDS
        .iter()
        .map(|c| {
            let schema = for_command!(c.name, schema_of).expect("every row has an argument type");
            (c.name, schema.to_value())
        })
        .collect()
}

/// The committed schema document: the table, the events, the error codes (the only thing a
/// rejection carries, ADR 0040) and the argument schemas.
pub fn schema_document() -> Value {
    serde_json::json!({
        "$comment": "Generated from mog_core::ipc (#223). Do not edit; rerun the tests with BOTTLE_UPDATE_SCHEMAS=1.",
        "commands": COMMANDS,
        "events": events::ALL,
        "errors": crate::ipc_gateway::ERROR_CODES,
        "arguments": schemas(),
    })
}

/// The fixed reason for arguments that do not parse. serde messages quote the bad value or
/// key (for example "unknown variant `dapi...`"), so they never leave the gateway (4.6 rule 7).
pub const INVALID_ARGS_REASON: &str = "the arguments do not match the command schema";

fn parse<T: for<'de> Deserialize<'de>>(command: &str, args: Value) -> Result<T, IpcError> {
    serde_json::from_value(args).map_err(|_| IpcError::InvalidArgs {
        command: command.to_owned(),
        reason: INVALID_ARGS_REASON.to_owned(),
    })
}

fn check<T: for<'de> Deserialize<'de>>() -> fn(&str, Value) -> Result<(), IpcError> {
    |command, args| parse::<T>(command, args).map(|_| ())
}

/// The single entry for every command (R12). Checks the window, then the arguments.
pub fn handle(window_label: &str, command_name: &str, args: Value) -> Result<Handled, IpcError> {
    let spec = command(command_name).ok_or(IpcError::UnknownCommand)?;
    let not_allowed = || IpcError::WindowNotAllowed {
        command: spec.name.to_owned(),
    };
    let (class, own_id) = WindowClass::of_label(window_label).ok_or_else(not_allowed)?;
    if !spec.windows.contains(&class) {
        return Err(not_allowed());
    }
    let validate = for_command!(spec.name, check).expect("every row has an argument type");
    validate(spec.name, args.clone())?;
    match spec.name {
        "cli_install" => {
            let a: CliInstallArgs = parse(spec.name, args)?;
            if !a.consent {
                return Err(IpcError::InvalidArgs {
                    command: spec.name.to_owned(),
                    reason: "consent must be true".to_owned(),
                });
            }
        }
        "view_emit" => {
            // #507: `ready` has no payload, not even `null` (ADR 0091).
            let has_payload = args.get("payload").is_some();
            let a: ViewEmitArgs = parse(spec.name, args)?;
            if a.kind == ViewEmitKind::Ready && has_payload {
                return Err(IpcError::InvalidArgs {
                    command: spec.name.to_owned(),
                    reason: "ready takes no payload".to_owned(),
                });
            }
            // A MOG or card view may only speak for its own MOG.
            if let Some(id) = own_id {
                if a.mog_id != id {
                    return Err(IpcError::InvalidArgs {
                        command: spec.name.to_owned(),
                        reason: "mogId does not match the sending window".to_owned(),
                    });
                }
            }
            return Ok(Handled::RelayToBrain {
                from: window_label.to_owned(),
            });
        }
        "brain_subscribe" => {
            let a: BrainSubscribeArgs = parse(spec.name, args)?;
            return Ok(Handled::SubscribeBrain {
                channel: a.on_event,
            });
        }
        // SEC-02a: only the brain gets here (the window check above). The host sends the call
        // to R8 `McpCallComponent`, which runs the G-06a allowlist (#467).
        "mcp_call" => return Ok(Handled::Component),
        // RTR-21a, ARC-7 (#307): the host sends these to the one R16 `SharedAgentHost`.
        "agent_start" | "agent_stop" | "agent_send" => return Ok(Handled::Component),
        // #413, #45, #529 (ADR 0104): the hub's Local speech step, over the managed R4
        // `SttSetup` (`src-tauri/src/stt_model_host.rs`). The gateway let only the hub through.
        "stt_model_status" | "stt_model_consent" | "stt_model_install" | "stt_model_verify"
        | "stt_model_cancel" => return Ok(Handled::Component),
        // `config_write`: `mog_config::MogConfigComponent` runs `token_guard::check_config_json`
        // (MB-02, #25, #379).
        _ => {}
    }
    // Each feature issue replaces this with a call into its R1-R17 component (stubs from #225).
    Err(IpcError::NotImplemented {
        command: spec.name.to_owned(),
    })
}
