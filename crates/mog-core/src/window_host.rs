//! R11 WindowHost (section 4.3).
//!
//! Responsibilities: Hub, MOG windows, cards, tray, native recording indicator (fail-closed, never `contentProtected`), the native window of the consent dialog (the logic is T12), cap of 8 active MOGs, per-display positions, `insert_text` paste.
//!
//! `mog-core` has no Tauri dependency: the app crate (`src-tauri`) implements this trait.
//! The Tauri host routes the `window_*` commands of 4.5 to it (#285, ADR 0040). A refusal is
//! [`CoreError::IpcRefused`] with a code from `ipc_gateway::ERROR_CODES`.

use crate::error::{CoreError, CoreResult};
use crate::insertion::PASTE_LOCK;
use crate::ipc::{ClipboardWriteTextArgs, Geometry};
use crate::ipc_gateway::GatewayComponents;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::PoisonError;

const ID: &str = "R11";

/// Native windows. The Tauri shell (or a later shell) implements it; the core only calls it.
pub trait WindowHost: Send + Sync {
    /// Open the MOG window of `mog_id` (cap of 8 active MOGs). `geometry` is the logical
    /// position and size; `None` keeps the default.
    fn open_mog_window(&self, mog_id: &str, geometry: Option<&Geometry>) -> CoreResult<()>;
    /// Close the MOG window of `mog_id`.
    fn close_mog_window(&self, mog_id: &str) -> CoreResult<()>;
    /// Show the card window of `mog_id`, made on the first open.
    fn show_card(&self, mog_id: &str) -> CoreResult<()>;
    /// Hide the card window of `mog_id`. The host destroys it later (MOG-18b).
    fn hide_card(&self, mog_id: &str) -> CoreResult<()>;
}

/// The R11 stub. Every method returns [`CoreError::NotImplemented`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubWindowHost;

impl WindowHost for StubWindowHost {
    fn open_mog_window(&self, _mog_id: &str, _geometry: Option<&Geometry>) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn close_mog_window(&self, _mog_id: &str) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn show_card(&self, _mog_id: &str) -> CoreResult<()> {
        // The Tauri shell implements R11 (`TauriWindowHost`); this stub stays for other hosts.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn hide_card(&self, _mog_id: &str) -> CoreResult<()> {
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

/// The one format that carries `text` itself, with no privacy flags.
fn text_format(platform: ClipboardPlatform, text: &str) -> ClipboardFormat {
    match platform {
        ClipboardPlatform::MacOs => ClipboardFormat {
            name: "public.utf8-plain-text",
            data: text.as_bytes().to_vec(),
        },
        ClipboardPlatform::Windows => ClipboardFormat {
            name: "CF_UNICODETEXT",
            data: text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect(),
        },
    }
}

/// The formats to write for `text` on `platform` (SEC-30): the text and the privacy flags.
pub fn clipboard_formats(platform: ClipboardPlatform, text: &str) -> Vec<ClipboardFormat> {
    let marker = |name| ClipboardFormat {
        name,
        data: Vec::new(),
    };
    let dword_zero = |name| ClipboardFormat {
        name,
        data: 0u32.to_le_bytes().to_vec(),
    };
    match platform {
        ClipboardPlatform::MacOs => vec![
            text_format(platform, text),
            marker("org.nspasteboard.ConcealedType"),
            marker("org.nspasteboard.TransientType"),
        ],
        ClipboardPlatform::Windows => vec![
            text_format(platform, text),
            marker("ExcludeClipboardContentFromMonitorProcessing"),
            dword_zero("CanIncludeInClipboardHistory"),
            dword_zero("CanUploadToCloudClipboard"),
        ],
    }
}

/// The pasteboard type that carries `html` on macOS (`NSPasteboardTypeHTML`'s UTI). The data is
/// the HTML fragment's UTF-8 bytes, verbatim: the brain already escaped and sanitised it
/// (`src/shared/answer-clipboard-format.ts`) before this command ever saw it.
fn html_format(html: &str) -> ClipboardFormat {
    ClipboardFormat {
        name: "public.html",
        data: html.as_bytes().to_vec(),
    }
}

/// The formats to write for `text` (and, on macOS, `html`) on `platform` as a normal, durable
/// clipboard item (`clipboard_write_text`, ADR 0124): no concealed or transient marker, so
/// clipboard managers, history and "paste again later" all see it, unlike the SEC-30 insertion
/// write above. See [`ClipboardWriteTextComponent`] for why this command marks nothing concealed
/// either.
///
/// ADR 0124 amendment (paste formatted into Docs, email and Excel): `html`, when given, is
/// written as `public.html` next to the plain text, so macOS apps that prefer rich content (Gmail,
/// Google Docs, Word) paste the formatted answer; `public.utf8-plain-text` stays for apps that
/// paste text only. Windows has no small, well-tested path to a CF_HTML byte-offset header in
/// this codebase today (`crates/mog-core/src/insertion/windows.rs`'s `WindowsClipboard` writes
/// plain memory formats only), so `html` is silently dropped there: Windows keeps writing
/// `CF_UNICODETEXT` only, exactly as it did before this amendment.
pub fn clipboard_formats_durable(
    platform: ClipboardPlatform,
    text: &str,
    html: Option<&str>,
) -> Vec<ClipboardFormat> {
    match (platform, html) {
        (ClipboardPlatform::MacOs, Some(html)) => {
            vec![html_format(html), text_format(platform, text)]
        }
        _ => vec![text_format(platform, text)],
    }
}

/// The platform of this build, or `None` where no clipboard flags are defined.
pub fn current_clipboard_platform() -> Option<ClipboardPlatform> {
    if cfg!(target_os = "macos") {
        Some(ClipboardPlatform::MacOs)
    } else if cfg!(windows) {
        Some(ClipboardPlatform::Windows)
    } else {
        None
    }
}

/// The OS clipboard. Kept behind a trait so tests run without a real clipboard.
pub trait ClipboardBackend: Send + Sync {
    /// Replace the clipboard contents with exactly `formats`.
    ///
    /// A backend must publish all formats together, so that a clipboard manager never sees the
    /// text without its flags. On failure the writer calls [`ClipboardBackend::clear`].
    fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()>;
    /// Remove everything from the clipboard.
    fn clear(&self) -> CoreResult<()>;
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
    pub fn write_text_for(&self, platform: ClipboardPlatform, text: &str) -> CoreResult<()> {
        let result = self.backend.write(&clipboard_formats(platform, text));
        if result.is_err() {
            // Fail closed: no unflagged text may stay on the clipboard. The write error wins.
            let _ = self.backend.clear();
        }
        result
    }

    /// Write `text` for the platform of this build.
    /// On a platform without flags it fails closed and writes nothing.
    pub fn write_text(&self, text: &str) -> CoreResult<()> {
        match current_clipboard_platform() {
            Some(platform) => self.write_text_for(platform, text),
            None => Err(CoreError::NotImplemented { component: ID }),
        }
    }

    /// Write `text` (and, on macOS, `html`) for `platform` as a normal, durable item
    /// (`clipboard_write_text`): no concealed or transient marker (see
    /// [`ClipboardWriteTextComponent`]).
    pub fn write_durable_text_for(
        &self,
        platform: ClipboardPlatform,
        text: &str,
        html: Option<&str>,
    ) -> CoreResult<()> {
        let result = self
            .backend
            .write(&clipboard_formats_durable(platform, text, html));
        if result.is_err() {
            // Fail closed, as `write_text_for` does: no half-written item stays on the clipboard.
            let _ = self.backend.clear();
        }
        result
    }
}

/// The macOS clipboard: the general `NSPasteboard`.
///
/// One `NSPasteboardItem` carries the text and the marker types, and one `writeObjects` call
/// publishes it, so `changeCount` never exposes the text without its flags. `NSPasteboard`
/// writes are not bound to the main thread; do not add a main-thread hop.
#[cfg(target_os = "macos")]
#[derive(Debug, Default, Clone, Copy)]
pub struct MacPasteboard;

#[cfg(target_os = "macos")]
impl ClipboardBackend for MacPasteboard {
    fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()> {
        use objc2::rc::autoreleasepool;
        use objc2::runtime::ProtocolObject;
        use objc2_app_kit::{
            NSPasteboard, NSPasteboardContentsOptions, NSPasteboardItem, NSPasteboardWriting,
        };
        use objc2_foundation::{NSArray, NSData, NSString};

        // A worker thread has no autorelease pool of its own: drain the AppKit objects here (as
        // `insertion::macos`'s own pasteboard calls already do).
        autoreleasepool(|_| {
            let item = NSPasteboardItem::new();
            for format in formats {
                let ty = NSString::from_str(format.name);
                if !item.setData_forType(&NSData::with_bytes(&format.data), &ty) {
                    return Err(CoreError::NotImplemented { component: ID });
                }
            }
            let pasteboard = NSPasteboard::generalPasteboard();
            // Bottle is a one-machine app (ADR 0124): every clipboard write stays off Universal
            // Clipboard, so business data (a Genie answer, or dictated text) never syncs to the
            // user's other Apple devices.
            pasteboard
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
            let objects =
                NSArray::from_slice(&[ProtocolObject::<dyn NSPasteboardWriting>::from_ref(&*item)]);
            if pasteboard.writeObjects(&objects) {
                Ok(())
            } else {
                self.clear()?;
                Err(CoreError::NotImplemented { component: ID })
            }
        })
    }

    fn clear(&self) -> CoreResult<()> {
        use objc2::rc::autoreleasepool;
        use objc2_app_kit::{NSPasteboard, NSPasteboardContentsOptions};
        autoreleasepool(|_| {
            NSPasteboard::generalPasteboard()
                .prepareForNewContentsWithOptions(NSPasteboardContentsOptions::CurrentHostOnly);
        });
        Ok(())
    }
}

/// The general pasteboard's type names and change count (macOS only): the owner's `last-copy.json`
/// proof diagnostic (ADR 0124 amendment). Read-only, and content-free on purpose (SEC-03): never
/// the text or the HTML, only the fact of which types are on the pasteboard and its change count,
/// so the owner can see for themselves whether Bottle's own `clipboard_write_text` wrote
/// `public.html`, with no risk to anything sensitive the pasteboard may hold. The caller (the
/// Tauri host, which alone knows the app data dir) writes the file; this only reads.
#[cfg(target_os = "macos")]
pub fn pasteboard_diagnostic() -> (Vec<String>, u64) {
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::NSPasteboard;
    autoreleasepool(|_| {
        let pasteboard = NSPasteboard::generalPasteboard();
        let types = pasteboard
            .types()
            .map(|t| t.to_vec().iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        (types, pasteboard.changeCount() as u64)
    })
}

/// A backend for a platform with no clipboard write at all (Linux). `write` and `clear` are
/// never reached: [`ClipboardWriteTextComponent::unsupported`] never calls them.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoClipboard;

impl ClipboardBackend for NoClipboard {
    fn write(&self, _formats: &[ClipboardFormat]) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn clear(&self) -> CoreResult<()> {
        Ok(())
    }
}

/// The longest `text` that `clipboard_write_text` accepts, in UTF-8 bytes (100 KB). A Genie
/// answer never needs more as plain text; a longer one answers `too_large` rather than hang the
/// pasteboard on a giant write.
pub const CLIPBOARD_WRITE_MAX_BYTES: usize = 100 * 1024;

/// The longest `html` that `clipboard_write_text` accepts, in UTF-8 bytes. `html` is checked on
/// its own, not combined with `text` (bug found 2026-11-25, owner report: a table-heavy answer
/// pasted as plain text only, with no `public.html`): `markdownToHtml`'s per-cell inline style
/// (`CELL_STYLE`, `src/shared/answer-clipboard-format.ts`) is required for Gmail, Outlook, Docs
/// and Word, which strip `<style>` blocks and `class` attributes on paste, but it multiplies a
/// table's bytes well past the "about doubles" rule of thumb the combined cap once assumed: a
/// plain-text table a few KB long can become a `html` form several times that size. A 4x
/// allowance over the text cap covers a genuinely large table; `html` still answers `too_large`
/// well short of a write big enough to hang the pasteboard.
pub const CLIPBOARD_WRITE_HTML_MAX_BYTES: usize = CLIPBOARD_WRITE_MAX_BYTES * 4;

/// The reply of `clipboard_write_text`: `{result: written|too_large|unsupported}`. A typed
/// outcome, not a thrown error, the same convention as `insertion::InsertOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClipboardWriteOutcome {
    /// The text is now the durable, normal clipboard item: Cmd+V pastes it like anything else
    /// the user copied themselves.
    Written,
    /// `text` is over [`CLIPBOARD_WRITE_MAX_BYTES`], or `html` is over
    /// [`CLIPBOARD_WRITE_HTML_MAX_BYTES`]. Nothing was written.
    TooLarge,
    /// This build or OS has no clipboard write (Linux; or Windows with no path wired yet).
    Unsupported,
}

/// The `clipboard_write_text` component (the owner's "attach Genie's answer to the clipboard"
/// request, one new IPC command the owner's sign-off authorizes).
///
/// Unlike [`ClipboardWriter::write_text_for`] (SEC-30, concealed and transient, used only by the
/// dictation paste's own clipboard round-trip), this write carries **no** privacy marker:
/// - Not transient: the owner asked for the answer to sit on the clipboard, pasteable later, not
///   to vanish like a password manager's short-lived copy.
/// - Not concealed either, on purpose, even though a Genie answer can hold business data: the
///   concealed marker is also the one signal [`crate::insertion::mac_snapshot_is_concealed`]
///   uses to decide a saved clipboard item looks like a password manager secret and must not be
///   restored after a paste (ADR 0048, decision 12). Marking the answer concealed would make the
///   very next dictation silently drop it instead of restoring it after the paste, which directly
///   contradicts "durable, paste it later". The user chose, with a plain Settings toggle, to put
///   this specific answer on their own clipboard; that is the same trust boundary as copying it
///   by hand, so it gets no extra pasteboard-manager privacy flag.
#[derive(Debug)]
pub struct ClipboardWriteTextComponent<B> {
    /// `None`: this OS has no clipboard write at all (`Unsupported`, never calls `backend`).
    target: Option<(B, ClipboardPlatform)>,
}

impl<B: ClipboardBackend + Copy> ClipboardWriteTextComponent<B> {
    /// A component that writes through `backend` with the conventions of `platform`.
    pub fn new(backend: B, platform: ClipboardPlatform) -> Self {
        Self {
            target: Some((backend, platform)),
        }
    }

    /// A component for an OS with no clipboard write: every call answers `unsupported`.
    pub fn unsupported() -> Self {
        Self { target: None }
    }
}

impl<B: ClipboardBackend + Copy> GatewayComponents for ClipboardWriteTextComponent<B> {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        let refused = |code| CoreError::IpcRefused { code };
        if command != "clipboard_write_text" {
            return Err(refused("not_implemented"));
        }
        let a: ClipboardWriteTextArgs =
            serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        // Bug fix 2026-11-25: `text` and `html` each answer `too_large` on their own bound, not
        // combined (see `CLIPBOARD_WRITE_HTML_MAX_BYTES`): a table-heavy answer's `html` can be
        // several times its `text` size on its own merits, with no risk to the pasteboard write.
        let html_len = a.html.as_deref().map_or(0, str::len);
        if a.text.len() > CLIPBOARD_WRITE_MAX_BYTES || html_len > CLIPBOARD_WRITE_HTML_MAX_BYTES {
            return Ok(json!({ "result": ClipboardWriteOutcome::TooLarge }));
        }
        let Some((backend, platform)) = self.target else {
            return Ok(json!({ "result": ClipboardWriteOutcome::Unsupported }));
        };
        // The same lock `insertion::insert_text` takes (ADR 0124): this write must never land
        // between that paste's clipboard snapshot and its chord, or the answer, not the
        // dictation, would land in the focused app.
        let _paste = PASTE_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        ClipboardWriter::new(backend)
            .write_durable_text_for(platform, &a.text, a.html.as_deref())
            .map_err(|_| refused("host_failed"))?;
        Ok(json!({ "result": ClipboardWriteOutcome::Written }))
    }
}

/// The `clipboard_write_text` component over the OS of this build: macOS and Windows (the
/// existing [`crate::insertion::WindowsClipboard`] path). Linux has none: `unsupported`.
#[cfg(target_os = "macos")]
pub fn os_component() -> ClipboardWriteTextComponent<MacPasteboard> {
    ClipboardWriteTextComponent::new(MacPasteboard, ClipboardPlatform::MacOs)
}

/// As the macOS `os_component`, over the existing Windows pasteboard path.
#[cfg(windows)]
pub fn os_component() -> ClipboardWriteTextComponent<crate::insertion::WindowsClipboard> {
    ClipboardWriteTextComponent::new(
        crate::insertion::WindowsClipboard,
        ClipboardPlatform::Windows,
    )
}

/// Linux has no clipboard write: every call answers `unsupported`, never `host_failed`.
#[cfg(not(any(target_os = "macos", windows)))]
pub fn os_component() -> ClipboardWriteTextComponent<NoClipboard> {
    ClipboardWriteTextComponent::unsupported()
}

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Recorder {
        calls: Mutex<Vec<Vec<ClipboardFormat>>>,
        clears: Mutex<u32>,
        fail_write: bool,
    }

    impl ClipboardBackend for &Recorder {
        fn write(&self, formats: &[ClipboardFormat]) -> CoreResult<()> {
            self.calls.lock().unwrap().push(formats.to_vec());
            if self.fail_write {
                return Err(CoreError::NotImplemented { component: ID });
            }
            Ok(())
        }

        fn clear(&self) -> CoreResult<()> {
            *self.clears.lock().unwrap() += 1;
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
    fn sec_30_writer_sends_all_formats_in_one_backend_call() {
        let rec = Recorder::default();
        let w = ClipboardWriter::new(&rec);
        w.write_text_for(ClipboardPlatform::MacOs, "secret")
            .unwrap();
        let calls = rec.calls.lock().unwrap();
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
            assert!(rec.calls.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn sec_30_failed_write_clears_the_clipboard_and_reports_the_error() {
        let rec = Recorder {
            fail_write: true,
            ..Recorder::default()
        };
        let w = ClipboardWriter::new(&rec);
        assert!(w
            .write_text_for(ClipboardPlatform::MacOs, "secret")
            .is_err());
        assert_eq!(*rec.clears.lock().unwrap(), 1);
    }

    #[test]
    fn sec_30_successful_write_does_not_clear_the_clipboard() {
        let rec = Recorder::default();
        let w = ClipboardWriter::new(&rec);
        w.write_text_for(ClipboardPlatform::MacOs, "secret")
            .unwrap();
        assert_eq!(*rec.clears.lock().unwrap(), 0);
    }

    // ---------- `clipboard_write_text` ----------

    #[test]
    fn clipboard_write_text_writes_a_durable_item_with_no_privacy_marker() {
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let reply = c
            .call("clipboard_write_text", json!({"text": "the answer"}))
            .unwrap();
        assert_eq!(reply, json!({"result": "written"}));
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        // No ConcealedType, no TransientType: a plain, durable text item only.
        assert_eq!(
            calls[0],
            vec![ClipboardFormat {
                name: "public.utf8-plain-text",
                data: b"the answer".to_vec(),
            }]
        );
    }

    #[test]
    fn clipboard_write_text_refuses_text_over_the_size_bound() {
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let long = "x".repeat(CLIPBOARD_WRITE_MAX_BYTES + 1);
        let reply = c
            .call("clipboard_write_text", json!({"text": long}))
            .unwrap();
        assert_eq!(reply, json!({"result": "too_large"}));
        // A refused write never touches the real clipboard.
        assert!(rec.calls.lock().unwrap().is_empty());
        // The bound itself is a normal write.
        let ok = "x".repeat(CLIPBOARD_WRITE_MAX_BYTES);
        let reply = c.call("clipboard_write_text", json!({"text": ok})).unwrap();
        assert_eq!(reply, json!({"result": "written"}));
    }

    #[test]
    fn clipboard_write_text_writes_both_html_and_text_on_macos() {
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let reply = c
            .call(
                "clipboard_write_text",
                json!({"text": "the answer", "html": "<p>the answer</p>"}),
            )
            .unwrap();
        assert_eq!(reply, json!({"result": "written"}));
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0],
            vec![
                ClipboardFormat {
                    name: "public.html",
                    data: b"<p>the answer</p>".to_vec(),
                },
                ClipboardFormat {
                    name: "public.utf8-plain-text",
                    data: b"the answer".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn clipboard_write_text_still_writes_text_only_with_no_html() {
        // The existing text-only contract keeps working once `html` is optional.
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        c.call("clipboard_write_text", json!({"text": "plain"}))
            .unwrap();
        assert_eq!(
            rec.calls.lock().unwrap()[0],
            vec![ClipboardFormat {
                name: "public.utf8-plain-text",
                data: b"plain".to_vec(),
            }]
        );
    }

    #[test]
    fn clipboard_write_text_text_and_html_each_answer_too_large_on_their_own_bound() {
        // Bug fix 2026-11-25: a `text` within its own bound must not answer `too_large` just
        // because `html` (the same answer's bigger, tag-and-inline-style form) is still within
        // its own, bigger bound — this used to fail here, before the two bounds were split.
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let text = "x".repeat(CLIPBOARD_WRITE_MAX_BYTES);
        let html = "y".repeat(CLIPBOARD_WRITE_HTML_MAX_BYTES);
        let reply = c
            .call(
                "clipboard_write_text",
                json!({"text": text.clone(), "html": html}),
            )
            .unwrap();
        assert_eq!(reply, json!({"result": "written"}));
        // `text` one byte over its own bound answers `too_large`, regardless of `html`.
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let over_text = "x".repeat(CLIPBOARD_WRITE_MAX_BYTES + 1);
        let reply = c
            .call(
                "clipboard_write_text",
                json!({"text": over_text, "html": "y"}),
            )
            .unwrap();
        assert_eq!(reply, json!({"result": "too_large"}));
        assert!(rec.calls.lock().unwrap().is_empty());
        // `html` one byte over its own bound answers `too_large`, with a tiny `text`.
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let over_html = "y".repeat(CLIPBOARD_WRITE_HTML_MAX_BYTES + 1);
        let reply = c
            .call(
                "clipboard_write_text",
                json!({"text": "x", "html": over_html}),
            )
            .unwrap();
        assert_eq!(reply, json!({"result": "too_large"}));
        assert!(rec.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn clipboard_write_text_a_table_heavy_answer_keeps_its_html() {
        // The exact bug the owner reported: a plain-text table well under the text bound, whose
        // HTML form (inline `CELL_STYLE` on every `<td>`/`<th>`, `answer-clipboard-format.ts`)
        // costs several times as many bytes for the same content, used to be silently dropped to
        // text-only by the old combined bound.
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let text = "a\tb\tc\n".repeat(2_000); // 12,000 bytes.
        let html =
            "<tr><td style=\"border:1px solid #ccc;padding:4px 8px;\">a</td></tr>".repeat(2_000); // well over 2x `text`, still under the html bound.
        assert!(html.len() > text.len() * 2);
        let reply = c
            .call("clipboard_write_text", json!({"text": text, "html": html}))
            .unwrap();
        assert_eq!(reply, json!({"result": "written"}));
        let calls = rec.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].iter().any(|f| f.name == "public.html"));
    }

    #[test]
    fn clipboard_write_text_drops_html_on_windows_and_writes_text_only() {
        // ADR 0124 amendment: Windows has no small CF_HTML path in this codebase yet, so `html`
        // is silently dropped there and `CF_UNICODETEXT` is written exactly as before. This is a
        // platform-agnostic data-construction test (`clipboard_formats_durable`), not an FFI
        // call, so it runs and is checked on every host, not only a Windows build.
        let formats =
            clipboard_formats_durable(ClipboardPlatform::Windows, "hi", Some("<p>hi</p>"));
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].name, "CF_UNICODETEXT");
    }

    #[test]
    fn clipboard_formats_durable_macos_puts_html_before_text() {
        let formats = clipboard_formats_durable(ClipboardPlatform::MacOs, "hi", Some("<b>hi</b>"));
        assert_eq!(
            formats,
            vec![
                ClipboardFormat {
                    name: "public.html",
                    data: b"<b>hi</b>".to_vec(),
                },
                ClipboardFormat {
                    name: "public.utf8-plain-text",
                    data: b"hi".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn clipboard_write_text_is_unsupported_with_no_platform_target() {
        let c = ClipboardWriteTextComponent::<&Recorder>::unsupported();
        let reply = c
            .call("clipboard_write_text", json!({"text": "hi"}))
            .unwrap();
        assert_eq!(reply, json!({"result": "unsupported"}));
    }

    #[test]
    fn clipboard_write_text_refuses_any_other_command() {
        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        assert!(c.call("insert_text", json!({})).is_err());
    }

    #[test]
    fn clipboard_write_text_waits_for_the_paste_lock_insert_text_holds() {
        use std::sync::mpsc;
        use std::time::Duration;

        let rec = Recorder::default();
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        let (tx, rx) = mpsc::channel();
        std::thread::scope(|scope| {
            // Simulates `insertion::insert_text` mid-paste: it holds the same lock.
            let guard = crate::insertion::PASTE_LOCK
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            scope.spawn(|| {
                tx.send(()).unwrap();
                let reply = c
                    .call("clipboard_write_text", json!({"text": "hi"}))
                    .unwrap();
                assert_eq!(reply, json!({"result": "written"}));
                tx.send(()).unwrap();
            });
            // The call started, but the lock this test still holds must block it.
            rx.recv().unwrap();
            assert!(rx.recv_timeout(Duration::from_millis(80)).is_err());
            drop(guard);
            // Released: the write goes through soon after.
            rx.recv_timeout(Duration::from_secs(2)).unwrap();
        });
    }

    #[test]
    fn clipboard_write_text_a_failed_backend_write_clears_the_clipboard() {
        let rec = Recorder {
            fail_write: true,
            ..Recorder::default()
        };
        let c = ClipboardWriteTextComponent::new(&rec, ClipboardPlatform::MacOs);
        assert!(c
            .call("clipboard_write_text", json!({"text": "hi"}))
            .is_err());
        assert_eq!(*rec.clears.lock().unwrap(), 1);
    }
}
