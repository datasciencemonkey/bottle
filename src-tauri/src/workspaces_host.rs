//! ADR 0117 (one MOG with workspaces): the host side of the `workspace_*` commands.
//!
//! [`WorkspacesHost`] is managed from the start, so `ipc::route_from` always finds it. [`install`]
//! fills it with the one [`WorkspaceStore`] over `<app config dir>/workspaces.json`, the MOG
//! config store and T7. It runs after `profiles_host::install` and before `mcp_host::install`,
//! because R8 takes [`WorkspacesHost::gate`] (FLAG 7: `genie_ask` only for home and available
//! profiles). R6 is bound later, by `mcp_host::install`, because R8 and R6 are built after the gate
//! is needed. Without a store, every `workspace_*` command answers `host_failed` and the gate
//! allows nothing.

use std::sync::{Arc, OnceLock};

use serde_json::Value;
use tauri::Manager;

use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::mog_config::MogConfigStore;
use mog_core::workspaces::{SignInSource, WorkspaceGate, WorkspaceStore};

use crate::profiles_host::ProfilesHost;

/// The managed `workspace_*` component.
#[derive(Debug, Default)]
pub struct WorkspacesHost {
    store: Arc<OnceLock<Arc<WorkspaceStore>>>,
}

/// The fail-closed gate of R8 over the host.
#[derive(Debug)]
struct HostGate(Arc<OnceLock<Arc<WorkspaceStore>>>);

impl WorkspaceGate for HostGate {
    fn allows(&self, profile: &str) -> bool {
        self.0.get().is_some_and(|s| s.allows(profile))
    }
}

impl WorkspacesHost {
    /// A host over `store`.
    pub fn with_store(store: Arc<WorkspaceStore>) -> Self {
        let host = Self::default();
        host.set_store(store);
        host
    }

    /// Set the store. The first call wins.
    pub fn set_store(&self, store: Arc<WorkspaceStore>) {
        let _ = self.store.set(store);
    }

    /// The allow-list for R8. It allows nothing until a store is set.
    pub fn gate(&self) -> Arc<dyn WorkspaceGate> {
        Arc::new(HostGate(self.store.clone()))
    }

    /// Bind R6 to the store, for the sign-in column and the probe.
    pub fn bind_sign_in(&self, source: Arc<dyn SignInSource>) {
        if let Some(s) = self.store.get() {
            s.bind_sign_in(source);
        }
    }
}

impl GatewayComponents for WorkspacesHost {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match self.store.get() {
            Some(s) => s.call(command, args),
            None => Err(CoreError::IpcRefused {
                code: "host_failed",
            }),
        }
    }
}

/// Fill the managed [`WorkspacesHost`]. [`crate::mog_config_host::install`] and
/// [`crate::profiles_host::install`] run first; without them the host stays empty.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {
    let (Some(host), Some(configs), Some(t7), Ok(dir)) = (
        handle.try_state::<WorkspacesHost>(),
        handle.try_state::<Arc<MogConfigStore>>(),
        handle.try_state::<ProfilesHost>(),
        handle.path().app_config_dir(),
    ) else {
        return;
    };
    host.set_store(Arc::new(WorkspaceStore::new(
        dir,
        configs.inner().clone(),
        t7.0.clone(),
    )));
}
