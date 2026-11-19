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
use mog_core::profile_service::{ConfigFile, ConfigSource, ProfileService};
use serde_json::Value;
use tauri::Manager;

use crate::cli_host::SharedCli;

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
