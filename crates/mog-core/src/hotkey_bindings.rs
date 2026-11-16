//! Default global keys (D66), the first-run conflict check and the pure logic of the
//! `hotkeys_set` command (#24).
//!
//! This module has no OS calls. The host reads the keyboard environment ([`KeyboardEnv`]:
//! the Dictation shortcut, VoiceOver, the layout, the Copilot key, known voice apps) with
//! prompt-free OS reads (`crate::keyboard_env`) and passes it in. Tests use plain values.
//! The host hook-up is `src-tauri/src/hotkey_settings.rs` (#322, ADR 0020); see ADR 0017.
//!
//! Privacy: a binding names only the global keys of Bottle. Nothing here reads, stores or
//! logs a key that the user types. A voice app is kept only when its process name is on the
//! fixed list of [`VoiceApp`].
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

impl Os {
    fn name(self) -> &'static str {
        match self {
            Os::MacOs => "macOS",
            Os::Windows => "Windows",
        }
    }
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
    pub fn from_key(key: &str) -> Option<Self> {
        HotkeyAction::ALL
            .into_iter()
            .find(|a| a.key() == key || a.setting_key() == key)
    }

    fn label(self) -> &'static str {
        match self {
            HotkeyAction::Ptt => "talk key",
            HotkeyAction::Panic => "panic key",
            HotkeyAction::Session => "session key",
            HotkeyAction::OpenCard => "open-card key",
            HotkeyAction::Fallback => "Secure-Input fallback key",
        }
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
        use crate::hotkey_core::TalkKey as Core;
        match self {
            TalkBinding::RightOption => Some(Core::RightOption),
            TalkBinding::RightControl => Some(Core::RightControl),
            TalkBinding::RightCommand => Some(Core::RightCommand),
            TalkBinding::RightShift => Some(Core::RightShift),
            TalkBinding::CtrlWin => None,
        }
    }

    /// The Windows talk key of `hotkey_windows`. D66 gives Windows only Right-Alt,
    /// Right-Ctrl and Ctrl+Win, so Right-Win and Right-Shift fall back to the D66 default
    /// (Right-Alt). [`check_conflicts`] flags them as invalid on Windows (#313).
    pub fn windows_talk_key(self) -> crate::hotkey_windows::TalkKey {
        use crate::hotkey_windows::TalkKey as Win;
        match self {
            TalkBinding::RightControl => Win::RightCtrl,
            TalkBinding::CtrlWin => Win::CtrlWin,
            TalkBinding::RightOption | TalkBinding::RightCommand | TalkBinding::RightShift => {
                Win::RightAlt
            }
        }
    }

    /// Whether `os` has this talk key. Windows has only the D66 keys (#313).
    fn on(self, os: Os) -> bool {
        match os {
            Os::MacOs => self != TalkBinding::CtrlWin,
            Os::Windows => matches!(
                self,
                TalkBinding::RightOption | TalkBinding::RightControl | TalkBinding::CtrlWin
            ),
        }
    }

    fn token(self, os: Os) -> &'static str {
        match (self, os) {
            (TalkBinding::RightOption, Os::MacOs) => "RightOption",
            (TalkBinding::RightOption, Os::Windows) => "RightAlt",
            (TalkBinding::RightControl, Os::MacOs) => "RightControl",
            (TalkBinding::RightControl, Os::Windows) => "RightCtrl",
            (TalkBinding::RightCommand, Os::MacOs) => "RightCommand",
            (TalkBinding::RightCommand, Os::Windows) => "RightWin",
            (TalkBinding::RightShift, _) => "RightShift",
            (TalkBinding::CtrlWin, _) => "Ctrl+Win",
        }
    }

    fn display(self, os: Os) -> &'static str {
        match (self, os) {
            (TalkBinding::RightOption, Os::MacOs) => "Right-Option",
            (TalkBinding::RightOption, Os::Windows) => "Right-Alt",
            (TalkBinding::RightControl, Os::MacOs) => "Right-Control",
            (TalkBinding::RightControl, Os::Windows) => "Right-Ctrl",
            (TalkBinding::RightCommand, Os::MacOs) => "Right-Command",
            (TalkBinding::RightCommand, Os::Windows) => "Right-Win",
            (TalkBinding::RightShift, _) => "Right-Shift",
            (TalkBinding::CtrlWin, _) => "Ctrl+Win",
        }
    }

    fn from_token(token: &str) -> Option<TalkBinding> {
        Some(match token {
            "rightoption" | "rightalt" => TalkBinding::RightOption,
            "rightcontrol" | "rightctrl" => TalkBinding::RightControl,
            "rightcommand" | "rightcmd" | "rightwin" => TalkBinding::RightCommand,
            "rightshift" => TalkBinding::RightShift,
            _ => return None,
        })
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
    /// The Fn / Globe key held with another key.
    pub const FN: Mods = Mods(16);

    /// The union of two sets.
    pub const fn with(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }

    /// Every modifier of `other` is in this set.
    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    const fn is_empty(self) -> bool {
        self.0 == 0
    }

    fn from_token(os: Os, token: &str) -> Option<Mods> {
        Some(match (token, os) {
            ("ctrl" | "control", _) => Mods::CTRL,
            ("shift", _) => Mods::SHIFT,
            ("alt", _) | ("option" | "opt", Os::MacOs) => Mods::ALT,
            ("cmd" | "command", Os::MacOs) | ("win", Os::Windows) | ("meta", _) => Mods::META,
            ("fn" | "globe", _) => Mods::FN,
            _ => return None,
        })
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

impl Key {
    fn from_token(token: &str) -> Option<Key> {
        Some(match token {
            "esc" | "escape" => Key::Escape,
            "space" => Key::Space,
            "tab" => Key::Tab,
            _ => {
                let mut chars = token.chars();
                let first = chars.next()?;
                let rest = chars.as_str();
                if rest.is_empty() && first.is_ascii_lowercase() {
                    Key::Letter(first.to_ascii_uppercase())
                } else if rest.is_empty() && first.is_ascii_digit() {
                    Key::Digit(first as u8 - b'0')
                } else if first == 'f' && !rest.is_empty() && rest.len() <= 2 {
                    let n: u8 = rest.parse().ok()?;
                    if !(1..=24).contains(&n) || rest.starts_with('0') {
                        return None;
                    }
                    Key::F(n)
                } else {
                    return None;
                }
            }
        })
    }

    fn token(self) -> String {
        match self {
            Key::Escape => "Esc".to_owned(),
            Key::Space => "Space".to_owned(),
            Key::Tab => "Tab".to_owned(),
            Key::Letter(c) => c.to_string(),
            Key::Digit(d) => d.to_string(),
            Key::F(n) => format!("F{n}"),
            Key::FnGlobe => "Fn".to_owned(),
        }
    }

    /// A key that types or moves focus. A global binding needs a modifier with it.
    fn needs_modifier(self) -> bool {
        matches!(
            self,
            Key::Escape | Key::Space | Key::Tab | Key::Letter(_) | Key::Digit(_)
        )
    }
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
    /// Case, order and the usual synonyms (`Command`, `Option`, `Escape`) do not matter.
    pub fn parse(os: Os, text: &str) -> Result<Binding, ParseError> {
        let lower = text.trim().to_ascii_lowercase();
        if let Some(talk) = TalkBinding::from_token(&lower) {
            return Ok(Binding::Talk(talk));
        }
        let mut mods = Mods::default();
        let mut key = None;
        for token in lower.split('+').map(str::trim) {
            if let Some(m) = Mods::from_token(os, token) {
                if mods.contains(m) {
                    return Err(ParseError);
                }
                mods = mods.with(m);
            } else if let Some(k) = Key::from_token(token) {
                if key.replace(k).is_some() {
                    return Err(ParseError);
                }
            } else {
                return Err(ParseError);
            }
        }
        match key {
            Some(k) if k.needs_modifier() && mods.is_empty() => Err(ParseError),
            Some(k) => Ok(Binding::chord(mods, k)),
            None if os == Os::Windows && mods == Mods::CTRL.with(Mods::META) => {
                Ok(Binding::Talk(TalkBinding::CtrlWin))
            }
            None if mods == Mods::FN => Ok(Binding::chord(Mods::default(), Key::FnGlobe)),
            None => Err(ParseError),
        }
    }

    /// The binding string for `os`, for example `Ctrl+Win+Alt+K`.
    pub fn format(&self, os: Os) -> String {
        match *self {
            Binding::Talk(t) => t.token(os).to_owned(),
            Binding::Chord { mods, key } => {
                let meta = match os {
                    Os::MacOs => "Cmd",
                    Os::Windows => "Win",
                };
                let alt = match os {
                    Os::MacOs => "Option",
                    Os::Windows => "Alt",
                };
                let mut parts: Vec<String> = [
                    (Mods::FN, "Fn"),
                    (Mods::CTRL, "Ctrl"),
                    (Mods::META, meta),
                    (Mods::ALT, alt),
                    (Mods::SHIFT, "Shift"),
                ]
                .into_iter()
                .filter(|(m, _)| mods.contains(*m))
                .map(|(_, n)| n.to_owned())
                .collect();
                parts.push(key.token());
                parts.join("+")
            }
        }
    }

    /// The name that a message shows, for example `Right-Option` or `Cmd+Shift+Esc`.
    fn display(&self, os: Os) -> String {
        match self {
            Binding::Talk(t) => t.display(os).to_owned(),
            Binding::Chord { .. } => self.format(os),
        }
    }

    /// A chord that holds Ctrl and Option (Alt).
    pub fn has_ctrl_alt(&self) -> bool {
        matches!(self, Binding::Chord { mods, .. } if mods.contains(Mods::CTRL.with(Mods::ALT)))
    }

    fn uses_fn_globe(&self) -> bool {
        matches!(self, Binding::Chord { mods, key } if *key == Key::FnGlobe || mods.contains(Mods::FN))
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
    pub fn to_json(&self, os: Os) -> JsonObject {
        self.iter()
            .map(|(a, b)| (a.key().to_owned(), Value::String(b.format(os))))
            .collect()
    }
}

const MAC_LADDER: [TalkBinding; 2] = [TalkBinding::RightOption, TalkBinding::RightControl];
const WIN_LADDER: [TalkBinding; 3] = [
    TalkBinding::RightOption,
    TalkBinding::RightControl,
    TalkBinding::CtrlWin,
];

/// The D66 talk-key ladder for `os`.
pub fn talk_key_ladder(os: Os) -> &'static [TalkBinding] {
    match os {
        Os::MacOs => &MAC_LADDER,
        Os::Windows => &WIN_LADDER,
    }
}

const fn chord(mods: Mods, key: Key) -> Binding {
    Binding::chord(mods, key)
}

const CTRL_SHIFT: Mods = Mods::CTRL.with(Mods::SHIFT);
const CTRL_WIN_ALT: Mods = Mods::CTRL.with(Mods::META).with(Mods::ALT);

/// The D66 default bindings for `os`, with the top of the talk-key ladder (CAP-43a).
pub fn default_bindings(os: Os) -> HotkeyBindings {
    let mut bs = HotkeyBindings::default();
    bs.set(HotkeyAction::Ptt, Binding::Talk(talk_key_ladder(os)[0]));
    bs.set(HotkeyAction::Fallback, chord(CTRL_SHIFT, Key::Space));
    match os {
        Os::MacOs => {
            let cmd_shift = Mods::META.with(Mods::SHIFT);
            bs.set(HotkeyAction::Panic, chord(cmd_shift, Key::Escape));
            bs.set(HotkeyAction::Session, chord(CTRL_SHIFT, Key::Letter('M')));
            bs.set(HotkeyAction::OpenCard, chord(CTRL_SHIFT, Key::Letter('O')));
        }
        Os::Windows => {
            bs.set(HotkeyAction::Panic, chord(CTRL_WIN_ALT, Key::Letter('K')));
            bs.set(HotkeyAction::Session, chord(CTRL_WIN_ALT, Key::Letter('R')));
            bs.set(
                HotkeyAction::OpenCard,
                chord(CTRL_WIN_ALT, Key::Letter('O')),
            );
        }
    }
    bs
}

/// The chords that the OS keeps for itself (a short list of the well-known ones).
fn reserved(os: Os) -> &'static [Binding] {
    const MAC: [Binding; 8] = [
        chord(Mods::META, Key::Space),
        chord(Mods::CTRL, Key::Space),
        chord(Mods::META, Key::Tab),
        chord(Mods::META, Key::Letter('Q')),
        chord(Mods::META.with(Mods::ALT), Key::Escape),
        chord(Mods::META.with(Mods::SHIFT), Key::Digit(3)),
        chord(Mods::META.with(Mods::SHIFT), Key::Digit(4)),
        chord(Mods::CTRL.with(Mods::META), Key::Letter('Q')),
    ];
    const WIN: [Binding; 8] = [
        chord(Mods::ALT, Key::Tab),
        chord(Mods::ALT, Key::F(4)),
        chord(CTRL_SHIFT, Key::Escape),
        chord(Mods::CTRL, Key::Escape),
        chord(Mods::META, Key::Letter('L')),
        chord(Mods::META, Key::Letter('D')),
        chord(Mods::META, Key::Tab),
        chord(Mods::META, Key::Space),
    ];
    match os {
        Os::MacOs => &MAC,
        Os::Windows => &WIN,
    }
}

/// Windows voice typing.
const WIN_H: Binding = chord(Mods::META, Key::Letter('H'));
/// The chord that the Copilot key sends.
const COPILOT_CHORD: Binding = chord(Mods::META.with(Mods::SHIFT), Key::F(23));

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
        match self {
            VoiceApp::Superwhisper => "superwhisper",
            VoiceApp::WisprFlow => "wispr_flow",
            VoiceApp::MacWhisper => "macwhisper",
            VoiceApp::Handy => "handy",
            VoiceApp::Talon => "talon",
            VoiceApp::Dragon => "dragon",
            VoiceApp::VoiceAccess => "voice_access",
        }
    }

    /// The name that the fix text shows.
    pub fn name(self) -> &'static str {
        match self {
            VoiceApp::Superwhisper => "Superwhisper",
            VoiceApp::WisprFlow => "Wispr Flow",
            VoiceApp::MacWhisper => "MacWhisper",
            VoiceApp::Handy => "Handy",
            VoiceApp::Talon => "Talon",
            VoiceApp::Dragon => "Dragon",
            VoiceApp::VoiceAccess => "Voice Access",
        }
    }

    /// Match a process name against the fixed list. Any other name gives `None`, so the
    /// host keeps no name of an unknown app.
    pub fn from_process_name(name: &str) -> Option<VoiceApp> {
        let lower = name.trim().to_ascii_lowercase();
        let stem = lower
            .strip_suffix(".exe")
            .or_else(|| lower.strip_suffix(".app"))
            .unwrap_or(&lower);
        let norm: String = stem.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        Some(match norm.as_str() {
            "superwhisper" => VoiceApp::Superwhisper,
            "wisprflow" => VoiceApp::WisprFlow,
            "macwhisper" => VoiceApp::MacWhisper,
            "handy" => VoiceApp::Handy,
            "talon" => VoiceApp::Talon,
            "dragon" | "natspeak" => VoiceApp::Dragon,
            "voiceaccess" => VoiceApp::VoiceAccess,
            _ => return None,
        })
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
    /// Bottle cannot read the binding string, or the binding does not fit the action.
    Invalid,
}

impl ConflictKind {
    /// The stable id in the conflict JSON.
    pub fn id(self) -> &'static str {
        match self {
            ConflictKind::Dictation => "dictation",
            ConflictKind::FnGlobe => "fn_globe",
            ConflictKind::VoiceOver => "voice_over",
            ConflictKind::AltGr => "alt_gr",
            ConflictKind::CopilotKey => "copilot_key",
            ConflictKind::VoiceApp => "voice_app",
            ConflictKind::Duplicate => "duplicate",
            ConflictKind::Reserved => "reserved",
            ConflictKind::Invalid => "invalid",
        }
    }
}

/// An OS settings page that a fix opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPane {
    /// macOS Keyboard > Dictation.
    Dictation,
    /// macOS Accessibility > VoiceOver.
    VoiceOver,
    /// The keyboard layout settings of the OS.
    KeyboardLayout,
}

impl SettingsPane {
    fn id(self) -> &'static str {
        match self {
            SettingsPane::Dictation => "dictation",
            SettingsPane::VoiceOver => "voice_over",
            SettingsPane::KeyboardLayout => "keyboard_layout",
        }
    }

    fn path(self, os: Os) -> &'static str {
        match (self, os) {
            (SettingsPane::Dictation, _) => "System Settings > Keyboard > Dictation",
            (SettingsPane::VoiceOver, _) => "System Settings > Accessibility > VoiceOver",
            (SettingsPane::KeyboardLayout, Os::MacOs) => "System Settings > Keyboard",
            (SettingsPane::KeyboardLayout, Os::Windows) => "Settings > Time & language",
        }
    }
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

impl Fix {
    fn to_json(self, os: Os) -> Value {
        match self {
            Fix::UseBinding { action, binding } => json!({
                "kind": "use_binding",
                "action": action.key(),
                "binding": binding.format(os),
            }),
            Fix::OpenSystemSettings { pane } => json!({
                "kind": "open_system_settings",
                "pane": pane.id(),
            }),
            Fix::ChangeOtherApp { app } => json!({
                "kind": "change_other_app",
                "app": app.id(),
            }),
        }
    }

    fn text(self, os: Os) -> String {
        match self {
            Fix::UseBinding { action, binding } => {
                format!(
                    "Fix: use {} as the {}.",
                    binding.display(os),
                    action.label()
                )
            }
            Fix::OpenSystemSettings { pane } => {
                format!("Fix: change the setting in {}.", pane.path(os))
            }
            Fix::ChangeOtherApp { app } => {
                format!("Fix: give {} a different key, or quit it.", app.name())
            }
        }
    }
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
    fn new(kind: ConflictKind, action: HotkeyAction, binding: Binding, fix: Fix) -> Self {
        Self {
            kind,
            action,
            binding,
            fix,
            app: None,
        }
    }

    /// The text that the UI shows for this conflict and its fix.
    pub fn message(&self, os: Os) -> String {
        let key = self.binding.display(os);
        let problem = match self.kind {
            ConflictKind::Dictation => match os {
                Os::MacOs => format!("macOS Dictation starts when you press {key} twice."),
                Os::Windows => "Windows voice typing uses Win+H.".to_owned(),
            },
            ConflictKind::FnGlobe => {
                "The keyboard and the OS use the Fn / Globe key. Bottle cannot use it.".to_owned()
            }
            ConflictKind::VoiceOver => match self.binding {
                Binding::Talk(_) => {
                    format!("VoiceOver is on. A press of {key} stops the VoiceOver speech.")
                }
                Binding::Chord { .. } => "VoiceOver uses Ctrl+Option for its commands.".to_owned(),
            },
            ConflictKind::AltGr => {
                format!("Your keyboard layout uses {key} (AltGr) to type characters.")
            }
            ConflictKind::CopilotKey => {
                "The Copilot key is in place of Right-Ctrl and sends Win+Shift+F23.".to_owned()
            }
            ConflictKind::VoiceApp => {
                let name = self.app.map_or("A voice app", VoiceApp::name);
                format!("{name} is open. It can use the same talk key ({key}).")
            }
            ConflictKind::Duplicate => format!("Another Bottle key also uses {key}."),
            ConflictKind::Reserved => format!("{} keeps {key} for itself.", os.name()),
            ConflictKind::Invalid => format!("Bottle cannot use this {}.", self.action.label()),
        };
        format!("{problem} {}", self.fix.text(os))
    }

    /// The conflict as one item of the `conflicts` array of `hotkeys_set`.
    pub fn to_json(&self, os: Os) -> Value {
        let mut v = json!({
            "kind": self.kind.id(),
            "action": self.action.key(),
            "binding": self.binding.format(os),
            "fix": self.fix.to_json(os),
            "message": self.message(os),
        });
        if let (Some(app), Some(obj)) = (self.app, v.as_object_mut()) {
            obj.insert("app".to_owned(), json!(app.id()));
        }
        v
    }
}

/// The environment conflict of one talk key, if any.
fn talk_conflict(os: Os, talk: TalkBinding, env: &KeyboardEnv) -> Option<ConflictKind> {
    match os {
        Os::MacOs => {
            let dictation = matches!(
                (env.dictation, talk),
                (
                    Some(DictationShortcut::ControlTwice),
                    TalkBinding::RightControl
                ) | (
                    Some(
                        DictationShortcut::RightCommandTwice
                            | DictationShortcut::EitherCommandTwice
                    ),
                    TalkBinding::RightCommand
                )
            );
            if dictation {
                Some(ConflictKind::Dictation)
            } else if env.voice_over && talk == TalkBinding::RightControl {
                Some(ConflictKind::VoiceOver)
            } else {
                None
            }
        }
        Os::Windows => match talk {
            _ if !talk.on(os) => Some(ConflictKind::Invalid),
            TalkBinding::RightOption if env.alt_gr_layout => Some(ConflictKind::AltGr),
            TalkBinding::RightControl if env.copilot_key => Some(ConflictKind::CopilotKey),
            _ => None,
        },
    }
}

/// The first talk key of the ladder with no environment conflict.
fn ladder_pick(os: Os, env: &KeyboardEnv) -> Option<TalkBinding> {
    talk_key_ladder(os)
        .iter()
        .copied()
        .find(|t| talk_conflict(os, *t, env).is_none())
}

/// The fix of a talk-key conflict: the next free step of the ladder, else the OS setting.
fn talk_fix(os: Os, kind: ConflictKind, env: &KeyboardEnv) -> Fix {
    match ladder_pick(os, env) {
        Some(t) => Fix::UseBinding {
            action: HotkeyAction::Ptt,
            binding: Binding::Talk(t),
        },
        None => Fix::OpenSystemSettings {
            pane: match kind {
                ConflictKind::Dictation => SettingsPane::Dictation,
                ConflictKind::VoiceOver => SettingsPane::VoiceOver,
                _ => SettingsPane::KeyboardLayout,
            },
        },
    }
}

/// The fix of a chord conflict: the D66 default of the action.
fn default_fix(os: Os, action: HotkeyAction) -> Fix {
    let binding = default_bindings(os)
        .get(action)
        .unwrap_or(Binding::Talk(TalkBinding::RightOption));
    Fix::UseBinding { action, binding }
}

/// The key conflict of one chord, if any.
fn chord_conflict(os: Os, binding: Binding, env: &KeyboardEnv) -> Option<ConflictKind> {
    let Binding::Chord { mods, .. } = binding else {
        return None;
    };
    if binding.uses_fn_globe() {
        return Some(ConflictKind::FnGlobe);
    }
    if reserved(os).contains(&binding) {
        return Some(ConflictKind::Reserved);
    }
    match os {
        Os::MacOs if binding.has_ctrl_alt() => Some(ConflictKind::VoiceOver),
        Os::Windows if binding == WIN_H => Some(ConflictKind::Dictation),
        Os::Windows if env.copilot_key && binding == COPILOT_CHORD => {
            Some(ConflictKind::CopilotKey)
        }
        Os::Windows
            if env.alt_gr_layout && binding.has_ctrl_alt() && !mods.contains(Mods::META) =>
        {
            Some(ConflictKind::AltGr)
        }
        _ => None,
    }
}

/// Check `bindings` for conflicts in `env` (CAP-43b). Every conflict has a fix.
///
/// Order: per action in [`HotkeyAction::ALL`] order, the key conflict, then a duplicate of
/// an earlier action; then one notice per running voice app.
pub fn check_conflicts(os: Os, bindings: &HotkeyBindings, env: &KeyboardEnv) -> Vec<Conflict> {
    let mut out = Vec::new();
    let mut seen: Vec<(HotkeyAction, Binding)> = Vec::new();
    for (action, binding) in bindings.iter() {
        let own = match binding {
            Binding::Talk(t) => talk_conflict(os, t, env).map(|k| (k, talk_fix(os, k, env))),
            Binding::Chord { .. } => {
                chord_conflict(os, binding, env).map(|k| (k, default_fix(os, action)))
            }
        };
        if let Some((kind, fix)) = own {
            out.push(Conflict::new(kind, action, binding, fix));
        } else if seen.iter().any(|(_, b)| *b == binding) {
            out.push(Conflict::new(
                ConflictKind::Duplicate,
                action,
                binding,
                default_fix(os, action),
            ));
        }
        seen.push((action, binding));
    }
    if let Some(Binding::Talk(talk)) = bindings.get(HotkeyAction::Ptt) {
        for app in &env.voice_apps {
            out.push(Conflict {
                app: Some(*app),
                ..Conflict::new(
                    ConflictKind::VoiceApp,
                    HotkeyAction::Ptt,
                    Binding::Talk(talk),
                    Fix::ChangeOtherApp { app: *app },
                )
            });
        }
    }
    out
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
pub fn first_run_check(os: Os, env: &KeyboardEnv) -> FirstRunReport {
    let defaults = default_bindings(os);
    let conflicts = check_conflicts(os, &defaults, env);
    let mut suggested = defaults;
    if let Some(t) = ladder_pick(os, env) {
        suggested.set(HotkeyAction::Ptt, Binding::Talk(t));
    }
    FirstRunReport {
        conflicts,
        suggested,
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
    pub fn result_json(&self, os: Os) -> Value {
        let conflicts: Vec<Value> = self.conflicts.iter().map(|c| c.to_json(os)).collect();
        json!({ "conflicts": conflicts })
    }
}

/// The binding fits the action: the talk key is modifier-only and exists on `os`, every
/// other key is a chord.
fn fits(os: Os, action: HotkeyAction, binding: Binding) -> bool {
    match binding {
        Binding::Talk(t) => action == HotkeyAction::Ptt && t.on(os),
        Binding::Chord { .. } => action != HotkeyAction::Ptt,
    }
}

/// The pure logic of `hotkeys_set {bindings}`: apply the binding strings in `input` to
/// `current` and return the conflicts. A key that is not an action is an error
/// (`unknown_hotkey_action`). A string that Bottle cannot read, or that does not fit the
/// action, keeps the old binding and gives an [`ConflictKind::Invalid`] conflict.
pub fn hotkeys_set(
    os: Os,
    current: &HotkeyBindings,
    input: &JsonObject,
    env: &KeyboardEnv,
) -> CoreResult<HotkeysSetOutcome> {
    let mut bindings = current.clone();
    let mut invalid = Vec::new();
    for (key, value) in input {
        let action = HotkeyAction::from_key(key).ok_or(CoreError::Unavailable {
            component: ID,
            reason: "unknown_hotkey_action",
        })?;
        let parsed = value
            .as_str()
            .and_then(|s| Binding::parse(os, s).ok())
            .filter(|b| fits(os, action, *b));
        match parsed {
            Some(b) => bindings.set(action, b),
            None => {
                let old = current
                    .get(action)
                    .or_else(|| default_bindings(os).get(action))
                    .unwrap_or(Binding::Talk(TalkBinding::RightOption));
                invalid.push(Conflict::new(
                    ConflictKind::Invalid,
                    action,
                    old,
                    Fix::UseBinding {
                        action,
                        binding: old,
                    },
                ));
            }
        }
    }
    let mut conflicts = invalid;
    conflicts.extend(check_conflicts(os, &bindings, env));
    Ok(HotkeysSetOutcome {
        bindings,
        conflicts,
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
    fn cap_43a_windows_talk_keys_outside_d66_fall_back_to_the_default() {
        // D66 gives Windows only Right-Alt, Right-Ctrl and Ctrl+Win (#313).
        use crate::hotkey_windows::TalkKey as Win;
        assert_eq!(TalkBinding::RightCommand.windows_talk_key(), Win::RightAlt);
        assert_eq!(TalkBinding::RightShift.windows_talk_key(), Win::RightAlt);
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
    fn cap_43b_hotkeys_set_refuses_a_binding_that_does_not_fit_the_action() {
        // The talk key is modifier-only; the other keys are chords.
        for (key, s) in [("ptt", "Ctrl+Shift+K"), ("panic", "RightOption")] {
            let out = hotkeys_set(
                MAC,
                &default_bindings(MAC),
                &input(&[(key, json!(s))]),
                &KeyboardEnv::default(),
            )
            .unwrap();
            assert_eq!(out.bindings, default_bindings(MAC), "{key}");
            assert_eq!(kinds(&out.conflicts), vec![ConflictKind::Invalid], "{key}");
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

    // ---- #313: Windows talk keys outside D66 ----

    #[test]
    fn cap_43b_windows_rejects_right_win_and_right_shift_talk_keys() {
        let cur = default_bindings(WIN);
        for s in ["RightWin", "RightShift", "RightCommand"] {
            let out = hotkeys_set(
                WIN,
                &cur,
                &input(&[("ptt", json!(s))]),
                &KeyboardEnv::default(),
            )
            .unwrap();
            assert_eq!(out.bindings, cur, "{s} keeps the old talk key");
            assert_eq!(kinds(&out.conflicts), vec![ConflictKind::Invalid], "{s}");
            assert_eq!(
                out.conflicts[0].fix,
                Fix::UseBinding {
                    action: HotkeyAction::Ptt,
                    binding: Binding::Talk(TalkBinding::RightOption)
                }
            );
        }
    }

    #[test]
    fn cap_43b_macos_still_accepts_right_command_and_right_shift() {
        for s in ["RightCommand", "RightShift"] {
            let out = hotkeys_set(
                MAC,
                &default_bindings(MAC),
                &input(&[("ptt", json!(s))]),
                &KeyboardEnv::default(),
            )
            .unwrap();
            assert_eq!(out.bindings.get(HotkeyAction::Ptt), Some(b(MAC, s)), "{s}");
            assert_eq!(out.conflicts, vec![], "{s}");
        }
    }

    #[test]
    fn cap_43b_stored_windows_right_shift_is_flagged_with_a_ladder_fix() {
        let mut bs = default_bindings(WIN);
        bs.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightShift));
        let env = KeyboardEnv {
            alt_gr_layout: true,
            ..KeyboardEnv::default()
        };
        let c = check_conflicts(WIN, &bs, &env);
        assert_eq!(kinds(&c), vec![ConflictKind::Invalid]);
        // The fix is the first free step of the D66 ladder (AltGr rules out Right-Alt).
        assert_eq!(
            c[0].fix,
            Fix::UseBinding {
                action: HotkeyAction::Ptt,
                binding: Binding::Talk(TalkBinding::RightControl)
            }
        );
        assert_eq!(
            c[0].message(WIN),
            "Bottle cannot use this talk key. Fix: use Right-Ctrl as the talk key."
        );
    }

    #[test]
    fn cap_43b_stored_windows_right_win_is_flagged() {
        let mut bs = default_bindings(WIN);
        bs.set(HotkeyAction::Ptt, Binding::Talk(TalkBinding::RightCommand));
        let c = check_conflicts(WIN, &bs, &KeyboardEnv::default());
        assert_eq!(kinds(&c), vec![ConflictKind::Invalid]);
    }
}
