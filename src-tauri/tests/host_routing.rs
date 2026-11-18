//! #285, #331, #333 (ADR 0040): the Tauri host routes every command through the hardened
//! gateway. `HostComponents` runs the host actions (`view_emit`, `brain_subscribe`, the four
//! `window_*` ops, `hotkeys_set`) behind fakes, so no Tauri runtime is needed except for the
//! cap test, which uses the real R11 host on the Tauri mock runtime.

use bottle_lib::hotkey_settings::{
    HotkeySettings, HotkeySettingsStore, MemorySettingsStore, SaveError, TalkKeySink,
};
use bottle_lib::ipc::{
    rejection, route, runs_off_main_thread, schedule, BrainLink, HostComponents, HotkeysRoute,
    PermissionRoute,
};
use bottle_lib::window_host::{TauriWindowHost, WindowError};
use mog_core::app_settings::{AppSettings, MicPreference};
use mog_core::error::{CoreError, CoreResult};
use mog_core::hotkey_bindings::{DictationShortcut, Os, TalkBinding};
use mog_core::hotkey_core::{PermissionStatus, UserAction};
use mog_core::ipc::{events, ChannelRef, Geometry, JsonObject, PermissionKind, COMMANDS};
use mog_core::ipc_gateway::{GatewayComponents, ERROR_CODES};
use mog_core::keyboard_env::EnvReads;
use mog_core::window_host::WindowHost;
use serde_json::{json, Value};
use std::sync::Mutex;

// ---------- fakes ----------

/// One call that reached the fake R11 host.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    Open(String, Option<Geometry>),
    Close(String),
    Show(String),
    Hide(String),
}

/// A fake R11 host: it records each call and answers with `answer`.
#[derive(Default)]
struct FakeWindows {
    ops: Mutex<Vec<Op>>,
    answer: Option<&'static str>,
}

impl FakeWindows {
    fn failing(code: &'static str) -> Self {
        Self {
            ops: Mutex::default(),
            answer: Some(code),
        }
    }
    fn ops(&self) -> Vec<Op> {
        self.ops.lock().unwrap().clone()
    }
    fn reply(&self, op: Op) -> CoreResult<()> {
        self.ops.lock().unwrap().push(op);
        match self.answer {
            None => Ok(()),
            Some(code) => Err(CoreError::IpcRefused { code }),
        }
    }
}

impl WindowHost for FakeWindows {
    fn open_mog_window(&self, mog_id: &str, geometry: Option<&Geometry>) -> CoreResult<()> {
        self.reply(Op::Open(mog_id.to_owned(), geometry.cloned()))
    }
    fn close_mog_window(&self, mog_id: &str) -> CoreResult<()> {
        self.reply(Op::Close(mog_id.to_owned()))
    }
    fn show_card(&self, mog_id: &str) -> CoreResult<()> {
        self.reply(Op::Show(mog_id.to_owned()))
    }
    fn hide_card(&self, mog_id: &str) -> CoreResult<()> {
        self.reply(Op::Hide(mog_id.to_owned()))
    }
}

/// A fake brain: it keeps every relayed item and every subscribed Channel.
#[derive(Default)]
struct FakeBrain {
    items: Mutex<Vec<(String, Value)>>,
    channels: Mutex<Vec<String>>,
    absent: bool,
    /// #509: the notices sent on the hub's private Channel, and the hub Channels kept.
    hub_notices: Mutex<Vec<Value>>,
    hub_channels: Mutex<Vec<String>>,
    no_hub: bool,
}

impl FakeBrain {
    fn gone() -> Self {
        Self {
            absent: true,
            ..Self::default()
        }
    }
    fn items(&self) -> Vec<(String, Value)> {
        self.items.lock().unwrap().clone()
    }
}

impl BrainLink for FakeBrain {
    fn relay(&self, event: &str, payload: Value) -> bool {
        if self.absent {
            return false;
        }
        self.items.lock().unwrap().push((event.to_owned(), payload));
        true
    }
    fn subscribe(&self, channel: &ChannelRef) -> bool {
        self.channels
            .lock()
            .unwrap()
            .push(channel.as_str().to_owned());
        !self.absent
    }
    fn notify_hub(&self, notice: Value) -> bool {
        if self.no_hub {
            return false;
        }
        self.hub_notices.lock().unwrap().push(notice);
        true
    }
    fn subscribe_hub(&self, channel: &ChannelRef) -> bool {
        self.hub_channels
            .lock()
            .unwrap()
            .push(channel.as_str().to_owned());
        true
    }
}

/// A fake `hotkeys_set` route with a fixed answer.
struct FakeHotkeys(CoreResult<Value>, Mutex<Vec<JsonObject>>);

impl HotkeysRoute for FakeHotkeys {
    fn set_hotkeys(&self, bindings: &JsonObject) -> CoreResult<Value> {
        self.1.lock().unwrap().push(bindings.clone());
        self.0.clone()
    }
}

fn comps<'a>(
    from: &'a str,
    brain: &'a FakeBrain,
    windows: Option<&'a dyn WindowHost>,
    hotkeys: Option<&'a dyn HotkeysRoute>,
) -> HostComponents<'a> {
    HostComponents {
        windows,
        hotkeys,
        ..HostComponents::none(from, brain)
    }
}

fn brain_call(windows: &FakeWindows, command: &str, args: Value) -> Result<Value, Value> {
    let brain = FakeBrain::default();
    route(comps("brain", &brain, Some(windows), None), command, args)
}

fn err(code: &str) -> Result<Value, Value> {
    Err(json!({ "error": code }))
}

// ---------- window ops (ARC-3, MOG-18a, MB-03b) ----------

#[test]
fn arc_3_window_mog_open_reaches_r11_with_its_geometry() {
    let w = FakeWindows::default();
    let g = json!({"x": 10.5, "y": 20.0, "width": 160.0, "height": 160.0});
    assert_eq!(
        brain_call(&w, "window_mog_open", json!({"mogId": "a1", "geometry": g})),
        Ok(json!({}))
    );
    assert_eq!(
        brain_call(&w, "window_mog_open", json!({"mogId": "b2"})),
        Ok(json!({}))
    );
    let geometry = Geometry {
        x: 10.5,
        y: 20.0,
        width: 160.0,
        height: 160.0,
    };
    assert_eq!(
        w.ops(),
        vec![
            Op::Open("a1".into(), Some(geometry)),
            Op::Open("b2".into(), None)
        ]
    );
}

#[test]
fn arc_3_window_mog_open_refuses_a_size_that_is_not_positive() {
    let w = FakeWindows::default();
    for (width, height) in [(0.0, 10.0), (10.0, -1.0)] {
        let g = json!({"x": 0.0, "y": 0.0, "width": width, "height": height});
        assert_eq!(
            brain_call(&w, "window_mog_open", json!({"mogId": "a1", "geometry": g})),
            err("invalid_args")
        );
    }
    assert!(w.ops().is_empty(), "a bad geometry never reached R11");
}

#[test]
fn arc_3_window_mog_close_reaches_r11() {
    let w = FakeWindows::default();
    assert_eq!(
        brain_call(&w, "window_mog_close", json!({"mogId": "a1"})),
        Ok(json!({}))
    );
    assert_eq!(w.ops(), vec![Op::Close("a1".into())]);
}

#[test]
fn mog_18a_window_card_show_and_hide_reach_r11() {
    let w = FakeWindows::default();
    assert_eq!(
        brain_call(&w, "window_card_show", json!({"mogId": "a1"})),
        Ok(json!({}))
    );
    assert_eq!(
        brain_call(&w, "window_card_hide", json!({"mogId": "a1"})),
        Ok(json!({}))
    );
    assert_eq!(w.ops(), vec![Op::Show("a1".into()), Op::Hide("a1".into())]);
}

#[test]
fn arc_3_a_view_cannot_run_a_window_op() {
    let w = FakeWindows::default();
    let brain = FakeBrain::default();
    for label in ["hub", "mog-a1", "card-a1"] {
        let got = route(
            comps(label, &brain, Some(&w), None),
            "window_mog_open",
            json!({"mogId": "a1"}),
        );
        assert_eq!(got, err("window_not_allowed"), "{label}");
    }
    assert!(w.ops().is_empty(), "a refused call reached R11");
}

#[test]
fn arc_3_r11_refusal_codes_cross_unchanged() {
    for code in [
        "mog_cap_reached",
        "unknown_mog",
        "window_closing",
        "invalid_args",
    ] {
        let w = FakeWindows::failing(code);
        assert_eq!(
            brain_call(&w, "window_card_show", json!({"mogId": "a1"})),
            err(code)
        );
    }
}

#[test]
fn arc_3_no_window_host_means_host_failed() {
    let brain = FakeBrain::default();
    let got = route(
        comps("brain", &brain, None, None),
        "window_mog_close",
        json!({"mogId": "a1"}),
    );
    assert_eq!(got, err("host_failed"));
}

#[test]
fn arc_3_mb_03b_window_errors_map_to_ipc_codes() {
    for (e, code) in [
        (WindowError::InvalidId, "invalid_args"),
        (WindowError::CapReached { cap: 8 }, "mog_cap_reached"),
        (WindowError::UnknownMog, "unknown_mog"),
        (WindowError::Closing, "window_closing"),
        (WindowError::HostFailed, "host_failed"),
        (WindowError::MainThread, "host_failed"),
    ] {
        assert_eq!(e.ipc_code(), code, "{e:?}");
        assert!(ERROR_CODES.contains(&code));
        assert_eq!(CoreError::from(e), CoreError::IpcRefused { code });
    }
}

#[test]
fn mb_03b_the_ninth_mog_window_is_refused_with_mog_cap_reached() {
    use tauri::test::mock_app;
    let app = mock_app();
    let host = TauriWindowHost::new(app.handle().clone());
    let brain = FakeBrain::default();
    let open = |id: String| {
        route(
            comps("brain", &brain, Some(&host), None),
            "window_mog_open",
            json!({ "mogId": id }),
        )
    };
    for n in 0..8 {
        assert_eq!(open(format!("m{n}")), Ok(json!({})));
    }
    assert_eq!(open("m9".into()), err("mog_cap_reached"));
    assert_eq!(host.active_mogs(), 8);
}

#[test]
fn mog_18a_r11_makes_the_card_on_the_first_show_through_the_route() {
    use tauri::test::mock_app;
    use tauri::Manager;
    let app = mock_app();
    let host = TauriWindowHost::new(app.handle().clone());
    let brain = FakeBrain::default();
    let call = |cmd: &str| {
        route(
            comps("brain", &brain, Some(&host), None),
            cmd,
            json!({"mogId": "m1"}),
        )
    };
    assert_eq!(call("window_card_show"), err("unknown_mog"));
    assert_eq!(
        route(
            comps("brain", &brain, Some(&host), None),
            "window_mog_open",
            json!({"mogId": "m1", "geometry": {"x": 40.0, "y": 50.0, "width": 160.0, "height": 160.0}}),
        ),
        Ok(json!({}))
    );
    assert!(app.get_webview_window("card-m1").is_none());
    assert_eq!(call("window_card_show"), Ok(json!({})));
    assert!(app.get_webview_window("card-m1").is_some());
    assert_eq!(call("window_card_hide"), Ok(json!({})));
    assert_eq!(call("window_mog_close"), Ok(json!({})));
    assert_eq!(call("window_mog_close"), err("unknown_mog"));
}

#[test]
fn arc_3_cap_02a_g_04a_window_ops_capture_start_and_mcp_call_run_off_the_main_thread() {
    // ADR 0022 decision 6: an R11 op may wait for the main thread, so it runs on a worker.
    // ADR 0039: `permission_request` waits for an OS prompt, so it runs on a worker too.
    // ADR 0055: `capture_start` and `capture_resume` open the device, which can be slow. The
    // stops stay inline (MB-05).
    // #467: `mcp_call` is a blocking HTTP call to Genie One MCP (R8, G-04a).
    for c in COMMANDS {
        assert_eq!(
            runs_off_main_thread(c.name),
            // `insert_text` waits about 0.4 s for the paste and the restore (#58, ADR 0048).
            c.name.starts_with("window_")
                || matches!(
                    c.name,
                    "permission_request"
                        | "capture_start"
                        | "capture_resume"
                        | "insert_text"
                        | "mcp_call"
                ),
            "{}",
            c.name
        );
    }
}

// ---------- insert_text (#58) ----------

/// The R11 paste component double: it records the checked calls and answers `inserted`.
#[derive(Default)]
struct FakeInsert(Mutex<Vec<(String, Value)>>);

impl GatewayComponents for FakeInsert {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.0.lock().unwrap().push((command.to_owned(), args));
        Ok(json!({"result": "inserted"}))
    }
}

#[test]
fn mog_06a_insert_text_goes_to_the_paste_component() {
    let brain = FakeBrain::default();
    let insert = FakeInsert::default();
    let mut c = comps("brain", &brain, None, None);
    c.insert = Some(&insert);
    let args = json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "a\nb"});
    assert_eq!(
        route(c, "insert_text", args.clone()),
        Ok(json!({"result": "inserted"}))
    );
    assert_eq!(
        *insert.0.lock().unwrap(),
        vec![("insert_text".to_owned(), args)]
    );
}

#[test]
fn mog_06a_insert_text_without_a_paste_host_is_host_failed() {
    let brain = FakeBrain::default();
    let got = route(
        comps("brain", &brain, None, None),
        "insert_text",
        json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "x"}),
    );
    assert_eq!(got, err("host_failed"));
}

#[test]
fn mog_06b_a_view_cannot_reach_the_paste_component() {
    let brain = FakeBrain::default();
    let insert = FakeInsert::default();
    let mut c = comps("mog-a1", &brain, None, None);
    c.insert = Some(&insert);
    let got = route(
        c,
        "insert_text",
        json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "x"}),
    );
    assert_eq!(got, err("window_not_allowed"));
    assert!(insert.0.lock().unwrap().is_empty());
}

// ---------- view_emit and brain_subscribe ----------

#[test]
fn arc_5_view_emit_goes_to_the_brain_as_view_input() {
    let brain = FakeBrain::default();
    let got = route(
        comps("card-a1", &brain, None, None),
        "view_emit",
        json!({"mogId": "a1", "kind": "cancel"}),
    );
    assert_eq!(got, Ok(json!({})));
    assert_eq!(
        brain.items(),
        vec![(
            events::VIEW_INPUT.to_owned(),
            json!({"from": "card-a1", "input": {"mogId": "a1", "kind": "cancel"}})
        )]
    );
}

#[test]
fn arc_5_view_emit_with_no_brain_is_host_failed() {
    let brain = FakeBrain::gone();
    let got = route(
        comps("mog-a1", &brain, None, None),
        "view_emit",
        json!({"mogId": "a1", "kind": "poked"}),
    );
    assert_eq!(got, err("host_failed"));
}

#[test]
fn arc_5_brain_subscribe_keeps_the_channel() {
    let brain = FakeBrain::default();
    let got = route(
        comps("brain", &brain, None, None),
        "brain_subscribe",
        json!({"onEvent": "__CHANNEL__:7"}),
    );
    assert_eq!(got, Ok(json!({})));
    assert_eq!(*brain.channels.lock().unwrap(), vec!["__CHANNEL__:7"]);
    let gone = FakeBrain::gone();
    let got = route(
        comps("brain", &gone, None, None),
        "brain_subscribe",
        json!({"onEvent": "__CHANNEL__:8"}),
    );
    assert_eq!(got, err("host_failed"));
}

#[test]
fn arc_4_commands_without_a_host_action_stay_not_implemented() {
    let brain = FakeBrain::default();
    let w = FakeWindows::default();
    let got = route(
        comps("brain", &brain, Some(&w), None),
        "profile_status",
        json!({"profile": "p"}),
    );
    assert_eq!(got, err("not_implemented"));
    let got = route(
        comps("brain", &brain, Some(&w), None),
        "mog_set_active",
        json!({"mogId": "a1"}),
    );
    assert_eq!(got, err("not_implemented"));
    assert!(w.ops().is_empty() && brain.items().is_empty());
}

// ---------- hotkeys_set (CAP-43b, MB-14) ----------

#[derive(Default)]
struct NoEnv;

impl EnvReads for NoEnv {
    fn dictation(&self) -> Option<DictationShortcut> {
        None
    }
    fn voice_over(&self) -> bool {
        false
    }
    fn alt_gr_layout(&self) -> bool {
        false
    }
    fn copilot_key(&self) -> bool {
        false
    }
    fn visit_process_names(&self, _visit: &mut dyn FnMut(&str)) {}
}

struct NoTalk;

impl TalkKeySink for NoTalk {
    fn apply(&self, _talk: TalkBinding) {}
}

/// A store that has saved values but cannot save again.
struct BrokenStore;

impl HotkeySettingsStore for BrokenStore {
    fn load(&self) -> Option<JsonObject> {
        Some(JsonObject::new())
    }
    fn save(&self, _values: &JsonObject) -> Result<(), SaveError> {
        Err(SaveError("settings_save_failed"))
    }
}

fn hotkeys(store: Box<dyn HotkeySettingsStore>) -> HotkeySettings {
    HotkeySettings::load(Os::MacOs, store, Box::new(NoEnv), Box::new(NoTalk))
}

fn set(from: &str, h: Option<&dyn HotkeysRoute>, bindings: Value) -> Result<Value, Value> {
    let brain = FakeBrain::default();
    route(
        comps(from, &brain, None, h),
        "hotkeys_set",
        json!({ "bindings": bindings }),
    )
}

#[test]
fn cap_43b_hotkeys_set_reaches_the_host_settings_and_returns_conflicts() {
    let settings = hotkeys(Box::new(MemorySettingsStore::default()));
    // First run and Settings read the conflicts with an empty object (ADR 0020).
    let got = set("hub", Some(&settings), json!({})).unwrap();
    assert!(got["conflicts"].is_array(), "{got}");
    let before = settings.bindings();
    let got = set("brain", Some(&settings), json!({"session": "Ctrl+Shift+J"})).unwrap();
    assert!(got["conflicts"].is_array(), "{got}");
    assert_ne!(settings.bindings(), before, "the new key is saved");
}

#[test]
fn cap_43b_hotkeys_set_unknown_action_is_invalid_args() {
    let settings = hotkeys(Box::new(MemorySettingsStore::default()));
    assert_eq!(
        set("brain", Some(&settings), json!({"bogus": "Ctrl+Shift+B"})),
        err("invalid_args")
    );
}

#[test]
fn mb_14_hotkeys_set_save_failure_is_host_failed() {
    let settings = hotkeys(Box::new(BrokenStore));
    assert_eq!(
        set("brain", Some(&settings), json!({"session": "Ctrl+Shift+J"})),
        err("host_failed")
    );
}

#[test]
fn cap_43b_hotkeys_set_without_managed_settings_is_host_failed() {
    assert_eq!(set("hub", None, json!({})), err("host_failed"));
}

#[test]
fn cap_43b_hotkeys_set_passes_the_bindings_and_maps_other_errors() {
    let fake = FakeHotkeys(
        Err(CoreError::Unavailable {
            component: "R1",
            reason: "some_new_reason",
        }),
        Mutex::default(),
    );
    assert_eq!(
        set("brain", Some(&fake), json!({"ptt": "RightOption"})),
        err("host_failed")
    );
    assert_eq!(
        fake.1.lock().unwrap().clone(),
        vec![serde_json::from_value::<JsonObject>(json!({"ptt": "RightOption"})).unwrap()]
    );
    // A view window may not set keys.
    assert_eq!(
        set("mog-a1", Some(&fake), json!({})),
        err("window_not_allowed")
    );
}

// ---------- SEC-03: rejections are a code only ----------

const SECRET: &str = concat!("dap", "i0123456789abcdef0123456789abcdef");

#[test]
fn sec_03_every_rejection_is_only_an_error_code() {
    let w = FakeWindows::default();
    let brain = FakeBrain::default();
    let label = format!("mog-{}", &SECRET[..12]);
    let cases: Vec<(&str, &str, Value)> = vec![
        // wrong window, with a label that holds a value
        (&label, "cli_status", json!({})),
        // bad arguments that hold a value
        (
            "brain",
            "window_mog_open",
            json!({"mogId": SECRET, "x": SECRET}),
        ),
        ("brain", "hotkeys_set", json!({"bindings": SECRET})),
        // a checked call with no host action
        (
            "brain",
            "insert_text",
            json!({"mogId": "a1", "text": SECRET}),
        ),
        // a view that speaks for another MOG
        (
            "mog-a1",
            "view_emit",
            json!({"mogId": "b2", "kind": SECRET}),
        ),
    ];
    for (from, command, args) in cases {
        let got = route(comps(from, &brain, Some(&w), None), command, args).unwrap_err();
        let obj = got.as_object().expect("an object");
        assert_eq!(obj.len(), 1, "{command}: {got}");
        let code = obj["error"].as_str().unwrap();
        assert!(ERROR_CODES.contains(&code), "{command}: {code}");
        let text = got.to_string();
        assert!(!text.contains(&SECRET[..12]), "{command} leaked: {text}");
        assert!(
            !text.contains(command),
            "{command} named the command: {text}"
        );
    }
}

#[test]
fn sec_03_a_component_code_outside_the_list_becomes_host_failed() {
    assert_eq!(
        rejection(&CoreError::Unavailable {
            component: "R1",
            reason: "settings_save_failed",
        }),
        json!({"error": "host_failed"})
    );
    assert_eq!(
        rejection(&CoreError::IpcRefused { code: "mic_busy" }),
        json!({"error": "mic_busy"})
    );
    assert_eq!(
        rejection(&CoreError::NotImplemented { component: "R2" }),
        json!({"error": "not_implemented"})
    );
}

#[test]
fn g_05b_g_14_w_14_r8_codes_reach_the_window_as_themselves() {
    // #479: the R8 codes that the GL-5 mapper names cross as themselves; an R8 internal code
    // still crosses as `host_failed`.
    for reason in [
        "genie_one_off",
        "auth_expired",
        "rate_limited",
        "non_json_reply",
        "timeout",
        "offline",
        "host_refused",
        "redirect_refused",
        "reply_too_large",
        "bad_reply",
        "protocol_unsupported",
        "tool_not_allowed",
        "invalid_tool_args",
        "invalid_meta",
        "profile_invalid",
        "poll_inflight",
    ] {
        assert_eq!(
            rejection(&CoreError::Unavailable {
                component: "R8",
                reason,
            }),
            json!({ "error": reason })
        );
    }
    assert_eq!(
        rejection(&CoreError::Unavailable {
            component: "R8",
            reason: "transport_failed",
        }),
        json!({"error": "host_failed"})
    );
}

#[test]
fn sec_01_host_component_replies_are_scrubbed() {
    // The gateway's rule-7 scrub covers the host replies: a reply never carries a token.
    let fake = FakeHotkeys(
        Ok(json!({"conflicts": [{"app": format!("x {SECRET} y")}]})),
        Mutex::default(),
    );
    let got = set("brain", Some(&fake), json!({})).unwrap();
    assert!(!mog_core::token_guard::json_contains_secret(&got), "{got}");
    assert!(!got.to_string().contains(SECRET), "{got}");
    assert_eq!(got["conflicts"].as_array().map(Vec::len), Some(1));
}

#[test]
fn mb_14_hotkeys_set_route_does_not_save_a_duplicate_binding() {
    let settings = hotkeys(Box::new(MemorySettingsStore::default()));
    let before = settings.bindings();
    // The open-card key asks for the chord of the panic key (Mac default).
    let got = set(
        "hub",
        Some(&settings),
        json!({"openCard": "Cmd+Shift+Esc", "session": "Ctrl+Shift+J"}),
    )
    .unwrap();
    let kinds: Vec<&str> = got["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"duplicate"), "{got}");
    let after = settings.bindings();
    use mog_core::hotkey_bindings::HotkeyAction;
    assert_eq!(
        after.get(HotkeyAction::OpenCard),
        before.get(HotkeyAction::OpenCard)
    );
    assert_ne!(
        after.get(HotkeyAction::Session),
        before.get(HotkeyAction::Session)
    );
}

#[test]
fn arc_3_cap_02a_g_04a_schedule_runs_window_ops_capture_start_and_mcp_call_on_a_worker() {
    use std::sync::mpsc;
    let caller = std::thread::current().id();
    for c in COMMANDS {
        let (tx, rx) = mpsc::channel();
        schedule(c.name, move || {
            let _ = tx.send(std::thread::current().id());
        });
        let ran_on = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("the job ran");
        let worker = c.name.starts_with("window_")
            || matches!(
                c.name,
                "permission_request"
                    | "capture_start"
                    | "capture_resume"
                    | "insert_text"
                    | "mcp_call"
            );
        if worker {
            assert_ne!(ran_on, caller, "{} ran on the calling thread", c.name);
        } else {
            assert_eq!(ran_on, caller, "{} left the calling thread", c.name);
        }
    }
}

#[test]
fn arc_5_an_inline_command_has_run_when_schedule_returns() {
    // view_emit order: an inline job is done before the next call is scheduled.
    let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&done);
    schedule("view_emit", move || {
        flag.store(true, std::sync::atomic::Ordering::SeqCst)
    });
    assert!(done.load(std::sync::atomic::Ordering::SeqCst));
}

// ---------- permission_request (CAP-36, CAP-37, #312, ADR 0039) ----------

/// A fake permission route: it records each ask and answers with `answer`.
struct FakePermissionRoute {
    asked: Mutex<Vec<PermissionKind>>,
    answer: PermissionStatus,
}

impl FakePermissionRoute {
    fn answering(answer: PermissionStatus) -> Self {
        Self {
            asked: Mutex::default(),
            answer,
        }
    }
    fn asked(&self) -> Vec<PermissionKind> {
        self.asked.lock().unwrap().clone()
    }
}

impl PermissionRoute for FakePermissionRoute {
    fn request_permission(
        &self,
        permission: PermissionKind,
        _action: UserAction,
    ) -> PermissionStatus {
        self.asked.lock().unwrap().push(permission);
        self.answer
    }
}

fn ask(from: &str, route_to: Option<&dyn PermissionRoute>, args: Value) -> Result<Value, Value> {
    let brain = FakeBrain::default();
    let components = HostComponents {
        permissions: route_to,
        ..HostComponents::none(from, &brain)
    };
    route(components, "permission_request", args)
}

#[test]
fn cap_36_permission_request_input_monitoring_reaches_the_ptt_permission_call() {
    let p = FakePermissionRoute::answering(PermissionStatus::Granted);
    assert_eq!(
        ask("hub", Some(&p), json!({"permission": "input_monitoring"})),
        Ok(json!({"status": "granted"}))
    );
    assert_eq!(p.asked(), vec![PermissionKind::InputMonitoring]);
}

#[test]
fn cap_37_permission_request_accessibility_reaches_the_insertion_permission_call() {
    for (answer, status) in [
        (PermissionStatus::Granted, "granted"),
        (PermissionStatus::Denied, "denied"),
        (PermissionStatus::Unknown, "unknown"),
    ] {
        let p = FakePermissionRoute::answering(answer);
        assert_eq!(
            ask("hub", Some(&p), json!({"permission": "accessibility"})),
            Ok(json!({ "status": status }))
        );
        assert_eq!(p.asked(), vec![PermissionKind::Accessibility]);
    }
}

#[test]
fn cap_36_cap_37_permission_request_is_hub_only() {
    let p = FakePermissionRoute::answering(PermissionStatus::Granted);
    for label in ["brain", "mog-a1", "card-a1"] {
        assert_eq!(
            ask(label, Some(&p), json!({"permission": "input_monitoring"})),
            err("window_not_allowed"),
            "{label}"
        );
    }
    assert!(p.asked().is_empty(), "a refused call reached the OS prompt");
}

#[test]
fn cap_36_cap_37_permission_request_bad_args_are_invalid_args() {
    let p = FakePermissionRoute::answering(PermissionStatus::Granted);
    for bad in [
        json!({}),
        json!({"permission": "camera"}),
        json!({"permission": "accessibility", "prompt": true}),
        json!({"permission": SECRET}),
    ] {
        assert_eq!(
            ask("hub", Some(&p), bad.clone()),
            err("invalid_args"),
            "{bad}"
        );
    }
    assert!(p.asked().is_empty());
}

#[test]
fn cap_36_permission_request_without_the_hotkey_runtime_is_host_failed() {
    assert_eq!(
        ask("hub", None, json!({"permission": "input_monitoring"})),
        err("host_failed")
    );
}

// ---------- capture_resume (CAP-02a, #279, ADR 0039, ADR 0055) ----------

#[test]
fn cap_02a_capture_resume_is_brain_only_and_needs_the_capture_host() {
    let brain = FakeBrain::default();
    let lease = json!({"mogId": "a1", "leaseId": "17"});
    // #425 routes it to R2; with no managed capture host the answer is `host_failed`.
    assert_eq!(
        route(
            comps("brain", &brain, None, None),
            "capture_resume",
            lease.clone()
        ),
        err("host_failed")
    );
    for label in ["hub", "mog-a1", "card-a1"] {
        assert_eq!(
            route(
                comps(label, &brain, None, None),
                "capture_resume",
                lease.clone()
            ),
            err("window_not_allowed"),
            "{label}"
        );
    }
    assert_eq!(
        route(
            comps("brain", &brain, None, None),
            "capture_resume",
            json!({"mogId": "a1", "leaseId": "a b"})
        ),
        err("invalid_args")
    );
    assert!(brain.items().is_empty());
}

// ---------- consent records (#430, ADR 0060) ----------

/// A fake consent component: it keeps each call and answers with a fixed reply.
#[derive(Default)]
struct FakeConsent(Mutex<Vec<(String, Value)>>);

impl GatewayComponents for FakeConsent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.0.lock().unwrap().push((command.to_owned(), args));
        Ok(json!({"recordId": "rec-1"}))
    }
}

fn consent_call(
    from: &str,
    consent: Option<&dyn GatewayComponents>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let brain = FakeBrain::default();
    let components = HostComponents {
        consent,
        ..HostComponents::none(from, &brain)
    };
    route(components, command, args)
}

#[test]
fn sec_32a_consent_commands_reach_the_consent_component() {
    let c = FakeConsent::default();
    assert_eq!(
        consent_call(
            "brain",
            Some(&c),
            "consent_notice_record",
            json!({"noticeVersion": 2})
        ),
        Ok(json!({"recordId": "rec-1"}))
    );
    assert_eq!(
        consent_call("brain", Some(&c), "consent_records_list", json!({})),
        Ok(json!({"recordId": "rec-1"}))
    );
    assert_eq!(
        *c.0.lock().unwrap(),
        vec![
            (
                "consent_notice_record".to_owned(),
                json!({"noticeVersion": 2})
            ),
            ("consent_records_list".to_owned(), json!({})),
        ]
    );
}

#[test]
fn sec_03_no_managed_consent_store_answers_consent_storage_failed() {
    for (command, args) in [
        ("consent_notice_record", json!({"noticeVersion": 1})),
        ("consent_records_list", json!({})),
    ] {
        assert_eq!(
            consent_call("brain", None, command, args),
            err("consent_storage_failed"),
            "{command}"
        );
    }
}

#[test]
fn sec_02a_views_cannot_call_the_consent_commands() {
    let c = FakeConsent::default();
    for from in ["hub", "mog-a1", "card-a1"] {
        for (command, args) in [
            ("consent_notice_record", json!({"noticeVersion": 1})),
            ("consent_records_list", json!({})),
        ] {
            assert_eq!(
                consent_call(from, Some(&c), command, args),
                err("window_not_allowed"),
                "{from} {command}"
            );
        }
    }
    assert!(c.0.lock().unwrap().is_empty(), "no view call reached T12");
}

// ---------- app settings (CAP-22, #453) ----------

/// A fake mic: it keeps each preference the settings give it.
#[derive(Default)]
struct FakeMic(Mutex<Vec<bool>>);

impl MicPreference for FakeMic {
    fn set_prefer_built_in(&self, on: bool) {
        self.0.lock().unwrap().push(on);
    }
}

fn settings_call(
    from: &str,
    settings: Option<&dyn GatewayComponents>,
    command: &str,
    args: Value,
) -> Result<Value, Value> {
    let brain = FakeBrain::default();
    let components = HostComponents {
        app_settings: settings,
        ..HostComponents::none(from, &brain)
    };
    route(components, command, args)
}

const PREFER: &str = "app.capture.preferBuiltInMic";

#[test]
fn cap_22_app_setting_read_answers_the_default_off() {
    let settings = AppSettings::default();
    for from in ["brain", "hub"] {
        assert_eq!(
            settings_call(
                from,
                Some(&settings),
                "app_setting_read",
                json!({"key": PREFER})
            ),
            Ok(json!({"key": PREFER, "value": false})),
            "{from}"
        );
    }
}

#[test]
fn cap_22_app_setting_write_reaches_the_mic_and_read_returns_it() {
    let mic = std::sync::Arc::new(FakeMic::default());
    let settings = AppSettings::default();
    settings.set_mic_preference(mic.clone());
    assert_eq!(
        settings_call(
            "hub",
            Some(&settings),
            "app_setting_write",
            json!({"key": PREFER, "value": true})
        ),
        Ok(json!({"key": PREFER, "value": true}))
    );
    assert_eq!(
        settings_call(
            "hub",
            Some(&settings),
            "app_setting_read",
            json!({"key": PREFER})
        ),
        Ok(json!({"key": PREFER, "value": true}))
    );
    // The hook got the current value when it was set, then the write.
    assert_eq!(*mic.0.lock().unwrap(), vec![false, true]);
}

#[test]
fn cap_22_mog_and_card_views_cannot_call_the_app_setting_commands() {
    let mic = std::sync::Arc::new(FakeMic::default());
    let settings = AppSettings::default();
    settings.set_mic_preference(mic.clone());
    for from in ["mog-a1", "card-a1"] {
        for (command, args) in [
            ("app_setting_read", json!({"key": PREFER})),
            ("app_setting_write", json!({"key": PREFER, "value": true})),
        ] {
            assert_eq!(
                settings_call(from, Some(&settings), command, args),
                err("window_not_allowed"),
                "{from} {command}"
            );
        }
    }
    assert_eq!(
        *mic.0.lock().unwrap(),
        vec![false],
        "no refused write reached the mic"
    );
}

#[test]
fn cap_22_no_managed_settings_is_host_failed() {
    for (command, args) in [
        ("app_setting_read", json!({"key": PREFER})),
        ("app_setting_write", json!({"key": PREFER, "value": true})),
    ] {
        assert_eq!(
            settings_call("hub", None, command, args),
            err("host_failed")
        );
    }
}

// ---------- #509 the hub-only notice for an unshown dictation (ADR 0091) ----------

const MARKER: &str = "test-dictation-not-real";

fn unshown() -> Value {
    json!({"kind": "dictation_unshown", "mogId": "a1", "text": MARKER})
}

#[test]
fn mog_05b_mog_06d_hub_notice_send_goes_only_to_the_hub_channel() {
    let brain = FakeBrain::default();
    let got = route(comps("brain", &brain, None, None), "hub_notice_send", unshown());
    assert_eq!(got, Ok(json!({})));
    assert_eq!(*brain.hub_notices.lock().unwrap(), vec![unshown()]);
    // Not relayed on the brain Channel, and no Tauri event (see `arc_5_core_to_brain_items_are_never_tauri_events`).
    assert!(brain.items().is_empty());
}

#[test]
fn mog_06d_hub_notice_send_with_no_hub_is_host_failed() {
    // The brain then keeps the text (#498): nothing is dropped silently.
    let brain = FakeBrain {
        no_hub: true,
        ..FakeBrain::default()
    };
    let got = route(comps("brain", &brain, None, None), "hub_notice_send", unshown());
    assert_eq!(got, err("host_failed"));
}

#[test]
fn sec_02a_no_view_can_send_or_forward_a_hub_notice() {
    for label in ["hub", "mog-a1", "card-a1"] {
        let brain = FakeBrain::default();
        let got = route(comps(label, &brain, None, None), "hub_notice_send", unshown());
        assert_eq!(got, err("window_not_allowed"), "{label}");
        assert!(brain.hub_notices.lock().unwrap().is_empty(), "{label}");
        assert!(brain.items().is_empty(), "{label}");
    }
}

#[test]
fn sec_02a_only_the_hub_window_gets_the_notice_channel() {
    let args = json!({"onNotice": "__CHANNEL__:5"});
    let brain = FakeBrain::default();
    let got = route(
        comps("hub", &brain, None, None),
        "view_notice_subscribe",
        args.clone(),
    );
    assert_eq!(got, Ok(json!({})));
    assert_eq!(*brain.hub_channels.lock().unwrap(), vec!["__CHANNEL__:5"]);
    // A MOG or card window (or the brain) can never take the Channel, so it never gets a notice.
    for label in ["mog-a1", "card-a1", "brain"] {
        let brain = FakeBrain::default();
        let got = route(
            comps(label, &brain, None, None),
            "view_notice_subscribe",
            args.clone(),
        );
        assert_eq!(got, err("window_not_allowed"), "{label}");
        assert!(brain.hub_channels.lock().unwrap().is_empty(), "{label}");
    }
}

#[test]
fn sec_03_a_refused_hub_notice_never_echoes_the_text() {
    let brain = FakeBrain::default();
    let mut bad = unshown();
    bad["extra"] = json!(MARKER);
    for (label, args) in [("brain", bad), ("mog-a1", unshown())] {
        let got = route(comps(label, &brain, None, None), "hub_notice_send", args);
        let shown = format!("{got:?}");
        assert!(!shown.contains(MARKER), "{shown}");
    }
}
