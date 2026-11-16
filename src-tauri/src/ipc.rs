//! The Tauri binding of the IPC surface (contract, #223).
//!
//! One invoke handler serves every command of `mog_core::ipc::COMMANDS`. Two checks apply:
//! 1. Tauri's ACL: `build.rs` puts every command in the app manifest, so a window can call
//!    a command only if its capability file lists `allow-<command>` (ARC-4).
//! 2. The R12 gateway (`mog_core::ipc::handle`): window class, `deny_unknown_fields` args.
//!
//! The front end calls `invoke(name, { args })`; any other top-level key is rejected.
//! Core-to-brain traffic (including the `view_emit` relay) goes only to the brain, on the
//! Channel that the brain registers with `brain_subscribe`. It never uses Tauri events,
//! because every window that may listen gets every Tauri event (ADR 0003).

use mog_core::ipc::{self, events, ChannelRef, Handled, IpcError, INVALID_ARGS_REASON};
use serde_json::{json, Value};
use std::str::FromStr;
use std::sync::Mutex;
use tauri::ipc::{Channel, Invoke, InvokeBody, JavaScriptChannelId};
use tauri::{Manager, Runtime};

/// The brain's private delivery Channel. `brain_subscribe` sets it; a later call (brain reload)
/// replaces the earlier Channel.
#[derive(Default)]
pub struct BrainChannel(Mutex<Option<Channel<Value>>>);

impl BrainChannel {
    /// Deliver one core-to-brain item as `{ event, payload }`. `false` if no brain listens.
    pub fn send(&self, event: &str, payload: Value) -> bool {
        let guard = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match guard.as_ref() {
            Some(ch) => ch.send(brain_message(event, payload)).is_ok(),
            None => false,
        }
    }

    fn set(&self, channel: Channel<Value>) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(channel);
    }
}

/// The message shape on the brain Channel.
pub fn brain_message(event: &str, payload: Value) -> Value {
    json!({ "event": event, "payload": payload })
}

/// The `view:input` payload for a `view_emit` call from window `from`.
pub fn view_input_payload(from: &str, args: Value) -> Value {
    json!({ "from": from, "input": args })
}

/// Check the invoke envelope `{ args }` and return the arguments.
pub fn parse_envelope(command: &str, payload: &InvokeBody) -> Result<Value, IpcError> {
    let invalid = || IpcError::InvalidArgs {
        command: command.to_owned(),
        reason: INVALID_ARGS_REASON.to_owned(),
    };
    match payload {
        InvokeBody::Json(Value::Object(map)) if map.len() == 1 => {
            map.get("args").cloned().ok_or_else(invalid)
        }
        _ => Err(invalid()),
    }
}

/// The invoke handler for `tauri::Builder::invoke_handler`.
pub fn handler<R: Runtime>(invoke: Invoke<R>) -> bool {
    let name = invoke.message.command().to_owned();
    if ipc::command(&name).is_none() {
        // Not ours: Tauri answers "command not found".
        return false;
    }
    let webview = invoke.message.webview();
    let label = webview.window().label().to_owned();
    let args = match parse_envelope(&name, invoke.message.payload()) {
        Ok(a) => a,
        Err(e) => {
            invoke.resolver.reject(e);
            return true;
        }
    };
    let host_failed = || IpcError::HostFailed {
        command: name.clone(),
    };
    match ipc::handle(&label, &name, args.clone()) {
        Ok(Handled::RelayToBrain { from }) => {
            let ok = webview
                .state::<BrainChannel>()
                .send(events::VIEW_INPUT, view_input_payload(&from, args));
            if ok {
                invoke.resolver.resolve(json!({}));
            } else {
                invoke.resolver.reject(host_failed());
            }
        }
        Ok(Handled::SubscribeBrain { channel }) => match channel_on(&channel, &webview) {
            Some(ch) => {
                webview.state::<BrainChannel>().set(ch);
                invoke.resolver.resolve(json!({}));
            }
            None => invoke.resolver.reject(host_failed()),
        },
        Err(e) => invoke.resolver.reject(e),
    }
    true
}

fn channel_on<R: Runtime>(c: &ChannelRef, webview: &tauri::Webview<R>) -> Option<Channel<Value>> {
    JavaScriptChannelId::from_str(c.as_str())
        .ok()
        .map(|id| id.channel_on(webview.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> InvokeBody {
        InvokeBody::Json(v)
    }

    #[test]
    fn arc_4_envelope_accepts_only_args() {
        let ok = parse_envelope("profile_status", &body(json!({"args": {"profile": "p"}})));
        assert_eq!(ok.unwrap(), json!({"profile": "p"}));
        for bad in [
            json!({}),
            json!({"token": "x"}),
            json!({"args": {}, "headers": {"a": "b"}}),
            json!([1]),
            json!("args"),
        ] {
            let err = parse_envelope("cli_status", &body(bad.clone())).unwrap_err();
            assert!(matches!(err, IpcError::InvalidArgs { .. }), "{bad}");
            assert!(!err.to_string().contains("token") && !err.to_string().contains("headers"));
        }
        assert!(parse_envelope("cli_status", &InvokeBody::Raw(vec![1, 2])).is_err());
    }

    #[test]
    fn arc_5_view_input_goes_to_the_brain_channel_as_view_input() {
        let msg = brain_message(
            events::VIEW_INPUT,
            view_input_payload("mog-a1", json!({"mogId": "a1", "kind": "poked"})),
        );
        assert_eq!(
            msg,
            json!({"event": "view:input", "payload": {"from": "mog-a1", "input": {"mogId": "a1", "kind": "poked"}}})
        );
    }

    #[test]
    fn arc_5_no_brain_channel_means_no_delivery() {
        assert!(!BrainChannel::default().send(events::AGENT_TOOL_CALL, json!({})));
    }

    #[test]
    fn sec_01_brain_channel_scrubs_every_payload() {
        use std::sync::{Arc, Mutex};
        use tauri::ipc::InvokeResponseBody;
        let got: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = Arc::clone(&got);
        let channel = Channel::<Value>::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock().unwrap().push(text);
            }
            Ok(())
        });
        let brain = BrainChannel::default();
        brain.set(channel);
        let secret = concat!("dap", "i0123456789abcdef0123456789abcdef");
        assert!(brain.send(
            events::VIEW_INPUT,
            json!({"from": "mog-a1", "input": {"payload": {"note": format!("x {secret} y")}}})
        ));
        let sent = got.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert!(!sent[0].contains(secret), "the brain got a secret: {}", sent[0]);
        assert!(sent[0].contains(mog_core::token_guard::REDACTED), "{}", sent[0]);
        assert!(sent[0].contains("\"event\":\"view:input\""), "{}", sent[0]);
    }

    /// The production part of a source file: before the test module, no comment lines.
    fn production_code(text: &str) -> String {
        text.split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn sec_03_the_host_dispatches_only_through_hardened_gateway_with() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut uses = 0;
        for entry in std::fs::read_dir(&src).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let code = production_code(&std::fs::read_to_string(&path).unwrap());
            for banned in ["ipc::handle(", "CheckedGateway", "hardened_gateway(", "StubIpcGateway"] {
                assert!(!code.contains(banned), "{} uses {banned}", path.display());
            }
            uses += code.matches("hardened_gateway_with(").count();
        }
        assert_eq!(uses, 1, "one gateway entry for every command (#333)");
        let ipc = production_code(&std::fs::read_to_string(src.join("ipc.rs")).unwrap());
        assert!(ipc.contains("hardened_gateway_with("));
        // ADR 0022 decision 6: the R11 ops run on a worker, not on the IPC thread.
        assert!(ipc.contains("spawn_blocking("));
    }

    #[test]
    fn arc_5_core_to_brain_items_are_never_tauri_events() {
        // Any `emit`/`emit_to` in the app crate would broadcast to every listening window.
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        rust_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        assert!(!files.is_empty());
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            // Production code only: stop at the test module.
            let code: String = text
                .split("#[cfg(test)]")
                .next()
                .unwrap_or_default()
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n");
            for banned in [".emit(", ".emit_to(", ".emit_filter(", "Emitter"] {
                assert!(!code.contains(banned), "{} uses {banned}", path.display());
            }
        }
    }
}
