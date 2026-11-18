//! The IPC route of the R16 AgentHost (#307, ADR 0014).
//!
//! [`AgentComponent`] is the [`GatewayComponents`] side of `agent_start`, `agent_stop` and
//! `agent_send`. The hardened gateway runs the window check (brain only, ARC-4) and the
//! argument check first; the component then calls the one [`SharedAgentHost`] of the process
//! (RTR-21a, ARC-7). Each [`AgentError`] crosses as its own code ([`agent_ipc_error`]), so the
//! brain sees `agent_unavailable` and routes with the fallback classifier (ARC-F14, RTR-15b).
//! An error holds the command name only: never the turn, the question or a model name (SEC-03).
//!
//! [`BrainStatusSink`] emits `agent.status {mogId, agentSessionId, state}` and nothing else.

use std::fmt;
use std::sync::Arc;

use serde_json::{json, Value};

use super::{AgentError, AgentHost, AgentStatus, AgentStatusSink, SharedAgentHost};
use crate::bottle_mcp::EventEmit;
use crate::error::{CoreError, CoreResult};
use crate::ipc::{events, AgentSendArgs, AgentStartArgs, IpcError, MogArgs, INVALID_ARGS_REASON};
use crate::ipc_gateway::{ipc_error_code, GatewayComponents};

/// The [`IpcError`] of an [`AgentError`]. Its code is [`AgentError::code`].
pub fn agent_ipc_error(command: &str, e: AgentError) -> IpcError {
    let command = command.to_owned();
    match e {
        AgentError::UnknownMog => IpcError::AgentUnknownMog { command },
        AgentError::Unavailable => IpcError::AgentUnavailable { command },
        AgentError::LaunchFailed => IpcError::AgentLaunchFailed { command },
        AgentError::TargetUnavailable => IpcError::AgentTargetUnavailable { command },
        AgentError::InvalidTarget => IpcError::AgentInvalidTarget { command },
        AgentError::HarnessMismatch => IpcError::AgentHarnessMismatch { command },
    }
}

fn refused(e: IpcError) -> CoreError {
    CoreError::IpcRefused {
        code: ipc_error_code(&e),
    }
}

fn agent_refused(command: &str, e: AgentError) -> CoreError {
    refused(agent_ipc_error(command, e))
}

/// Parse checked arguments again into their type. The gateway checked them already.
fn args_of<T: serde::de::DeserializeOwned>(command: &str, args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| {
        refused(IpcError::InvalidArgs {
            command: command.to_owned(),
            reason: INVALID_ARGS_REASON.to_owned(),
        })
    })
}

/// `agent_start`, `agent_stop` and `agent_send` over the one [`SharedAgentHost`].
#[derive(Clone)]
pub struct AgentComponent {
    host: Arc<SharedAgentHost>,
}

impl fmt::Debug for AgentComponent {
    /// Shows nothing: no session, secret or turn.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentComponent").finish_non_exhaustive()
    }
}

impl AgentComponent {
    /// The component over `host`.
    pub fn new(host: Arc<SharedAgentHost>) -> Self {
        Self { host }
    }

    /// The host.
    pub fn host(&self) -> &Arc<SharedAgentHost> {
        &self.host
    }
}

impl GatewayComponents for AgentComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "agent_start" => {
                let a: AgentStartArgs = args_of(command, args)?;
                let id = self
                    .host
                    .start_session_with_model(&a.mog_id, a.harness.into(), a.model.as_deref())
                    .map_err(|e| agent_refused(command, e))?;
                Ok(json!({ "agentSessionId": id }))
            }
            "agent_stop" => {
                let a: MogArgs = args_of(command, args)?;
                self.host
                    .stop_session(&a.mog_id)
                    .map_err(|e| agent_refused(command, e))?;
                Ok(json!({}))
            }
            "agent_send" => {
                let a: AgentSendArgs = args_of(command, args)?;
                let turn = Value::Object(a.turn.into_iter().collect());
                self.host
                    .send(&a.mog_id, &turn)
                    .map_err(|e| agent_refused(command, e))?;
                Ok(json!({}))
            }
            _ => Err(refused(IpcError::NotImplemented {
                command: command.to_owned(),
            })),
        }
    }
}

/// Emits each session state change as `agent.status {mogId, agentSessionId, state}` on the
/// brain Channel. The payload holds the IDs and the state only (SEC-03).
pub struct BrainStatusSink {
    emit: EventEmit,
}

impl fmt::Debug for BrainStatusSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrainStatusSink").finish_non_exhaustive()
    }
}

impl BrainStatusSink {
    /// A sink that emits through `emit`.
    pub fn new(emit: EventEmit) -> Self {
        Self { emit }
    }
}

impl AgentStatusSink for BrainStatusSink {
    fn status(&self, s: &AgentStatus) {
        (self.emit)(
            events::AGENT_STATUS,
            json!({
                "mogId": s.mog_id,
                "agentSessionId": s.agent_session_id,
                "state": s.state.as_str(),
            }),
        );
    }
}
