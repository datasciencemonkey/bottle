//! R12 IpcGateway (section 4.3).
//!
//! Responsibilities: Capability check per window label, JSON Schema validation (`deny_unknown_fields`), secret scrubbing for payloads, logs and errors.
//!
//! The command table, the schemas and the per-window checks are `crate::ipc::handle` (#223),
//! which the Tauri app calls. [`CheckedGateway`] is the hardened R12 entry (#36, ADR 0017):
//! it runs `crate::ipc::handle`, turns every refusal into a content-free code and writes one
//! content-free log line per call. [`hardened_gateway`] wraps it in [`ScrubbingGateway`], so
//! every reply also gets the rule-7 scrub (SEC-01, SEC-02a, SEC-02b, SEC-03).
//! TODO(#176): the C ABI dispatches through [`hardened_gateway`] instead of the stub.

use crate::error::{CoreError, CoreResult};
use crate::ipc::{self, IpcError, WindowClass};
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
        // The table, schemas and window checks are `crate::ipc::handle` (#223). The Tauri host
        // calls it directly. TODO(#176): route the C ABI through it once the ABI decides the
        // window class of an `ffi` caller (ADR 0003).
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// Wraps any R12 gateway and scrubs every reply with the rule-7 patterns (4.6, ARC-6).
/// Errors already cross only as content-free codes ([`CoreError::code`]).
/// [`hardened_gateway`] uses it; TODO(#176) the C ABI dispatches through that gateway.
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

/// Every code that [`CheckedGateway`] returns in [`CoreError::IpcRefused`]: the serde tags of
/// the [`IpcError`] variants.
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
];

/// The fixed, content-free code of an [`IpcError`] (its serde tag). It never holds the
/// command name, the window label or an argument value (SEC-03).
pub fn ipc_error_code(e: &IpcError) -> &'static str {
    // No wildcard: a new variant must get a code here.
    match e {
        IpcError::UnknownCommand { .. } => "unknown_command",
        IpcError::WindowNotAllowed { .. } => "window_not_allowed",
        IpcError::InvalidArgs { .. } => "invalid_args",
        IpcError::NotImplemented { .. } => "not_implemented",
        IpcError::HostFailed { .. } => "host_failed",
        IpcError::UnknownLease { .. } => "unknown_lease",
        IpcError::MicBusy { .. } => "mic_busy",
        IpcError::ConsentRequired { .. } => "consent_required",
        IpcError::InvalidMode { .. } => "invalid_mode",
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

/// The hardened R12 gateway (#36, ADR 0017).
///
/// 1. SEC-02a: `ipc::handle` permits each command only for its window classes. A label outside
///    ARC-3 (for example `ffi`) gets `window_not_allowed` for every command.
/// 2. SEC-02b: `ipc::handle` parses the arguments with `deny_unknown_fields`.
/// 3. SEC-03: a refusal crosses only as [`CoreError::IpcRefused`] with a code from
///    [`ERROR_CODES`]. The log line names only a known command, the window class and the code.
///
/// A call that needs the host (`view_emit` relay, `brain_subscribe`) returns `host_failed`,
/// because this gateway has no brain Channel. The Tauri host does these itself.
#[derive(Debug, Default, Clone)]
pub struct CheckedGateway<L> {
    log: L,
}

impl<L: GatewayLog> CheckedGateway<L> {
    /// A gateway that writes its log lines to `log`.
    pub fn new(log: L) -> Self {
        Self { log }
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

impl<L: GatewayLog> IpcGateway for CheckedGateway<L> {
    fn dispatch(
        &self,
        window_label: &str,
        command: &str,
        args: serde_json::Value,
    ) -> CoreResult<serde_json::Value> {
        let name = ipc::command(command).map_or("unknown", |c| c.name);
        let code = match ipc::handle(window_label, command, args) {
            // The host actions need the Tauri host (see the type doc).
            Ok(_) => "host_failed",
            Err(e) => ipc_error_code(&e),
        };
        let line = format!(
            "ipc command={name} window={} outcome={code}",
            class_name(window_label)
        );
        // Every part is a fixed string already; the scrub is a second guard.
        self.log.record(&scrub(&line));
        Err(CoreError::IpcRefused { code })
    }
}

/// The hardened R12 gateway with the rule-7 reply scrub: `ScrubbingGateway<CheckedGateway>`.
pub fn hardened_gateway<L: GatewayLog>(log: L) -> ScrubbingGateway<CheckedGateway<L>> {
    ScrubbingGateway::new(CheckedGateway::new(log))
}
