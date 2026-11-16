//! R12 IpcGateway (section 4.3).
//!
//! Responsibilities: Capability check per window label, JSON Schema validation (`deny_unknown_fields`), secret scrubbing for payloads, logs and errors.
//!
//! The command table, the schemas and the per-window checks are `crate::ipc::handle` (#223),
//! which the Tauri app calls. The C ABI (R15) calls [`IpcGateway::dispatch`]; #176 routes it
//! through `crate::ipc::handle`. The hardening is #36.

use crate::error::{CoreError, CoreResult};

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
