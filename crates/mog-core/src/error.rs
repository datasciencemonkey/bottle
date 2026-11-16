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
    /// R10 could not open or use the local store (I/O, wrong key, not a database). No detail is
    /// kept, because a SQLite message can hold SQL or content.
    StoreUnavailable {
        /// The component ID, for example `"R10"`.
        component: &'static str,
    },
    /// The OS key store has no local key, but a database file exists (MEM-09a). R10 does not
    /// delete the file (U9); MemoryService decides what to do.
    StoreKeyLost {
        /// The component ID, for example `"R10"`.
        component: &'static str,
    },
    /// The OS key store (macOS Keychain, Windows Credential Manager) failed or is missing.
    KeychainUnavailable {
        /// The component ID, for example `"R10"`.
        component: &'static str,
    },
    /// The local store has a newer schema than this binary knows. R10 does not downgrade it
    /// (MEM-35a) and does not delete it (U9).
    StoreSchemaNewer {
        /// The component ID, for example `"R10"`.
        component: &'static str,
    },
    /// A local store statement or its parameters have no `mog_id` filter (MEM-01).
    MogIdRequired {
        /// The component ID, for example `"R10"`.
        component: &'static str,
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
            CoreError::StoreUnavailable { .. } => "store_unavailable",
            CoreError::StoreKeyLost { .. } => "store_key_lost",
            CoreError::KeychainUnavailable { .. } => "keychain_unavailable",
            CoreError::StoreSchemaNewer { .. } => "store_schema_newer",
            CoreError::MogIdRequired { .. } => "mog_id_required",
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
            CoreError::StoreUnavailable { component } => {
                write!(f, "{component} could not open or use the local store")
            }
            CoreError::StoreKeyLost { component } => {
                write!(f, "{component} has a local store but no key for it")
            }
            CoreError::KeychainUnavailable { component } => {
                write!(f, "{component} could not use the OS key store")
            }
            CoreError::StoreSchemaNewer { component } => {
                write!(
                    f,
                    "{component} has a newer local store schema than this build"
                )
            }
            CoreError::MogIdRequired { component } => {
                write!(f, "{component} needs a mog_id filter")
            }
        }
    }
}

impl std::error::Error for CoreError {}

/// The result type of every component method.
pub type CoreResult<T> = Result<T, CoreError>;
