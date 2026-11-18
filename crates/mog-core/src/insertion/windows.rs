// Adapted from cjpais/Handy src-tauri/src/utils.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! The Windows side of `insert_text` (#58, ADR 0048).
//!
//! - Permission: Windows drops input sent to a window of a higher integrity level (an app run as
//!   administrator) and `SendInput` does not say so. Handy's check
//!   (`foreground_window_is_elevated`) runs first; a blocked window is `no_permission`.
//! - Focus: no foreground window is `no_focus`. The focused control is `secure_field` when it has
//!   `ES_PASSWORD` or answers `EM_GETPASSWORDCHAR` with a mask character
//!   ([`super::windows_control_focus`]). Browser and UWP password fields need UI Automation (#409).
//! - Clipboard: [`WindowsClipboard`] writes all formats of
//!   [`crate::window_host::clipboard_formats`] (text plus the history, cloud and monitor
//!   opt-outs, SEC-30) in one open and close, so a listener never sees the text alone. It sets the
//!   opt-outs first and the text last, so a failed opt-out leaves no unflagged text. The
//!   snapshot keeps every memory (`HGLOBAL`) format; GDI handle formats (`CF_BITMAP`,
//!   `CF_ENHMETAFILE`, and others) come back through the `CF_DIB` copy that Windows makes. A
//!   snapshot with a monitor, history or cloud opt-out is a password manager secret: it is not
//!   restored ([`super::windows_snapshot_is_concealed`]).
//! - Keys: Handy's chord (`input.rs`, `send_paste_ctrl_v`): Ctrl down, V click, hold, Ctrl up,
//!   with `SendInput` and the virtual key `VK_V` (layout independent).

use super::{
    win_edit, windows_control_focus, windows_snapshot_is_concealed, windows_write_order, Focus,
    InsertHost, KeyStroke, WIN_CAN_INCLUDE_IN_HISTORY, WIN_CAN_UPLOAD_TO_CLOUD,
    WIN_EXCLUDE_FROM_MONITOR,
};
use crate::error::{CoreError, CoreResult};
use crate::window_host::{ClipboardBackend, ClipboardFormat, ClipboardPlatform, ClipboardWriter};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, GlobalFree, ERROR_ACCESS_DENIED, HANDLE,
};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows_sys::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MapVirtualKeyW, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetForegroundWindow, GetGUIThreadInfo, GetWindowLongW, GetWindowThreadProcessId,
    SendMessageTimeoutW, GUITHREADINFO, GWL_STYLE, SMTO_ABORTIFHUNG,
};

const ID: &str = "R11";
const CF_UNICODETEXT: u32 = 13;
/// The longest wait for the `EM_GETPASSWORDCHAR` reply of a busy app.
const PASSWORD_CHAR_TIMEOUT_MS: u32 = 50;
/// The formats that mark a password manager secret (ADR 0048, decision 12).
const CONCEALING_FORMATS: [&str; 3] = [
    WIN_EXCLUDE_FROM_MONITOR,
    WIN_CAN_INCLUDE_IN_HISTORY,
    WIN_CAN_UPLOAD_TO_CLOUD,
];

fn failed() -> CoreError {
    CoreError::NotImplemented { component: ID }
}

/// The clipboard, open for this thread until drop.
struct Open;

impl Open {
    fn new() -> CoreResult<Self> {
        // Another app may hold the clipboard for a moment: retry for about 100 ms.
        for _ in 0..10 {
            // SAFETY: a null owner opens the clipboard for this task.
            if unsafe { OpenClipboard(std::ptr::null_mut()) } != 0 {
                return Ok(Open);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Err(failed())
    }

    fn empty(&self) -> CoreResult<()> {
        // SAFETY: the clipboard is open on this thread.
        if unsafe { EmptyClipboard() } != 0 {
            Ok(())
        } else {
            Err(failed())
        }
    }

    /// Put `data` on the clipboard as `format`, in a new `HGLOBAL` that the clipboard then owns.
    fn set(&self, format: u32, data: &[u8]) -> CoreResult<()> {
        // SAFETY: a new movable block of at least one byte; it is freed on any failure below.
        unsafe {
            let mem = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
            if mem.is_null() {
                return Err(failed());
            }
            let dst = GlobalLock(mem);
            if dst.is_null() {
                GlobalFree(mem);
                return Err(failed());
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), dst.cast::<u8>(), data.len());
            GlobalUnlock(mem);
            if SetClipboardData(format, mem).is_null() {
                GlobalFree(mem);
                return Err(failed());
            }
        }
        Ok(())
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: this thread opened the clipboard in `Open::new`.
        unsafe { CloseClipboard() };
    }
}

/// The numeric ID of a [`ClipboardFormat`] name.
fn format_id(name: &str) -> CoreResult<u32> {
    if name == "CF_UNICODETEXT" {
        return Ok(CF_UNICODETEXT);
    }
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives for the call.
    match unsafe { RegisterClipboardFormatW(wide.as_ptr()) } {
        0 => Err(failed()),
        id => Ok(id),
    }
}

/// Formats whose data is a GDI or private handle, not an `HGLOBAL` block.
fn is_handle_format(format: u32) -> bool {
    matches!(format, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E)
        || (0x200..=0x3FF).contains(&format)
}

/// The Windows clipboard for [`ClipboardWriter`] (SEC-30, #213, #371).
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsClipboard;

impl ClipboardBackend for WindowsClipboard {
    fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()> {
        // The opt-outs first, the text last: if an opt-out fails, the close publishes no text.
        let ordered = windows_write_order(formats)
            .into_iter()
            .map(|f| format_id(f.name).map(|id| (id, f)))
            .collect::<CoreResult<Vec<_>>>()?;
        let open = Open::new()?;
        open.empty()?;
        for (id, f) in ordered {
            open.set(id, &f.data)?;
        }
        Ok(())
    }

    fn clear(&self) -> CoreResult<()> {
        Open::new()?.empty()
    }
}

/// The saved clipboard: (format, bytes) for every memory format.
pub type WindowsSnapshot = Vec<(u32, Vec<u8>)>;

/// The Windows [`InsertHost`]. The clipboard write goes through `backend` ([`WindowsClipboard`]
/// in the app), so a test can check the SEC-30 formats without the real clipboard.
#[derive(Debug, Default, Clone, Copy)]
pub struct WindowsInsertHost<B = WindowsClipboard> {
    backend: B,
}

impl<B: ClipboardBackend + Copy> WindowsInsertHost<B> {
    /// A host that writes the clipboard through `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

/// The window that has the keyboard focus: the focused control, else the foreground window.
fn focused_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    // SAFETY: plain queries; `info` is a correctly sized out struct.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if thread == 0 || GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndFocus.is_null() {
            return Some(window);
        }
        Some(info.hwndFocus)
    }
}

impl<B: ClipboardBackend + Copy> InsertHost for WindowsInsertHost<B> {
    type Snapshot = WindowsSnapshot;

    fn input_permitted(&self) -> bool {
        !foreground_window_is_elevated()
    }

    fn focus(&self) -> Focus {
        // TODO(#409): browser and UWP password fields have no Win32 edit control. They need UI
        // Automation (`IsPassword`); until then they get the paste.
        // SAFETY: plain queries; `info` is a correctly sized out struct.
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return Focus::Nothing;
            }
            let thread = GetWindowThreadProcessId(window, std::ptr::null_mut());
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if thread == 0 || GetGUIThreadInfo(thread, &mut info) == 0 || info.hwndFocus.is_null() {
                return Focus::Field;
            }
            let control = info.hwndFocus;
            let mut class = [0u16; win_edit::CLASS_BUFFER];
            let n = GetClassNameW(control, class.as_mut_ptr(), class.len() as i32);
            let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
            let style = GetWindowLongW(control, GWL_STYLE) as u32;
            // Works across processes for edit controls. A hung app or a timeout leaves 0.
            let mut password_char = 0usize;
            SendMessageTimeoutW(
                control,
                win_edit::EM_GETPASSWORDCHAR,
                0,
                0,
                SMTO_ABORTIFHUNG,
                PASSWORD_CHAR_TIMEOUT_MS,
                &mut password_char,
            );
            windows_control_focus(&class, style, password_char)
        }
    }

    fn focus_target(&self) -> u64 {
        focused_window().map_or(0, |w| w as usize as u64)
    }

    fn snapshot_clipboard(&self) -> CoreResult<WindowsSnapshot> {
        let concealing: Vec<u32> = CONCEALING_FORMATS
            .iter()
            .filter_map(|n| format_id(n).ok())
            .collect();
        let _open = Open::new()?;
        let mut saved = Vec::new();
        let mut format = 0;
        loop {
            // SAFETY: the clipboard is open on this thread; 0 starts and ends the list.
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if is_handle_format(format) {
                continue;
            }
            // SAFETY: a memory format's data is an HGLOBAL owned by the clipboard; it is only read.
            unsafe {
                let mem = GetClipboardData(format);
                let src = if mem.is_null() {
                    std::ptr::null_mut()
                } else {
                    GlobalLock(mem)
                };
                if src.is_null() {
                    // An opt-out marks a secret even with no readable data.
                    if concealing.contains(&format) {
                        saved.push((format, Vec::new()));
                    }
                    continue;
                }
                let size = GlobalSize(mem);
                saved.push((
                    format,
                    std::slice::from_raw_parts(src.cast::<u8>(), size).to_vec(),
                ));
                GlobalUnlock(mem);
            }
        }
        Ok(saved)
    }

    fn is_concealed(&self, snapshot: &WindowsSnapshot) -> bool {
        let named: Vec<(u32, &str)> = CONCEALING_FORMATS
            .iter()
            .filter_map(|n| format_id(n).ok().map(|id| (id, *n)))
            .collect();
        windows_snapshot_is_concealed(snapshot.iter().filter_map(|(format, data)| {
            named
                .iter()
                .find(|(id, _)| id == format)
                .map(|(_, name)| (*name, data.as_slice()))
        }))
    }

    fn write_concealed(&self, text: &str) -> CoreResult<()> {
        ClipboardWriter::new(self.backend).write_text_for(ClipboardPlatform::Windows, text)
    }

    fn clipboard_mark(&self) -> u64 {
        // SAFETY: no arguments.
        u64::from(unsafe { GetClipboardSequenceNumber() })
    }

    fn send_keys(&self, keys: &[KeyStroke]) -> CoreResult<()> {
        let inputs: Vec<INPUT> = keys
            .iter()
            .map(|k| INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: k.key,
                        // SAFETY: a pure table lookup.
                        wScan: unsafe { MapVirtualKeyW(u32::from(k.key), MAPVK_VK_TO_VSC) } as u16,
                        dwFlags: if k.down { 0 } else { KEYEVENTF_KEYUP },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            })
            .collect();
        // SAFETY: `inputs` is a live slice of INPUT with the matching size.
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(failed())
        }
    }

    fn restore_clipboard(&self, snapshot: WindowsSnapshot) -> CoreResult<()> {
        let open = Open::new()?;
        open.empty()?;
        for (format, data) in &snapshot {
            open.set(*format, data)?;
        }
        Ok(())
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// What Windows said about the integrity level of the foreground window's process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ForegroundIntegrity {
    Level(u32),
    /// Windows refused the query: a process elevated under another account.
    Denied,
    /// No foreground window, or the query failed for another reason.
    Unknown,
}

/// Fail open only when a level is unknown; an access-denied query is `Denied` and blocks input
/// (Handy `integrity_blocks_input`).
fn integrity_blocks_input(foreground: ForegroundIntegrity, ours: Option<u32>) -> bool {
    match foreground {
        ForegroundIntegrity::Level(theirs) => ours.is_some_and(|ours| theirs > ours),
        ForegroundIntegrity::Denied => true,
        ForegroundIntegrity::Unknown => false,
    }
}

fn foreground_window_is_elevated() -> bool {
    // SAFETY: the pseudo-handle of GetCurrentProcess needs no close.
    let ours = unsafe { process_integrity_level(GetCurrentProcess()) }.ok();
    integrity_blocks_input(foreground_integrity(), ours)
}

fn foreground_integrity() -> ForegroundIntegrity {
    // SAFETY: every handle is checked before use and the process handle is closed.
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return ForegroundIntegrity::Unknown;
        }
        let mut pid = 0u32;
        if GetWindowThreadProcessId(window, &mut pid) == 0 {
            return ForegroundIntegrity::Unknown;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return if GetLastError() == ERROR_ACCESS_DENIED {
                ForegroundIntegrity::Denied
            } else {
                ForegroundIntegrity::Unknown
            };
        }
        let level = process_integrity_level(process);
        CloseHandle(process);
        integrity_of_query(level)
    }
}

/// The integrity of the foreground process from the token query: `Err` holds `GetLastError`.
/// Access denied means a process elevated under another account: `Denied`, so the paste is
/// refused and the MOG shows Copy (MOG-06d), as in Handy (`E_ACCESSDENIED`).
fn integrity_of_query(query: Result<u32, u32>) -> ForegroundIntegrity {
    match query {
        Ok(level) => ForegroundIntegrity::Level(level),
        Err(ERROR_ACCESS_DENIED) => ForegroundIntegrity::Denied,
        Err(_) => ForegroundIntegrity::Unknown,
    }
}

/// The integrity level of `process` (for example `0x2000` normal, `0x3000` administrator).
///
/// # Safety
/// `process` must be a valid handle with `PROCESS_QUERY_LIMITED_INFORMATION`.
unsafe fn process_integrity_level(process: HANDLE) -> Result<u32, u32> {
    let mut token: HANDLE = std::ptr::null_mut();
    if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
        return Err(GetLastError());
    }
    // A TOKEN_MANDATORY_LABEL and the SID it points to (68 bytes at most); u64 keeps it aligned.
    let mut buffer = [0u64; 16];
    let mut written = 0u32;
    let ok = GetTokenInformation(
        token,
        TokenIntegrityLevel,
        buffer.as_mut_ptr().cast(),
        std::mem::size_of_val(&buffer) as u32,
        &mut written,
    );
    // Read the error before CloseHandle can change it.
    let error = GetLastError();
    CloseHandle(token);
    if ok == 0 {
        return Err(error);
    }
    // The level is the last sub-authority of the SID.
    let sid = (*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
    let count = u32::from(*GetSidSubAuthorityCount(sid));
    Ok(*GetSidSubAuthority(sid, count.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mog_06a_elevated_or_denied_foreground_blocks_input() {
        use ForegroundIntegrity::*;
        assert!(integrity_blocks_input(Level(0x3000), Some(0x2000)));
        assert!(!integrity_blocks_input(Level(0x2000), Some(0x2000)));
        assert!(integrity_blocks_input(Denied, Some(0x2000)));
        assert!(!integrity_blocks_input(Unknown, Some(0x2000)));
        assert!(!integrity_blocks_input(Level(0x3000), None));
    }

    #[test]
    fn mog_06a_a_denied_token_query_blocks_input_as_handy_does() {
        use ForegroundIntegrity::*;
        // OpenProcess can succeed on an elevated window; Windows then refuses the token query.
        assert_eq!(integrity_of_query(Err(ERROR_ACCESS_DENIED)), Denied);
        assert!(integrity_blocks_input(
            integrity_of_query(Err(ERROR_ACCESS_DENIED)),
            Some(0x2000)
        ));
        assert_eq!(integrity_of_query(Err(6)), Unknown); // ERROR_INVALID_HANDLE
        assert_eq!(integrity_of_query(Ok(0x3000)), Level(0x3000));
    }

    /// Records the formats that the host hands to the clipboard. It never opens the clipboard.
    #[derive(Default)]
    struct Recorder(std::sync::Mutex<Vec<Vec<ClipboardFormat>>>);

    impl ClipboardBackend for &Recorder {
        fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()> {
            self.0.lock().unwrap().push(formats.to_vec());
            Ok(())
        }
        fn clear(&self) -> CoreResult<()> {
            Ok(())
        }
    }

    #[test]
    fn sec_30_windows_insert_write_has_the_monitor_history_and_cloud_opt_outs() {
        let rec = Recorder::default();
        WindowsInsertHost::new(&rec).write_concealed("hi").unwrap();
        let writes = rec.0.lock().unwrap().clone();
        assert_eq!(writes.len(), 1, "{writes:?}");
        let find = |n: &str| writes[0].iter().find(|f| f.name == n).cloned();
        assert!(find(WIN_EXCLUDE_FROM_MONITOR).is_some());
        assert_eq!(
            find(WIN_CAN_INCLUDE_IN_HISTORY).unwrap().data,
            vec![0, 0, 0, 0]
        );
        assert_eq!(
            find(WIN_CAN_UPLOAD_TO_CLOUD).unwrap().data,
            vec![0, 0, 0, 0]
        );
        assert!(find("CF_UNICODETEXT").is_some());
    }

    #[test]
    fn mog_06c_windows_host_sees_a_password_manager_secret() {
        let host: WindowsInsertHost = WindowsInsertHost::default();
        let id = |n: &str| format_id(n).unwrap();
        let text = (CF_UNICODETEXT, vec![b'p', 0, b'w', 0, 0, 0]);
        assert!(host.is_concealed(&vec![
            text.clone(),
            (id(WIN_EXCLUDE_FROM_MONITOR), Vec::new())
        ]));
        assert!(host.is_concealed(&vec![
            text.clone(),
            (id(WIN_CAN_INCLUDE_IN_HISTORY), vec![0; 4])
        ]));
        assert!(!host.is_concealed(&vec![
            text.clone(),
            (id(WIN_CAN_UPLOAD_TO_CLOUD), vec![1, 0, 0, 0])
        ]));
        assert!(!host.is_concealed(&vec![text]));
    }

    #[test]
    fn mog_06c_snapshot_skips_handle_formats_only() {
        for f in [2, 3, 9, 14, 0x80, 0x200, 0x3FF] {
            assert!(is_handle_format(f), "{f:#x}");
        }
        for f in [1, 7, 8, 13, 15, 16, 17, 0xC000] {
            assert!(!is_handle_format(f), "{f:#x}");
        }
    }
}
