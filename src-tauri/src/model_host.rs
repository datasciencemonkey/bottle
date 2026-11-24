//! `model_invoke` host (ADR 0122): manages the [`ModelInvokeComponent`] and the shared
//! [`GatewayChatClient`] on their own R7 [`ReqwestHttpTransport`], over the one R6 of
//! [`crate::mcp_host`], the managed T7 profile hosts and the one [`SharedRateBucket`] (G-09a).
//! [`crate::mcp_host::install`] and [`crate::profiles_host::install`] run first. Without them
//! nothing is managed and `model_invoke` answers `host_failed`.
//!
//! The ask loop (`agent_run::orchestrator`) takes its own [`AskChatClient`] from managed state
//! (see `agent_session_host::install`).

use std::sync::Arc;

use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport};
use mog_core::model_gateway::client::GatewayChatClient;
use mog_core::model_gateway::invoke::ModelInvokeComponent;
use tauri::Manager;

use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;
use crate::workspaces_host::WorkspacesHost;

type HomeSource = Arc<dyn Fn() -> Option<String> + Send + Sync>;

const ID: &str = "model_host";
/// The MOG ID of the request tags for a call that no MOG made (the ask loop).
const ORCHESTRATOR_TAG: &str = "orchestrator";

/// The MOG ID of the request tags of a cloud dictation call (ADR 0123).
const DICTATION_TAG: &str = "dictation";

/// The chat client of the ask loop: its own in-flight cap, the same transport as `model_invoke`.
pub struct AskChatClient(pub Arc<GatewayChatClient>);

/// The chat client of cloud dictation (ADR 0123): its own in-flight cap, the same transport.
pub struct DictationChatClient(pub Arc<GatewayChatClient>);

/// R4's cloud half (ADR 0123). R4 is built before the model host exists, so each call looks up
/// the managed parts: the dictation client, the home profile and the Settings choice. Without
/// them the choice reads as off, and a call answers `host_failed`: R4 then keeps the local text.
pub struct LazyCloudStt(pub tauri::AppHandle<tauri::Wry>);

impl mog_core::cloud_dictation::CloudStt for LazyCloudStt {
    fn enabled(&self) -> bool {
        self.0
            .try_state::<WorkspacesHost>()
            .is_some_and(|w| (w.cloud_dictation_source())())
    }

    fn transcribe(
        &self,
        samples: &mut [f32],
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<String, &'static str> {
        let client = self
            .0
            .try_state::<DictationChatClient>()
            .ok_or("host_failed")?
            .0
            .clone();
        let w = self.0.try_state::<WorkspacesHost>().ok_or("host_failed")?;
        let stt = mog_core::cloud_dictation::GatewayCloudStt::new(
            client,
            w.home_source(),
            w.cloud_dictation_source(),
            DICTATION_TAG,
        );
        stt.transcribe(samples, cancel)
    }
}

/// Managed when [`install`] failed: the content-free reason code, for the status of the
/// orchestrator (ADR 0122). Nothing else is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallFailure(pub &'static str);

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
    // Two clients over the one transport and rate bucket. Each has its own in-flight cap, so a
    // burst of dictation calls cannot starve the ask loop (G-09a still limits both together).
    let ask_client = Arc::new(GatewayChatClient::new(transport.clone(), ORCHESTRATOR_TAG));
    let dictation_client = Arc::new(GatewayChatClient::new(transport.clone(), DICTATION_TAG));
    let client = Arc::new(GatewayChatClient::new(transport, ORCHESTRATOR_TAG));
    // The model is the user's orchestrator model and the home profile comes from the workspace
    // store (read only). Without the host, the model is the default and there is no home:
    // `model_invoke` refuses every call.
    let (home, model): (HomeSource, HomeSource) = match handle.try_state::<WorkspacesHost>() {
        Some(w) => (w.home_source(), w.model_source()),
        None => (Arc::new(|| None), Arc::new(|| None)),
    };
    let component = Arc::new(ModelInvokeComponent::new(client, model, home));
    handle.manage(AskChatClient(ask_client));
    handle.manage(DictationChatClient(dictation_client));
    handle.manage(component);
    Ok(())
}
