//! `model_invoke` host (ADR 0122): manages the [`ModelInvokeComponent`] and the shared
//! [`GatewayChatClient`] on their own R7 [`ReqwestHttpTransport`], over the one R6 of
//! [`crate::mcp_host`], the managed T7 profile hosts and the one [`SharedRateBucket`] (G-09a).
//! [`crate::mcp_host::install`] and [`crate::profiles_host::install`] run first. Without them
//! nothing is managed and `model_invoke` answers `host_failed`.
//!
//! The ask loop (`agent_run::orchestrator`) takes the `Arc<GatewayChatClient>` from managed state.

use std::sync::Arc;

use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport};
use mog_core::model_gateway::client::GatewayChatClient;
use mog_core::model_gateway::invoke::ModelInvokeComponent;
use tauri::Manager;

use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;

const ID: &str = "model_host";
/// The MOG ID of the request tags for a call that no MOG made (the ask loop).
const ORCHESTRATOR_TAG: &str = "orchestrator";

fn missing(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// Build and manage the model host.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) -> CoreResult<()> {
    let bucket = handle
        .try_state::<SharedRateBucket>()
        .ok_or(missing("rate_bucket_missing"))?
        .0
        .clone();
    let hosts: Arc<dyn ProfileHosts> = handle
        .try_state::<ProfilesHost>()
        .ok_or(missing("profiles_missing"))?
        .0
        .clone();
    let tokens = handle
        .try_state::<McpHost>()
        .ok_or(missing("token_broker_missing"))?
        .token_broker();
    let transport = Arc::new(ReqwestHttpTransport::new(hosts, tokens, bucket)?);
    let client = Arc::new(GatewayChatClient::new(transport, ORCHESTRATOR_TAG));
    // The model is the default until the integration step reads the user's orchestrator setting.
    let component = Arc::new(ModelInvokeComponent::new(client.clone(), Arc::new(|| None)));
    handle.manage(client);
    handle.manage(component);
    Ok(())
}
