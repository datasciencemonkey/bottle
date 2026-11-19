//! The one T7 ProfileService of the process (#17, ADR 0094).
//!
//! It answers `profiles_list` and gives R6, R7 and R8 their profile hosts
//! ([`crate::mcp_host::install`]), so a PAT, service-principal or plaintext-storage profile
//! never reaches Genie (W-03b, W-10a). It reads only the names, hosts and auth types of
//! `~/.databrickscfg`, never writes it (W-08), and never picks a profile.
//!
//! The host routes `profiles_list` here through `HostComponents` in `src-tauri/src/ipc.rs`
//! (#555); without a managed `ProfilesHost` the call answers `host_failed`.

use std::sync::Arc;

use mog_core::cli_runner::CliRunner;
use mog_core::error::CoreResult;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::profile_login::ProfileLogin;
use mog_core::profile_service::{ConfigFile, ConfigSource, ProfileService};
use serde_json::Value;
use tauri::Manager;

use crate::cli_host::SharedCli;
use crate::mcp_host::McpHost;

/// The `profiles_list` component of the app, over the one T7.
#[derive(Debug, Clone)]
pub struct ProfilesHost(pub Arc<ProfileService>);

impl GatewayComponents for ProfilesHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.0.call(command, args)
    }
}

/// T7 over `config` and R5 `cli`.
pub fn build(config: Arc<dyn ConfigSource>, cli: Arc<dyn CliRunner>) -> ProfilesHost {
    ProfilesHost(Arc::new(ProfileService::new(config, cli)))
}

/// Manage T7 over the user's CLI config file and the shared R5 ([`crate::cli_host`]), so
/// `auth describe` checks the token storage (W-10a). Nothing is read until the first call.
/// [`crate::cli_host::install`] runs first; T7 is not managed without R5.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {
    let Some(cli) = handle.try_state::<SharedCli>().map(|c| c.0.clone()) else {
        return;
    };
    handle.manage(build(Arc::new(ConfigFile::of_user()), cli));
}

/// The `profile_login` and `profile_login_cancel` component of the app (#162).
#[derive(Debug, Clone)]
pub struct ProfileLoginHost(pub Arc<ProfileLogin>);

impl GatewayComponents for ProfileLoginHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.0.call(command, args)
    }
}

/// Manage the "Connect a workspace" sign-in over the user's CLI config file (read only, for the
/// name check), the shared R5 and the W-06a check of R6 ([`McpHost::login_verifier`]).
/// [`crate::cli_host::install`] and [`crate::mcp_host::install`] run first; without R5 or R8
/// it is not managed, and both rows answer `host_failed`.
pub fn install_login(handle: &tauri::AppHandle<tauri::Wry>) {
    let Some(cli) = handle.try_state::<SharedCli>().map(|c| c.0.clone()) else {
        return;
    };
    let Some(verifier) = handle.try_state::<McpHost>().map(|m| m.login_verifier()) else {
        return;
    };
    handle.manage(ProfileLoginHost(Arc::new(ProfileLogin::new(
        Arc::new(ConfigFile::of_user()),
        cli,
        verifier,
    ))));
}
