//! T7 ProfileService, host side (#17, ADR 0094). Red stub: every method is not built yet.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::cli_runner::CliRunner;
use crate::error::{CoreError, CoreResult};
use crate::http_transport::ProfileHosts;
use crate::ipc_gateway::GatewayComponents;
use crate::types::Profile;

/// The auth type of an OAuth U2M profile from `databricks auth login`.
pub const U2M_AUTH_TYPE: &str = "databricks-cli";
const ID: &str = "T7";

/// One profile of the CLI config file: only the fields T7 needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProfile {
    pub name: String,
    pub host: String,
    pub auth_type: String,
}

/// Parse the CLI config text.
pub fn parse_config(_text: &[u8]) -> Vec<ConfigProfile> {
    Vec::new()
}

/// The CLI config file path.
pub fn config_path(_env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    None
}

/// The `auth describe` arguments for `profile`.
pub fn describe_args(profile: &str) -> [&str; 5] {
    ["auth", "describe", "--profile", profile, "--output"]
}

/// Where the CLI profiles come from.
pub trait ConfigSource: Send + Sync {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>>;
}

/// The CLI config file.
pub struct ConfigFile;

impl ConfigFile {
    pub fn new(_path: Option<PathBuf>) -> Self {
        Self
    }
}

impl ConfigSource for ConfigFile {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// T7 host side.
pub struct ProfileService;

impl ProfileService {
    pub fn new(_config: Arc<dyn ConfigSource>, _cli: Arc<dyn CliRunner>) -> Self {
        Self
    }

    pub fn list(&self, _refresh: bool) -> CoreResult<Vec<Profile>> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

impl ProfileHosts for ProfileService {
    fn workspace_url(&self, _profile: &str) -> CoreResult<String> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

impl GatewayComponents for ProfileService {
    fn call(&self, _command: &'static str, _args: Value) -> CoreResult<Value> {
        Err(CoreError::NotImplemented { component: ID })
    }
}
