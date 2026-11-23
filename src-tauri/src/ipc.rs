//! The Tauri binding of the IPC surface (contract, #223).
//!
//! One invoke handler serves every command of `mog_core::ipc::COMMANDS`. Two checks apply:
//! 1. Tauri's ACL: `build.rs` puts every command in the app manifest, so a window can call
//!    a command only if its capability file lists `allow-<command>` (ARC-4).
//! 2. The hardened R12 gateway (`mog_core::ipc_gateway::hardened_gateway_with`, #333): window
//!    class, `deny_unknown_fields` args, the rule-7 reply scrub and content-free codes.
//!
//! The gateway is the only entry (ADR 0040). It sends each checked call to [`HostComponents`],
//! which runs the host actions: `view_emit` (relay to the brain), `brain_subscribe`, the four
//! `window_*` ops (R11), `hotkeys_set` (R1 settings), `permission_request` (R1, ADR 0039) and
//! `permission_status` (R1, #552, ADR 0101),
//! `consent_*` (T12 records, ADR 0060), the `capture_*` commands (R2, #425, ADR 0055),
//! `app_setting_*` (the app-wide settings, #453), `insert_text` (the R11 paste, ADR 0048),
//! `mcp_call` (R8 `McpCallComponent`, #467) and `agent_start`, `agent_stop` and `agent_send`
//! (the one R16 `SharedAgentHost`, #307), `stt_transcribe` (R4 `LocalStt`, #514) and
//! `mem_local_exec` (the one R10 store, through `mog_core::local_store::mem_local_exec`, #426).
//! Every other command stays `not_implemented` until its feature issue wires its component.
//! A rejection is only `{ "error": "<code>" }`, with a code from `ipc_gateway::ERROR_CODES`
//! (SEC-03).
//!
//! The `window_*` ops can wait for the main thread, so they run on a worker and resolve later
//! (ADR 0022 decision 6). `permission_request` can wait for the OS prompt, so it also runs on
//! a worker (ADR 0039), and so does `permission_status`, which can start the talk key after a
//! grant in Settings (ADR 0101). `capture_start` and `capture_resume` open the audio device, which can
//! be slow, so they also run on a worker (ADR 0055). `insert_text` waits for the paste and the
//! clipboard restore, so it also runs on a worker (ADR 0048). `mcp_call` is a blocking HTTP call
//! to Genie One MCP, so it also runs on a worker (#467). The `agent_*` commands run on a worker
//! too (#307): a start can spawn the runtime and wait for the target lookup, a send can wait for
//! the host lock while another MOG starts, and a stop kills and reaps the runtime. The brain
//! awaits each call, so their order holds. `stt_transcribe` takes the R4 store lock that the R4
//! worker shares, so it also runs on a worker (#514). `mem_local_exec` does SQLite IO under the
//! R10 connection lock, so it also runs on a worker (#426). Every other command runs inline, so
//! `view_emit` keeps its order and the panic stop never waits.
//! The last `hotkey:status` item is sticky: a new brain Channel gets it again (ADR 0039).
//!
//! The front end calls `invoke(name, { args })`; any other top-level key is rejected.
//! Core-to-brain traffic (including the `view_emit` relay) goes only to the brain, on the
//! Channel that the brain registers with `brain_subscribe`. It never uses Tauri events,
//! because every window that may listen gets every Tauri event (ADR 0003). Each payload on
//! that Channel gets the rule-7 scrub (SEC-01).
//! The hub notice of an unshown dictation (#509, ADR 0091) goes the same way, on the hub's
//! private Channel (`view_notice_subscribe`), so no MOG or card window can get its text.

use crate::capture_host::{CaptureHost, CaptureTicket};
use crate::window_host::TauriWindowHost;
use mog_core::error::{CoreError, CoreResult};
use mog_core::hotkey_core::{PermissionStatus, UserAction};
use mog_core::ipc::{
    self, events, BrainSubscribeArgs, ChannelRef, HotkeysSetArgs, HubNoticeSendArgs, IpcError,
    JsonObject, MogArgs, PermissionKind, PermissionRequestArgs, SttTranscribeArgs,
    ViewNoticeSubscribeArgs, WindowMogOpenArgs, INVALID_ARGS_REASON,
};
use mog_core::ipc_gateway::{hardened_gateway_with, GatewayComponents, IpcGateway, NoLog};
use mog_core::ipc_gateway::{ipc_error_code, ERROR_CODES};
use mog_core::stt::SttEngine;
use mog_core::token_guard::scrub_json;
use mog_core::types::TranscriptProvider;
use mog_core::window_host::WindowHost;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::Mutex;
use tauri::ipc::{Channel, Invoke, InvokeBody, JavaScriptChannelId};
use tauri::{Manager, Runtime};

/// The last payload of each sticky event (`events::STICKY`, ADR 0039). A pure type: no Tauri
/// runtime. The payload is kept scrubbed, so no secret stays in memory (SEC-01).
#[derive(Debug, Default)]
pub struct StickyEvents(BTreeMap<String, Value>);

impl StickyEvents {
    /// Keep `payload` as the last value of `event` if the event is sticky. `true` if kept.
    pub fn keep(&mut self, event: &str, payload: &Value) -> bool {
        if !events::is_sticky(event) {
            return false;
        }
        self.0.insert(event.to_owned(), scrub_json(payload));
        true
    }

    /// The brain messages to send to a new brain Channel, scrubbed, in event-name order.
    pub fn replay(&self) -> Vec<Value> {
        self.0
            .iter()
            .map(|(event, payload)| brain_message(event, scrub_json(payload)))
            .collect()
    }
}

#[derive(Default)]
struct BrainState {
    channel: Option<Channel<Value>>,
    sticky: StickyEvents,
}

/// The brain's private delivery Channel. `brain_subscribe` sets it; a later call (brain reload)
/// replaces the earlier Channel. The last payload of each sticky event goes again to each new
/// Channel (ADR 0039).
#[derive(Default)]
pub struct BrainChannel(Mutex<BrainState>);

impl BrainChannel {
    fn state(&self) -> std::sync::MutexGuard<'_, BrainState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Deliver one core-to-brain item as `{ event, payload }`. `false` if no brain listens.
    /// The payload gets the rule-7 scrub first (SEC-01, #333).
    pub fn send(&self, event: &str, payload: Value) -> bool {
        Self::send_on(&self.state(), event, &payload)
    }

    /// Deliver a sticky item and keep it for a brain that subscribes later (ADR 0039).
    /// `false` if no brain listens now; the item is still kept. One lock covers the keep and
    /// the send, so a new Channel never misses the item and never gets it out of order.
    pub fn send_sticky(&self, event: &str, payload: Value) -> bool {
        let mut state = self.state();
        state.sticky.keep(event, &payload);
        Self::send_on(&state, event, &payload)
    }

    fn send_on(state: &BrainState, event: &str, payload: &Value) -> bool {
        match state.channel.as_ref() {
            Some(ch) => ch.send(brain_message(event, scrub_json(payload))).is_ok(),
            None => false,
        }
    }

    /// Use `channel` from now on and send it the kept sticky items.
    pub(crate) fn set(&self, channel: Channel<Value>) {
        let mut state = self.state();
        for message in state.sticky.replay() {
            // A failed send means the new brain is already gone; nothing to do.
            let _ = channel.send(message);
        }
        state.channel = Some(channel);
    }
}

/// The hub's private notice Channel (#509, ADR 0091). `view_notice_subscribe` sets it; a hub
/// reload replaces it. The host keeps no notice: with no hub Channel, `send` is `false` and
/// the brain keeps the text. Never logged.
#[derive(Default)]
pub struct HubChannel(Mutex<Option<Channel<Value>>>);

impl HubChannel {
    fn channel(&self) -> std::sync::MutexGuard<'_, Option<Channel<Value>>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Deliver one notice to the hub. `false` if no hub listens.
    pub fn send(&self, notice: Value) -> bool {
        match self.channel().as_ref() {
            Some(ch) => ch.send(notice).is_ok(),
            None => false,
        }
    }

    /// Use `channel` from now on.
    pub(crate) fn set(&self, channel: Channel<Value>) {
        *self.channel() = Some(channel);
    }
}

/// The message shape on the brain Channel.
pub fn brain_message(event: &str, payload: Value) -> Value {
    json!({ "event": event, "payload": payload })
}

/// The `view:input` payload for a `view_emit` call from window `from`.
pub fn view_input_payload(from: &str, args: Value, action_ticket: Option<String>) -> Value {
    match action_ticket {
        // ADR 0113: a host-minted #574 ticket for a user click rides beside the input.
        Some(t) => json!({ "from": from, "input": args, "actionTicket": t }),
        None => json!({ "from": from, "input": args }),
    }
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

// ---------- the host components (ADR 0040) ----------

/// The brain side of the host actions. The app uses the brain Channel of the calling webview.
pub trait BrainLink: Send + Sync {
    /// Deliver one core-to-brain item. `false` if no brain listens.
    fn relay(&self, event: &str, payload: Value) -> bool;
    /// Keep `channel` as the brain's delivery path. `false` if the reference is not usable.
    fn subscribe(&self, channel: &ChannelRef) -> bool;
    /// #509 (ADR 0091): deliver one notice on the hub's private Channel. `false` if no hub
    /// listens. Required, like `subscribe`, so a missing wiring does not compile.
    fn notify_hub(&self, notice: Value) -> bool;
    /// #509: keep `channel` (of the calling hub webview) as the hub's notice path.
    fn subscribe_hub(&self, channel: &ChannelRef) -> bool;
}

/// The R1 settings side of `hotkeys_set` (#331). `HotkeySettings` implements it.
pub trait HotkeysRoute: Send + Sync {
    /// Apply `bindings` and return `{ conflicts }`.
    fn set_hotkeys(&self, bindings: &JsonObject) -> CoreResult<Value>;
}

/// The R1 side of `permission_request` (#312, ADR 0039) and `permission_status` (#552,
/// ADR 0101). `HotkeyRuntime` implements it.
pub trait PermissionRoute: Send + Sync {
    /// Ask the OS for `permission`. `action` proves a user click (CAP-36, CAP-37).
    fn request_permission(
        &self,
        permission: PermissionKind,
        action: UserAction,
    ) -> PermissionStatus;
    /// The state of `permission` now. It never shows a prompt (CAP-36, CAP-37).
    fn permission_status(&self, permission: PermissionKind) -> PermissionStatus;
}

/// The wire value of a permission status: `granted`, `denied` or `unknown`.
pub fn permission_status_name(status: PermissionStatus) -> &'static str {
    match status {
        PermissionStatus::Granted => "granted",
        PermissionStatus::Denied => "denied",
        PermissionStatus::Unknown => "unknown",
    }
}

/// The components behind the hardened gateway for one call. `from` is the label of the
/// calling window, which the app set. A missing host part answers `host_failed`.
pub struct HostComponents<'a> {
    pub from: &'a str,
    pub brain: &'a dyn BrainLink,
    pub windows: Option<&'a dyn WindowHost>,
    pub hotkeys: Option<&'a dyn HotkeysRoute>,
    pub permissions: Option<&'a dyn PermissionRoute>,
    /// The `consent_*` component (#430): `consent_ipc::ConsentComponent` over the managed
    /// consent records. `None` when no records are managed: `consent_storage_failed`.
    pub consent: Option<&'a dyn GatewayComponents>,
    /// The `config_*` component (#379): `mog_config::MogConfigComponent` over the managed
    /// store. `None` when the store is missing: `config_storage_failed`.
    pub config: Option<&'a dyn GatewayComponents>,
    /// The `mcp_call` component (#467): `mcp_host::McpHost`, R8 `McpCallComponent` over the one
    /// R8 of the process. `None` when R8 could not start: `host_failed`.
    pub mcp: Option<&'a dyn GatewayComponents>,
    /// The one R2 CaptureService of the process (#425).
    pub capture: Option<&'a CaptureHost>,
    /// The admission of this call, taken on the IPC thread before `schedule` (ADR 0055).
    /// `None`: the admission is when the call reaches the host.
    pub capture_ticket: Option<CaptureTicket>,
    /// The `app_setting_*` component (#453): `mog_core::app_settings::AppSettings`. `None`
    /// when no settings are managed: `host_failed`.
    pub app_settings: Option<&'a dyn GatewayComponents>,
    /// The R11 paste for `insert_text` (#58, ADR 0048): `mog_core::insertion::os_component`.
    pub insert: Option<&'a dyn GatewayComponents>,
    /// `agent_start`, `agent_stop` and `agent_send` (#307): `agent_host::AgentHostApp`, the one
    /// R16 `SharedAgentHost` of the process. `None` when it could not start: `host_failed`.
    pub agent: Option<&'a dyn GatewayComponents>,
    /// R4 for `stt_transcribe` (#514, ADR 0082): the managed `stt_engine::LocalStt`. `None`
    /// when R4 is not managed: `host_failed`.
    pub stt: Option<&'a dyn SttEngine>,
    /// R10 for `mem_local_exec` (#426): the one managed `SqlCipherLocalStore` that the consent
    /// records and R14 WipeService share. `None` when it is not managed: `host_failed`.
    pub mem: Option<&'a dyn mog_core::local_store::LocalStore>,
    /// #574 (ADR 0113): the one `ActionTickets` of the process, for user-only memory writes.
    /// `None` when it is not managed: no ticket is minted and each such write is refused.
    pub tickets: Option<&'a mog_core::action_ticket::ActionTickets>,
    /// R14 for `mem_wipe` (#525, ADR 0087): the managed `LocalWipeService` over the same R10
    /// store. `None` when it is not managed: `host_failed`.
    pub wipe: Option<&'a dyn mog_core::wipe::WipeService>,
    /// T7 for `profiles_list` (#555, #17): `profiles_host::ProfilesHost`, the one managed
    /// ProfileService. `None` when it is not managed: `host_failed`.
    pub profiles: Option<&'a dyn GatewayComponents>,
    /// `cli_status` and `cli_install` (#16, #161): `cli_host::CliStatusHost` over the one R5 of
    /// the process. `None` when R5 is not managed: `host_failed`.
    pub cli: Option<&'a dyn GatewayComponents>,
    /// `profile_login` and `profile_login_cancel` (#162): `profiles_host::ProfileLoginHost`.
    /// `None` when R5 or R8 could not start: `host_failed`.
    pub login: Option<&'a dyn GatewayComponents>,
    /// The `recording_*` commands (ADR 0103): `mog_core::recording::RecordingComponent`.
    /// `None` when it is not managed: `host_failed`.
    pub recording: Option<&'a dyn GatewayComponents>,
    /// `stt_model_*` (#413, #45, #529, ADR 0104): `stt_model_host::SttModelHost` over the one
    /// managed R4 `SttSetup`. `None` when the OS gives no models folder: `host_failed`.
    pub stt_model: Option<&'a dyn GatewayComponents>,
    /// `link_open` (#523): `mog_core::link_open::LinkOpenComponent`. `None` when T7 is not
    /// managed: `host_failed`.
    pub links: Option<&'a dyn GatewayComponents>,
    /// The `workspace_*` commands (ADR 0117): `workspaces_host::WorkspacesHost`. `None` when it
    /// is not managed: `host_failed`.
    pub workspaces: Option<&'a dyn GatewayComponents>,
    /// The `genie_code_*`, `agent_turn_*` and `agent_session_*` commands (slice 2a):
    /// `agent_session_host::AgentSessionHost`. `None` when it is not managed: `host_failed`.
    pub agent_session: Option<&'a dyn GatewayComponents>,
}

fn refused(e: IpcError) -> CoreError {
    CoreError::IpcRefused {
        code: ipc_error_code(&e),
    }
}

fn host_failed(command: &str) -> CoreError {
    refused(IpcError::HostFailed {
        command: command.to_owned(),
    })
}

fn invalid_args(command: &str) -> CoreError {
    refused(IpcError::InvalidArgs {
        command: command.to_owned(),
        reason: INVALID_ARGS_REASON.to_owned(),
    })
}

/// Parse checked arguments again into their type. The gateway checked them already.
fn args_of<T: serde::de::DeserializeOwned>(command: &str, args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| invalid_args(command))
}

/// CAP-43b, MB-14: an unknown action is the caller's error; every other failure (for example
/// a failed save) is the host's.
fn hotkeys_error(command: &str, e: CoreError) -> CoreError {
    match e {
        CoreError::Unavailable {
            reason: "unknown_hotkey_action",
            ..
        } => invalid_args(command),
        _ => host_failed(command),
    }
}

impl<'a> HostComponents<'a> {
    /// For tests (#532): every component `None`. A fixture names only the fields it needs,
    /// then `..HostComponents::none(from, brain)`, so a new `Option` component needs no
    /// fixture edit. `from` and `brain` are not `Option`, so they are arguments, not a
    /// `Default`. Production code (`route_from`) names every field; a unit test checks it.
    pub fn none(from: &'a str, brain: &'a dyn BrainLink) -> Self {
        Self {
            from,
            brain,
            windows: None,
            hotkeys: None,
            permissions: None,
            consent: None,
            config: None,
            mcp: None,
            capture: None,
            capture_ticket: None,
            app_settings: None,
            insert: None,
            agent: None,
            stt: None,
            mem: None,
            tickets: None,
            wipe: None,
            profiles: None,
            cli: None,
            login: None,
            recording: None,
            stt_model: None,
            links: None,
            workspaces: None,
            agent_session: None,
        }
    }
}

impl HostComponents<'_> {
    fn stt_transcribe(&self, command: &str, args: Value) -> CoreResult<Value> {
        let a: SttTranscribeArgs = args_of(command, args)?;
        if a.provider != TranscriptProvider::LocalParakeet {
            // CAP-57c: local STT is the default. TODO(#49): route Databricks ASR.
            return Err(refused(IpcError::NotImplemented {
                command: command.to_owned(),
            }));
        }
        let stt = self.stt.ok_or_else(|| host_failed(command))?;
        // R4's reasons (`stt_segment_unknown`, `stt_model_missing`, `stt_engine_failed`) are not
        // in `ERROR_CODES`, so each crosses as the content-free `host_failed` (SEC-03).
        let mut transcript = stt
            .transcribe(&a.segment_id)
            .map_err(|_| host_failed(command))?;
        // ADR 0113: Rust's own grammar finds the voice "remember", never the brain's.
        crate::memory_host::attach_remember(
            &mut transcript,
            &a,
            self.tickets,
            crate::memory_host::now_ms(),
        );
        serde_json::to_value(transcript).map_err(|_| host_failed(command))
    }

    /// #426: the brain's R10 call. `mem_local_exec` refuses an unknown or Rust-only (`Access::Core`)
    /// `stmtId`, so the brain cannot run a Wipe step. Never `LocalStore::exec` here (#37 review).
    fn mem_local_exec(&self, command: &str, args: Value) -> CoreResult<Value> {
        // MEM-28b, #574 (ADR 0113): a pin, a hub add, a rating, a release or an edit needs a
        // host-minted user-action ticket. `memory_host` redeems it, and refuses each such write
        // that has none.
        let store = self.mem.ok_or_else(|| host_failed(command))?;
        crate::memory_host::gate_mem_exec(self.tickets, store, args, crate::memory_host::now_ms())
    }

    fn windows(&self, command: &str) -> CoreResult<&dyn WindowHost> {
        self.windows.ok_or_else(|| host_failed(command))
    }

    fn window_op(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        let host = self.windows(command)?;
        if command == "window_mog_open" {
            let a: WindowMogOpenArgs = args_of(command, args)?;
            if let Some(g) = &a.geometry {
                // JSON has no NaN or infinity; a size must be positive.
                if !(g.width > 0.0 && g.height > 0.0) {
                    return Err(invalid_args(command));
                }
            }
            host.open_mog_window(&a.mog_id, a.geometry.as_ref())?;
        } else {
            let a: MogArgs = args_of(command, args)?;
            match command {
                "window_mog_close" => host.close_mog_window(&a.mog_id)?,
                "window_card_show" => host.show_card(&a.mog_id)?,
                _ => host.hide_card(&a.mog_id)?,
            }
        }
        Ok(json!({}))
    }
}

impl GatewayComponents for HostComponents<'_> {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "view_emit" => {
                // lamp-alive: the hide notice of a MOG view is for the host (the eye poller), not
                // a user click: it never reaches the brain.
                if crate::mog_look_host::hide_notice(self.from, &args) {
                    return Ok(json!({}));
                }
                let ticket = crate::memory_host::mint_for_view(
                    self.from,
                    &args,
                    self.tickets,
                    crate::memory_host::now_ms(),
                );
                let item = view_input_payload(self.from, args, ticket);
                if self.brain.relay(events::VIEW_INPUT, item) {
                    Ok(json!({}))
                } else {
                    Err(host_failed(command))
                }
            }
            "brain_subscribe" => {
                let a: BrainSubscribeArgs = args_of(command, args)?;
                if self.brain.subscribe(&a.on_event) {
                    Ok(json!({}))
                } else {
                    Err(host_failed(command))
                }
            }
            // #509 (ADR 0091): the gateway let only the brain through. The notice goes only to
            // the hub's private Channel, never to the brain Channel or a Tauri event.
            "hub_notice_send" => {
                let a: HubNoticeSendArgs = args_of(command, args)?;
                let notice = serde_json::to_value(&a).map_err(|_| host_failed(command))?;
                if self.brain.notify_hub(notice) {
                    Ok(json!({}))
                } else {
                    Err(host_failed(command))
                }
            }
            // #509: the gateway let only the hub window through.
            "view_notice_subscribe" => {
                let a: ViewNoticeSubscribeArgs = args_of(command, args)?;
                if self.brain.subscribe_hub(&a.on_notice) {
                    Ok(json!({}))
                } else {
                    Err(host_failed(command))
                }
            }
            "window_mog_open" | "window_mog_close" | "window_card_show" | "window_card_hide" => {
                self.window_op(command, args)
            }
            "hotkeys_set" => {
                let a: HotkeysSetArgs = args_of(command, args)?;
                let hotkeys = self.hotkeys.ok_or_else(|| host_failed(command))?;
                hotkeys
                    .set_hotkeys(&a.bindings)
                    .map_err(|e| hotkeys_error(command, e))
            }
            "permission_request" => {
                let a: PermissionRequestArgs = args_of(command, args)?;
                let route = self.permissions.ok_or_else(|| host_failed(command))?;
                // The gateway lets only the brain and the hub through. The brain sends it only
                // for a click on the hub or pop-up control of this permission (ADR 0101), so
                // this call is the user's click (CAP-36, CAP-37).
                let status = route.request_permission(a.permission, UserAction::from_user_click());
                Ok(json!({ "status": permission_status_name(status) }))
            }
            "permission_status" => {
                let a: PermissionRequestArgs = args_of(command, args)?;
                let route = self.permissions.ok_or_else(|| host_failed(command))?;
                // No prompt (CAP-36, CAP-37, #552).
                let status = route.permission_status(a.permission);
                Ok(json!({ "status": permission_status_name(status) }))
            }
            // #430 (ADR 0060): T12 consent records. Its codes cross as themselves (ERROR_CODES).
            "consent_notice_record" | "consent_records_list" => match self.consent {
                Some(consent) => consent.call(command, args),
                None => Err(refused(IpcError::ConsentStorageFailed {
                    command: command.to_owned(),
                })),
            },
            // #379: the MOG config store (#25). Its codes cross as themselves (ERROR_CODES).
            "config_list" | "config_read" | "config_write" | "config_delete" => match self.config {
                Some(config) => config.call(command, args),
                None => Err(refused(IpcError::ConfigStorageFailed {
                    command: command.to_owned(),
                })),
            },
            // #453 (CAP-22): the app-wide settings. The gateway let only the hub through.
            "app_setting_read" | "app_setting_write" => self
                .app_settings
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #58 (ADR 0048): the R11 paste. The gateway let only the brain through.
            "insert_text" => self
                .insert
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #467: R8. The gateway let only the brain through (SEC-02a). An R8 code outside
            // `ERROR_CODES` crosses as `host_failed` (`rejection`).
            "mcp_call" => match self.mcp {
                Some(mcp) => mcp.call(command, args),
                None => Err(host_failed(command)),
            },
            // #307: R16. The gateway let only the brain through (ARC-4). Each `AgentError`
            // code crosses as itself (`ERROR_CODES`), so `agent_unavailable` reaches the brain.
            "agent_start" | "agent_stop" | "agent_send" => self
                .agent
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #514 (ADR 0082): R4. The gateway let only the brain through (SEC-02a).
            "stt_transcribe" => self.stt_transcribe(command, args),
            // #426 (MEM-34a): R10. The gateway let only the brain through (SEC-02a, ARC-4).
            "mem_local_exec" => self.mem_local_exec(command, args),
            // #525 (MEM-33a): R14. The gateway let only the brain through; the brain sends it
            // only from the confirm click of the hub Wipe dialog.
            "mem_wipe" => {
                mog_core::wipe::mem_wipe(self.wipe.ok_or_else(|| host_failed(command))?, args)
            }
            // #555 (#17, W-03a): T7. The gateway let only the brain and the hub through.
            "profiles_list" => self
                .profiles
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #161: R5. The gateway let only the brain and the hub through, and `cli_install`
            // only with `consent: true` (W-17b). An install failure crosses as `host_failed`.
            "cli_status" | "cli_install" => self
                .cli
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #162: "Connect a workspace". Its codes cross as themselves (`ERROR_CODES`).
            // ADR 0117: `profile_reauth` signs an existing profile in again, on the login host.
            "profile_login" | "profile_login_cancel" | "profile_reauth" => self
                .login
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // ADR 0103: call recording. The component checks the knob in Rust before any
            // capture path (`recording_off`). Its codes cross as themselves (`ERROR_CODES`).
            c if c.starts_with("recording_") => self
                .recording
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #413, #45, #529 (ADR 0104): the hub's Local speech step. The gateway let only the
            // hub through. Its codes cross as themselves (`ERROR_CODES`).
            "stt_model_status" | "stt_model_consent" | "stt_model_install" | "stt_model_verify"
            | "stt_model_cancel" => self
                .stt_model
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // #523: the gateway let only the brain through. Rust checks the URL again (https,
            // the profile's workspace host) before the OS opener runs.
            "link_open" => self
                .links
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // ADR 0117: the workspace list, its switches, the home and the agent settings.
            "workspace_list"
            | "workspace_set_available"
            | "workspace_make_home"
            | "workspace_settings_write" => self
                .workspaces
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // Slice 2a: the Genie Code binary and the one live agent session. Its codes cross as
            // themselves (`ERROR_CODES`).
            "genie_code_status"
            | "genie_code_install"
            | "agent_turn_submit"
            | "agent_turn_stop"
            | "agent_turn_call"
            | "agent_turn_result"
            | "agent_session_state"
            | "agent_session_end" => self
                .agent_session
                .ok_or_else(|| host_failed(command))?
                .call(command, args),
            // Each feature issue wires its R1-R17 component here.
            "capture_start" | "capture_stop" | "capture_pause" | "capture_resume"
            | "capture_status" | "capture_stop_all" | "capture_subscribe" => {
                let host = self.capture.ok_or_else(|| host_failed(command))?;
                match self.capture_ticket {
                    Some(ticket) => host.call_admitted(command, args, ticket),
                    None => host.call(command, args),
                }
            }
            _ => Err(refused(IpcError::NotImplemented {
                command: command.to_owned(),
            })),
        }
    }
}

/// The rejection that crosses to the window: `{ "error": "<code>" }` and nothing else. A code
/// outside `ERROR_CODES` (for example a component's own reason) becomes `host_failed`.
pub fn rejection(e: &CoreError) -> Value {
    let code = e.code();
    let code = if ERROR_CODES.contains(&code) {
        code
    } else {
        "host_failed"
    };
    json!({ "error": code })
}

/// Run one command through the hardened gateway (the only entry, #333).
pub fn route(components: HostComponents<'_>, command: &str, args: Value) -> Result<Value, Value> {
    let from = components.from;
    hardened_gateway_with(NoLog, components)
        .dispatch(from, command, args)
        .map_err(|e| rejection(&e))
}

/// True for the commands that run on a worker: the R11 ops, which can wait for the main
/// thread (ADR 0022 decision 6), and `permission_request`, which can wait while the OS prompt
/// shows (ADR 0039), `permission_status`, which can start the talk key (ADR 0101), `capture_start` and `capture_resume`, which open the device (ADR 0055),
/// `insert_text`, which waits about 0.4 s for the paste and the clipboard restore (ADR 0048),
/// `mcp_call`, a blocking HTTP call to Genie One MCP (#467), and the `agent_*` commands (#307),
/// which can spawn, wait for the host lock or reap the runtime, `stt_transcribe`, which
/// takes the R4 store lock that the R4 worker shares (#514), `mem_local_exec`, which does
/// SQLite IO under the R10 connection lock (#426), `profiles_list`, where T7 runs
/// `databricks auth describe` once per `databricks-cli` profile when R5 lands (#555),
/// `cli_install`, which downloads and checks the CLI (#161), `profile_login`, which waits
/// up to 5 min for `databricks auth login` (#162), `stt_model_install`, which can download
/// 487 MB over the network before it checks every SHA-256 (#413, #45, ADR 0104),
/// `stt_model_verify`, which SHA-256s the same ~670 MB with no network IO: run on the main
/// thread it freezes every window, the MOG drag and the double-click monitor for 1-2 s each
/// time the hub mounts (#413, PR #600 review), and every `recording_*` command except
/// `recording_state`, which does file IO, opens the device or calls the workspace (ADR 0103).
pub fn runs_off_main_thread(command: &str) -> bool {
    (command.starts_with("recording_") && command != "recording_state")
        || matches!(
            command,
            "window_mog_open"
                | "window_mog_close"
                | "window_card_show"
                | "window_card_hide"
                | "permission_request"
                | "permission_status"
                | "capture_start"
                | "capture_resume"
                | "insert_text"
                | "mcp_call"
                | "agent_start"
                | "agent_stop"
                | "agent_send"
                | "stt_transcribe"
                | "mem_local_exec"
                // #525: the Wipe deletes rows in one R10 transaction.
                | "mem_wipe"
                | "profiles_list"
                | "cli_install"
                | "profile_login"
                // ADR 0117: the re-sign-in waits for the browser; the list can probe profiles.
                | "profile_reauth"
                | "workspace_list"
                | "workspace_set_available"
                | "workspace_make_home"
                // Slice 2a: the status runs the binary, the install downloads it, a submit can
                // start the server and a stop or an end waits up to 5 s for the process.
                | "genie_code_status"
                | "genie_code_install"
                | "agent_turn_submit"
                | "agent_turn_stop"
                | "agent_turn_call"
                | "agent_turn_result"
                | "agent_session_state"
                | "agent_session_end"
                // #413, #45: the install can run for minutes over the network (487 MB).
                | "stt_model_install"
                // #413, PR #600 review: verify SHA-256s ~670 MB with no network IO.
                | "stt_model_verify"
        )
}

/// The paste component of this OS, or `None` where Bottle has no paste (`host_failed`).
#[cfg(any(target_os = "macos", windows))]
fn os_insert() -> Option<&'static dyn GatewayComponents> {
    use std::sync::OnceLock;
    #[cfg(target_os = "macos")]
    type Host = mog_core::insertion::MacInsertHost;
    #[cfg(windows)]
    type Host = mog_core::insertion::WindowsInsertHost;
    static INSERT: OnceLock<mog_core::insertion::InsertTextComponent<Host>> = OnceLock::new();
    Some(INSERT.get_or_init(mog_core::insertion::os_component))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_insert() -> Option<&'static dyn GatewayComponents> {
    None
}

/// The brain link of one calling webview: its brain Channel state and its Channel IDs.
struct WebviewBrain<'a, R: Runtime>(&'a tauri::Webview<R>);

impl<R: Runtime> BrainLink for WebviewBrain<'_, R> {
    fn relay(&self, event: &str, payload: Value) -> bool {
        self.0.state::<BrainChannel>().send(event, payload)
    }

    fn subscribe(&self, channel: &ChannelRef) -> bool {
        match channel_on(channel, self.0) {
            Some(ch) => {
                self.0.state::<BrainChannel>().set(ch);
                true
            }
            None => false,
        }
    }

    fn notify_hub(&self, notice: Value) -> bool {
        self.0
            .try_state::<HubChannel>()
            .is_some_and(|hub| hub.send(notice))
    }

    fn subscribe_hub(&self, channel: &ChannelRef) -> bool {
        match (
            self.0.try_state::<HubChannel>(),
            channel_on(channel, self.0),
        ) {
            (Some(hub), Some(ch)) => {
                hub.set(ch);
                true
            }
            _ => false,
        }
    }
}

/// Route one call from `webview` with the app's managed host parts.
fn route_from<R: Runtime>(
    webview: &tauri::Webview<R>,
    label: &str,
    command: &str,
    args: Value,
    capture_ticket: Option<CaptureTicket>,
) -> Result<Value, Value> {
    let brain = WebviewBrain(webview);
    let windows = webview.try_state::<TauriWindowHost<R>>();
    let hotkeys = webview.try_state::<crate::hotkey_settings::HotkeySettings>();
    let runtime = webview.try_state::<crate::hotkey_host::HotkeyRuntime>();
    // `try_state`: no records are managed yet, so `consent_*` answers `consent_storage_failed`.
    // TODO(#431): manage `Arc<ConsentRecords>` (local store, device ID) and the signed-in user.
    let consent = webview
        .try_state::<std::sync::Arc<mog_core::consent::ConsentRecords>>()
        .map(|records| {
            mog_core::consent_ipc::ConsentComponent::new(
                records.inner().clone(),
                std::sync::Arc::new(mog_core::consent_ipc::NoSignedInUser),
            )
        });
    // `try_state`: the app still starts when the OS gives no config dir (#372 review).
    let config = webview
        .try_state::<std::sync::Arc<mog_core::mog_config::MogConfigStore>>()
        .map(|store| mog_core::mog_config::MogConfigComponent::new(store.inner().clone()));
    let capture = webview.try_state::<CaptureHost>();
    let app_settings = webview.try_state::<std::sync::Arc<mog_core::app_settings::AppSettings>>();
    // `try_state`: if R8 could not start, `mcp_call` answers `host_failed` (#467).
    let mcp = webview.try_state::<crate::mcp_host::McpHost>();
    // `try_state`: if the loopback model proxy could not bind, `agent_*` answer `host_failed`.
    let agent = webview.try_state::<crate::agent_host::AgentHostApp>();
    // `try_state`: `install_capture` manages R4 (#46); without it `stt_transcribe` answers
    // `host_failed` (#514).
    let stt = webview.try_state::<mog_core::stt_engine::LocalStt>();
    // `try_state`: `consent_host::install` manages the one R10 store; without it (no key store
    // or no data dir) `mem_local_exec` answers `host_failed` (#426).
    let mem = webview.try_state::<std::sync::Arc<mog_core::local_store::SqlCipherLocalStore>>();
    // `try_state`: `consent_host::install` manages R14 over the same store (#525).
    let wipe = webview.try_state::<std::sync::Arc<mog_core::wipe::LocalWipeService>>();
    // `try_state`: `lib.rs` manages the one ticket store (ADR 0113).
    let tickets = webview.try_state::<std::sync::Arc<mog_core::action_ticket::ActionTickets>>();
    // `try_state`: `profiles_host::install` manages T7; without it `profiles_list` answers
    // `host_failed` (#555).
    let profiles = webview.try_state::<crate::profiles_host::ProfilesHost>();
    // `try_state`: `cli_host::start` manages R5 (#16); without it `cli_*` answer `host_failed`.
    let cli = webview.try_state::<crate::cli_host::CliStatusHost>();
    let login = webview.try_state::<crate::profiles_host::ProfileLoginHost>();
    // `try_state`: `lib.rs` manages the one recording component (ADR 0103).
    let recording = webview.try_state::<std::sync::Arc<mog_core::recording::RecordingComponent>>();
    // `try_state`: `install_stt_model` manages the one R4 `SttSetup` (#413, #45); without it
    // (no models folder on this device) `stt_model_*` answer `host_failed`.
    let stt_model = webview.try_state::<crate::stt_model_host::SttModelHost>();
    // `try_state`: `lib.rs` manages the link opener after T7 (#523).
    let links = webview.try_state::<std::sync::Arc<mog_core::link_open::LinkOpenComponent>>();
    // `try_state`: `lib.rs` manages the workspace component (ADR 0117).
    let workspaces = webview.try_state::<crate::workspaces_host::WorkspacesHost>();
    // `try_state`: `lib.rs` manages the agent session component (slice 2a).
    let agent_session = webview.try_state::<crate::agent_session_host::AgentSessionHost>();
    let components = HostComponents {
        from: label,
        brain: &brain,
        windows: windows.as_ref().map(|w| w.inner() as &dyn WindowHost),
        hotkeys: hotkeys.as_ref().map(|h| h.inner() as &dyn HotkeysRoute),
        permissions: runtime.as_ref().map(|r| r.inner() as &dyn PermissionRoute),
        consent: consent.as_ref().map(|c| c as &dyn GatewayComponents),
        config: config.as_ref().map(|c| c as &dyn GatewayComponents),
        mcp: mcp.as_ref().map(|m| m.inner() as &dyn GatewayComponents),
        capture: capture.as_ref().map(|c| c.inner()),
        capture_ticket,
        app_settings: app_settings
            .as_ref()
            .map(|s| s.inner().as_ref() as &dyn GatewayComponents),
        insert: os_insert(),
        agent: agent.as_ref().map(|a| a.inner() as &dyn GatewayComponents),
        stt: stt.as_ref().map(|s| s.inner() as &dyn SttEngine),
        mem: mem
            .as_ref()
            .map(|m| m.inner().as_ref() as &dyn mog_core::local_store::LocalStore),
        tickets: tickets.as_ref().map(|t| t.inner().as_ref()),
        wipe: wipe
            .as_ref()
            .map(|w| w.inner().as_ref() as &dyn mog_core::wipe::WipeService),
        profiles: profiles
            .as_ref()
            .map(|p| p.inner() as &dyn GatewayComponents),
        cli: cli
            .as_ref()
            .map(|c| c.inner().0.as_ref() as &dyn GatewayComponents),
        login: login.as_ref().map(|l| l.inner() as &dyn GatewayComponents),
        recording: recording
            .as_ref()
            .map(|r| r.inner().as_ref() as &dyn GatewayComponents),
        stt_model: stt_model
            .as_ref()
            .map(|s| s.inner() as &dyn GatewayComponents),
        links: links
            .as_ref()
            .map(|l| l.inner().as_ref() as &dyn GatewayComponents),
        workspaces: workspaces
            .as_ref()
            .map(|w| w.inner() as &dyn GatewayComponents),
        agent_session: agent_session
            .as_ref()
            .map(|a| a.inner() as &dyn GatewayComponents),
    };
    route(components, command, args)
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
            invoke
                .resolver
                .reject(json!({ "error": ipc_error_code(&e) }));
            return true;
        }
    };
    let resolver = invoke.resolver;
    let command = name.clone();
    // ADR 0055: a capture call takes its admission here, in arrival order, so a stop that
    // arrives later wins over a start or resume that waits for its worker.
    let capture_ticket = admit(&command, webview.try_state::<CaptureHost>().as_deref());
    schedule(&command, move || {
        match route_from(&webview, &label, &name, args, capture_ticket) {
            Ok(v) => resolver.resolve(v),
            Err(e) => resolver.reject(e),
        }
    });
    true
}

/// The admission of a capture call, taken on the IPC thread before `schedule`, in arrival
/// order (ADR 0055, decision 7). `None` for any other command, or with no capture host.
pub fn admit(command: &str, capture: Option<&CaptureHost>) -> Option<CaptureTicket> {
    if command.starts_with("capture_") {
        capture.map(CaptureHost::ticket)
    } else {
        None
    }
}

/// Run the job of one call. An R11 op goes to a worker (`spawn_blocking`) and resolves later,
/// because it can wait for the main thread (ADR 0022 decision 6). `permission_request` also
/// goes to a worker, because the OS prompt can wait for the user (ADR 0039), and so do
/// `capture_start` and `capture_resume`, whose device open can be slow (ADR 0055), and
/// `mcp_call`, which waits for Genie One MCP (#467), `agent_*` (#307), `stt_transcribe` (#514)
/// and `mem_local_exec` (#426).
/// Every other command runs now, on the calling thread, so `view_emit` keeps its order.
pub fn schedule<F: FnOnce() + Send + 'static>(command: &str, job: F) {
    if runs_off_main_thread(command) {
        tauri::async_runtime::spawn_blocking(job);
    } else {
        job();
    }
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
            view_input_payload("mog-a1", json!({"mogId": "a1", "kind": "poked"}), None),
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
        assert!(
            !sent[0].contains(secret),
            "the brain got a secret: {}",
            sent[0]
        );
        assert!(
            sent[0].contains(mog_core::token_guard::REDACTED),
            "{}",
            sent[0]
        );
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
            for banned in [
                "ipc::handle(",
                "CheckedGateway",
                "hardened_gateway(",
                "StubIpcGateway",
            ] {
                assert!(!code.contains(banned), "{} uses {banned}", path.display());
            }
            uses += code.matches("hardened_gateway_with(").count();
        }
        assert_eq!(uses, 1, "one gateway entry for every command (#333)");
        let ipc = production_code(&std::fs::read_to_string(src.join("ipc.rs")).unwrap());
        assert!(ipc.contains("hardened_gateway_with("));
        // ADR 0022 decision 6: the R11 ops run on a worker, not on the IPC thread.
        assert!(ipc.contains("spawn_blocking("));
        assert!(
            ipc.contains("schedule(&command"),
            "the handler schedules every call"
        );
    }

    /// Records the commands it gets; answers a fake `cli_status` reply.
    struct FakeCli(std::sync::Mutex<Vec<(String, Value)>>);

    impl GatewayComponents for FakeCli {
        fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
            self.0.lock().unwrap().push((command.to_owned(), args));
            Ok(json!({"path": "", "version": "", "found": false, "plaintextStorage": false}))
        }
    }

    /// [W-17b] [U16] `cli_install` reaches R5 only with `consent: true`, from the brain or the hub,
    /// and runs off the main thread (#161)
    #[test]
    fn w_17b_cli_install_routes_to_r5_only_with_consent() {
        let cli = FakeCli(std::sync::Mutex::new(Vec::new()));
        let host = |from| HostComponents {
            cli: Some(&cli),
            ..HostComponents::none(from, &NoBrain)
        };
        for bad in [json!({}), json!({"consent": false})] {
            assert_eq!(
                route(host("hub"), "cli_install", bad),
                Err(json!({"error": "invalid_args"}))
            );
        }
        assert!(route(host("mog-a1"), "cli_install", json!({"consent": true})).is_err());
        assert!(
            cli.0.lock().unwrap().is_empty(),
            "no install without consent"
        );
        assert!(route(host("hub"), "cli_install", json!({"consent": true})).is_ok());
        assert!(route(host("brain"), "cli_status", json!({})).is_ok());
        let calls: Vec<String> = cli.0.lock().unwrap().iter().map(|c| c.0.clone()).collect();
        assert_eq!(calls, ["cli_install", "cli_status"]);
        assert!(runs_off_main_thread("cli_install"));
        // No R5: `host_failed`.
        assert_eq!(
            route(
                HostComponents::none("hub", &NoBrain),
                "cli_install",
                json!({"consent": true})
            ),
            Err(json!({"error": "host_failed"}))
        );
    }

    /// #413, PR #600 review (HIGH): `stt_model_verify` SHA-256s ~670 MB with no network IO.
    /// Run on the main thread it freezes every window, the MOG drag and the double-click
    /// monitor for 1-2 s each time the hub mounts. It must run off the main thread, same as
    /// `stt_model_install`.
    #[test]
    fn issue_413_stt_model_verify_runs_off_the_main_thread() {
        assert!(runs_off_main_thread("stt_model_verify"));
        assert!(runs_off_main_thread("stt_model_install"));
    }

    // ---- ARC-5 (#532): the test constructor and the production guard ----

    struct NoBrain;

    impl BrainLink for NoBrain {
        fn relay(&self, _event: &str, _payload: Value) -> bool {
            false
        }
        fn subscribe(&self, _channel: &ChannelRef) -> bool {
            false
        }
        fn notify_hub(&self, _notice: Value) -> bool {
            false
        }
        fn subscribe_hub(&self, _channel: &ChannelRef) -> bool {
            false
        }
    }

    #[test]
    fn arc_5_issue_532_none_sets_every_component_to_none() {
        let c = HostComponents {
            from: "brain",
            ..HostComponents::none("mog-a1", &NoBrain)
        };
        assert_eq!(c.from, "brain");
        assert!(c.windows.is_none());
        assert!(c.hotkeys.is_none());
        assert!(c.permissions.is_none());
        assert!(c.consent.is_none());
        assert!(c.config.is_none());
        assert!(c.mcp.is_none());
        assert!(c.capture.is_none());
        assert!(c.capture_ticket.is_none());
        assert!(c.app_settings.is_none());
        assert!(c.insert.is_none());
        assert!(c.agent.is_none());
        assert!(c.stt.is_none());
        assert!(c.mem.is_none());
        assert!(c.recording.is_none());
        // A missing host part still answers `host_failed`.
        assert_eq!(
            route(c, "window_card_show", json!({"mogId": "a1"})),
            Err(json!({"error": "host_failed"}))
        );
    }

    /// Every `HostComponents { .. }` literal in `code`: the text between its braces.
    fn host_component_literals(code: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = code;
        while let Some(at) = rest.find("HostComponents {") {
            let before = &rest[..at];
            let open = at + "HostComponents ".len();
            let mut depth = 0usize;
            let mut end = open;
            for (i, ch) in rest[open..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = open + i;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            // The type definition is not a constructor.
            if !before.trim_end().ends_with("struct") {
                out.push(rest[open + 1..end].to_owned());
            }
            rest = &rest[end..];
        }
        out
    }

    /// The production constructors name every field, so the host cannot miss a new
    /// component (#532). Only tests may use `HostComponents::none` or a `..base`.
    #[test]
    fn arc_5_issue_532_production_host_components_name_every_field() {
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
        let mut literals = 0;
        for path in files {
            let code = production_code(&std::fs::read_to_string(&path).unwrap());
            for banned in [
                "HostComponents::none(",
                "HostComponents::default(",
                "HostComponents<'_>::default(",
            ] {
                assert!(!code.contains(banned), "{} uses {banned}", path.display());
            }
            for body in host_component_literals(&code) {
                literals += 1;
                assert!(
                    !body.contains(".."),
                    "{}: a production HostComponents uses a `..` base: {body}",
                    path.display()
                );
            }
        }
        assert!(literals >= 1, "route_from builds HostComponents");
    }

    /// Every fixture in `src-tauri/tests` takes a `..HostComponents::none(..)` base, so a new
    /// `Option` component needs no fixture edit (#532).
    #[test]
    fn arc_5_issue_532_test_fixtures_use_a_none_base() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            for body in host_component_literals(&text) {
                assert!(
                    body.contains("..HostComponents::none("),
                    "{}: a HostComponents fixture names every field; end it with \
                     `..HostComponents::none(from, brain)`: {body}",
                    path.display()
                );
            }
        }
    }

    #[test]
    fn arc_5_issue_532_the_guard_sees_a_base_in_a_literal() {
        let bad = "let c = HostComponents {\n from: label,\n ..HostComponents::none(label, &b)\n};";
        let found = host_component_literals(bad);
        assert_eq!(found.len(), 1);
        assert!(found[0].contains(".."));
        let def = "pub struct HostComponents { pub from: String }";
        assert!(host_component_literals(def).is_empty());
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

    // ---- ARC-5, SEC-01 (#297, ADR 0039): sticky delivery ----

    /// A plainly fake value that the rule-7 scrub still matches (`password=`).
    const SECRET: &str = "password=test-not-real";

    fn status(talk_key: &str, reason: Value) -> Value {
        json!({ "talkKey": talk_key, "reason": reason })
    }

    #[test]
    fn arc_5_sticky_events_keep_only_the_last_payload_of_a_sticky_event() {
        let mut sticky = StickyEvents::default();
        assert!(sticky.replay().is_empty());
        assert!(sticky.keep(
            events::HOTKEY_STATUS,
            &status("paused", json!("secure_input"))
        ));
        assert!(sticky.keep(events::HOTKEY_STATUS, &status("active", Value::Null)));
        // A plain event is not kept.
        assert!(!sticky.keep(events::HOTKEY_PTT, &json!({"mogId": "a1", "phase": "down"})));
        assert!(!sticky.keep(events::VIEW_INPUT, &json!({})));
        assert_eq!(
            sticky.replay(),
            vec![brain_message(
                events::HOTKEY_STATUS,
                status("active", Value::Null)
            )]
        );
    }

    #[test]
    fn sec_01_sticky_replay_is_scrubbed() {
        let mut sticky = StickyEvents::default();
        sticky.keep(
            events::HOTKEY_STATUS,
            &json!({"talkKey": format!("x {SECRET}")}),
        );
        let text = serde_json::to_string(&sticky.replay()).unwrap();
        assert!(!text.contains(SECRET), "{text}");
        assert!(text.contains(mog_core::token_guard::REDACTED), "{text}");
    }

    fn recorder() -> (Channel<Value>, std::sync::Arc<Mutex<Vec<Value>>>) {
        use tauri::ipc::InvokeResponseBody;
        let got: std::sync::Arc<Mutex<Vec<Value>>> = Default::default();
        let sink = std::sync::Arc::clone(&got);
        let channel = Channel::<Value>::new(move |body| {
            if let InvokeResponseBody::Json(text) = body {
                sink.lock()
                    .unwrap()
                    .push(serde_json::from_str(&text).unwrap());
            }
            Ok(())
        });
        (channel, got)
    }

    #[test]
    fn arc_5_a_new_brain_channel_gets_the_sticky_status_replayed() {
        let brain = BrainChannel::default();
        // No brain yet: the status is kept, not lost.
        assert!(!brain.send_sticky(
            events::HOTKEY_STATUS,
            status("unavailable", json!("hook_failed"))
        ));
        assert!(!brain.send(events::HOTKEY_PTT, json!({"mogId": "a1", "phase": "down"})));
        let (first, got) = recorder();
        brain.set(first);
        assert_eq!(
            *got.lock().unwrap(),
            vec![brain_message(
                events::HOTKEY_STATUS,
                status("unavailable", json!("hook_failed"))
            )],
            "only the sticky item is replayed"
        );
        // A live change goes out now and replaces the kept one.
        assert!(brain.send_sticky(events::HOTKEY_STATUS, status("active", Value::Null)));
        assert_eq!(got.lock().unwrap().len(), 2);
        // A brain reload subscribes again and gets the latest status once.
        let (second, again) = recorder();
        brain.set(second);
        assert_eq!(
            *again.lock().unwrap(),
            vec![brain_message(
                events::HOTKEY_STATUS,
                status("active", Value::Null)
            )]
        );
    }

    #[test]
    fn sec_01_a_replayed_sticky_payload_is_scrubbed() {
        let brain = BrainChannel::default();
        brain.send_sticky(
            events::HOTKEY_STATUS,
            json!({"talkKey": format!("x {SECRET} y")}),
        );
        let (ch, got) = recorder();
        brain.set(ch);
        let text = serde_json::to_string(&*got.lock().unwrap()).unwrap();
        assert!(!text.contains(SECRET), "{text}");
        assert!(text.contains(mog_core::token_guard::REDACTED), "{text}");
    }

    // ---- #509 (ADR 0091): the hub's private notice Channel ----

    #[test]
    fn mog_06d_hub_channel_delivers_only_to_the_hub_channel_and_keeps_nothing() {
        let notice =
            json!({"kind": "dictation_unshown", "mogId": "a1", "text": "test-dictation-not-real"});
        let hub = HubChannel::default();
        // No hub yet: `false`, so the brain keeps the text. The host keeps no copy.
        assert!(!hub.send(notice.clone()));
        let (first, got) = recorder();
        hub.set(first);
        assert!(got.lock().unwrap().is_empty(), "nothing is replayed");
        assert!(hub.send(notice.clone()));
        assert_eq!(*got.lock().unwrap(), vec![notice.clone()]);
        // A hub reload replaces the Channel; the old one gets no more notices.
        let (second, again) = recorder();
        hub.set(second);
        assert!(hub.send(notice.clone()));
        assert_eq!(got.lock().unwrap().len(), 1);
        assert_eq!(*again.lock().unwrap(), vec![notice]);
    }

    #[test]
    fn mog_06d_the_app_manages_the_hub_channel() {
        // `try_state` turns a missing `HubChannel` into a silent `host_failed` (#544 review).
        let lib = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
        let code = production_code(&std::fs::read_to_string(lib).unwrap());
        assert!(
            code.contains(".manage(ipc::HubChannel::default())"),
            "lib.rs must manage ipc::HubChannel"
        );
    }

    #[test]
    fn mem_36a_mem_34a_issue_426_the_app_manages_one_store_that_mem_local_exec_reads() {
        // `try_state` turns a missing store into a silent `host_failed` for every
        // `mem_local_exec` call, so the setup must manage it. One store per app: the consent
        // host opens it, and R14 WipeService and `mem_local_exec` share it (#431, #124, #426).
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let lib = production_code(&std::fs::read_to_string(src.join("lib.rs")).unwrap());
        assert!(
            lib.contains("consent_host::install(handle)"),
            "lib.rs setup must call consent_host::install"
        );
        let host = production_code(&std::fs::read_to_string(src.join("consent_host.rs")).unwrap());
        assert!(
            host.contains("app.manage(store)"),
            "consent_host manages the store"
        );
        assert_eq!(
            host.matches("SqlCipherLocalStore::new(").count(),
            0,
            "consent_host builds the one store through prepare_device_records"
        );
        let ipc = production_code(&std::fs::read_to_string(src.join("ipc.rs")).unwrap());
        assert!(
            ipc.contains(
                "try_state::<std::sync::Arc<mog_core::local_store::SqlCipherLocalStore>>()"
            ),
            "route_from reads the managed store"
        );
        // ADR 0113: the route goes through the ticket gate, which calls mem_local_exec.
        assert!(
            ipc.contains("crate::memory_host::gate_mem_exec("),
            "the route calls the memory_host ticket gate"
        );
        let gate = production_code(&std::fs::read_to_string(src.join("memory_host.rs")).unwrap());
        assert!(
            gate.contains("mog_core::local_store::mem_local_exec("),
            "the gate calls mem_local_exec, never LocalStore::exec (#37 review)"
        );
        for code in [&ipc, &gate] {
            assert!(
                !code.contains(".exec("),
                "the host never runs a statement itself"
            );
        }
        // No other host file builds a second store.
        for entry in std::fs::read_dir(&src).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let code = production_code(&std::fs::read_to_string(&path).unwrap());
            assert!(
                !code.contains("SqlCipherLocalStore::new("),
                "{} builds a second store",
                path.display()
            );
        }
    }

    #[test]
    fn rtr_21a_arc_7_the_app_installs_the_shared_agent_host() {
        // `try_state` turns a missing agent host into a silent `host_failed` for every
        // `agent_*` call (#307), so the production setup must install it.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let lib = production_code(&std::fs::read_to_string(src.join("lib.rs")).unwrap());
        assert!(
            lib.contains("agent_host::install(handle)"),
            "lib.rs setup must call agent_host::install"
        );
        let host = production_code(&std::fs::read_to_string(src.join("agent_host.rs")).unwrap());
        assert!(
            host.contains("SharedAgentHost::new("),
            "one SharedAgentHost"
        );
        assert!(host.contains(".manage("), "agent_host::install manages it");
        let ipc = production_code(&std::fs::read_to_string(src.join("ipc.rs")).unwrap());
        assert!(
            ipc.contains("try_state::<crate::agent_host::AgentHostApp>()"),
            "route_from reads the managed agent host"
        );
    }
}
