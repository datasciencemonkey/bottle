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
pub fn read_keyboard_env(os: Os, reads: &dyn EnvReads) -> KeyboardEnv {
    let mut voice_apps: Vec<VoiceApp> = Vec::new();
    reads.visit_process_names(&mut |name| {
        if let Some(app) = VoiceApp::from_process_name(name) {
            if !voice_apps.contains(&app) {
                voice_apps.push(app);
            }
        }
    });
    voice_apps.sort();
    match os {
        Os::MacOs => KeyboardEnv {
            dictation: reads.dictation(),
            voice_over: reads.voice_over(),
            voice_apps,
            ..KeyboardEnv::default()
        },
        Os::Windows => KeyboardEnv {
            alt_gr_layout: reads.alt_gr_layout(),
            copilot_key: reads.copilot_key(),
            voice_apps,
            ..KeyboardEnv::default()
        },
    }
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
///
/// The double-press shortcuts are entry 164 with no key (`65535, 65535`) and a modifier mask
/// as the third parameter: Control `0x40000`, Command `0x100000`, plus the device bit of the
/// left (`0x8`) or right (`0x10`) Command key. The real-device check of these values is #303.
pub fn decode_dictation(prefs: &DictationPrefs) -> Option<DictationShortcut> {
    const NO_KEY: i64 = 65535;
    if !prefs.enabled {
        return None;
    }
    if prefs.fn_usage_type == Some(3) {
        return Some(DictationShortcut::Globe);
    }
    let hot = prefs.hot_key_164.as_ref().filter(|h| h.enabled)?;
    match hot.parameters.as_slice() {
        [NO_KEY, NO_KEY, mods] => Some(match *mods {
            0x40000 => DictationShortcut::ControlTwice,
            0x100010 => DictationShortcut::RightCommandTwice,
            0x100008 => DictationShortcut::LeftCommandTwice,
            0x100000 => DictationShortcut::EitherCommandTwice,
            _ => return None,
        }),
        _ => None,
    }
}

/// The characters that an AltGr layout types with Ctrl+Alt (Right-Alt).
pub const ALT_GR_PROBE: [char; 10] = ['@', '€', '{', '}', '[', ']', '\\', '|', '~', '#'];

/// `true` if one `VkKeyScanExW` result of the probe characters needs Ctrl and Alt together.
/// A result is the virtual key in the low byte and the shift state in the high byte; `-1`
/// means the layout has no key for the character.
pub fn alt_gr_from_scans(scans: &[i16]) -> bool {
    const CTRL_ALT: i16 = 0x06;
    scans
        .iter()
        .any(|&scan| scan != -1 && (scan >> 8) & CTRL_ALT == CTRL_ALT)
}

/// The OS of this build. Other platforms use the macOS rules (they have no talk key).
pub fn host_os() -> Os {
    if cfg!(windows) {
        Os::Windows
    } else {
        Os::MacOs
    }
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
    #[cfg(target_os = "macos")]
    return Box::new(macos::MacEnvReads);
    #[cfg(windows)]
    return Box::new(win32::WindowsEnvReads);
    #[cfg(not(any(target_os = "macos", windows)))]
    return Box::new(NoEnvReads);
}

/// The macOS reads: CFPreferences values and the BSD process names. None of them prompts.
#[cfg(target_os = "macos")]
mod macos {
    use super::{decode_dictation, DictationPrefs, EnvReads, SymbolicHotKey};
    use crate::hotkey_bindings::DictationShortcut;
    use std::ffi::{c_char, c_int, c_void, CString};
    use std::ptr;

    type CFTypeRef = *const c_void;
    type CFStringRef = *const c_void;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_CF_NUMBER_SINT64_TYPE: c_int = 4;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFPreferencesCopyAppValue(key: CFStringRef, application_id: CFStringRef) -> CFTypeRef;
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: CFTypeRef);
        fn CFGetTypeID(cf: CFTypeRef) -> usize;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(number: CFTypeRef, the_type: c_int, value_ptr: *mut c_void) -> u8;
        fn CFBooleanGetTypeID() -> usize;
        static kCFBooleanTrue: CFTypeRef;
        fn CFDictionaryGetTypeID() -> usize;
        fn CFDictionaryGetValue(dict: CFTypeRef, key: *const c_void) -> *const c_void;
        fn CFArrayGetTypeID() -> usize;
        fn CFArrayGetCount(array: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> *const c_void;
    }

    extern "C" {
        fn proc_listallpids(buffer: *mut c_void, buffersize: c_int) -> c_int;
        fn proc_name(pid: c_int, buffer: *mut c_void, buffersize: u32) -> c_int;
    }

    /// An owned CF object (released on drop).
    struct Owned(CFTypeRef);

    impl Owned {
        fn new(cf: CFTypeRef) -> Option<Owned> {
            // `then`, not `then_some`: a null pointer must never reach `Drop` (CFRelease(NULL) traps).
            (!cf.is_null()).then(|| Owned(cf))
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a non-null object that this process owns (Copy/Create rule).
            unsafe { CFRelease(self.0) }
        }
    }

    fn cf_string(s: &str) -> Option<Owned> {
        let c = CString::new(s).ok()?;
        // SAFETY: `c` is a valid NUL-terminated UTF-8 string for the call.
        Owned::new(unsafe {
            CFStringCreateWithCString(ptr::null(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8)
        })
    }

    /// Read one preference value of another app's domain (read-only, no prompt).
    fn pref(domain: &str, key: &str) -> Option<Owned> {
        let (domain, key) = (cf_string(domain)?, cf_string(key)?);
        // SAFETY: both arguments are valid CFStrings; the result follows the Copy rule.
        Owned::new(unsafe { CFPreferencesCopyAppValue(key.0, domain.0) })
    }

    /// A CFNumber or CFBoolean as an integer. Any other type gives `None`.
    fn as_i64(cf: CFTypeRef) -> Option<i64> {
        if cf.is_null() {
            return None;
        }
        // SAFETY: `cf` is a valid, non-null CF object that the caller keeps alive.
        unsafe {
            let id = CFGetTypeID(cf);
            if id == CFBooleanGetTypeID() {
                // Compare with the shared true object (no `Boolean` return to declare).
                Some(i64::from(cf == kCFBooleanTrue))
            } else if id == CFNumberGetTypeID() {
                let mut v: i64 = 0;
                (CFNumberGetValue(cf, K_CF_NUMBER_SINT64_TYPE, (&mut v as *mut i64).cast()) != 0)
                    .then_some(v)
            } else {
                None
            }
        }
    }

    /// The value of `key` in a CFDictionary, borrowed from the dictionary.
    fn dict_get(dict: CFTypeRef, key: &str) -> Option<CFTypeRef> {
        // SAFETY: `dict` is a valid CF object; the type is checked before the lookup.
        unsafe {
            if dict.is_null() || CFGetTypeID(dict) != CFDictionaryGetTypeID() {
                return None;
            }
            let key = cf_string(key)?;
            let v = CFDictionaryGetValue(dict, key.0);
            (!v.is_null()).then_some(v)
        }
    }

    fn array_i64(array: CFTypeRef) -> Option<Vec<i64>> {
        // SAFETY: `array` is a valid CF object; the type is checked before the reads.
        unsafe {
            if array.is_null() || CFGetTypeID(array) != CFArrayGetTypeID() {
                return None;
            }
            (0..CFArrayGetCount(array))
                .map(|i| as_i64(CFArrayGetValueAtIndex(array, i)))
                .collect()
        }
    }

    fn pref_i64(domain: &str, key: &str) -> Option<i64> {
        pref(domain, key).and_then(|v| as_i64(v.0))
    }

    fn dictation_prefs() -> DictationPrefs {
        let enabled = pref_i64("com.apple.HIToolbox", "AppleDictationAutoEnable") == Some(1)
            || pref_i64("com.apple.assistant.support", "Dictation Enabled") == Some(1);
        let hot_key_164 =
            pref("com.apple.symbolichotkeys", "AppleSymbolicHotKeys").and_then(|all| {
                let entry = dict_get(all.0, "164")?;
                let enabled = dict_get(entry, "enabled").and_then(as_i64) == Some(1);
                let parameters = dict_get(entry, "value")
                    .and_then(|v| dict_get(v, "parameters"))
                    .and_then(array_i64)
                    .unwrap_or_default();
                Some(SymbolicHotKey {
                    enabled,
                    parameters,
                })
            });
        DictationPrefs {
            enabled,
            fn_usage_type: pref_i64("com.apple.HIToolbox", "AppleFnUsageType"),
            hot_key_164,
        }
    }

    #[cfg(test)]
    mod owned_tests {
        use super::*;

        #[test]
        fn cap_43b_a_missing_preference_is_never_released() {
            // A missing value is a null pointer. Releasing it would trap (CFRelease(NULL)).
            assert!(Owned::new(ptr::null()).is_none());
        }
    }

    /// The macOS reads. Each one is a CFPreferences read or a BSD process-name read.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct MacEnvReads;

    impl EnvReads for MacEnvReads {
        fn dictation(&self) -> Option<DictationShortcut> {
            decode_dictation(&dictation_prefs())
        }
        fn voice_over(&self) -> bool {
            pref_i64("com.apple.universalaccess", "voiceOverOnOffKey") == Some(1)
        }
        fn alt_gr_layout(&self) -> bool {
            false
        }
        fn copilot_key(&self) -> bool {
            false
        }
        fn visit_process_names(&self, visit: &mut dyn FnMut(&str)) {
            // SAFETY: a null buffer asks for the count only.
            let count = unsafe { proc_listallpids(ptr::null_mut(), 0) };
            if count <= 0 {
                return;
            }
            // Room for processes that start between the two calls.
            let mut pids: Vec<c_int> = vec![0; count as usize + 32];
            let bytes = (pids.len() * std::mem::size_of::<c_int>()) as c_int;
            // SAFETY: `pids` has room for `bytes` bytes.
            let n = unsafe { proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
            let n = (n.max(0) as usize).min(pids.len());
            let mut buf = [0u8; 256];
            for &pid in &pids[..n] {
                // SAFETY: `buf` has room for `buf.len()` bytes; the result is the length.
                let len = unsafe { proc_name(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
                if len > 0 {
                    if let Ok(name) = std::str::from_utf8(&buf[..len as usize]) {
                        visit(name);
                    }
                }
            }
        }
    }
}

/// The Windows reads: the keyboard layout and the Toolhelp process snapshot. None prompts.
#[cfg(windows)]
mod win32 {
    use super::{alt_gr_from_scans, EnvReads, ALT_GR_PROBE};
    use crate::hotkey_bindings::DictationShortcut;
    use std::ptr;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, VkKeyScanExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };

    /// The Windows reads.
    #[derive(Debug, Default, Clone, Copy)]
    pub struct WindowsEnvReads;

    impl EnvReads for WindowsEnvReads {
        fn dictation(&self) -> Option<DictationShortcut> {
            None
        }
        fn voice_over(&self) -> bool {
            false
        }
        fn alt_gr_layout(&self) -> bool {
            // SAFETY: plain reads. The layout is the one of the foreground window's thread
            // (the user's active input layout); with no foreground window, this thread's.
            let scans: Vec<i16> = unsafe {
                let thread = GetWindowThreadProcessId(GetForegroundWindow(), ptr::null_mut());
                let layout = GetKeyboardLayout(thread);
                ALT_GR_PROBE
                    .iter()
                    .filter_map(|c| u16::try_from(u32::from(*c)).ok())
                    .map(|c| VkKeyScanExW(c, layout))
                    .collect()
            };
            alt_gr_from_scans(&scans)
        }
        fn copilot_key(&self) -> bool {
            // TODO(#332): Windows has no documented prompt-free read for the Copilot key.
            false
        }
        fn visit_process_names(&self, visit: &mut dyn FnMut(&str)) {
            // SAFETY: the snapshot handle is closed below; `entry` has its size set.
            unsafe {
                let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
                if snap == INVALID_HANDLE_VALUE || snap.is_null() {
                    return;
                }
                let mut entry: PROCESSENTRY32W = std::mem::zeroed();
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                let mut more = Process32FirstW(snap, &mut entry) != 0;
                while more {
                    let len = entry
                        .szExeFile
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(entry.szExeFile.len());
                    let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                    visit(&name);
                    more = Process32NextW(snap, &mut entry) != 0;
                }
                CloseHandle(snap);
            }
        }
    }
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
