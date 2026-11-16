//! The host side of `hotkeys_set` and the first-run key check (#322, ADR 0020).
//!
//! [`HotkeySettings`] owns the `app.hotkeys.*` settings (Appendix F). It calls the pure logic
//! of `mog_core::hotkey_bindings` (#24, ADR 0017) with a [`KeyboardEnv`] from prompt-free OS
//! reads (`mog_core::keyboard_env`), saves the result, and gives a new talk key to the
//! platform HotkeyService through a [`TalkKeySink`].
//!
//! Seams:
//! - The IPC route of `hotkeys_set` to [`HotkeySettings::set`] is contract change #331:
//!   TODO(#331).
//! - Main has no settings store yet. The app uses [`MemorySettingsStore`], so a saved key
//!   lasts until quit: TODO(#77).
//! - The first-run screen shows [`HotkeySettings::first_run_conflicts`]: TODO(#43).
//!
//! Nothing here logs. A binding names only Bottle's own global keys.
//!
//! Requirements: CAP-43a, CAP-43b (AT-30), MB-14 (host save rule).

use mog_core::error::CoreResult;
use mog_core::hotkey_bindings::{HotkeyBindings, KeyboardEnv, Os, TalkBinding};
use mog_core::ipc::JsonObject;
use mog_core::keyboard_env::EnvReads;
use serde_json::Value;

/// A settings save failed. The code is content-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveError(pub &'static str);

/// The store of the `app.hotkeys.*` values (Appendix F), keyed by the setting key.
pub trait HotkeySettingsStore: Send + Sync {
    /// The saved values, or `None` if Bottle never saved them (first run).
    fn load(&self) -> Option<JsonObject>;
    /// Save every `app.hotkeys.*` value.
    fn save(&self, values: &JsonObject) -> Result<(), SaveError>;
}

/// A store in memory. The values last until quit: TODO(#77) moves them to the settings store.
#[derive(Debug, Default)]
pub struct MemorySettingsStore(std::sync::Mutex<Option<JsonObject>>);

impl HotkeySettingsStore for MemorySettingsStore {
    fn load(&self) -> Option<JsonObject> {
        None
    }
    fn save(&self, _values: &JsonObject) -> Result<(), SaveError> {
        Ok(())
    }
}

/// Receives the talk key when it changes. The app restarts the platform service.
pub trait TalkKeySink: Send + Sync {
    /// Use `talk` as the talk key from now on.
    fn apply(&self, talk: TalkBinding);
}

/// The bindings of saved `app.hotkeys.*` values. A missing or bad value gives the D66 default.
pub fn bindings_from_settings(os: Os, _values: &JsonObject) -> HotkeyBindings {
    mog_core::hotkey_bindings::default_bindings(os)
}

/// The `app.hotkeys.*` values of `bindings`.
pub fn settings_from_bindings(_os: Os, _bindings: &HotkeyBindings) -> JsonObject {
    JsonObject::new()
}

/// The host owner of the global key settings.
pub struct HotkeySettings {
    os: Os,
    _store: Box<dyn HotkeySettingsStore>,
    _env: Box<dyn EnvReads>,
    _talk: Box<dyn TalkKeySink>,
}

impl HotkeySettings {
    /// Load the saved keys. On first run (nothing saved) run the first-run check, and save
    /// the D66 defaults with the first free talk key of the ladder (CAP-43a, CAP-43b).
    pub fn load(
        os: Os,
        store: Box<dyn HotkeySettingsStore>,
        env: Box<dyn EnvReads>,
        talk: Box<dyn TalkKeySink>,
    ) -> Self {
        Self {
            os,
            _store: store,
            _env: env,
            _talk: talk,
        }
    }

    /// The keys now.
    pub fn bindings(&self) -> HotkeyBindings {
        mog_core::hotkey_bindings::default_bindings(self.os)
    }

    /// The talk key now.
    pub fn talk_binding(&self) -> TalkBinding {
        TalkBinding::RightOption
    }

    /// The `{conflicts}` of the first-run check, if this launch was the first run.
    /// The first-run screen shows them, each with its fix: TODO(#43).
    pub fn first_run_conflicts(&self) -> Option<Value> {
        None
    }

    /// The `hotkeys_set {bindings}` command: returns `{conflicts}` of
    /// `hotkey_bindings::hotkeys_set` with a fresh [`KeyboardEnv`]. Saves the new keys, but
    /// not a binding that duplicates another action (MB-14). A new talk key goes to the
    /// [`TalkKeySink`]. TODO(#331): the IPC route.
    pub fn set(&self, _input: &JsonObject) -> CoreResult<Value> {
        Ok(serde_json::json!({ "conflicts": [] }))
    }

    /// The keyboard environment now (prompt-free reads).
    pub fn keyboard_env(&self) -> KeyboardEnv {
        KeyboardEnv::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mog_core::hotkey_bindings::{
        self as hb, Binding, DictationShortcut, HotkeyAction, VoiceApp,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    const MAC: Os = Os::MacOs;
    const WIN: Os = Os::Windows;

    /// A store double that records every save.
    #[derive(Default, Clone)]
    struct FakeStore {
        saved: Arc<Mutex<Option<JsonObject>>>,
        saves: Arc<Mutex<Vec<JsonObject>>>,
        fail: bool,
    }

    impl HotkeySettingsStore for FakeStore {
        fn load(&self) -> Option<JsonObject> {
            self.saved.lock().unwrap().clone()
        }
        fn save(&self, values: &JsonObject) -> Result<(), SaveError> {
            if self.fail {
                return Err(SaveError("settings_save_failed"));
            }
            self.saves.lock().unwrap().push(values.clone());
            *self.saved.lock().unwrap() = Some(values.clone());
            Ok(())
        }
    }

    /// An [`EnvReads`] double with fixed values. It counts the reads.
    #[derive(Default, Clone)]
    struct FakeEnv {
        env: Arc<Mutex<KeyboardEnv>>,
        processes: Arc<Mutex<Vec<&'static str>>>,
        reads: Arc<Mutex<usize>>,
    }

    impl EnvReads for FakeEnv {
        fn dictation(&self) -> Option<DictationShortcut> {
            *self.reads.lock().unwrap() += 1;
            self.env.lock().unwrap().dictation
        }
        fn voice_over(&self) -> bool {
            self.env.lock().unwrap().voice_over
        }
        fn alt_gr_layout(&self) -> bool {
            *self.reads.lock().unwrap() += 1;
            self.env.lock().unwrap().alt_gr_layout
        }
        fn copilot_key(&self) -> bool {
            self.env.lock().unwrap().copilot_key
        }
        fn visit_process_names(&self, visit: &mut dyn FnMut(&str)) {
            for p in self.processes.lock().unwrap().iter() {
                visit(p);
            }
        }
    }

    #[derive(Default, Clone)]
    struct FakeTalk(Arc<Mutex<Vec<TalkBinding>>>);

    impl TalkKeySink for FakeTalk {
        fn apply(&self, talk: TalkBinding) {
            self.0.lock().unwrap().push(talk);
        }
    }

    fn settings(os: Os, store: &FakeStore, env: &FakeEnv, talk: &FakeTalk) -> HotkeySettings {
        HotkeySettings::load(
            os,
            Box::new(store.clone()),
            Box::new(env.clone()),
            Box::new(talk.clone()),
        )
    }

    fn obj(v: Value) -> JsonObject {
        serde_json::from_value(v).unwrap()
    }

    fn kinds(result: &Value) -> Vec<String> {
        result["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["kind"].as_str().unwrap().to_owned())
            .collect()
    }

    #[test]
    fn cap_43a_settings_round_trip_through_the_appendix_f_keys() {
        for os in [MAC, WIN] {
            let d = hb::default_bindings(os);
            let values = settings_from_bindings(os, &d);
            let keys: Vec<&str> = values.keys().map(String::as_str).collect();
            let mut expect: Vec<&str> = HotkeyAction::ALL.iter().map(|a| a.setting_key()).collect();
            expect.sort();
            assert_eq!(keys, expect);
            assert_eq!(bindings_from_settings(os, &values), d);
        }
        assert_eq!(
            settings_from_bindings(MAC, &hb::default_bindings(MAC))["app.hotkeys.ptt"],
            json!("RightOption")
        );
    }

    #[test]
    fn cap_43a_bad_or_missing_saved_values_fall_back_to_d66() {
        let values = obj(json!({
            "app.hotkeys.ptt": "RightControl",
            "app.hotkeys.panic": "not a key",
            "app.hotkeys.session": 7,
            "app.hotkeys.unknown": "Ctrl+Shift+U",
        }));
        let b = bindings_from_settings(MAC, &values);
        let mut expect = hb::default_bindings(MAC);
        expect.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightControl));
        assert_eq!(b, expect);
        // A chord is not a talk key: the talk key falls back to the default.
        let values = obj(json!({ "app.hotkeys.ptt": "Ctrl+Shift+P" }));
        assert_eq!(
            bindings_from_settings(MAC, &values),
            hb::default_bindings(MAC)
        );
    }

    #[test]
    fn cap_43b_first_run_saves_the_defaults_with_the_free_ladder_step() {
        let store = FakeStore::default();
        let env = FakeEnv::default();
        env.env.lock().unwrap().alt_gr_layout = true;
        let talk = FakeTalk::default();
        let s = settings(WIN, &store, &env, &talk);
        // AltGr takes Right-Alt, so the ladder gives Right-Ctrl.
        assert_eq!(s.talk_binding(), TalkBinding::RightControl);
        let report = hb::first_run_check(WIN, &env.env.lock().unwrap().clone());
        assert_eq!(s.bindings(), report.suggested);
        assert_eq!(
            store.saves.lock().unwrap().clone(),
            vec![settings_from_bindings(WIN, &report.suggested)]
        );
        // The first-run conflicts are the ones of `first_run_check`, each with a fix.
        let shown = s.first_run_conflicts().expect("first run");
        let expect: Vec<Value> = report.conflicts.iter().map(|c| c.to_json(WIN)).collect();
        assert_eq!(shown, json!({ "conflicts": expect }));
        assert!(kinds(&shown).contains(&"alt_gr".to_owned()));
        for c in shown["conflicts"].as_array().unwrap() {
            assert!(c["fix"].is_object(), "{c}");
        }
        // Load gives the key to the service through install, not through the sink.
        assert!(talk.0.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_43b_a_saved_store_skips_the_first_run_check() {
        let store = FakeStore::default();
        let mut saved = hb::default_bindings(MAC);
        saved.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightControl));
        *store.saved.lock().unwrap() = Some(settings_from_bindings(MAC, &saved));
        let env = FakeEnv::default();
        let s = settings(MAC, &store, &env, &FakeTalk::default());
        assert_eq!(s.bindings(), saved);
        assert_eq!(s.talk_binding(), TalkBinding::RightControl);
        assert_eq!(s.first_run_conflicts(), None);
        assert!(store.saves.lock().unwrap().is_empty());
        assert_eq!(
            *env.reads.lock().unwrap(),
            0,
            "no OS read without a first run"
        );
    }

    #[test]
    fn cap_43b_set_returns_the_conflicts_of_hotkey_bindings_hotkeys_set() {
        let store = FakeStore::default();
        let env = FakeEnv::default();
        let s = settings(MAC, &store, &env, &FakeTalk::default());
        // The environment changes after the first run: it is read again on each call.
        env.env.lock().unwrap().dictation = Some(DictationShortcut::ControlTwice);
        env.processes
            .lock()
            .unwrap()
            .extend(["Finder", "superwhisper"]);
        let input = obj(json!({
            "ptt": "RightControl",
            "panic": "Fn+K",
            "session": "Ctrl+Option+S",
            "openCard": "nonsense",
        }));
        let before = s.bindings();
        let got = s.set(&input).unwrap();
        let fresh = KeyboardEnv {
            dictation: Some(DictationShortcut::ControlTwice),
            voice_apps: vec![VoiceApp::Superwhisper],
            ..KeyboardEnv::default()
        };
        let expect = hb::hotkeys_set(MAC, &before, &input, &fresh).unwrap();
        assert_eq!(got, expect.result_json(MAC));
        for k in [
            "dictation",
            "fn_globe",
            "voice_over",
            "voice_app",
            "invalid",
        ] {
            assert!(kinds(&got).contains(&k.to_owned()), "missing {k}: {got}");
        }
        assert_eq!(s.keyboard_env(), fresh);
    }

    #[test]
    fn cap_43b_set_saves_the_new_keys_to_app_hotkeys() {
        let store = FakeStore::default();
        let s = settings(MAC, &store, &FakeEnv::default(), &FakeTalk::default());
        s.set(&obj(json!({ "session": "Ctrl+Shift+J" }))).unwrap();
        let saved = store.saved.lock().unwrap().clone().unwrap();
        assert_eq!(saved["app.hotkeys.session"], json!("Ctrl+Shift+J"));
        assert_eq!(saved["app.hotkeys.ptt"], json!("RightOption"));
        assert_eq!(
            s.bindings().get(HotkeyAction::Session),
            Some(hb::Binding::parse(MAC, "Ctrl+Shift+J").unwrap())
        );
    }

    #[test]
    fn cap_43b_empty_bindings_change_nothing_and_return_the_current_conflicts() {
        let store = FakeStore::default();
        let env = FakeEnv::default();
        let talk = FakeTalk::default();
        let s = settings(MAC, &store, &env, &talk);
        env.processes.lock().unwrap().push("Wispr Flow");
        let before = s.bindings();
        let got = s.set(&JsonObject::new()).unwrap();
        assert_eq!(kinds(&got), vec!["voice_app".to_owned()]);
        assert_eq!(s.bindings(), before);
        assert!(talk.0.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_43b_an_unknown_action_is_an_error_and_saves_nothing() {
        let store = FakeStore::default();
        let s = settings(MAC, &store, &FakeEnv::default(), &FakeTalk::default());
        let saves = store.saves.lock().unwrap().len();
        let err = s.set(&obj(json!({ "bogus": "Ctrl+Shift+B" }))).unwrap_err();
        assert_eq!(err.code(), "unknown_hotkey_action");
        assert_eq!(store.saves.lock().unwrap().len(), saves);
    }

    #[test]
    fn mb_14_a_duplicate_binding_shows_a_conflict_and_is_not_saved() {
        let store = FakeStore::default();
        let s = settings(MAC, &store, &FakeEnv::default(), &FakeTalk::default());
        let panic = s.bindings().get(HotkeyAction::Panic).unwrap();
        // The open-card key takes the panic key's chord, and the session key changes too.
        let got = s
            .set(&obj(
                json!({ "openCard": "Cmd+Shift+Esc", "session": "Ctrl+Shift+J" }),
            ))
            .unwrap();
        assert!(kinds(&got).contains(&"duplicate".to_owned()), "{got}");
        let saved = store.saved.lock().unwrap().clone().unwrap();
        assert_eq!(
            saved["app.hotkeys.openCard"],
            json!("Ctrl+Shift+O"),
            "duplicate not saved"
        );
        assert_eq!(
            saved["app.hotkeys.session"],
            json!("Ctrl+Shift+J"),
            "the rest is saved"
        );
        assert_eq!(s.bindings().get(HotkeyAction::Panic), Some(panic));
        assert_ne!(s.bindings().get(HotkeyAction::OpenCard), Some(panic));
    }

    #[test]
    fn mb_14_a_change_that_copies_a_later_action_is_not_saved_either() {
        let store = FakeStore::default();
        let s = settings(MAC, &store, &FakeEnv::default(), &FakeTalk::default());
        // The check names the later action (open card), but the panic key is the new one.
        let got = s.set(&obj(json!({ "panic": "Ctrl+Shift+O" }))).unwrap();
        assert!(kinds(&got).contains(&"duplicate".to_owned()), "{got}");
        let saved = store.saved.lock().unwrap().clone().unwrap();
        assert_eq!(saved["app.hotkeys.panic"], json!("Cmd+Shift+Esc"));
        assert_eq!(saved["app.hotkeys.openCard"], json!("Ctrl+Shift+O"));
    }

    #[test]
    fn cap_43a_a_new_talk_key_goes_to_the_service_once() {
        let store = FakeStore::default();
        let talk = FakeTalk::default();
        let s = settings(WIN, &store, &FakeEnv::default(), &talk);
        s.set(&obj(json!({ "ptt": "Ctrl+Win" }))).unwrap();
        assert_eq!(s.talk_binding(), TalkBinding::CtrlWin);
        // The same key again, or another action, does not restart the service.
        s.set(&obj(json!({ "ptt": "Ctrl+Win" }))).unwrap();
        s.set(&obj(json!({ "panic": "Ctrl+Win+Alt+P" }))).unwrap();
        assert_eq!(*talk.0.lock().unwrap(), vec![TalkBinding::CtrlWin]);
    }

    #[test]
    fn cap_43b_a_failed_save_changes_nothing() {
        let store = FakeStore::default();
        let talk = FakeTalk::default();
        let s = settings(MAC, &store, &FakeEnv::default(), &talk);
        let s = HotkeySettings::load(
            MAC,
            Box::new(FakeStore {
                saved: Arc::new(Mutex::new(Some(settings_from_bindings(MAC, &s.bindings())))),
                fail: true,
                ..FakeStore::default()
            }),
            Box::new(FakeEnv::default()),
            Box::new(talk.clone()),
        );
        let before = s.bindings();
        let err = s.set(&obj(json!({ "ptt": "RightControl" }))).unwrap_err();
        assert_eq!(err.code(), "settings_save_failed");
        assert_eq!(s.bindings(), before);
        assert!(talk.0.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_43b_memory_store_keeps_the_last_save() {
        let store = MemorySettingsStore::default();
        assert_eq!(store.load(), None);
        let values = settings_from_bindings(MAC, &hb::default_bindings(MAC));
        store.save(&values).unwrap();
        assert_eq!(store.load(), Some(values));
    }

    #[test]
    fn cap_43b_hotkey_settings_never_log() {
        let text = include_str!("hotkey_settings.rs");
        let code: String = text
            .split("#[cfg(test)]")
            .next()
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for banned in [
            "println!",
            "eprintln!",
            "dbg!",
            "log::",
            "tracing::",
            "{:?}",
        ] {
            assert!(!code.contains(banned), "hotkey_settings.rs uses {banned}");
        }
        for seam in ["TODO(#331)", "TODO(#77)", "TODO(#43)"] {
            assert!(text.contains(seam), "missing {seam}");
        }
    }
}
