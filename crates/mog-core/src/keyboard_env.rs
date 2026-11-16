//! The OS reads of the first-run key check (#322, ADR 0020).
//!
//! The host fills a [`KeyboardEnv`] (ADR 0017) from the reads in [`EnvReads`]: the macOS
//! Dictation shortcut, VoiceOver, the Windows AltGr layout, the Copilot key, and the names of
//! running processes. Each read is behind the trait, so tests use a double and never touch
//! the OS.
//!
//! Binding rules here:
//! - Every read is prompt-free and read-only. No read needs Accessibility or Input
//!   Monitoring, and none opens an OS prompt.
//! - A process name lives only for one call of the visitor. [`read_keyboard_env`] keeps only
//!   the [`VoiceApp`] matches, so no other name is stored. Nothing here logs.
//!
//! Requirements: CAP-43b (AT-30).

use crate::hotkey_bindings::{DictationShortcut, KeyboardEnv, Os, VoiceApp};

/// The prompt-free OS reads of the first-run key check. Each method is one read.
pub trait EnvReads: Send + Sync {
    /// macOS: the Dictation shortcut when Dictation is on, else `None`.
    fn dictation(&self) -> Option<DictationShortcut>;
    /// macOS: VoiceOver is on.
    fn voice_over(&self) -> bool;
    /// Windows: the keyboard layout has AltGr (Right-Alt types characters).
    fn alt_gr_layout(&self) -> bool;
    /// Windows: the keyboard has a Copilot key in place of Right-Ctrl.
    fn copilot_key(&self) -> bool;
    /// Call `visit` once for each running process name. The caller must not keep the name.
    fn visit_process_names(&self, visit: &mut dyn FnMut(&str));
}

/// Build the [`KeyboardEnv`] for `os` from `reads`. Only the reads of `os` run.
pub fn read_keyboard_env(_os: Os, _reads: &dyn EnvReads) -> KeyboardEnv {
    KeyboardEnv::default()
}

/// The `com.apple.symbolichotkeys` entry 164 (Dictation).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SymbolicHotKey {
    /// The `enabled` value.
    pub enabled: bool,
    /// The `value.parameters` numbers.
    pub parameters: Vec<i64>,
}

/// The raw macOS preference values that tell the Dictation state and shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DictationPrefs {
    /// `com.apple.HIToolbox AppleDictationAutoEnable` or
    /// `com.apple.assistant.support "Dictation Enabled"` is on.
    pub enabled: bool,
    /// `com.apple.HIToolbox AppleFnUsageType` (3 = the Globe key starts Dictation).
    pub fn_usage_type: Option<i64>,
    /// `com.apple.symbolichotkeys AppleSymbolicHotKeys` entry 164.
    pub hot_key_164: Option<SymbolicHotKey>,
}

/// The Dictation shortcut of `prefs`, or `None` when Dictation is off or the shortcut is
/// not one that a Bottle key can share.
pub fn decode_dictation(_prefs: &DictationPrefs) -> Option<DictationShortcut> {
    None
}

/// The characters that an AltGr layout types with Ctrl+Alt (Right-Alt).
pub const ALT_GR_PROBE: [char; 10] = ['@', '€', '{', '}', '[', ']', '\\', '|', '~', '#'];

/// `true` if one `VkKeyScanExW` result of the probe characters needs Ctrl and Alt together.
/// A result is the virtual key in the low byte and the shift state in the high byte; `-1`
/// means the layout has no key for the character.
pub fn alt_gr_from_scans(_scans: &[i16]) -> bool {
    false
}

/// The OS of this build. Other platforms use the macOS rules (they have no talk key).
pub fn host_os() -> Os {
    Os::MacOs
}

/// The reads of an OS without a first-run check: everything off, no process.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoEnvReads;

impl EnvReads for NoEnvReads {
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

/// The OS reads of this platform.
pub fn os_env_reads() -> Box<dyn EnvReads> {
    Box::new(NoEnvReads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// An [`EnvReads`] double that counts each read.
    #[derive(Default)]
    struct FakeReads {
        dictation: Option<DictationShortcut>,
        voice_over: bool,
        alt_gr: bool,
        copilot: bool,
        processes: Vec<&'static str>,
        mac_reads: AtomicUsize,
        win_reads: AtomicUsize,
        process_reads: AtomicUsize,
        seen: Mutex<Vec<String>>,
    }

    impl EnvReads for FakeReads {
        fn dictation(&self) -> Option<DictationShortcut> {
            self.mac_reads.fetch_add(1, Ordering::SeqCst);
            self.dictation
        }
        fn voice_over(&self) -> bool {
            self.mac_reads.fetch_add(1, Ordering::SeqCst);
            self.voice_over
        }
        fn alt_gr_layout(&self) -> bool {
            self.win_reads.fetch_add(1, Ordering::SeqCst);
            self.alt_gr
        }
        fn copilot_key(&self) -> bool {
            self.win_reads.fetch_add(1, Ordering::SeqCst);
            self.copilot
        }
        fn visit_process_names(&self, visit: &mut dyn FnMut(&str)) {
            self.process_reads.fetch_add(1, Ordering::SeqCst);
            for p in &self.processes {
                self.seen.lock().unwrap().push((*p).to_owned());
                visit(p);
            }
        }
    }

    fn all_on() -> FakeReads {
        FakeReads {
            dictation: Some(DictationShortcut::ControlTwice),
            voice_over: true,
            alt_gr: true,
            copilot: true,
            processes: vec!["Finder", "superwhisper", "Wispr Flow", "secret-tool"],
            ..FakeReads::default()
        }
    }

    #[test]
    fn cap_43b_macos_env_reads_dictation_voiceover_and_voice_apps_only() {
        let reads = all_on();
        let env = read_keyboard_env(Os::MacOs, &reads);
        assert_eq!(
            env,
            KeyboardEnv {
                dictation: Some(DictationShortcut::ControlTwice),
                voice_over: true,
                alt_gr_layout: false,
                copilot_key: false,
                voice_apps: vec![VoiceApp::Superwhisper, VoiceApp::WisprFlow],
            }
        );
        assert_eq!(reads.mac_reads.load(Ordering::SeqCst), 2);
        assert_eq!(
            reads.win_reads.load(Ordering::SeqCst),
            0,
            "no Windows read on macOS"
        );
        assert_eq!(reads.process_reads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cap_43b_windows_env_reads_layout_copilot_and_voice_apps_only() {
        let reads = FakeReads {
            processes: vec!["explorer.exe", "natspeak.exe", "VoiceAccess.exe"],
            ..all_on()
        };
        let env = read_keyboard_env(Os::Windows, &reads);
        assert_eq!(
            env,
            KeyboardEnv {
                dictation: None,
                voice_over: false,
                alt_gr_layout: true,
                copilot_key: true,
                voice_apps: vec![VoiceApp::Dragon, VoiceApp::VoiceAccess],
            }
        );
        assert_eq!(
            reads.mac_reads.load(Ordering::SeqCst),
            0,
            "no macOS read on Windows"
        );
        assert_eq!(reads.win_reads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cap_43b_voice_apps_keep_only_matches_once_each_in_a_fixed_order() {
        let reads = FakeReads {
            processes: vec![
                "Talon",
                "handy",
                "zsh",
                "Handy",
                "Talon",
                "Superwhisper.app",
            ],
            ..FakeReads::default()
        };
        let env = read_keyboard_env(Os::MacOs, &reads);
        assert_eq!(
            env.voice_apps,
            vec![VoiceApp::Superwhisper, VoiceApp::Handy, VoiceApp::Talon]
        );
        // Every process was offered to the matcher, but only enum values were kept.
        assert_eq!(reads.seen.lock().unwrap().len(), 6);
    }

    #[test]
    fn cap_43b_no_voice_app_running_gives_an_empty_list() {
        let reads = FakeReads {
            processes: vec!["Finder", "Safari", "bash"],
            ..FakeReads::default()
        };
        assert_eq!(read_keyboard_env(Os::MacOs, &reads), KeyboardEnv::default());
        assert_eq!(
            reads.process_reads.load(Ordering::SeqCst),
            1,
            "the processes were read"
        );
    }

    fn hk(enabled: bool, mods: i64) -> Option<SymbolicHotKey> {
        Some(SymbolicHotKey {
            enabled,
            parameters: vec![65535, 65535, mods],
        })
    }

    fn prefs(enabled: bool, fn_usage: Option<i64>, hot: Option<SymbolicHotKey>) -> DictationPrefs {
        DictationPrefs {
            enabled,
            fn_usage_type: fn_usage,
            hot_key_164: hot,
        }
    }

    #[test]
    fn cap_43b_dictation_off_gives_no_shortcut() {
        assert_eq!(
            decode_dictation(&prefs(false, Some(3), hk(true, 0x40000))),
            None
        );
        assert_eq!(decode_dictation(&DictationPrefs::default()), None);
    }

    #[test]
    fn cap_43b_globe_key_dictation_decodes_to_globe() {
        assert_eq!(
            decode_dictation(&prefs(true, Some(3), None)),
            Some(DictationShortcut::Globe)
        );
        // Another Globe use is not Dictation.
        assert_eq!(decode_dictation(&prefs(true, Some(1), None)), None);
    }

    #[test]
    fn cap_43b_double_press_shortcuts_decode_from_entry_164() {
        let cases = [
            (0x40000, DictationShortcut::ControlTwice),
            (0x100010, DictationShortcut::RightCommandTwice),
            (0x100008, DictationShortcut::LeftCommandTwice),
            (0x100000, DictationShortcut::EitherCommandTwice),
        ];
        for (mods, expect) in cases {
            assert_eq!(
                decode_dictation(&prefs(true, None, hk(true, mods))),
                Some(expect),
                "{mods:#x}"
            );
        }
    }

    #[test]
    fn cap_43b_a_disabled_or_unknown_entry_164_gives_no_shortcut() {
        assert_eq!(
            decode_dictation(&prefs(true, None, hk(false, 0x40000))),
            None
        );
        assert_eq!(decode_dictation(&prefs(true, None, hk(true, 0))), None);
        // A key chord (a real key code) is not a double press of a modifier.
        let chord = Some(SymbolicHotKey {
            enabled: true,
            parameters: vec![100, 2, 0x40000],
        });
        assert_eq!(decode_dictation(&prefs(true, None, chord)), None);
        let short = Some(SymbolicHotKey {
            enabled: true,
            parameters: vec![65535],
        });
        assert_eq!(decode_dictation(&prefs(true, None, short)), None);
    }

    #[test]
    fn cap_43b_alt_gr_scan_needs_ctrl_and_alt_together() {
        // US: '@' is Shift+2, '[' is a plain key, '€' has no key.
        assert!(!alt_gr_from_scans(&[0x0132, 0x00DB, -1]));
        // German: '@' is Ctrl+Alt+Q.
        assert!(alt_gr_from_scans(&[0x0651, -1]));
        // Ctrl+Alt+Shift also counts as AltGr.
        assert!(alt_gr_from_scans(&[0x0751]));
        // Ctrl alone or Alt alone is not AltGr.
        assert!(!alt_gr_from_scans(&[0x0251, 0x0451]));
        assert!(!alt_gr_from_scans(&[]));
    }

    #[test]
    fn cap_43b_host_os_matches_the_build() {
        let expect = if cfg!(windows) {
            Os::Windows
        } else {
            Os::MacOs
        };
        assert_eq!(host_os(), expect);
    }

    #[test]
    fn cap_43b_os_reads_never_prompt_or_log() {
        let code = include_str!("keyboard_env.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap()
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
            "kAXTrustedCheckOptionPrompt",
            "AXIsProcessTrustedWithOptions",
            "CGRequestListenEventAccess",
            "IOHIDRequestAccess",
            "std::process::Command",
        ] {
            assert!(!code.contains(banned), "keyboard_env.rs uses {banned}");
        }
        // The real reads exist for both OSes.
        assert!(code.contains("CFPreferencesCopyAppValue"));
        assert!(code.contains("proc_listallpids"));
        assert!(code.contains("VkKeyScanExW"));
        assert!(code.contains("CreateToolhelp32Snapshot"));
    }
}
