//! Default global keys (D66), the first-run conflict check and the pure logic of the
//! `hotkeys_set` command (#24).
//!
//! This module has no OS calls. The host reads the keyboard environment ([`KeyboardEnv`]:
//! the Dictation shortcut, VoiceOver, the layout, the Copilot key, known voice apps) and
//! passes it in. Tests use plain values. The host hook-up is TODO(#24-host) (see ADR 0014).
//!
//! Privacy: a binding names only the global keys of Bottle. Nothing here reads, stores or
//! logs a key that the user types.
//!
//! Requirements: CAP-43a, CAP-43b (AT-30).

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::error::{CoreError, CoreResult};
use crate::ipc::JsonObject;

const ID: &str = "R1";

/// The desktop OS of the key map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Os {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
}

/// A global key of Bottle. Each one has an Appendix F setting `app.hotkeys.<key>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HotkeyAction {
    /// The talk key (PTT).
    Ptt,
    /// The panic key: stop all capture (MB-05).
    Panic,
    /// Start or stop a meeting session.
    Session,
    /// Open the card of the last-interacted MOG.
    OpenCard,
    /// The Secure-Input fallback chord (CAP-39b).
    Fallback,
}

impl HotkeyAction {
    /// Every action, in the order of Appendix F.
    pub const ALL: [HotkeyAction; 5] = [
        HotkeyAction::Ptt,
        HotkeyAction::Panic,
        HotkeyAction::Session,
        HotkeyAction::OpenCard,
        HotkeyAction::Fallback,
    ];

    /// The short key in the `bindings` object of `hotkeys_set`.
    pub fn key(self) -> &'static str {
        match self {
            HotkeyAction::Ptt => "ptt",
            HotkeyAction::Panic => "panic",
            HotkeyAction::Session => "session",
            HotkeyAction::OpenCard => "openCard",
            HotkeyAction::Fallback => "fallback",
        }
    }

    /// The Appendix F setting key, for example `app.hotkeys.ptt`.
    pub fn setting_key(self) -> &'static str {
        match self {
            HotkeyAction::Ptt => "app.hotkeys.ptt",
            HotkeyAction::Panic => "app.hotkeys.panic",
            HotkeyAction::Session => "app.hotkeys.session",
            HotkeyAction::OpenCard => "app.hotkeys.openCard",
            HotkeyAction::Fallback => "app.hotkeys.fallback",
        }
    }

    /// The action for a short key or a setting key.
    pub fn from_key(_key: &str) -> Option<Self> {
        None
    }
}

/// A modifier-only talk key. The D66 ladder is Right-Option / Right-Alt, then Right-Ctrl,
/// then Ctrl+Win (Windows only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TalkBinding {
    /// Right-Option on macOS, Right-Alt on Windows.
    RightOption,
    /// Right-Control.
    RightControl,
    /// Right-Command on macOS, Right-Win on Windows.
    RightCommand,
    /// Right-Shift.
    RightShift,
    /// Ctrl and Win held together (Windows only).
    CtrlWin,
}

impl TalkBinding {
    /// The shared talk key of `hotkey_core` (macOS). `None` for Ctrl+Win.
    pub fn core_talk_key(self) -> Option<crate::hotkey_core::TalkKey> {
        None
    }

    /// The Windows talk key of `hotkey_windows`.
    pub fn windows_talk_key(self) -> crate::hotkey_windows::TalkKey {
        crate::hotkey_windows::TalkKey::default()
    }
}

/// A set of chord modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Mods(u8);

impl Mods {
    /// Control.
    pub const CTRL: Mods = Mods(1);
    /// Shift.
    pub const SHIFT: Mods = Mods(2);
    /// Option on macOS, Alt on Windows.
    pub const ALT: Mods = Mods(4);
    /// Command on macOS, Win on Windows.
    pub const META: Mods = Mods(8);

    /// The union of two sets.
    pub const fn with(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    /// Every modifier of `other` is in this set.
    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }
}

/// The non-modifier key of a chord.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// Esc.
    Escape,
    /// Space.
    Space,
    /// Tab.
    Tab,
    /// A letter A-Z (upper case).
    Letter(char),
    /// A digit 0-9.
    Digit(u8),
    /// A function key F1-F24.
    F(u8),
    /// The Fn / Globe key.
    FnGlobe,
}

/// One global key binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// A modifier-only talk key.
    Talk(TalkBinding),
    /// Modifiers and one key.
    Chord {
        /// The modifiers.
        mods: Mods,
        /// The key.
        key: Key,
    },
}

/// A binding string that Bottle cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParseError;

impl Binding {
    /// A chord.
    pub const fn chord(mods: Mods, key: Key) -> Binding {
        Binding::Chord { mods, key }
    }

    /// Read a binding string such as `RightOption`, `Cmd+Shift+Esc` or `Ctrl+Win+Alt+K`.
    pub fn parse(_os: Os, _text: &str) -> Result<Binding, ParseError> {
        Err(ParseError)
    }

    /// The binding string for `os`, for example `Ctrl+Win+Alt+K`.
    pub fn format(&self, _os: Os) -> String {
        String::new()
    }

    /// A chord that holds Ctrl and Option (Alt).
    pub fn has_ctrl_alt(&self) -> bool {
        false
    }
}

/// The bindings of every [`HotkeyAction`].
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HotkeyBindings(BTreeMap<HotkeyAction, Binding>);

impl HotkeyBindings {
    /// The binding of `action`.
    pub fn get(&self, action: HotkeyAction) -> Option<Binding> {
        self.0.get(&action).copied()
    }

    /// Set the binding of `action`.
    pub fn set(&mut self, action: HotkeyAction, binding: Binding) {
        self.0.insert(action, binding);
    }

    /// Every action and its binding, in the order of [`HotkeyAction::ALL`].
    pub fn iter(&self) -> impl Iterator<Item = (HotkeyAction, Binding)> + '_ {
        self.0.iter().map(|(a, b)| (*a, *b))
    }

    /// The bindings as a JSON object of binding strings, keyed by [`HotkeyAction::key`].
    pub fn to_json(&self, _os: Os) -> JsonObject {
        JsonObject::new()
    }
}

/// The D66 talk-key ladder for `os`.
pub fn talk_key_ladder(_os: Os) -> &'static [TalkBinding] {
    &[]
}

/// The D66 default bindings for `os`, with the top of the talk-key ladder (CAP-43a).
pub fn default_bindings(_os: Os) -> HotkeyBindings {
    HotkeyBindings::default()
}

/// The macOS Dictation shortcut (System Settings > Keyboard > Dictation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DictationShortcut {
    /// Press the Globe (Fn) key.
    Globe,
    /// Press Control twice.
    ControlTwice,
    /// Press Right Command twice.
    RightCommandTwice,
    /// Press Left Command twice.
    LeftCommandTwice,
    /// Press either Command key twice.
    EitherCommandTwice,
}

/// A known voice app that can own the same key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VoiceApp {
    /// Superwhisper.
    Superwhisper,
    /// Wispr Flow.
    WisprFlow,
    /// MacWhisper.
    MacWhisper,
    /// Handy.
    Handy,
    /// Talon Voice.
    Talon,
    /// Dragon.
    Dragon,
    /// Windows Voice Access.
    VoiceAccess,
}

impl VoiceApp {
    /// Every known voice app.
    pub const ALL: [VoiceApp; 7] = [
        VoiceApp::Superwhisper,
        VoiceApp::WisprFlow,
        VoiceApp::MacWhisper,
        VoiceApp::Handy,
        VoiceApp::Talon,
        VoiceApp::Dragon,
        VoiceApp::VoiceAccess,
    ];

    /// The stable id in the conflict JSON.
    pub fn id(self) -> &'static str {
        ""
    }

    /// The name that the fix text shows.
    pub fn name(self) -> &'static str {
        ""
    }

    /// Match a process name against the fixed list. Any other name gives `None`, so the
    /// host keeps no name of an unknown app.
    pub fn from_process_name(_name: &str) -> Option<VoiceApp> {
        None
    }
}

/// What the first-run check reads from the OS. The host fills it with prompt-free reads.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KeyboardEnv {
    /// macOS Dictation is on with this shortcut.
    pub dictation: Option<DictationShortcut>,
    /// macOS VoiceOver is on.
    pub voice_over: bool,
    /// The Windows keyboard layout has AltGr (Right-Alt types characters).
    pub alt_gr_layout: bool,
    /// The Windows keyboard has a Copilot key in place of Right-Ctrl.
    pub copilot_key: bool,
    /// Known voice apps that run now.
    pub voice_apps: Vec<VoiceApp>,
}

/// The class of a conflict (CAP-43b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConflictKind {
    /// The OS Dictation shortcut uses the same key.
    Dictation,
    /// The binding uses the Fn / Globe key.
    FnGlobe,
    /// VoiceOver uses the same keys (Ctrl+Option, or Control stops speech).
    VoiceOver,
    /// The AltGr layout uses Right-Alt (or Ctrl+Alt) to type characters.
    AltGr,
    /// The Copilot key takes the place of Right-Ctrl.
    CopilotKey,
    /// A known voice app runs and can own the same key.
    VoiceApp,
    /// Two Bottle actions have the same binding.
    Duplicate,
    /// The OS keeps this chord for itself.
    Reserved,
    /// Bottle cannot read the binding string.
    Invalid,
}

impl ConflictKind {
    /// The stable id in the conflict JSON.
    pub fn id(self) -> &'static str {
        ""
    }
}

/// An OS settings page that a fix opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPane {
    /// macOS Keyboard > Dictation.
    Dictation,
}

/// How the user removes a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fix {
    /// Use this binding for the action instead.
    UseBinding {
        /// The action.
        action: HotkeyAction,
        /// The new binding.
        binding: Binding,
    },
    /// Change the setting on this OS page.
    OpenSystemSettings {
        /// The page.
        pane: SettingsPane,
    },
    /// Change the key of the other app, or quit it.
    ChangeOtherApp {
        /// The app.
        app: VoiceApp,
    },
}

/// One conflict and its fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Conflict {
    /// The class.
    pub kind: ConflictKind,
    /// The Bottle action whose binding collides.
    pub action: HotkeyAction,
    /// That binding.
    pub binding: Binding,
    /// The fix.
    pub fix: Fix,
    /// The voice app, for [`ConflictKind::VoiceApp`].
    pub app: Option<VoiceApp>,
}

impl Conflict {
    /// The text that the UI shows for this conflict and its fix.
    pub fn message(&self, _os: Os) -> String {
        String::new()
    }

    /// The conflict as one item of the `conflicts` array of `hotkeys_set`.
    pub fn to_json(&self, _os: Os) -> Value {
        json!({})
    }
}

/// Check `bindings` for conflicts in `env` (CAP-43b). Every conflict has a fix.
pub fn check_conflicts(_os: Os, _bindings: &HotkeyBindings, _env: &KeyboardEnv) -> Vec<Conflict> {
    Vec::new()
}

/// The result of the first-run check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstRunReport {
    /// The conflicts of the D66 defaults in this environment, each with a fix.
    pub conflicts: Vec<Conflict>,
    /// The defaults with the talk-key ladder step for this environment.
    pub suggested: HotkeyBindings,
}

/// The first-run conflict check (CAP-43b): check the D66 defaults in `env`, and pick the
/// first talk key of the ladder with no conflict.
pub fn first_run_check(os: Os, _env: &KeyboardEnv) -> FirstRunReport {
    FirstRunReport {
        conflicts: Vec::new(),
        suggested: default_bindings(os),
    }
}

/// The result of `hotkeys_set`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeysSetOutcome {
    /// The bindings after the change. An invalid string keeps the old binding.
    pub bindings: HotkeyBindings,
    /// The conflicts of the new bindings, each with a fix.
    pub conflicts: Vec<Conflict>,
}

impl HotkeysSetOutcome {
    /// The `{conflicts}` result of the `hotkeys_set` command (section 4.5).
    pub fn result_json(&self, _os: Os) -> Value {
        json!({})
    }
}

/// The pure logic of `hotkeys_set {bindings}`: apply the binding strings in `input` to
/// `current` and return the conflicts. A key that is not an action is an error
/// (`unknown_hotkey_action`).
pub fn hotkeys_set(
    _os: Os,
    current: &HotkeyBindings,
    _input: &JsonObject,
    _env: &KeyboardEnv,
) -> CoreResult<HotkeysSetOutcome> {
    let _ = CoreError::Unavailable {
        component: ID,
        reason: "unknown_hotkey_action",
    };
    Ok(HotkeysSetOutcome {
        bindings: current.clone(),
        conflicts: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAC: Os = Os::MacOs;
    const WIN: Os = Os::Windows;

    fn b(os: Os, s: &str) -> Binding {
        Binding::parse(os, s).unwrap_or_else(|_| panic!("parse {s}"))
    }

    fn kinds(c: &[Conflict]) -> Vec<ConflictKind> {
        c.iter().map(|c| c.kind).collect()
    }

    fn with(os: Os, action: HotkeyAction, s: &str) -> HotkeyBindings {
        let mut bs = default_bindings(os);
        bs.set(action, b(os, s));
        bs
    }

    // ---- CAP-43a: D66 defaults, no Ctrl+Option ----

    #[test]
    fn cap_43a_at_30_macos_defaults_follow_d66() {
        let d = default_bindings(MAC).to_json(MAC);
        let expect = [
            ("ptt", "RightOption"),
            ("panic", "Cmd+Shift+Esc"),
            ("session", "Ctrl+Shift+M"),
            ("openCard", "Ctrl+Shift+O"),
            ("fallback", "Ctrl+Shift+Space"),
        ];
        assert_eq!(d.len(), expect.len());
        for (k, v) in expect {
            assert_eq!(d.get(k), Some(&json!(v)), "{k}");
        }
    }

    #[test]
    fn cap_43a_at_30_windows_defaults_follow_d66() {
        let d = default_bindings(WIN).to_json(WIN);
        let expect = [
            ("ptt", "RightAlt"),
            ("panic", "Ctrl+Win+Alt+K"),
            ("session", "Ctrl+Win+Alt+R"),
            ("openCard", "Ctrl+Win+Alt+O"),
            ("fallback", "Ctrl+Shift+Space"),
        ];
        assert_eq!(d.len(), expect.len());
        for (k, v) in expect {
            assert_eq!(d.get(k), Some(&json!(v)), "{k}");
        }
    }

    #[test]
    fn cap_43a_at_30_no_macos_default_uses_ctrl_option() {
        for (a, binding) in default_bindings(MAC).iter() {
            assert!(!binding.has_ctrl_alt(), "{a:?}");
        }
        // The rule itself sees Ctrl+Option.
        assert!(b(MAC, "Ctrl+Option+Space").has_ctrl_alt());
    }

    #[test]
    fn cap_43a_at_30_no_windows_default_is_a_ctrl_alt_chord_without_win() {
        for (a, binding) in default_bindings(WIN).iter() {
            if let Binding::Chord { mods, .. } = binding {
                let ctrl_alt = mods.contains(Mods::CTRL.with(Mods::ALT));
                assert!(!ctrl_alt || mods.contains(Mods::META), "{a:?}");
            }
        }
    }

    #[test]
    fn cap_43a_at_30_defaults_have_no_conflict_in_a_clean_environment() {
        for os in [MAC, WIN] {
            let c = check_conflicts(os, &default_bindings(os), &KeyboardEnv::default());
            assert_eq!(c, vec![], "{os:?}");
        }
    }

    #[test]
    fn cap_43a_talk_key_ladder_follows_d66() {
        assert_eq!(
            talk_key_ladder(WIN),
            &[
                TalkBinding::RightOption,
                TalkBinding::RightControl,
                TalkBinding::CtrlWin
            ]
        );
        assert_eq!(
            talk_key_ladder(MAC),
            &[TalkBinding::RightOption, TalkBinding::RightControl]
        );
    }

    #[test]
    fn cap_43a_talk_bindings_map_to_the_platform_talk_keys() {
        use crate::hotkey_core::TalkKey as Core;
        use crate::hotkey_windows::TalkKey as Win;
        assert_eq!(
            TalkBinding::RightOption.core_talk_key(),
            Some(Core::RightOption)
        );
        assert_eq!(
            TalkBinding::RightControl.core_talk_key(),
            Some(Core::RightControl)
        );
        assert_eq!(TalkBinding::CtrlWin.core_talk_key(), None);
        assert_eq!(TalkBinding::RightOption.windows_talk_key(), Win::RightAlt);
        assert_eq!(TalkBinding::RightControl.windows_talk_key(), Win::RightCtrl);
        assert_eq!(TalkBinding::CtrlWin.windows_talk_key(), Win::CtrlWin);
    }

    #[test]
    fn cap_43a_binding_strings_round_trip() {
        for (os, s) in [
            (MAC, "RightOption"),
            (MAC, "RightControl"),
            (MAC, "RightCommand"),
            (MAC, "Cmd+Shift+Esc"),
            (MAC, "Ctrl+Shift+Space"),
            (MAC, "Ctrl+Shift+1"),
            (WIN, "RightAlt"),
            (WIN, "RightCtrl"),
            (WIN, "Ctrl+Win"),
            (WIN, "Ctrl+Win+Alt+K"),
            (WIN, "Win+Shift+F23"),
        ] {
            assert_eq!(b(os, s).format(os), s, "{os:?} {s}");
        }
        // Order, case and synonyms do not matter.
        assert_eq!(b(MAC, "shift+command+escape"), b(MAC, "Cmd+Shift+Esc"));
        assert_eq!(b(WIN, "alt+win+ctrl+k"), b(WIN, "Ctrl+Win+Alt+K"));
    }

    #[test]
    fn cap_43a_bad_binding_strings_are_refused() {
        for (os, s) in [
            (MAC, ""),
            (MAC, "Ctrl+Shift"),
            (MAC, "Ctrl+Win"),
            (MAC, "Cmd+Q+W"),
            (MAC, "Hyper+K"),
            (WIN, "Ctrl+Ctrl+K"),
            (WIN, "F99"),
        ] {
            assert_eq!(Binding::parse(os, s), Err(ParseError), "{os:?} {s:?}");
        }
    }

    #[test]
    fn cap_43a_actions_use_the_appendix_f_setting_keys() {
        let keys: Vec<_> = HotkeyAction::ALL.iter().map(|a| a.setting_key()).collect();
        assert_eq!(
            keys,
            [
                "app.hotkeys.ptt",
                "app.hotkeys.panic",
                "app.hotkeys.session",
                "app.hotkeys.openCard",
                "app.hotkeys.fallback"
            ]
        );
        for a in HotkeyAction::ALL {
            assert_eq!(HotkeyAction::from_key(a.key()), Some(a));
            assert_eq!(HotkeyAction::from_key(a.setting_key()), Some(a));
        }
        assert_eq!(HotkeyAction::from_key("app.hotkeys.chord"), None);
    }

    // ---- CAP-43b: the first-run conflict check, one test per class ----

    fn assert_has_fix(c: &Conflict, os: Os) {
        let v = c.to_json(os);
        assert!(v["fix"]["kind"].is_string(), "{v}");
        assert!(!c.message(os).is_empty(), "{v}");
    }

    #[test]
    fn cap_43b_at_30_dictation_on_control_twice_flags_a_right_control_talk_key() {
        let env = KeyboardEnv {
            dictation: Some(DictationShortcut::ControlTwice),
            ..KeyboardEnv::default()
        };
        let bs = with(MAC, HotkeyAction::Ptt, "RightControl");
        let c = check_conflicts(MAC, &bs, &env);
        assert_eq!(kinds(&c), vec![ConflictKind::Dictation]);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::RightOption)
            }
        );
        assert_has_fix(&c[0], MAC);
        // The default Right-Option does not collide with any Dictation shortcut.
        for d in [
            DictationShortcut::Globe,
            DictationShortcut::ControlTwice,
            DictationShortcut::RightCommandTwice,
            DictationShortcut::LeftCommandTwice,
            DictationShortcut::EitherCommandTwice,
        ] {
            let env = KeyboardEnv {
                dictation: Some(d),
                ..KeyboardEnv::default()
            };
            assert_eq!(check_conflicts(MAC, &default_bindings(MAC), &env), vec![]);
        }
    }

    #[test]
    fn cap_43b_at_30_dictation_on_command_twice_flags_a_right_command_talk_key() {
        let bs = with(MAC, HotkeyAction::Ptt, "RightCommand");
        for d in [
            DictationShortcut::RightCommandTwice,
            DictationShortcut::EitherCommandTwice,
        ] {
            let env = KeyboardEnv {
                dictation: Some(d),
                ..KeyboardEnv::default()
            };
            let c = check_conflicts(MAC, &bs, &env);
            assert_eq!(kinds(&c), vec![ConflictKind::Dictation], "{d:?}");
            assert_has_fix(&c[0], MAC);
        }
        let env = KeyboardEnv {
            dictation: Some(DictationShortcut::LeftCommandTwice),
            ..KeyboardEnv::default()
        };
        assert_eq!(check_conflicts(MAC, &bs, &env), vec![]);
    }

    #[test]
    fn cap_43b_at_30_windows_voice_typing_win_h_is_a_dictation_conflict() {
        let bs = with(WIN, HotkeyAction::Session, "Win+H");
        let c = check_conflicts(WIN, &bs, &KeyboardEnv::default());
        assert_eq!(kinds(&c), vec![ConflictKind::Dictation]);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Session,
                binding: b(WIN, "Ctrl+Win+Alt+R")
            }
        );
    }

    #[test]
    fn cap_43b_at_30_fn_globe_is_flagged_with_a_fix() {
        for os in [MAC, WIN] {
            let bs = with(os, HotkeyAction::OpenCard, "Fn+O");
            let c = check_conflicts(os, &bs, &KeyboardEnv::default());
            assert_eq!(kinds(&c), vec![ConflictKind::FnGlobe], "{os:?}");
            assert_eq!(
                c[0].fix,
                Fix::UseBinding {
                    action: HotkeyAction::OpenCard,
                    binding: default_bindings(os).get(HotkeyAction::OpenCard).unwrap()
                }
            );
            assert_has_fix(&c[0], os);
        }
        // "Globe" is the same key.
        assert_eq!(b(MAC, "Globe+O"), b(MAC, "Fn+O"));
    }

    #[test]
    fn cap_43b_at_30_ctrl_option_is_a_voiceover_conflict_on_macos() {
        let bs = with(MAC, HotkeyAction::Panic, "Ctrl+Option+K");
        // Flagged even with VoiceOver off: the user can turn it on at any time.
        let c = check_conflicts(MAC, &bs, &KeyboardEnv::default());
        assert_eq!(kinds(&c), vec![ConflictKind::VoiceOver]);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Panic,
                binding: b(MAC, "Cmd+Shift+Esc")
            }
        );
        assert_has_fix(&c[0], MAC);
    }

    #[test]
    fn cap_43b_at_30_voiceover_on_flags_a_right_control_talk_key() {
        // With VoiceOver on, a Control press stops the speech.
        let env = KeyboardEnv {
            voice_over: true,
            ..KeyboardEnv::default()
        };
        let bs = with(MAC, HotkeyAction::Ptt, "RightControl");
        let c = check_conflicts(MAC, &bs, &env);
        assert_eq!(kinds(&c), vec![ConflictKind::VoiceOver]);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::RightOption)
            }
        );
        assert_eq!(check_conflicts(MAC, &default_bindings(MAC), &env), vec![]);
    }

    #[test]
    fn cap_43b_at_30_altgr_layout_flags_right_alt_and_steps_down_the_ladder() {
        let env = KeyboardEnv {
            alt_gr_layout: true,
            ..KeyboardEnv::default()
        };
        let c = check_conflicts(WIN, &default_bindings(WIN), &env);
        assert_eq!(kinds(&c), vec![ConflictKind::AltGr]);
        assert_eq!(c[0].action, HotkeyAction::Ptt);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::RightControl)
            }
        );
        assert_has_fix(&c[0], WIN);
        // A Ctrl+Alt chord without Win types characters on an AltGr layout too.
        let bs = with(WIN, HotkeyAction::OpenCard, "Ctrl+Alt+O");
        let c = check_conflicts(WIN, &bs, &env);
        assert!(kinds(&c).contains(&ConflictKind::AltGr));
    }

    #[test]
    fn cap_43b_at_30_copilot_key_flags_right_ctrl() {
        let env = KeyboardEnv {
            copilot_key: true,
            ..KeyboardEnv::default()
        };
        let bs = with(WIN, HotkeyAction::Ptt, "RightCtrl");
        let c = check_conflicts(WIN, &bs, &env);
        assert_eq!(kinds(&c), vec![ConflictKind::CopilotKey]);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::RightOption)
            }
        );
        // The Copilot key sends Win+Shift+F23.
        let bs = with(WIN, HotkeyAction::Session, "Win+Shift+F23");
        let c = check_conflicts(WIN, &bs, &env);
        assert_eq!(kinds(&c), vec![ConflictKind::CopilotKey]);
        assert_has_fix(&c[0], WIN);
    }

    #[test]
    fn cap_43b_at_30_altgr_and_copilot_together_step_to_ctrl_win() {
        let env = KeyboardEnv {
            alt_gr_layout: true,
            copilot_key: true,
            ..KeyboardEnv::default()
        };
        let c = check_conflicts(WIN, &default_bindings(WIN), &env);
        assert_eq!(kinds(&c), vec![ConflictKind::AltGr]);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::CtrlWin)
            }
        );
    }

    #[test]
    fn cap_43b_at_30_known_voice_apps_are_flagged_with_a_fix() {
        let env = KeyboardEnv {
            voice_apps: vec![VoiceApp::Superwhisper, VoiceApp::WisprFlow],
            ..KeyboardEnv::default()
        };
        let c = check_conflicts(MAC, &default_bindings(MAC), &env);
        assert_eq!(
            kinds(&c),
            vec![ConflictKind::VoiceApp, ConflictKind::VoiceApp]
        );
        assert_eq!(c[0].app, Some(VoiceApp::Superwhisper));
        assert_eq!(
            c[0].fix,
            Fix::ChangeOtherApp {
                app: VoiceApp::Superwhisper
            }
        );
        assert_eq!(c[1].app, Some(VoiceApp::WisprFlow));
        let v = c[0].to_json(MAC);
        assert_eq!(v["app"], json!("superwhisper"));
        assert!(c[0].message(MAC).contains("Superwhisper"));
        assert!(c[0].message(MAC).contains("Right-Option"));
    }

    #[test]
    fn cap_43b_voice_apps_match_a_fixed_list_only() {
        assert_eq!(
            VoiceApp::from_process_name("superwhisper"),
            Some(VoiceApp::Superwhisper)
        );
        assert_eq!(
            VoiceApp::from_process_name("Wispr Flow"),
            Some(VoiceApp::WisprFlow)
        );
        assert_eq!(
            VoiceApp::from_process_name("VoiceAccess.exe"),
            Some(VoiceApp::VoiceAccess)
        );
        assert_eq!(VoiceApp::from_process_name("Safari"), None);
        assert_eq!(VoiceApp::from_process_name(""), None);
        for app in VoiceApp::ALL {
            assert!(!app.id().is_empty());
            assert!(!app.name().is_empty());
        }
    }

    #[test]
    fn cap_43b_at_30_duplicate_bindings_are_flagged() {
        let bs = with(MAC, HotkeyAction::OpenCard, "Ctrl+Shift+M");
        let c = check_conflicts(MAC, &bs, &KeyboardEnv::default());
        assert_eq!(kinds(&c), vec![ConflictKind::Duplicate]);
        assert_eq!(c[0].action, HotkeyAction::OpenCard);
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::OpenCard,
                binding: b(MAC, "Ctrl+Shift+O")
            }
        );
    }

    #[test]
    fn cap_43b_at_30_os_reserved_chords_are_flagged() {
        for (os, action, s) in [
            (MAC, HotkeyAction::Session, "Cmd+Space"),
            (MAC, HotkeyAction::Session, "Cmd+Tab"),
            (MAC, HotkeyAction::Panic, "Cmd+Option+Esc"),
            (WIN, HotkeyAction::Panic, "Ctrl+Shift+Esc"),
            (WIN, HotkeyAction::Session, "Win+L"),
            (WIN, HotkeyAction::Session, "Alt+Tab"),
        ] {
            let bs = with(os, action, s);
            let c = check_conflicts(os, &bs, &KeyboardEnv::default());
            assert_eq!(kinds(&c), vec![ConflictKind::Reserved], "{os:?} {s}");
            assert_eq!(
                c[0].fix,
                Fix::UseBinding {
                    action,
                    binding: default_bindings(os).get(action).unwrap()
                }
            );
        }
    }

    #[test]
    fn cap_43b_at_30_first_run_check_flags_and_suggests_the_ladder_step() {
        let env = KeyboardEnv {
            alt_gr_layout: true,
            copilot_key: true,
            voice_apps: vec![VoiceApp::Dragon],
            ..KeyboardEnv::default()
        };
        let r = first_run_check(WIN, &env);
        assert_eq!(
            kinds(&r.conflicts),
            vec![ConflictKind::AltGr, ConflictKind::VoiceApp]
        );
        for c in &r.conflicts {
            assert_has_fix(c, WIN);
        }
        assert_eq!(
            r.suggested.get(HotkeyAction::Ptt),
            Some(Binding::Talk(TalkBinding::CtrlWin))
        );
        // The other suggested keys stay the D66 defaults.
        for a in [
            HotkeyAction::Panic,
            HotkeyAction::Session,
            HotkeyAction::OpenCard,
            HotkeyAction::Fallback,
        ] {
            assert_eq!(r.suggested.get(a), default_bindings(WIN).get(a));
        }
        // The suggestion has no key conflict left (the voice app stays a notice).
        let left = check_conflicts(WIN, &r.suggested, &env);
        assert_eq!(kinds(&left), vec![ConflictKind::VoiceApp]);
    }

    #[test]
    fn cap_43b_first_run_check_in_a_clean_environment_is_empty() {
        for os in [MAC, WIN] {
            let r = first_run_check(os, &KeyboardEnv::default());
            assert_eq!(r.conflicts, vec![]);
            assert_eq!(r.suggested, default_bindings(os));
        }
    }

    #[test]
    fn cap_43b_at_30_every_conflict_kind_has_an_id_and_json_shape() {
        let c = Conflict {
            kind: ConflictKind::AltGr,
            action: HotkeyAction::Ptt,
            binding: Binding::Talk(TalkBinding::RightOption),
            fix: Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::RightControl),
            },
            app: None,
        };
        let v = c.to_json(WIN);
        assert_eq!(v["kind"], json!("alt_gr"));
        assert_eq!(v["action"], json!("ptt"));
        assert_eq!(v["binding"], json!("RightAlt"));
        assert_eq!(v["fix"]["kind"], json!("use_binding"));
        assert_eq!(v["fix"]["binding"], json!("RightCtrl"));
        assert!(v["message"]
            .as_str()
            .is_some_and(|m| m.contains("Right-Ctrl")));
        for k in [
            ConflictKind::Dictation,
            ConflictKind::FnGlobe,
            ConflictKind::VoiceOver,
            ConflictKind::AltGr,
            ConflictKind::CopilotKey,
            ConflictKind::VoiceApp,
            ConflictKind::Duplicate,
            ConflictKind::Reserved,
            ConflictKind::Invalid,
        ] {
            assert!(!k.id().is_empty(), "{k:?}");
        }
    }

    // ---- hotkeys_set: {bindings} -> {conflicts} ----

    fn input(pairs: &[(&str, Value)]) -> JsonObject {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), v.clone()))
            .collect()
    }

    #[test]
    fn cap_43b_hotkeys_set_applies_bindings_and_returns_conflicts() {
        let cur = default_bindings(MAC);
        let out = hotkeys_set(
            MAC,
            &cur,
            &input(&[("panic", json!("Ctrl+Option+K"))]),
            &KeyboardEnv::default(),
        )
        .unwrap();
        assert_eq!(
            out.bindings.get(HotkeyAction::Panic),
            Some(b(MAC, "Ctrl+Option+K"))
        );
        assert_eq!(kinds(&out.conflicts), vec![ConflictKind::VoiceOver]);
        let r = out.result_json(MAC);
        let arr = r["conflicts"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["kind"], json!("voice_over"));
        assert_eq!(r.as_object().unwrap().len(), 1, "only {{conflicts}}");
    }

    #[test]
    fn cap_43b_hotkeys_set_accepts_setting_keys_and_a_clean_change() {
        let cur = default_bindings(WIN);
        let out = hotkeys_set(
            WIN,
            &cur,
            &input(&[("app.hotkeys.ptt", json!("RightCtrl"))]),
            &KeyboardEnv::default(),
        )
        .unwrap();
        assert_eq!(
            out.bindings.get(HotkeyAction::Ptt),
            Some(Binding::Talk(TalkBinding::RightControl))
        );
        assert_eq!(out.conflicts, vec![]);
        assert_eq!(out.result_json(WIN), json!({ "conflicts": [] }));
    }

    #[test]
    fn cap_43b_hotkeys_set_keeps_the_old_binding_for_an_invalid_string() {
        let cur = default_bindings(MAC);
        for bad in [json!("Hyper+K"), json!(7), json!(null)] {
            let out = hotkeys_set(
                MAC,
                &cur,
                &input(&[("session", bad.clone())]),
                &KeyboardEnv::default(),
            )
            .unwrap();
            assert_eq!(out.bindings, cur, "{bad}");
            assert_eq!(kinds(&out.conflicts), vec![ConflictKind::Invalid], "{bad}");
            assert_eq!(
                out.conflicts[0].fix,
                Fix::UseBinding {
                    action: HotkeyAction::Session,
                    binding: b(MAC, "Ctrl+Shift+M")
                }
            );
        }
    }

    #[test]
    fn cap_43b_hotkeys_set_refuses_an_unknown_action() {
        let err = hotkeys_set(
            MAC,
            &default_bindings(MAC),
            &input(&[("launchRockets", json!("Cmd+K"))]),
            &KeyboardEnv::default(),
        )
        .unwrap_err();
        assert_eq!(err.code(), "unknown_hotkey_action");
    }
}
