//! R5 CliRunner (section 4.3).
//!
//! Responsibilities: Resolves `databricks` to an absolute path, checks version ≥ 1.17, runs it as a child process outside the boundary with a clean env, parses JSON. Installs the official CLI only after consent.
//!
//! The output of `databricks auth token` goes only to R6 TokenBroker (ARC-6). So
//! [`CliRunner::run_json`] refuses `auth token` for every caller ([`refuse_token_command`]);
//! R6 gets its tokens through its own path, [`CliRunner::run_auth_token`] (#18, W-04a).

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{compiler_fence, Ordering};

use crate::error::{CoreError, CoreResult};

const ID: &str = "R5";

/// Refuse `databricks auth token` (in any case, with any flags before, between or after).
///
/// The rule is wide on purpose: it refuses when the word `auth` comes before a later word
/// `token` anywhere in the arguments (a flag value such as `-p token` is refused too).
/// Every [`CliRunner`] implementation must call this first in `run_json`, so a token never
/// comes back to a caller as plain JSON (ARC-6).
pub fn refuse_token_command(args: &[&str]) -> CoreResult<()> {
    let words: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let refused = words
        .iter()
        .position(|w| w == "auth")
        .is_some_and(|i| words[i + 1..].iter().any(|w| w == "token"));
    if refused {
        Err(CoreError::TokenCommandRefused { component: ID })
    } else {
        Ok(())
    }
}

/// The one command that gives a token (W-04a): `auth token --profile <p> --output json`.
/// Every [`CliRunner::run_auth_token`] implementation runs exactly these arguments.
pub fn auth_token_args(profile: &str) -> [&str; 6] {
    ["auth", "token", "--profile", profile, "--output", "json"]
}

/// Runs the Databricks CLI as a child process with a clean environment.
pub trait CliRunner: Send + Sync {
    /// Resolve `databricks` to an absolute path and check its version.
    fn resolve(&self) -> CoreResult<PathBuf>;
    /// Run the CLI with these arguments and parse its JSON output.
    /// It refuses `auth token` with [`CoreError::TokenCommandRefused`].
    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value>;
    /// Run [`auth_token_args`] for `profile` and return its raw stdout, unparsed and unlogged.
    ///
    /// ARC-6: only R6 TokenBroker calls this (a test scans the sources). R6 parses the bytes,
    /// keeps the token in memory only, and wipes the [`SecretStdout`]. The caller passes the
    /// profile: R5 never picks one.
    fn run_auth_token(&self, _profile: &str) -> CoreResult<SecretStdout> {
        // TODO(#16): run `auth_token_args(profile)` as a child process with a clean env.
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The raw stdout of `databricks auth token`: it holds a token in plain text (W-04b).
///
/// Not `Clone`, `Debug` is redacted, and the buffer is overwritten with zeros on
/// [`SecretStdout::wipe`] and on drop.
pub struct SecretStdout {
    bytes: Vec<u8>,
}

impl SecretStdout {
    /// Take ownership of the stdout bytes (no copy).
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The stdout bytes, for R6 to parse.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Overwrite the buffer with zeros now. `Drop` calls this too.
    pub fn wipe(&mut self) {
        self.bytes.fill(0);
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut self.bytes);
    }

    /// True if every byte is zero (after [`SecretStdout::wipe`]).
    pub fn is_zeroed(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }
}

impl Drop for SecretStdout {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl fmt::Debug for SecretStdout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretStdout(<redacted>)")
    }
}

/// The R5 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubCliRunner;

impl CliRunner for StubCliRunner {
    fn resolve(&self) -> CoreResult<PathBuf> {
        // TODO(#16): resolve and vet the CLI path and version.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value> {
        refuse_token_command(args)?;
        // TODO(#16): run the CLI with a clean env and parse JSON.
        Err(CoreError::NotImplemented { component: ID })
    }
}
