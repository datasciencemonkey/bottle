//! R5 CliRunner (section 4.3).
//!
//! Responsibilities: Resolves `databricks` to an absolute path, checks version ≥ 1.17, runs it as a child process outside the boundary with a clean env, parses JSON. Installs the official CLI only after consent.
//!
//! The output of `databricks auth token` goes only to R6 TokenBroker (ARC-6).

use std::path::PathBuf;

use crate::error::{CoreError, CoreResult};

const ID: &str = "R5";

/// Runs the Databricks CLI as a child process with a clean environment.
pub trait CliRunner: Send + Sync {
    /// Resolve `databricks` to an absolute path and check its version.
    fn resolve(&self) -> CoreResult<PathBuf>;
    /// Run the CLI with these arguments and parse its JSON output.
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

    fn run_json(&self, _args: &[&str]) -> CoreResult<serde_json::Value> {
        // TODO(#16): run the CLI with a clean env and parse JSON.
        Err(CoreError::NotImplemented { component: ID })
    }
}
