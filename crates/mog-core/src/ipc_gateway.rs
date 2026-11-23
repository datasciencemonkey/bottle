//! R12 IpcGateway (section 4.3).
//!
//! Responsibilities: Capability check per window label, JSON Schema validation (`deny_unknown_fields`), secret scrubbing for payloads, logs and errors.
//!
//! The command table, the schemas and the per-window checks are `crate::ipc::handle` (#223),
//! which the Tauri app calls. [`CheckedGateway`] is the hardened R12 entry (#36, ADR 0019):
//! it runs `crate::ipc::handle`, turns every refusal into a content-free code and writes one
//! content-free log line per call. Only checked calls reach [`GatewayComponents`].
//! [`hardened_gateway_with`] wraps it in [`ScrubbingGateway`], so
//! every reply also gets the rule-7 scrub (SEC-01, SEC-02a, SEC-02b, SEC-03).
//! The C ABI (`mog_call`, R15) dispatches through [`hardened_gateway_with`] with the label
//! `brain` (#176, ADR 0006).

use crate::error::{CoreError, CoreResult};
use crate::ipc::{self, IpcError, WindowClass};
use crate::model_request_guard::model_invoke_has_tools;
use crate::token_guard::{scrub, scrub_json};

const ID: &str = "R12";

/// The IPC gateway: the entry point for commands from a host (see the module doc).
pub trait IpcGateway: Send + Sync {
    /// Check, validate and run one command from the window `window_label`. Returns the JSON reply.
    fn dispatch(
        &self,
        window_label: &str,
        command: &str,
        args: serde_json::Value,
    ) -> CoreResult<serde_json::Value>;
}

/// The R12 stub. It returns [`CoreError::NotImplemented`] for every command.
#[derive(Debug, Default, Clone, Copy)]
pub struct StubIpcGateway;

impl IpcGateway for StubIpcGateway {
    fn dispatch(
        &self,
        _window_label: &str,
        _command: &str,
        _args: serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        // The table, schemas and window checks are `crate::ipc::handle` (#223). Neither host
        // uses this stub: the Tauri host dispatches through `hardened_gateway_with` (ADR 0040),
        // and the C ABI does too (#176, ADR 0006).
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// Wraps any R12 gateway and scrubs every reply with the rule-7 patterns (4.6, ARC-6).
/// Errors already cross only as content-free codes ([`CoreError::code`]).
/// [`hardened_gateway`] and [`hardened_gateway_with`] use it; the C ABI dispatches through the
/// latter (#176).
#[derive(Debug, Default, Clone, Copy)]
pub struct ScrubbingGateway<G> {
    inner: G,
}

impl<G: IpcGateway> ScrubbingGateway<G> {
    /// Wrap `inner`.
    pub fn new(inner: G) -> Self {
        Self { inner }
    }
}

impl<G: IpcGateway> IpcGateway for ScrubbingGateway<G> {
    fn dispatch(
        &self,
        window_label: &str,
        command: &str,
        args: serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        self.inner
            .dispatch(window_label, command, args)
            .map(|v| scrub_json(&v))
    }
}

// ---------- the hardened gateway (#36) ----------

/// Every code that [`CheckedGateway`] can send to a window: the serde tags of the [`IpcError`]
/// variants. The gateway itself returns some of them in [`CoreError::IpcRefused`]. The config
/// codes come from `MogConfigComponent`, and the R8 `mcp_call` codes (#479) from R8, as
/// `CoreError::Unavailable { reason }`; their variants
/// exist so that this list equals the serde tags.
pub const ERROR_CODES: &[&str] = &[
    "unknown_command",
    "window_not_allowed",
    "invalid_args",
    "not_implemented",
    "host_failed",
    "unknown_lease",
    "mic_busy",
    "consent_required",
    "invalid_mode",
    "mog_cap_reached",
    "unknown_mog",
    "window_closing",
    "config_storage_failed",
    "consent_storage_failed",
    "invalid_config_id",
    "config_too_large",
    "unsupported_schema_version",
    "invalid_config",
    "config_not_found",
    "config_limit_reached",
    "secret_named_key",
    "token_like_value",
    // Local memory (ADR 0113): the secret check refused a memory write. No row was stored.
    "secret_refused",
    "genie_beta_mcp_rejected",
    "genie_space_mcp_rejected",
    "genie_one_sse_rejected",
    "mcp_url_not_canonical",
    "genie_one_off",
    "auth_expired",
    "rate_limited",
    "non_json_reply",
    "timeout",
    "offline",
    "host_refused",
    "redirect_refused",
    "reply_too_large",
    "bad_reply",
    "protocol_unsupported",
    "tool_not_allowed",
    "invalid_tool_args",
    "invalid_meta",
    "profile_invalid",
    // ADR 0117 (one MOG with workspaces).
    "workspace_not_available",
    "home_always_available",
    "workspace_storage_failed",
    "poll_inflight",
    // `agent_*` (#307): the R16 `AgentError::code` values, so the brain sees each one.
    "agent_unknown_mog",
    "agent_unavailable",
    "agent_launch_failed",
    "agent_target_unavailable",
    "agent_invalid_target",
    "agent_harness_mismatch",
    // `profile_login` (#162): the sign-in codes that have no older code (W-07g, W-07h).
    "profile_exists",
    "login_cancelled",
    "cli_failed",
    "no_grant",
    // `recording_*` and `capture_start` (ADR 0103): the call recording codes that have no older
    // code. `consent_required`, `mic_busy`, `rate_limited`, `offline`, `auth_expired` and
    // `host_failed` also apply.
    "recording_off",
    "recording_busy",
    "recording_in_progress",
    "unknown_recording",
    "no_storage",
    "volume_permission",
    "transcribe_endpoint_missing",
    "notes_endpoint_missing",
    "too_long",
    "audio_unreadable",
    "upload_failed",
    "system_audio_denied",
    // `stt_transcribe` (#413, ADR 0104): R4 has no installed, checked model (#46, #413).
    "stt_model_missing",
    // `stt_model_*` (#413, #45, #529, ADR 0104): the `mog_core::stt_setup::SetupError` and
    // `mog_core::stt_model::InstallError` codes, so the hub step shows the real reason.
    "stt_consent_required",
    "stt_url_not_allowed",
    "stt_download_failed",
    "stt_hash_mismatch",
    "stt_extract_failed",
    "stt_io_failed",
    "stt_download_cancelled",
    "stt_install_busy",
    "stt_models_dir_unavailable",
    // The host-side code for a build with no `local-stt` feature (#534, #414): the hub shows
    // "Local speech setup is not available in this build yet."
    "stt_setup_unavailable",
    // Slice 2a (the MOG agent on one live Genie Code session). `timeout` and `offline` also apply.
    // The model-facing codes `budget_reached`, `cross_workspace_off`, `no_active_turn` and
    // `run_stopped` are Rust strings in tool results, never IPC codes.
    "ask_not_approved",
    "agent_queue_full",
    "agent_turn_unknown",
    "agent_session_unavailable",
    "agent_rate_limited",
    "genie_code_missing",
    "genie_code_version",
    "agent_profile_unsupported",
    "agent_signed_out",
    "agent_not_eligible",
    "agent_offline",
    "agent_timeout",
    "agent_failed",
    "agent_lockdown_violation",
    "agent_refusals",
    "install_failed",
    "checksum_mismatch",
];

/// The fixed, content-free code of an [`IpcError`] (its serde tag). It never holds the
/// command name, the window label or an argument value (SEC-03).
pub fn ipc_error_code(e: &IpcError) -> &'static str {
    // No wildcard: a new variant must get a code here.
    match e {
        IpcError::UnknownCommand => "unknown_command",
        IpcError::WindowNotAllowed { .. } => "window_not_allowed",
        IpcError::InvalidArgs { .. } => "invalid_args",
        IpcError::NotImplemented { .. } => "not_implemented",
        IpcError::HostFailed { .. } => "host_failed",
        IpcError::UnknownLease { .. } => "unknown_lease",
        IpcError::MicBusy { .. } => "mic_busy",
        IpcError::ConsentRequired { .. } => "consent_required",
        IpcError::InvalidMode { .. } => "invalid_mode",
        IpcError::MogCapReached { .. } => "mog_cap_reached",
        IpcError::UnknownMog { .. } => "unknown_mog",
        IpcError::WindowClosing { .. } => "window_closing",
        IpcError::ConfigStorageFailed { .. } => "config_storage_failed",
        IpcError::ConsentStorageFailed { .. } => "consent_storage_failed",
        IpcError::InvalidConfigId { .. } => "invalid_config_id",
        IpcError::ConfigTooLarge { .. } => "config_too_large",
        IpcError::UnsupportedSchemaVersion { .. } => "unsupported_schema_version",
        IpcError::InvalidConfig { .. } => "invalid_config",
        IpcError::ConfigNotFound { .. } => "config_not_found",
        IpcError::ConfigLimitReached { .. } => "config_limit_reached",
        IpcError::SecretNamedKey { .. } => "secret_named_key",
        IpcError::TokenLikeValue { .. } => "token_like_value",
        IpcError::SecretRefused { .. } => "secret_refused",
        IpcError::GenieBetaMcpRejected { .. } => "genie_beta_mcp_rejected",
        IpcError::GenieSpaceMcpRejected { .. } => "genie_space_mcp_rejected",
        IpcError::GenieOneSseRejected { .. } => "genie_one_sse_rejected",
        IpcError::McpUrlNotCanonical { .. } => "mcp_url_not_canonical",
        IpcError::GenieOneOff { .. } => "genie_one_off",
        IpcError::AuthExpired { .. } => "auth_expired",
        IpcError::RateLimited { .. } => "rate_limited",
        IpcError::NonJsonReply { .. } => "non_json_reply",
        IpcError::Timeout { .. } => "timeout",
        IpcError::Offline { .. } => "offline",
        IpcError::HostRefused { .. } => "host_refused",
        IpcError::RedirectRefused { .. } => "redirect_refused",
        IpcError::ReplyTooLarge { .. } => "reply_too_large",
        IpcError::BadReply { .. } => "bad_reply",
        IpcError::ProtocolUnsupported { .. } => "protocol_unsupported",
        IpcError::ToolNotAllowed { .. } => "tool_not_allowed",
        IpcError::InvalidToolArgs { .. } => "invalid_tool_args",
        IpcError::InvalidMeta { .. } => "invalid_meta",
        IpcError::ProfileInvalid { .. } => "profile_invalid",
        IpcError::WorkspaceNotAvailable { .. } => "workspace_not_available",
        IpcError::HomeAlwaysAvailable { .. } => "home_always_available",
        IpcError::WorkspaceStorageFailed { .. } => "workspace_storage_failed",
        IpcError::PollInflight { .. } => "poll_inflight",
        IpcError::AgentUnknownMog { .. } => "agent_unknown_mog",
        IpcError::AgentUnavailable { .. } => "agent_unavailable",
        IpcError::AgentLaunchFailed { .. } => "agent_launch_failed",
        IpcError::AgentTargetUnavailable { .. } => "agent_target_unavailable",
        IpcError::AgentInvalidTarget { .. } => "agent_invalid_target",
        IpcError::AgentHarnessMismatch { .. } => "agent_harness_mismatch",
        IpcError::ProfileExists { .. } => "profile_exists",
        IpcError::LoginCancelled { .. } => "login_cancelled",
        IpcError::CliFailed { .. } => "cli_failed",
        IpcError::NoGrant { .. } => "no_grant",
        IpcError::RecordingOff { .. } => "recording_off",
        IpcError::RecordingBusy { .. } => "recording_busy",
        IpcError::RecordingInProgress { .. } => "recording_in_progress",
        IpcError::UnknownRecording { .. } => "unknown_recording",
        IpcError::NoStorage { .. } => "no_storage",
        IpcError::VolumePermission { .. } => "volume_permission",
        IpcError::TranscribeEndpointMissing { .. } => "transcribe_endpoint_missing",
        IpcError::NotesEndpointMissing { .. } => "notes_endpoint_missing",
        IpcError::TooLong { .. } => "too_long",
        IpcError::AudioUnreadable { .. } => "audio_unreadable",
        IpcError::UploadFailed { .. } => "upload_failed",
        IpcError::SystemAudioDenied { .. } => "system_audio_denied",
        IpcError::SttModelMissing { .. } => "stt_model_missing",
        IpcError::SttConsentRequired { .. } => "stt_consent_required",
        IpcError::SttUrlNotAllowed { .. } => "stt_url_not_allowed",
        IpcError::SttDownloadFailed { .. } => "stt_download_failed",
        IpcError::SttHashMismatch { .. } => "stt_hash_mismatch",
        IpcError::SttExtractFailed { .. } => "stt_extract_failed",
        IpcError::SttIoFailed { .. } => "stt_io_failed",
        IpcError::SttDownloadCancelled { .. } => "stt_download_cancelled",
        IpcError::SttInstallBusy { .. } => "stt_install_busy",
        IpcError::SttModelsDirUnavailable { .. } => "stt_models_dir_unavailable",
        IpcError::SttSetupUnavailable { .. } => "stt_setup_unavailable",
        IpcError::AskNotApproved { .. } => "ask_not_approved",
        IpcError::AgentQueueFull { .. } => "agent_queue_full",
        IpcError::AgentTurnUnknown { .. } => "agent_turn_unknown",
        IpcError::AgentSessionUnavailable { .. } => "agent_session_unavailable",
        IpcError::AgentRateLimited { .. } => "agent_rate_limited",
        IpcError::GenieCodeMissing { .. } => "genie_code_missing",
        IpcError::GenieCodeVersion { .. } => "genie_code_version",
        IpcError::AgentProfileUnsupported { .. } => "agent_profile_unsupported",
        IpcError::AgentSignedOut { .. } => "agent_signed_out",
        IpcError::AgentNotEligible { .. } => "agent_not_eligible",
        IpcError::AgentOffline { .. } => "agent_offline",
        IpcError::AgentTimeout { .. } => "agent_timeout",
        IpcError::AgentFailed { .. } => "agent_failed",
        IpcError::AgentLockdownViolation { .. } => "agent_lockdown_violation",
        IpcError::AgentRefusals { .. } => "agent_refusals",
        IpcError::InstallFailed { .. } => "install_failed",
        IpcError::ChecksumMismatch { .. } => "checksum_mismatch",
    }
}

/// Where [`CheckedGateway`] writes its log lines. Each line is content-free (SEC-03).
pub trait GatewayLog: Send + Sync {
    /// Keep one log line.
    fn record(&self, line: &str);
}

/// A log sink that drops every line.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLog;

impl GatewayLog for NoLog {
    fn record(&self, _line: &str) {}
}

/// The R1-R17 components behind [`CheckedGateway`]. The gateway calls them only for a call
/// that passed the window and schema checks. `args` is the checked payload.
pub trait GatewayComponents: Send + Sync {
    /// Run one checked command and return its JSON reply.
    fn call(&self, command: &'static str, args: serde_json::Value)
        -> CoreResult<serde_json::Value>;
}

/// No component is wired yet: every checked call returns `not_implemented`.
/// Each feature issue replaces this with its R1-R17 component.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoComponents;

impl GatewayComponents for NoComponents {
    fn call(
        &self,
        _command: &'static str,
        _args: serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        Err(CoreError::IpcRefused {
            code: "not_implemented",
        })
    }
}

/// The hardened R12 gateway (#36, ADR 0019).
///
/// 1. SEC-02a: `ipc::handle` permits each command only for its window classes. A label outside
///    ARC-3 (for example `ffi`) gets `window_not_allowed` for every command.
/// 2. SEC-02b: `ipc::handle` parses the arguments with `deny_unknown_fields`.
/// 3. Only a call that passed both checks reaches the components.
/// 4. SEC-03: a refusal crosses only as [`CoreError::IpcRefused`] with a code from
///    [`ERROR_CODES`]. The log line names only a known command, the window class and the
///    outcome (`ok` or a code). Wrap it in [`ScrubbingGateway`] ([`hardened_gateway`]) so the
///    component replies get the rule-7 scrub.
#[derive(Debug, Default, Clone)]
pub struct CheckedGateway<L, C = NoComponents> {
    log: L,
    components: C,
}

impl<L: GatewayLog> CheckedGateway<L> {
    /// A gateway with no component wired, that writes its log lines to `log`.
    pub fn new(log: L) -> Self {
        Self::with_components(log, NoComponents)
    }
}

impl<L: GatewayLog, C: GatewayComponents> CheckedGateway<L, C> {
    /// A gateway that sends checked calls to `components`.
    pub fn with_components(log: L, components: C) -> Self {
        Self { log, components }
    }
}

fn class_name(label: &str) -> &'static str {
    match WindowClass::of_label(label) {
        Some((WindowClass::Brain, _)) => "brain",
        Some((WindowClass::Hub, _)) => "hub",
        Some((WindowClass::Mog, _)) => "mog",
        Some((WindowClass::Card, _)) => "card",
        None => match label.split_once('-') {
            // A malformed view label still names its class, but never its ID.
            Some(("mog", _)) => "mog",
            Some(("card", _)) => "card",
            _ => "unknown",
        },
    }
}

impl<L: GatewayLog, C: GatewayComponents> IpcGateway for CheckedGateway<L, C> {
    fn dispatch(
        &self,
        window_label: &str,
        command: &str,
        args: serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        let spec = ipc::command(command);
        let checked = match ipc::handle(window_label, command, args.clone()) {
            // RTR-20a (#117, ADR 0056): a small-model call never carries tools.
            Ok(_) | Err(IpcError::NotImplemented { .. })
                if command == "model_invoke" && model_invoke_has_tools(&args) =>
            {
                Err(CoreError::IpcRefused {
                    code: "invalid_args",
                })
            }
            // `handle` ends every checked call that has no host action in `NotImplemented`.
            Ok(_) | Err(IpcError::NotImplemented { .. }) => Ok(()),
            Err(e) => Err(CoreError::IpcRefused {
                code: ipc_error_code(&e),
            }),
        };
        let result = match (checked, spec) {
            (Ok(()), Some(spec)) => self.components.call(spec.name, args),
            (Err(e), _) => Err(e),
            // `handle` refuses a name that is not in the table, so this does not happen.
            (Ok(()), None) => Err(CoreError::IpcRefused {
                code: "unknown_command",
            }),
        };
        let outcome = match &result {
            Ok(_) => "ok",
            Err(e) => e.code(),
        };
        let line = format!(
            "ipc command={} window={} outcome={outcome}",
            spec.map_or("unknown", |c| c.name),
            class_name(window_label)
        );
        // Every part is a fixed string already; the scrub is a second guard.
        self.log.record(&scrub(&line));
        result
    }
}

/// The hardened R12 gateway with no component wired, plus the rule-7 reply scrub.
pub fn hardened_gateway<L: GatewayLog>(log: L) -> ScrubbingGateway<CheckedGateway<L>> {
    ScrubbingGateway::new(CheckedGateway::new(log))
}

/// The hardened R12 gateway over `components`, plus the rule-7 reply scrub.
pub fn hardened_gateway_with<L: GatewayLog, C: GatewayComponents>(
    log: L,
    components: C,
) -> ScrubbingGateway<CheckedGateway<L, C>> {
    ScrubbingGateway::new(CheckedGateway::with_components(log, components))
}
