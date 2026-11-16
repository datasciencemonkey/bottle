//! R5 CliRunner (section 4.3).
//!
//! Responsibilities: Resolves `databricks` to an absolute path, checks version ≥ 1.17, runs it as a child process outside the boundary with a clean env, parses JSON. Installs the official CLI only after consent.
//!
//! The output of `databricks auth token` goes only to R6 TokenBroker (ARC-6). So
//! [`CliRunner::run_json`] refuses `auth token` for every caller ([`refuse_token_command`]);
//! R6 gets its tokens through its own path (#18).

use std::path::PathBuf;

use crate::error::{CoreError, CoreResult};

const ID: &str = "R5";

/// Refuse `databricks auth token` (in any case, with any flags around it).
///
/// Every [`CliRunner`] implementation must call this first in `run_json`, so a token never
/// comes back to a caller as plain JSON (ARC-6).
pub fn refuse_token_command(args: &[&str]) -> CoreResult<()> {
    let words: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let refused = words.windows(2).any(|w| w[0] == "auth" && w[1] == "token");
    if refused {
        Err(CoreError::TokenCommandRefused { component: ID })
    } else {
        Ok(())
    }
}

/// Runs the Databricks CLI as a child process with a clean environment.
pub trait CliRunner: Send + Sync {
    /// Resolve `databricks` to an absolute path and check its version.
    fn resolve(&self) -> CoreResult<PathBuf>;
    /// Run the CLI with these arguments and parse its JSON output.
    /// It refuses `auth token` with [`CoreError::TokenCommandRefused`].
    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value>;
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
