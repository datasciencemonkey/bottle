// Adapted from cjpais/Handy src-tauri/src/clipboard.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! R11 `insert_text` (#58, MOG-06a..c, ADR 0048): the UC1 paste of cleaned dictation text into
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
use crate::window_host::{ClipboardFormat, ClipboardPlatform};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
pub use macos::MacInsertHost;
#[cfg(windows)]
pub use windows::{WindowsClipboard, WindowsInsertHost};

/// The reply of `insert_text` (section 4.5):
/// `{result: inserted|no_permission|no_focus|secure_field|target_changed}`.
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
    /// ADR 0118 (UX slice 2): the focused app is not the app that had the focus when the
    /// capture lease was granted. No key event was sent; the clipboard is as ADR 0048 restores it.
    TargetChanged,
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

/// The macOS pasteboard types that mark a secret or a short-lived item (nspasteboard.org). A
/// password manager writes them and clears its item after some seconds.
pub const MAC_CONCEALED_TYPES: [&str; 2] = [
    "org.nspasteboard.ConcealedType",
    "org.nspasteboard.TransientType",
];

/// Whether the types of a saved macOS clipboard mark it as concealed or transient (ADR 0048,
/// decision 12). Bottle then leaves the clipboard empty instead of restoring the secret.
pub fn mac_snapshot_is_concealed<'a>(types: impl IntoIterator<Item = &'a str>) -> bool {
    types.into_iter().any(|t| MAC_CONCEALED_TYPES.contains(&t))
}

/// The saved (type, data) pairs of one macOS pasteboard item. A type with no readable data is
/// left out, except a concealed or transient marker: it is kept with no data, so the secret
/// check still sees it (ADR 0048, decision 12; as on Windows).
pub fn mac_saved_item(
    types: impl IntoIterator<Item = (String, Option<Vec<u8>>)>,
) -> Vec<(String, Vec<u8>)> {
    types
        .into_iter()
        .filter_map(|(ty, data)| match data {
            Some(d) => Some((ty, d)),
            None if MAC_CONCEALED_TYPES.contains(&ty.as_str()) => Some((ty, Vec::new())),
            None => None,
        })
        .collect()
}

/// Windows: the format that keeps a clipboard monitor away from the content.
pub const WIN_EXCLUDE_FROM_MONITOR: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// Windows: the `DWORD` format whose 0 keeps the content out of the clipboard history.
pub const WIN_CAN_INCLUDE_IN_HISTORY: &str = "CanIncludeInClipboardHistory";
/// Windows: the `DWORD` format whose 0 keeps the content off the cloud clipboard.
pub const WIN_CAN_UPLOAD_TO_CLOUD: &str = "CanUploadToCloudClipboard";

/// Whether the (format name, data) pairs of a saved Windows clipboard mark it as concealed
/// (ADR 0048, decision 12): the monitor opt-out is present, or a history or cloud flag is 0.
pub fn windows_snapshot_is_concealed<'a>(
    formats: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> bool {
    formats.into_iter().any(|(name, data)| match name {
        WIN_EXCLUDE_FROM_MONITOR => true,
        // The flag is a DWORD. A short or empty value counts as 0: fail closed for a secret.
        WIN_CAN_INCLUDE_IN_HISTORY | WIN_CAN_UPLOAD_TO_CLOUD => {
            data.iter().take(4).all(|b| *b == 0)
        }
        _ => false,
    })
}

/// The order of the Windows clipboard write (ADR 0048, decision 8): the opt-out formats first,
/// the text (`CF_UNICODETEXT`) last. If an opt-out fails, the close publishes no text.
pub fn windows_write_order(formats: &[ClipboardFormat]) -> Vec<&ClipboardFormat> {
    let (text, flags): (Vec<_>, Vec<_>) = formats.iter().partition(|f| f.name == "CF_UNICODETEXT");
    flags.into_iter().chain(text).collect()
}

/// Windows edit-control constants (`WinUser.h`).
pub mod win_edit {
    /// `ES_PASSWORD`, in `GWL_STYLE`.
    pub const ES_PASSWORD: u32 = 0x20;
    /// `EM_GETPASSWORDCHAR`: a nonzero reply is the mask character of a password box.
    pub const EM_GETPASSWORDCHAR: u32 = 0x00D2;
    /// The class-name buffer, in UTF-16 units (a window class name has at most 256).
    pub const CLASS_BUFFER: usize = 256;
}

/// Whether `class` is a Win32 edit control: `Edit`, `RichEdit20W`, `RichEdit50W`,
/// `RICHEDIT60W` or a WinForms `TextBox` (`WindowsForms10.EDIT.*`). Case does not matter.
pub fn is_windows_edit_class(class: &str) -> bool {
    let c = class.to_ascii_lowercase();
    matches!(
        c.as_str(),
        "edit" | "richedit20w" | "richedit50w" | "richedit60w"
    ) || c.starts_with("windowsforms10.edit.")
}

/// The focus of a focused Win32 control, from its class name, its `GWL_STYLE` and its
/// `EM_GETPASSWORDCHAR` reply (ADR 0048, decision 3). An edit control with `ES_PASSWORD` or a
/// mask character is a secure field. A control of an unknown class (a superclassed or
/// owner-drawn edit can have any name) with either signal is a secure field too: fail closed.
pub fn windows_control_focus(class: &str, style: u32, password_char: usize) -> Focus {
    let password = style & win_edit::ES_PASSWORD != 0 || password_char != 0;
    match (is_windows_edit_class(class), password) {
        (_, false) => Focus::Field,
        (true, true) | (false, true) => Focus::SecureField,
    }
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
    let (modifier, v) = match platform {
        ClipboardPlatform::MacOs => (mac_keys::COMMAND, mac_keys::V),
        ClipboardPlatform::Windows => (win_keys::CONTROL, win_keys::V),
    };
    let ks = |key, down| KeyStroke { key, down };
    vec![
        ks(modifier, true),
        ks(v, true),
        ks(v, false),
        ks(modifier, false),
    ]
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
    /// The saved clipboard content. The default value is an empty clipboard.
    type Snapshot: Default;
    /// Whether the OS lets Bottle send key input to the focused app.
    fn input_permitted(&self) -> bool;
    /// What has the keyboard focus.
    fn focus(&self) -> Focus;
    /// The identity of the focused element (macOS: the AX element and its app; Windows: the
    /// focused window). It changes when the focus moves. 0 when nothing has the focus.
    fn focus_target(&self) -> u64;
    /// Save the whole clipboard content (every item and type).
    fn snapshot_clipboard(&self) -> CoreResult<Self::Snapshot>;
    /// Whether the saved content is a concealed or transient item (a password manager secret).
    fn is_concealed(&self, snapshot: &Self::Snapshot) -> bool;
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

impl<H: InsertHost> InsertHost for &H {
    type Snapshot = H::Snapshot;
    fn input_permitted(&self) -> bool {
        (**self).input_permitted()
    }
    fn focus(&self) -> Focus {
        (**self).focus()
    }
    fn focus_target(&self) -> u64 {
        (**self).focus_target()
    }
    fn snapshot_clipboard(&self) -> CoreResult<Self::Snapshot> {
        (**self).snapshot_clipboard()
    }
    fn is_concealed(&self, snapshot: &Self::Snapshot) -> bool {
        (**self).is_concealed(snapshot)
    }
    fn write_concealed(&self, text: &str) -> CoreResult<()> {
        (**self).write_concealed(text)
    }
    fn clipboard_mark(&self) -> u64 {
        (**self).clipboard_mark()
    }
    fn send_keys(&self, keys: &[KeyStroke]) -> CoreResult<()> {
        (**self).send_keys(keys)
    }
    fn restore_clipboard(&self, snapshot: Self::Snapshot) -> CoreResult<()> {
        (**self).restore_clipboard(snapshot)
    }
    fn sleep(&self, d: Duration) {
        (**self).sleep(d)
    }
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
    let _paste = PASTE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    // Check first, so a refusal leaves the clipboard and the keyboard alone (MOG-06d shows Copy).
    if !host.input_permitted() {
        return Ok(InsertOutcome::NoPermission);
    }
    match host.focus() {
        Focus::Nothing => return Ok(InsertOutcome::NoFocus),
        Focus::SecureField => return Ok(InsertOutcome::SecureField),
        Focus::Field => {}
    }
    let target = host.focus_target();
    let saved = host.snapshot_clipboard()?;
    // A password manager secret is not restored: the restore gives it a new change count, and the
    // manager's auto-clear then never fires. An empty snapshot leaves the clipboard empty.
    let saved = if host.is_concealed(&saved) {
        H::Snapshot::default()
    } else {
        saved
    };
    if let Err(e) = host.write_concealed(text) {
        // The writer cleared the clipboard (fail closed); put the user's content back.
        let _ = host.restore_clipboard(saved);
        return Err(e);
    }
    let ours = host.clipboard_mark();
    host.sleep(SETTLE_BEFORE_PASTE);
    // The snapshot and the settle wait take time, and the chord goes to whatever has the focus
    // then. Check again: if the focus moved, became a secure field or the permission went away,
    // send no key and put the earlier clipboard back (ADR 0048, decision 14).
    if let Some(refusal) = refusal_before_chord(host, target) {
        if host.clipboard_mark() == ours {
            host.restore_clipboard(saved)?;
        }
        return Ok(refusal);
    }
    let chord = paste_chord(platform);
    let (press, release) = chord.split_at(chord.len() - 1);
    let pressed = host.send_keys(press);
    host.sleep(CHORD_HOLD);
    // Always release the modifier, even when a press failed.
    let released = host.send_keys(release);
    host.sleep(RESTORE_AFTER);
    // A copy made after our write is the user's newest action: it wins (Handy paste_tx rule 2).
    let restored = if host.clipboard_mark() == ours {
        host.restore_clipboard(saved)
    } else {
        Ok(())
    };
    pressed.and(released).and(restored)?;
    Ok(InsertOutcome::Inserted)
}

/// The refusal when the target of the chord is not the field checked before the snapshot.
fn refusal_before_chord<H: InsertHost>(host: &H, target: u64) -> Option<InsertOutcome> {
    if !host.input_permitted() {
        return Some(InsertOutcome::NoPermission);
    }
    match host.focus() {
        Focus::Nothing => Some(InsertOutcome::NoFocus),
        Focus::SecureField => Some(InsertOutcome::SecureField),
        // Another field: the checked field lost the focus.
        Focus::Field if host.focus_target() != target => Some(InsertOutcome::NoFocus),
        Focus::Field => None,
    }
}

/// The `insert_text` component behind the hardened gateway (ADR 0040, ADR 0048).
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
        let refused = |code| CoreError::IpcRefused { code };
        if command != "insert_text" {
            return Err(refused("not_implemented"));
        }
        let a: InsertTextArgs =
            serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        let platform = self.platform.ok_or(refused("host_failed"))?;
        // A host error never carries the text; it crosses only as `host_failed` (SEC-03).
        let outcome =
            insert_text(&self.host, platform, &a.text).map_err(|_| refused("host_failed"))?;
        Ok(json!({ "result": outcome }))
    }
}

/// The `insert_text` component over the OS of this build: macOS and Windows.
#[cfg(target_os = "macos")]
pub fn os_component() -> InsertTextComponent<MacInsertHost> {
    InsertTextComponent::new(
        MacInsertHost::default(),
        crate::window_host::current_clipboard_platform(),
    )
}

/// The `insert_text` component over the OS of this build: macOS and Windows.
#[cfg(windows)]
pub fn os_component() -> InsertTextComponent<WindowsInsertHost> {
    InsertTextComponent::new(
        WindowsInsertHost::default(),
        crate::window_host::current_clipboard_platform(),
    )
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
        /// The earlier clipboard is a password manager item (concealed or transient).
        concealed: bool,
        /// The identity of the focused element.
        target: u64,
        /// What the permission, the focus and the focused element become once the snapshot is
        /// taken: the user moved the focus while the clipboard was saved.
        after_snapshot: Option<(bool, Focus, u64)>,
        live: Mutex<Option<(bool, Focus, u64)>>,
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
                concealed: false,
                target: 7,
                after_snapshot: None,
                live: Mutex::new(None),
            }
        }
        fn now(&self) -> (bool, Focus, u64) {
            self.live
                .lock()
                .unwrap()
                .unwrap_or((self.permitted, self.focus, self.target))
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
            self.now().0
        }
        fn focus(&self) -> Focus {
            self.now().1
        }
        fn focus_target(&self) -> u64 {
            self.now().2
        }
        fn snapshot_clipboard(&self) -> CoreResult<String> {
            self.log.lock().unwrap().push(Ev::Snapshot);
            if let Some(after) = self.after_snapshot {
                *self.live.lock().unwrap() = Some(after);
            }
            Ok(self.clip())
        }
        fn is_concealed(&self, snapshot: &String) -> bool {
            self.concealed && !snapshot.is_empty()
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
            assert_eq!(
                insert_text(&f, p, "hello").unwrap(),
                InsertOutcome::Inserted
            );
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
            assert_eq!(
                writes,
                vec![Ev::Write(text.into())],
                "one write, unchanged text"
            );
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
        assert!(is_enter_key(
            ClipboardPlatform::MacOs,
            mac_keys::KEYPAD_ENTER
        ));
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
            assert_eq!(
                insert_text(&f, ClipboardPlatform::MacOs, "pw").unwrap(),
                want
            );
            assert!(f.log().is_empty(), "{want:?}: {:?}", f.log());
            assert_eq!(f.clip(), "earlier");
        }
    }

    #[test]
    fn mog_06a_component_replies_with_the_result_code() {
        let c = InsertTextComponent::new(Fake::new("e"), Some(ClipboardPlatform::MacOs));
        let args =
            json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "hi", "leaseId": "1"});
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
        let args =
            json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "hi", "leaseId": "1"});
        let c = InsertTextComponent::new(Fake::new("e"), Some(ClipboardPlatform::MacOs));
        assert_eq!(
            c.call("config_list", json!({})).unwrap_err().code(),
            "not_implemented"
        );
        assert_eq!(
            c.call("insert_text", json!({"text": 1}))
                .unwrap_err()
                .code(),
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
        let args = json!({"mogId": "00000000-0000-4000-8000-0000000000a1", "text": "secret words", "leaseId": "1"});
        let err = c.call("insert_text", args).unwrap_err();
        assert_eq!(err.code(), "host_failed");
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn mog_06c_a_concealed_clipboard_is_left_empty_not_restored() {
        for p in BOTH {
            let mut f = Fake::new("pw-from-manager");
            f.concealed = true;
            assert_eq!(
                insert_text(&f, p, "hello").unwrap(),
                InsertOutcome::Inserted
            );
            assert_eq!(f.clip(), "", "{p:?}");
            assert!(
                !f.log().contains(&Ev::Restore("pw-from-manager".into())),
                "{p:?}: {:?}",
                f.log()
            );
        }
    }

    #[test]
    fn mog_06c_a_concealed_clipboard_stays_empty_when_the_write_fails() {
        let mut f = Fake::new("pw-from-manager");
        f.concealed = true;
        f.fail_write = true;
        assert!(insert_text(&f, ClipboardPlatform::MacOs, "hello").is_err());
        assert_eq!(f.clip(), "");
    }

    #[test]
    fn mog_06c_mac_concealed_or_transient_types_mark_a_secret() {
        let manager = ["public.utf8-plain-text", "org.nspasteboard.ConcealedType"];
        assert!(mac_snapshot_is_concealed(manager));
        let transient = ["public.utf8-plain-text", "org.nspasteboard.TransientType"];
        assert!(mac_snapshot_is_concealed(transient));
        let plain = ["public.utf8-plain-text", "public.html"];
        assert!(!mac_snapshot_is_concealed(plain));
        assert!(!mac_snapshot_is_concealed([]));
    }

    #[test]
    fn mog_06c_windows_monitor_or_history_opt_outs_mark_a_secret() {
        let text: &[u8] = &[b'p', 0, b'w', 0, 0, 0];
        let zero: &[u8] = &[0, 0, 0, 0];
        let one: &[u8] = &[1, 0, 0, 0];
        assert!(windows_snapshot_is_concealed([
            ("CF_UNICODETEXT", text),
            (WIN_EXCLUDE_FROM_MONITOR, &[][..]),
        ]));
        assert!(windows_snapshot_is_concealed([
            ("CF_UNICODETEXT", text),
            (WIN_CAN_INCLUDE_IN_HISTORY, zero),
        ]));
        assert!(windows_snapshot_is_concealed([
            ("CF_UNICODETEXT", text),
            (WIN_CAN_UPLOAD_TO_CLOUD, zero),
        ]));
        // A flag of 1 allows the history and the cloud: the content is not a secret.
        assert!(!windows_snapshot_is_concealed([
            ("CF_UNICODETEXT", text),
            (WIN_CAN_INCLUDE_IN_HISTORY, one),
            (WIN_CAN_UPLOAD_TO_CLOUD, one),
        ]));
        assert!(!windows_snapshot_is_concealed([("CF_UNICODETEXT", text)]));
    }

    #[test]
    fn mog_06a_windows_edit_classes_include_rich_edit_and_winforms() {
        for c in [
            "Edit",
            "EDIT",
            "RichEdit20W",
            "RichEdit50W",
            "RICHEDIT60W",
            "WindowsForms10.EDIT.app.0.141b42a_r6_ad1",
        ] {
            assert!(is_windows_edit_class(c), "{c}");
        }
        for c in [
            "Button",
            "Chrome_RenderWidgetHostHWND",
            "WindowsForms10.BUTTON.app.0.1",
        ] {
            assert!(!is_windows_edit_class(c), "{c}");
        }
    }

    #[test]
    fn mog_06a_windows_password_box_is_a_secure_field_by_style_or_mask_char() {
        use win_edit::ES_PASSWORD;
        let winforms = "WindowsForms10.EDIT.app.0.141b42a_r6_ad1";
        for class in ["Edit", "RichEdit50W", winforms, "SomeOwnerDrawnEdit"] {
            assert_eq!(
                windows_control_focus(class, ES_PASSWORD, 0),
                Focus::SecureField,
                "{class} style"
            );
            assert_eq!(
                windows_control_focus(class, 0, usize::from(b'*')),
                Focus::SecureField,
                "{class} mask char"
            );
            assert_eq!(windows_control_focus(class, 0, 0), Focus::Field, "{class}");
        }
    }
    #[test]
    fn mog_06a_the_focus_is_checked_again_right_before_the_chord() {
        // The user moves the focus while the clipboard is saved: to a password field, to
        // nothing, to another field, or the permission goes away. No key is sent, the earlier
        // clipboard is back, and the reply is the refusal code (MOG-06d shows Copy).
        let cases = [
            ((true, Focus::SecureField, 7), InsertOutcome::SecureField),
            ((true, Focus::Nothing, 7), InsertOutcome::NoFocus),
            ((true, Focus::Field, 8), InsertOutcome::NoFocus),
            ((false, Focus::Field, 7), InsertOutcome::NoPermission),
        ];
        for p in BOTH {
            for (after, want) in cases {
                let mut f = Fake::new("earlier");
                f.after_snapshot = Some(after);
                assert_eq!(
                    insert_text(&f, p, "hello").unwrap(),
                    want,
                    "{p:?} {after:?}"
                );
                assert!(f.keys().is_empty(), "{p:?} {after:?}: {:?}", f.log());
                assert_eq!(f.clip(), "earlier", "{p:?} {after:?}");
                // The check comes after the settle wait, so it sees the focus of the chord.
                let log = f.log();
                let settle = log
                    .iter()
                    .position(|e| *e == Ev::Sleep(SETTLE_BEFORE_PASTE));
                assert!(settle.is_some(), "{p:?} {after:?}: {log:?}");
            }
        }
    }

    #[test]
    fn mog_06a_an_unchanged_focus_still_pastes() {
        let mut f = Fake::new("earlier");
        f.after_snapshot = Some((true, Focus::Field, 7));
        assert_eq!(
            insert_text(&f, ClipboardPlatform::MacOs, "hello").unwrap(),
            InsertOutcome::Inserted
        );
        assert_eq!(f.keys(), paste_chord(ClipboardPlatform::MacOs));
    }

    #[test]
    fn mog_06c_mac_a_concealed_marker_without_data_is_still_saved() {
        // A password manager can declare the marker type with no data. The saved item keeps it
        // (empty), so the secret check still sees it, as on Windows.
        let item = mac_saved_item([
            ("public.utf8-plain-text".to_owned(), Some(b"pw".to_vec())),
            ("org.nspasteboard.ConcealedType".to_owned(), None),
            ("public.html".to_owned(), None),
        ]);
        assert_eq!(
            item,
            vec![
                ("public.utf8-plain-text".to_owned(), b"pw".to_vec()),
                ("org.nspasteboard.ConcealedType".to_owned(), Vec::new()),
            ]
        );
        assert!(mac_snapshot_is_concealed(
            item.iter().map(|(t, _)| t.as_str())
        ));
        // A plain type with no data is not saved: there is nothing to put back.
        let plain = mac_saved_item([("public.html".to_owned(), None)]);
        assert!(plain.is_empty());
    }

    #[test]
    fn sec_30_windows_writes_the_opt_outs_first_and_the_text_last() {
        let formats = crate::window_host::clipboard_formats(ClipboardPlatform::Windows, "hi");
        let order: Vec<&str> = windows_write_order(&formats)
            .iter()
            .map(|f| f.name)
            .collect();
        assert_eq!(
            order,
            vec![
                WIN_EXCLUDE_FROM_MONITOR,
                WIN_CAN_INCLUDE_IN_HISTORY,
                WIN_CAN_UPLOAD_TO_CLOUD,
                "CF_UNICODETEXT",
            ]
        );
        assert_eq!(windows_write_order(&formats).len(), formats.len());
    }
}
