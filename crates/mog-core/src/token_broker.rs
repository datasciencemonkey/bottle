//! R6 TokenBroker (section 4.3).
//!
//! Responsibilities: In-memory tokens per profile in buffers that it overwrites with zeros. Refresh at expiry − 60 s, single-flight, drop on 401. Resolves `workspace_id` from `x-databricks-org-id`.
//!
//! Dependency rule (ARC-6, section 4.6): only this module names the token type [`SecretToken`].
//! Other components never hold a token. They lend the bytes for one call through
//! [`TokenBroker::with_token`]. A test fails if any other source file names the type.

use std::fmt;
use std::sync::atomic::{compiler_fence, Ordering};

use crate::error::{CoreError, CoreResult};

const ID: &str = "R6";

/// A Databricks token in memory.
///
/// It is not `Clone` and not `Serialize`, its `Debug` output is redacted, and its buffer is
/// overwritten with zeros when it drops.
///
/// ```compile_fail
/// let t = mog_core::token_broker::SecretToken::new(String::from("x"));
/// let _copy = t.clone();
/// ```
pub struct SecretToken {
    bytes: Vec<u8>,
}

impl SecretToken {
    /// Take ownership of a token string. The string buffer moves in; no copy is made.
    pub fn new(value: String) -> Self {
        Self {
            bytes: value.into_bytes(),
        }
    }

    /// The token length in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True if the token is empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Overwrite the buffer with zeros now. `Drop` calls this too.
    pub fn wipe(&mut self) {
        for b in self.bytes.iter_mut() {
            *b = 0;
        }
        // Keep the compiler from removing the zero writes as dead stores.
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut self.bytes);
    }

    /// True if every byte is zero (after [`SecretToken::wipe`]).
    pub fn is_zeroed(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }
}

impl Drop for SecretToken {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

/// Holds the tokens per profile and lends them for one call at a time.
pub trait TokenBroker: Send + Sync {
    /// Lend the current token of `profile` to `f` for one call. The bytes must not be kept.
    fn with_token(&self, profile: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<()>;
    /// Drop the token of `profile` (for example after a 401).
    fn invalidate(&self, profile: &str) -> CoreResult<()>;
    /// The `workspace_id` of `profile`, from `x-databricks-org-id`.
    fn workspace_id(&self, profile: &str) -> CoreResult<String>;
}

/// The R6 stub. It holds no token. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubTokenBroker;

impl TokenBroker for StubTokenBroker {
    fn with_token(&self, _profile: &str, _f: &mut dyn FnMut(&[u8])) -> CoreResult<()> {
        // TODO(#18): in-memory tokens, refresh at expiry - 60 s, single-flight.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn invalidate(&self, _profile: &str) -> CoreResult<()> {
        // TODO(#18): drop on 401.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        // TODO(#18): resolve workspace_id from x-databricks-org-id.
        Err(CoreError::NotImplemented { component: ID })
    }
}
