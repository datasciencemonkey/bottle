//! #285, #331, #333 (ADR 0040): the Tauri host routes every command through the hardened
//! gateway. `HostComponents` runs the host actions (`view_emit`, `brain_subscribe`, the four
//! `window_*` ops, `hotkeys_set`) behind fakes, so no Tauri runtime is needed except for the
//! cap test, which uses the real R11 host on the Tauri mock runtime.

use bottle_lib::hotkey_settings::{
    HotkeySettings, HotkeySettingsStore, MemorySettingsStore, SaveError, TalkKeySink,
};
use bottle_lib::ipc::{
    rejection, route, runs_off_main_thread, BrainLink, HostComponents, HotkeysRoute,
};
use bottle_lib::window_host::{TauriWindowHost, WindowError};
use mog_core::error::{CoreError, CoreResult};
use mog_core::hotkey_bindings::{DictationShortcut, Os, TalkBinding};
use mog_core::ipc::{events, ChannelRef, Geometry, JsonObject, COMMANDS};
use mog_core::ipc_gateway::ERROR_CODES;
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
        from,
        brain,
        windows,
        hotkeys,
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
fn arc_3_window_ops_run_off_the_main_thread_and_other_commands_inline() {
    // ADR 0022 decision 6: an R11 op may wait for the main thread, so it runs on a worker.
    for c in COMMANDS {
        assert_eq!(
            runs_off_main_thread(c.name),
            c.name.starts_with("window_"),
            "{}",
            c.name
        );
    }
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
fn mb_14_hotkeys_set_unknown_action_is_invalid_args() {
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
fn sec_01_window_op_replies_are_scrubbed() {
    // The gateway's rule-7 scrub covers the host replies too: a reply never holds a token.
    let w = FakeWindows::default();
    let got = brain_call(&w, "window_card_show", json!({"mogId": "a1"})).unwrap();
    assert!(!mog_core::token_guard::json_contains_secret(&got));
}
