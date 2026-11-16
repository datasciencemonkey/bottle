//! The Tauri binding of the IPC surface (contract, #223).
//!
//! One invoke handler serves every command of `mog_core::ipc::COMMANDS`. Two checks apply:
//! 1. Tauri's ACL: `build.rs` puts every command in the app manifest, so a window can call
//!    a command only if its capability file lists `allow-<command>` (ARC-4).
//! 2. The R12 gateway (`mog_core::ipc::handle`): window class, `deny_unknown_fields` args.
//!
//! The front end calls `invoke(name, { args })`. `view_emit` is relayed to the brain only,
//! as the `view:input` event (ARC-5).

use mog_core::ipc::{self, events, Handled, IpcError};
use serde_json::{json, Value};
use tauri::ipc::{Invoke, InvokeBody};
use tauri::{Emitter, Runtime};

/// The invoke handler for `tauri::Builder::invoke_handler`.
pub fn handler<R: Runtime>(invoke: Invoke<R>) -> bool {
    let name = invoke.message.command().to_owned();
    if ipc::command(&name).is_none() {
        // Not ours: Tauri answers "command not found".
        return false;
    }
    let webview = invoke.message.webview();
    let label = webview.window().label().to_owned();
    let args = match invoke.message.payload() {
        InvokeBody::Json(Value::Object(map)) => map.get("args").cloned().unwrap_or(json!({})),
        InvokeBody::Json(_) | InvokeBody::Raw(_) => {
            invoke.resolver.reject(IpcError::InvalidArgs {
                command: name,
                reason: "the payload must be a JSON object { args }".to_owned(),
            });
            return true;
        }
    };
    match ipc::handle(&label, &name, args.clone()) {
        Ok(Handled::RelayToBrain { from }) => {
            let relayed = webview.emit_to(
                "brain",
                events::VIEW_INPUT,
                json!({ "from": from, "input": args }),
            );
            match relayed {
                Ok(()) => invoke.resolver.resolve(json!({})),
                Err(_) => invoke
                    .resolver
                    .reject(IpcError::NotImplemented { command: name }),
            }
        }
        Err(e) => invoke.resolver.reject(e),
    }
    true
}
