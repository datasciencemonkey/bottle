//! The shared error type of the Rust core components (R1-R17).
//!
//! An error never carries a token, a credential or user content (ARC-6). The IPC gateway (R12)
//! and the C ABI (R15) send only [`CoreError::code`] across the boundary.

use std::fmt;

/// An error from a `mog-core` component.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CoreError {
    /// The component is a stub. `component` is its ID in section 4.3 (for example `"R12"`).
    NotImplemented {
        /// The component ID, for example `"R1"`.
        component: &'static str,
    },
    /// A component refused a command that would return a token outside R6 TokenBroker (ARC-6).
    TokenCommandRefused {
        /// The component ID that refused, for example `"R5"`.
        component: &'static str,
    },
    /// A component cannot run on this device now. `reason` is a stable, content-free code
    /// (for example `"input_monitoring_required"` from R1). It crosses IPC as the code.
    Unavailable {
        /// The component ID, for example `"R1"`.
        component: &'static str,
        /// The stable reason code.
        reason: &'static str,
    },
    /// R12 IpcGateway refused an IPC call (#36). `code` is the fixed tag of the `IpcError`
    /// variant (for example `"window_not_allowed"`), never a command name, label or value.
    IpcRefused {
        /// The stable, content-free code from `ipc_gateway::ERROR_CODES`.
        code: &'static str,
    },
}

impl CoreError {
    /// The stable, content-free error code that may cross IPC and the C ABI.
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::NotImplemented { .. } => "not_implemented",
            CoreError::TokenCommandRefused { .. } => "token_command_refused",
            CoreError::Unavailable { reason, .. } => reason,
            CoreError::IpcRefused { code } => code,
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::NotImplemented { component } => {
                write!(f, "{component} is not implemented yet")
            }
            CoreError::TokenCommandRefused { component } => {
                write!(f, "{component} refused a command that returns a token")
            }
            CoreError::Unavailable { component, reason } => {
                write!(f, "{component} is not available: {reason}")
            }
            CoreError::IpcRefused { code } => write!(f, "R12 refused the call: {code}"),
        }
    }
}

impl std::error::Error for CoreError {}

/// The result type of every component method.
pub type CoreResult<T> = Result<T, CoreError>;
