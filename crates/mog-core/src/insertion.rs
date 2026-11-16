// Adapted from cjpais/Handy src-tauri/src/clipboard.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! R11 `insert_text` (#58, MOG-06a..c, ADR 0044): the UC1 paste of cleaned dictation text into
//! the focused app.
//!
//! The sequence is Handy's clipboard paste (`paste_via_clipboard`): save the clipboard, write the
//! text, send the paste chord, wait, put the earlier clipboard back. Bottle changes:
//! - The text goes on the clipboard only with the concealed and transient flags (SEC-30, #213,
//!   [`crate::window_host::ClipboardWriter`]).
//! - The only key input is the paste chord of [`paste_chord`]. There is no auto-submit, no Enter
//!   and no trailing text, so a multi-line text stays in one field (MOG-06b).
//! - Before any clipboard change, the host checks the input permission, the focused field and a
//!   secure (password) field. Each gives its result code and leaves the clipboard alone.
//! - The restore runs [`RESTORE_AFTER`] after the chord, well inside the 1 s of MOG-06c. If the
//!   clipboard changed after our write (the user copied something), the newer copy wins.
//! - Nothing is logged: no text, no clipboard content (SEC-03).

use crate::error::{CoreError, CoreResult};
use crate::ipc::InsertTextArgs;
use crate::ipc_gateway::GatewayComponents;
use crate::window_host::ClipboardPlatform;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// The reply of `insert_text` (section 4.5): `{result: inserted|no_permission|no_focus|secure_field}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsertOutcome {
    /// The text was pasted into the focused field.
    Inserted,
    /// The OS does not let Bottle send input (macOS Accessibility, or an elevated window on Windows).
    NoPermission,
    /// No app or field has the keyboard focus.
    NoFocus,
    /// The focused field is a secure (password) field.
    SecureField,
}

/// What has the keyboard focus, as the host sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    /// A field (or an app that may take a paste) has the focus.
    Field,
    /// A secure (password) field has the focus.
    SecureField,
    /// Nothing has the focus.
    Nothing,
}

/// One synthetic key event: a platform virtual key code, down or up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyStroke {
    /// The virtual key code (macOS `kVK_*`, Windows `VK_*`).
    pub key: u16,
    /// `true` for key down, `false` for key up.
    pub down: bool,
}

/// macOS virtual key codes (`Carbon/HIToolbox/Events.h`).
pub mod mac_keys {
    /// `kVK_Command`.
    pub const COMMAND: u16 = 0x37;
    /// `kVK_ANSI_V`.
    pub const V: u16 = 0x09;
    /// `kVK_Return`.
    pub const RETURN: u16 = 0x24;
    /// `kVK_ANSI_KeypadEnter`.
    pub const KEYPAD_ENTER: u16 = 0x4C;
}

/// Windows virtual key codes (`WinUser.h`).
pub mod win_keys {
    /// `VK_CONTROL`.
    pub const CONTROL: u16 = 0x11;
    /// `VK_V`.
    pub const V: u16 = 0x56;
    /// `VK_RETURN` (also the keypad Enter, with the extended flag).
    pub const RETURN: u16 = 0x0D;
}

/// The wait between the clipboard write and the chord (Handy `paste_delay_ms` default).
pub const SETTLE_BEFORE_PASTE: Duration = Duration::from_millis(60);
/// How long the modifier stays down after the V key (Handy keeps 100 ms, Handy #165).
pub const CHORD_HOLD: Duration = Duration::from_millis(100);
/// The wait from the modifier release to the restore. The target reads the clipboard when it
/// handles the chord; 250 ms covers a slow app and ends far inside the 1 s of MOG-06c.
pub const RESTORE_AFTER: Duration = Duration::from_millis(250);

/// The paste chord for `platform`: modifier down, V down, V up, modifier up. It never holds
/// Return or Enter (MOG-06b).
pub fn paste_chord(platform: ClipboardPlatform) -> Vec<KeyStroke> {
    let _ = platform;
    Vec::new()
}

/// Whether `key` is an Enter or Return key on `platform`.
pub fn is_enter_key(platform: ClipboardPlatform, key: u16) -> bool {
    match platform {
        ClipboardPlatform::MacOs => key == mac_keys::RETURN || key == mac_keys::KEYPAD_ENTER,
        ClipboardPlatform::Windows => key == win_keys::RETURN,
    }
}

/// The OS side of the paste. Kept behind a trait so the sequence runs in tests without a real
/// clipboard or real key events.
pub trait InsertHost: Send + Sync {
    /// The saved clipboard content.
    type Snapshot;
    /// Whether the OS lets Bottle send key input to the focused app.
    fn input_permitted(&self) -> bool;
    /// What has the keyboard focus.
    fn focus(&self) -> Focus;
    /// Save the whole clipboard content (every item and type).
    fn snapshot_clipboard(&self) -> CoreResult<Self::Snapshot>;
    /// Put `text` on the clipboard with the concealed and transient flags (SEC-30).
    fn write_concealed(&self, text: &str) -> CoreResult<()>;
    /// A value that changes on every clipboard change (macOS `changeCount`, Windows sequence number).
    fn clipboard_mark(&self) -> u64;
    /// Send these key events, in order.
    fn send_keys(&self, keys: &[KeyStroke]) -> CoreResult<()>;
    /// Put the saved content back (an empty snapshot clears the clipboard).
    fn restore_clipboard(&self, snapshot: Self::Snapshot) -> CoreResult<()>;
    /// Wait `d`.
    fn sleep(&self, d: Duration);
}

/// Serializes pastes: overlapping ones would save each other's text as the user's clipboard.
static PASTE_LOCK: Mutex<()> = Mutex::new(());

/// Paste `text` into the focused app (MOG-06a), with the chord only (MOG-06b), and put the
/// earlier clipboard back (MOG-06c). Blocks for about 0.4 s: call it off the UI thread.
pub fn insert_text<H: InsertHost>(
    host: &H,
    platform: ClipboardPlatform,
    text: &str,
) -> CoreResult<InsertOutcome> {
    let _ = (host, platform, text, &PASTE_LOCK, PoisonError::<()>::into_inner);
    Err(CoreError::NotImplemented { component: "R11" })
}

/// The `insert_text` component behind the hardened gateway (ADR 0040, ADR 0044).
#[derive(Debug)]
pub struct InsertTextComponent<H> {
    host: H,
    platform: Option<ClipboardPlatform>,
}

impl<H: InsertHost> InsertTextComponent<H> {
    /// A component that pastes through `host` with the conventions of `platform`.
    /// `None` (no clipboard flags on this OS) refuses every call with `host_failed`.
    pub fn new(host: H, platform: Option<ClipboardPlatform>) -> Self {
        Self { host, platform }
    }
}

impl<H: InsertHost> GatewayComponents for InsertTextComponent<H> {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        let _ = (command, args, &self.host, self.platform, json!({}));
        Err(CoreError::NotImplemented { component: "R11" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq)]
    enum Ev {
        Snapshot,
        Write(String),
        Keys(Vec<KeyStroke>),
        Sleep(Duration),
        Restore(String),
    }

    /// A clipboard and keyboard double. The clipboard is one string; `mark` counts changes.
    struct Fake {
        permitted: bool,
        focus: Focus,
        clipboard: Mutex<String>,
        mark: Mutex<u64>,
        log: Mutex<Vec<Ev>>,
        fail_write: bool,
        fail_keys: bool,
        /// Simulates the user copying "newer" during the restore wait.
        user_copies_during_wait: bool,
    }

    impl Fake {
        fn new(earlier: &str) -> Self {
            Self {
                permitted: true,
                focus: Focus::Field,
                clipboard: Mutex::new(earlier.to_owned()),
                mark: Mutex::new(1),
                log: Mutex::new(Vec::new()),
                fail_write: false,
                fail_keys: false,
                user_copies_during_wait: false,
            }
        }
        fn log(&self) -> Vec<Ev> {
            self.log.lock().unwrap().clone()
        }
        fn clip(&self) -> String {
            self.clipboard.lock().unwrap().clone()
        }
        fn keys(&self) -> Vec<KeyStroke> {
            self.log()
                .into_iter()
                .filter_map(|e| match e {
                    Ev::Keys(k) => Some(k),
                    _ => None,
                })
                .flatten()
                .collect()
        }
    }

    impl InsertHost for Fake {
        type Snapshot = String;
        fn input_permitted(&self) -> bool {
            self.permitted
        }
        fn focus(&self) -> Focus {
            self.focus
        }
        fn snapshot_clipboard(&self) -> CoreResult<String> {
            self.log.lock().unwrap().push(Ev::Snapshot);
            Ok(self.clip())
        }
        fn write_concealed(&self, text: &str) -> CoreResult<()> {
            self.log.lock().unwrap().push(Ev::Write(text.to_owned()));
            if self.fail_write {
                return Err(CoreError::NotImplemented { component: "R11" });
            }
            *self.clipboard.lock().unwrap() = text.to_owned();
            *self.mark.lock().unwrap() += 1;
            Ok(())
        }
        fn clipboard_mark(&self) -> u64 {
            *self.mark.lock().unwrap()
        }
        fn send_keys(&self, keys: &[KeyStroke]) -> CoreResult<()> {
            self.log.lock().unwrap().push(Ev::Keys(keys.to_vec()));
            if self.fail_keys {
                return Err(CoreError::NotImplemented { component: "R11" });
            }
            Ok(())
        }
        fn restore_clipboard(&self, snapshot: String) -> CoreResult<()> {
            self.log.lock().unwrap().push(Ev::Restore(snapshot.clone()));
            *self.clipboard.lock().unwrap() = snapshot;
            *self.mark.lock().unwrap() += 1;
            Ok(())
        }
        fn sleep(&self, d: Duration) {
            self.log.lock().unwrap().push(Ev::Sleep(d));
            if d == RESTORE_AFTER && self.user_copies_during_wait {
                *self.clipboard.lock().unwrap() = "newer".to_owned();
                *self.mark.lock().unwrap() += 1;
            }
        }
    }

    const BOTH: [ClipboardPlatform; 2] = [ClipboardPlatform::MacOs, ClipboardPlatform::Windows];

    #[test]
    fn mog_06a_pastes_the_text_with_the_chord_and_reports_inserted() {
        for p in BOTH {
            let f = Fake::new("earlier");
            assert_eq!(insert_text(&f, p, "hello").unwrap(), InsertOutcome::Inserted);
            let log = f.log();
            assert_eq!(log[0], Ev::Snapshot);
            assert_eq!(log[1], Ev::Write("hello".into()));
            assert_eq!(f.keys(), paste_chord(p), "{p:?}");
        }
    }

    #[test]
    fn mog_06a_the_chord_is_modifier_v_down_then_up() {
        let mac = paste_chord(ClipboardPlatform::MacOs);
        let ks = |key, down| KeyStroke { key, down };
        assert_eq!(
            mac,
            vec![
                ks(mac_keys::COMMAND, true),
                ks(mac_keys::V, true),
                ks(mac_keys::V, false),
                ks(mac_keys::COMMAND, false),
            ]
        );
        let win = paste_chord(ClipboardPlatform::Windows);
        assert_eq!(
            win,
            vec![
                ks(win_keys::CONTROL, true),
                ks(win_keys::V, true),
                ks(win_keys::V, false),
                ks(win_keys::CONTROL, false),
            ]
        );
    }

    #[test]
    fn mog_06a_the_modifier_is_held_between_v_and_its_release() {
        let f = Fake::new("earlier");
        insert_text(&f, ClipboardPlatform::MacOs, "hello").unwrap();
        let log = f.log();
        let first_keys = log.iter().position(|e| matches!(e, Ev::Keys(_))).unwrap();
        assert_eq!(log[first_keys + 1], Ev::Sleep(CHORD_HOLD));
        assert!(matches!(&log[first_keys + 2], Ev::Keys(k) if k.len() == 1 && !k[0].down));
        // The text is on the clipboard before the chord.
        assert!(log[..first_keys].contains(&Ev::Sleep(SETTLE_BEFORE_PASTE)));
    }

    #[test]
    fn mog_06b_never_sends_enter_and_keeps_multi_line_text_in_one_write() {
        for p in BOTH {
            let f = Fake::new("earlier");
            let text = "line one\nline two\r\nline three\n";
            assert_eq!(insert_text(&f, p, text).unwrap(), InsertOutcome::Inserted);
            let writes: Vec<_> = f
                .log()
                .into_iter()
                .filter(|e| matches!(e, Ev::Write(_)))
                .collect();
            assert_eq!(writes, vec![Ev::Write(text.into())], "one write, unchanged text");
            assert!(!f.keys().is_empty());
            assert!(f.keys().iter().all(|k| !is_enter_key(p, k.key)), "{p:?}");
        }
    }

    #[test]
    fn mog_06b_no_chord_key_is_an_enter_key() {
        for p in BOTH {
            assert!(paste_chord(p).iter().all(|k| !is_enter_key(p, k.key)));
        }
        assert!(is_enter_key(ClipboardPlatform::MacOs, mac_keys::RETURN));
        assert!(is_enter_key(ClipboardPlatform::MacOs, mac_keys::KEYPAD_ENTER));
        assert!(is_enter_key(ClipboardPlatform::Windows, win_keys::RETURN));
    }

    #[test]
    fn mog_06c_restores_the_earlier_clipboard_after_the_paste() {
        for p in BOTH {
            let f = Fake::new("earlier");
            insert_text(&f, p, "hello").unwrap();
            assert_eq!(f.clip(), "earlier");
            let log = f.log();
            let n = log.len();
            assert_eq!(log[n - 2], Ev::Sleep(RESTORE_AFTER));
            assert_eq!(log[n - 1], Ev::Restore("earlier".into()));
        }
    }

    #[test]
    fn mog_06c_restore_runs_well_inside_one_second() {
        assert!(CHORD_HOLD + RESTORE_AFTER < Duration::from_millis(500));
        assert!(SETTLE_BEFORE_PASTE + CHORD_HOLD + RESTORE_AFTER < Duration::from_secs(1));
    }

    #[test]
    fn mog_06c_a_newer_copy_during_the_wait_wins() {
        let mut f = Fake::new("earlier");
        f.user_copies_during_wait = true;
        insert_text(&f, ClipboardPlatform::MacOs, "hello").unwrap();
        assert_eq!(f.clip(), "newer");
        assert!(!f.log().iter().any(|e| matches!(e, Ev::Restore(_))));
    }

    #[test]
    fn mog_06c_failed_chord_still_releases_the_modifier_and_restores() {
        let mut f = Fake::new("earlier");
        f.fail_keys = true;
        assert!(insert_text(&f, ClipboardPlatform::Windows, "hello").is_err());
        assert_eq!(f.clip(), "earlier");
        let ups: Vec<_> = f.keys().into_iter().filter(|k| !k.down).collect();
        assert!(ups.contains(&KeyStroke {
            key: win_keys::CONTROL,
            down: false
        }));
    }

    #[test]
    fn mog_06c_failed_write_restores_and_sends_no_keys() {
        let mut f = Fake::new("earlier");
        f.fail_write = true;
        assert!(insert_text(&f, ClipboardPlatform::MacOs, "hello").is_err());
        assert!(f.keys().is_empty());
        assert_eq!(f.clip(), "earlier");
        assert!(f.log().contains(&Ev::Restore("earlier".into())));
    }

    #[test]
    fn mog_06a_no_permission_no_focus_and_secure_field_touch_nothing() {
        let cases = [
            (false, Focus::Field, InsertOutcome::NoPermission),
            (true, Focus::Nothing, InsertOutcome::NoFocus),
            (true, Focus::SecureField, InsertOutcome::SecureField),
        ];
        for (permitted, focus, want) in cases {
            let mut f = Fake::new("earlier");
            f.permitted = permitted;
            f.focus = focus;
            assert_eq!(insert_text(&f, ClipboardPlatform::MacOs, "pw").unwrap(), want);
            assert!(f.log().is_empty(), "{want:?}: {:?}", f.log());
            assert_eq!(f.clip(), "earlier");
        }
    }

    #[test]
    fn mog_06a_component_replies_with_the_result_code() {
        let c = InsertTextComponent::new(Fake::new("e"), Some(ClipboardPlatform::MacOs));
        let args = json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "hi"});
        assert_eq!(
            c.call("insert_text", args.clone()).unwrap(),
            json!({"result": "inserted"})
        );
        let mut secure = Fake::new("e");
        secure.focus = Focus::SecureField;
        let c = InsertTextComponent::new(secure, Some(ClipboardPlatform::MacOs));
        assert_eq!(
            c.call("insert_text", args.clone()).unwrap(),
            json!({"result": "secure_field"})
        );
        for (outcome, code) in [
            (InsertOutcome::NoPermission, "no_permission"),
            (InsertOutcome::NoFocus, "no_focus"),
        ] {
            assert_eq!(serde_json::to_value(outcome).unwrap(), json!(code));
        }
    }

    #[test]
    fn mog_06a_component_refuses_other_commands_bad_args_and_no_platform() {
        let args = json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "hi"});
        let c = InsertTextComponent::new(Fake::new("e"), Some(ClipboardPlatform::MacOs));
        assert_eq!(
            c.call("config_list", json!({})).unwrap_err().code(),
            "not_implemented"
        );
        assert_eq!(
            c.call("insert_text", json!({"text": 1})).unwrap_err().code(),
            "invalid_args"
        );
        let none = InsertTextComponent::new(Fake::new("e"), None);
        assert_eq!(
            none.call("insert_text", args).unwrap_err().code(),
            "host_failed"
        );
        assert!(none.host.log().is_empty());
    }

    #[test]
    fn mog_06b_component_errors_carry_no_text() {
        let mut f = Fake::new("e");
        f.fail_write = true;
        let c = InsertTextComponent::new(f, Some(ClipboardPlatform::MacOs));
        let args = json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "secret words"});
        let err = c.call("insert_text", args).unwrap_err();
        assert_eq!(err.code(), "host_failed");
        assert!(!err.to_string().contains("secret"));
    }
}
