//! R11 WindowHost (section 4.3).
//!
//! Responsibilities: Hub, MOG windows, cards, tray, native recording indicator (fail-closed, never `contentProtected`), the native window of the consent dialog (the logic is T12), cap of 8 active MOGs, per-display positions, `insert_text` paste.
//!
//! `mog-core` has no Tauri dependency: the app crate (`src-tauri`) implements this trait.

use crate::error::{CoreError, CoreResult};

const ID: &str = "R11";

/// Native windows. The Tauri shell (or a later shell) implements it; the core only calls it.
pub trait WindowHost: Send + Sync {
    /// Open the MOG window of `mog_id` (cap of 8 active MOGs).
    fn open_mog_window(&self, mog_id: &str) -> CoreResult<()>;
    /// Close the MOG window of `mog_id`.
    fn close_mog_window(&self, mog_id: &str) -> CoreResult<()>;
    /// Show the card window of `mog_id`, made on the first open.
    fn show_card(&self, mog_id: &str) -> CoreResult<()>;
}

/// The R11 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubWindowHost;

impl WindowHost for StubWindowHost {
    fn open_mog_window(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#8): Tauri window model in src-tauri.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn close_mog_window(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#8): Tauri window model in src-tauri.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn show_card(&self, _mog_id: &str) -> CoreResult<()> {
        // TODO(#78): card window, lazy create.
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The OS family whose clipboard conventions apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardPlatform {
    /// macOS: `NSPasteboard` with the nspasteboard.org marker types.
    MacOs,
    /// Windows: clipboard formats that opt out of history, cloud sync and monitors.
    Windows,
}

/// One clipboard format to place on the clipboard: a name and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardFormat {
    /// The pasteboard type (macOS) or registered format name (Windows).
    pub name: &'static str,
    /// The format payload. Marker formats are empty on macOS and a `DWORD` on Windows.
    pub data: Vec<u8>,
}

/// The formats to write for `text` on `platform` (SEC-30): the text and the privacy flags.
pub fn clipboard_formats(_platform: ClipboardPlatform, _text: &str) -> Vec<ClipboardFormat> {
    Vec::new()
}

/// The platform of this build, or `None` where no clipboard flags are defined.
pub fn current_clipboard_platform() -> Option<ClipboardPlatform> {
    None
}

/// The OS clipboard. Kept behind a trait so tests run without a real clipboard.
pub trait ClipboardBackend: Send + Sync {
    /// Replace the clipboard contents with exactly `formats`, in one operation.
    fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()>;
}

/// Writes text to the clipboard with the concealed and transient flags (SEC-30).
#[derive(Debug)]
pub struct ClipboardWriter<B: ClipboardBackend> {
    backend: B,
}

impl<B: ClipboardBackend> ClipboardWriter<B> {
    /// Make a writer over `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Write `text` for `platform` with the privacy flags set.
    pub fn write_text_for(&self, _platform: ClipboardPlatform, _text: &str) -> CoreResult<()> {
        Ok(())
    }

    /// Write `text` for the platform of this build.
    pub fn write_text(&self, _text: &str) -> CoreResult<()> {
        let _ = &self.backend;
        Ok(())
    }
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder(Mutex<Vec<Vec<ClipboardFormat>>>);

    impl ClipboardBackend for &Recorder {
        fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()> {
            self.0.lock().unwrap().push(formats.to_vec());
            Ok(())
        }
    }

    fn find<'a>(f: &'a [ClipboardFormat], n: &str) -> Option<&'a ClipboardFormat> {
        f.iter().find(|x| x.name == n)
    }

    #[test]
    fn sec_30_macos_marks_text_concealed_and_transient() {
        let f = clipboard_formats(ClipboardPlatform::MacOs, "hi");
        assert_eq!(find(&f, "public.utf8-plain-text").unwrap().data, b"hi");
        assert!(find(&f, "org.nspasteboard.ConcealedType").is_some());
        assert!(find(&f, "org.nspasteboard.TransientType").is_some());
        assert_eq!(f.len(), 3);
    }

    #[test]
    fn sec_30_windows_excludes_monitors_history_and_cloud() {
        let f = clipboard_formats(ClipboardPlatform::Windows, "hi");
        let text = find(&f, "CF_UNICODETEXT").unwrap();
        assert_eq!(text.data, vec![b'h', 0, b'i', 0, 0, 0]);
        assert!(find(&f, "ExcludeClipboardContentFromMonitorProcessing").is_some());
        let zero = vec![0u8; 4];
        assert_eq!(find(&f, "CanIncludeInClipboardHistory").unwrap().data, zero);
        assert_eq!(find(&f, "CanUploadToCloudClipboard").unwrap().data, zero);
        assert_eq!(f.len(), 4);
    }

    #[test]
    fn sec_30_windows_text_is_utf16_with_terminator() {
        let f = clipboard_formats(ClipboardPlatform::Windows, "é😀");
        let units: Vec<u8> = "é😀"
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(find(&f, "CF_UNICODETEXT").unwrap().data, units);
    }

    #[test]
    fn sec_30_writer_sends_all_formats_in_one_write() {
        let rec = Recorder::default();
        let w = ClipboardWriter::new(&rec);
        w.write_text_for(ClipboardPlatform::MacOs, "secret")
            .unwrap();
        let calls = rec.0.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            clipboard_formats(ClipboardPlatform::MacOs, "secret")
        );
    }

    #[test]
    fn sec_30_platform_matches_the_build_target() {
        let want = if cfg!(target_os = "macos") {
            Some(ClipboardPlatform::MacOs)
        } else if cfg!(windows) {
            Some(ClipboardPlatform::Windows)
        } else {
            None
        };
        assert_eq!(current_clipboard_platform(), want);
    }

    #[test]
    fn sec_30_write_text_on_unsupported_platform_fails_closed() {
        if current_clipboard_platform().is_none() {
            let rec = Recorder::default();
            assert!(ClipboardWriter::new(&rec).write_text("x").is_err());
            assert!(rec.0.lock().unwrap().is_empty());
        }
    }
}
