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
}

impl CoreError {
    /// The stable, content-free error code that may cross IPC and the C ABI.
    pub fn code(&self) -> &'static str {
        match self {
            CoreError::NotImplemented { .. } => "not_implemented",
        }
    }
}

impl fmt::Display for CoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CoreError::NotImplemented { component } => {
                write!(f, "{component} is not implemented yet")
            }
        }
    }
}

impl std::error::Error for CoreError {}

/// The result type of every component method.
pub type CoreResult<T> = Result<T, CoreError>;
