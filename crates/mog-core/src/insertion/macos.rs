// Adapted from cjpais/Handy src-tauri/src/input.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! The macOS side of `insert_text` (#58, ADR 0048).
//!
//! - Permission: `AXIsProcessTrusted` (Accessibility). Posting key events and reading the focused
//!   element both need it (CAP-37: Bottle asks for it only when insertion is on).
//! - Focus: the system-wide `AXFocusedUIElement`. No focused element (`kAXErrorNoValue`) is
//!   `no_focus`. The subrole `AXSecureTextField`, or Secure Event Input held by any process
//!   (Handy `secure_input.rs`), is `secure_field`. Any other AX error (for example an app with no
//!   AX tree) is a field: Bottle tries the paste rather than refuse a working app.
//! - Clipboard: the general `NSPasteboard`. The snapshot keeps every item with every type; the
//!   write is [`MacPasteboard`] with the concealed and transient types (SEC-30, #213). A saved
//!   item with the concealed or transient type is a password manager secret: it is not restored.
//! - Keys: Handy's chord (`send_paste_ctrl_v`): Command down, V click, hold, Command up, posted
//!   with `CGEventPost` and explicit flags, so a still-held Shift or Option does not change the
//!   chord. Handy resolves the layout key of V with TIS, which must run on the main thread;
//!   Bottle uses its fallback, the ANSI V key code (`kVK_ANSI_V`).
//!
//! None of these calls needs the main thread, so `insert_text` can run on a worker.

use super::{mac_keys, mac_saved_item, mac_snapshot_is_concealed, Focus, InsertHost, KeyStroke};
use crate::error::{CoreError, CoreResult};
use crate::window_host::{ClipboardBackend, ClipboardPlatform, ClipboardWriter, MacPasteboard};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{NSPasteboard, NSPasteboardItem, NSPasteboardWriting};
use objc2_foundation::{NSArray, NSData, NSString};
use std::ffi::c_void;
use std::time::Duration;

const ID: &str = "R11";

type CFTypeRef = *const c_void;

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: CFTypeRef);
    fn CFEqual(a: CFTypeRef, b: CFTypeRef) -> u8;
    fn CFHash(cf: CFTypeRef) -> usize;
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementGetPid(element: CFTypeRef, pid: *mut i32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn CGEventSourceCreate(state: i32) -> CFTypeRef;
    fn CGEventCreateKeyboardEvent(source: CFTypeRef, key: u16, down: bool) -> CFTypeRef;
    fn CGEventSetFlags(event: CFTypeRef, flags: u64);
    fn CGEventPost(tap: u32, event: CFTypeRef);
}

#[link(name = "Carbon", kind = "framework")]
extern "C" {
    fn IsSecureEventInputEnabled() -> u8;
}

const AX_SUCCESS: i32 = 0;
const AX_NO_VALUE: i32 = -25212;
/// `kAXErrorFailure`.
const AX_FAILURE: i32 = -25200;
/// `kCGEventSourceStatePrivate`: the events do not merge with the physical key state.
const SOURCE_PRIVATE: i32 = -1;
/// `kCGHIDEventTap`.
const HID_EVENT_TAP: u32 = 0;
/// `kCGEventFlagMaskCommand`.
const FLAG_COMMAND: u64 = 0x0010_0000;

/// A CF object that this module owns (Create or Copy rule).
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a Create or Copy call and is released once.
            unsafe { CFRelease(self.0) };
        }
    }
}

fn cf(s: &NSString) -> CFTypeRef {
    // NSString is toll-free bridged to CFString.
    (s as *const NSString).cast()
}

/// Copy `attribute` of `element`. `Err` holds the AX error code.
fn ax_attribute(element: CFTypeRef, attribute: &str) -> Result<Owned, i32> {
    let name = NSString::from_str(attribute);
    let mut value: CFTypeRef = std::ptr::null();
    // SAFETY: `element` is a live AXUIElement, `name` lives for the call, `value` is an out pointer.
    let err = unsafe { AXUIElementCopyAttributeValue(element, cf(&name), &mut value) };
    if err == AX_SUCCESS && !value.is_null() {
        Ok(Owned(value))
    } else if err == AX_SUCCESS {
        Err(AX_NO_VALUE)
    } else {
        Err(err)
    }
}

/// The saved general pasteboard: each item with its (type, data) pairs.
pub type MacSnapshot = Vec<Vec<(String, Vec<u8>)>>;

/// The focused AX element of the system, or the AX error code.
fn focused_element() -> Result<Owned, i32> {
    // SAFETY: returns a new AXUIElement that `Owned` releases.
    let system = Owned(unsafe { AXUIElementCreateSystemWide() });
    if system.0.is_null() {
        return Err(AX_FAILURE);
    }
    ax_attribute(system.0, "AXFocusedUIElement")
}

/// The macOS [`InsertHost`]. The clipboard write goes through `backend` (the general pasteboard
/// in the app), so a test can check the SEC-30 types without a real pasteboard.
#[derive(Debug, Default, Clone, Copy)]
pub struct MacInsertHost<B = MacPasteboard> {
    backend: B,
}

impl<B: ClipboardBackend + Copy> MacInsertHost<B> {
    /// A host that writes the clipboard through `backend`.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

impl<B: ClipboardBackend + Copy> InsertHost for MacInsertHost<B> {
    type Snapshot = MacSnapshot;

    fn input_permitted(&self) -> bool {
        // SAFETY: no arguments; reads the TCC state of this process.
        unsafe { AXIsProcessTrusted() != 0 }
    }

    fn focus(&self) -> Focus {
        let focused = match focused_element() {
            Ok(e) => e,
            Err(AX_NO_VALUE) => return Focus::Nothing,
            // The app has no usable AX tree: try the paste.
            Err(_) => return secure_input_or_field(),
        };
        if let Ok(subrole) = ax_attribute(focused.0, "AXSubrole") {
            let secure = NSString::from_str("AXSecureTextField");
            // SAFETY: both are live CF objects; CFEqual is false for different types.
            if unsafe { CFEqual(subrole.0, cf(&secure)) } != 0 {
                return Focus::SecureField;
            }
        }
        secure_input_or_field()
    }

    fn focus_target(&self) -> u64 {
        let Ok(focused) = focused_element() else {
            return 0;
        };
        let mut pid = 0i32;
        // SAFETY: `focused` is a live AXUIElement and `pid` an out pointer.
        unsafe { AXUIElementGetPid(focused.0, &mut pid) };
        // SAFETY: `focused` is a live CF object. CFHash agrees with CFEqual for AX elements.
        let hash = unsafe { CFHash(focused.0) } as u64;
        (u64::from(pid as u32) << 32) ^ hash
    }

    fn snapshot_clipboard(&self) -> CoreResult<MacSnapshot> {
        // A worker thread has no autorelease pool of its own: drain the AppKit objects here.
        autoreleasepool(|_| {
            let pasteboard = NSPasteboard::generalPasteboard();
            let Some(items) = pasteboard.pasteboardItems() else {
                return Ok(Vec::new());
            };
            Ok(items
                .to_vec()
                .iter()
                .map(|item| {
                    mac_saved_item(
                        item.types()
                            .to_vec()
                            .iter()
                            .map(|ty| (ty.to_string(), item.dataForType(ty).map(|d| d.to_vec()))),
                    )
                })
                .collect())
        })
    }

    fn is_concealed(&self, snapshot: &MacSnapshot) -> bool {
        mac_snapshot_is_concealed(snapshot.iter().flatten().map(|(ty, _)| ty.as_str()))
    }

    fn write_concealed(&self, text: &str) -> CoreResult<()> {
        autoreleasepool(|_| {
            ClipboardWriter::new(self.backend).write_text_for(ClipboardPlatform::MacOs, text)
        })
    }

    fn clipboard_mark(&self) -> u64 {
        autoreleasepool(|_| NSPasteboard::generalPasteboard().changeCount() as u64)
    }

    fn send_keys(&self, keys: &[KeyStroke]) -> CoreResult<()> {
        // SAFETY: creates a private event source that `Owned` releases.
        let source = Owned(unsafe { CGEventSourceCreate(SOURCE_PRIVATE) });
        for k in keys {
            // SAFETY: `source` may be null (CG then uses no source); the event is released below.
            let event = Owned(unsafe { CGEventCreateKeyboardEvent(source.0, k.key, k.down) });
            if event.0.is_null() {
                return Err(CoreError::NotImplemented { component: ID });
            }
            // Command is down for every chord event except its own release.
            let flags = if k.key == mac_keys::COMMAND && !k.down {
                0
            } else {
                FLAG_COMMAND
            };
            // SAFETY: `event` is a live CGEvent.
            unsafe {
                CGEventSetFlags(event.0, flags);
                CGEventPost(HID_EVENT_TAP, event.0);
            }
        }
        Ok(())
    }

    fn restore_clipboard(&self, snapshot: MacSnapshot) -> CoreResult<()> {
        autoreleasepool(|_| restore_general_pasteboard(snapshot))
    }

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
    }
}

/// Put `snapshot` back on the general pasteboard (an empty one clears it).
fn restore_general_pasteboard(snapshot: MacSnapshot) -> CoreResult<()> {
    {
        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        if snapshot.is_empty() {
            return Ok(());
        }
        let items: Vec<Retained<NSPasteboardItem>> = snapshot
            .iter()
            .map(|types| {
                let item = NSPasteboardItem::new();
                for (ty, data) in types {
                    item.setData_forType(&NSData::with_bytes(data), &NSString::from_str(ty));
                }
                item
            })
            .collect();
        let writers: Vec<&ProtocolObject<dyn NSPasteboardWriting>> = items
            .iter()
            .map(|i| ProtocolObject::from_ref(&**i))
            .collect();
        if pasteboard.writeObjects(&NSArray::from_slice(&writers)) {
            Ok(())
        } else {
            Err(CoreError::NotImplemented { component: ID })
        }
    }
}

fn secure_input_or_field() -> Focus {
    // SAFETY: no arguments; reads the system Secure Event Input state.
    if unsafe { IsSecureEventInputEnabled() } != 0 {
        Focus::SecureField
    } else {
        Focus::Field
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window_host::{ClipboardBackend, ClipboardFormat};
    use std::sync::Mutex;

    /// Records the formats that the host hands to the clipboard. It never touches a pasteboard.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<Vec<ClipboardFormat>>>);

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
    fn sec_30_mac_insert_write_has_the_concealed_and_transient_types() {
        let rec = Recorder::default();
        MacInsertHost::new(&rec)
            .write_concealed("line one\nline two")
            .unwrap();
        let writes = rec.0.lock().unwrap().clone();
        assert_eq!(writes.len(), 1, "one write with every type: {writes:?}");
        let names: Vec<&str> = writes[0].iter().map(|f| f.name).collect();
        assert!(
            names.contains(&"org.nspasteboard.ConcealedType"),
            "{names:?}"
        );
        assert!(
            names.contains(&"org.nspasteboard.TransientType"),
            "{names:?}"
        );
        let text = writes[0]
            .iter()
            .find(|f| f.name == "public.utf8-plain-text")
            .expect("the text type");
        assert_eq!(text.data, b"line one\nline two");
    }

    /// The body of `fn <name>` in this file's `InsertHost` impl, up to the next `fn `.
    fn body_of<'a>(src: &'a str, name: &str) -> &'a str {
        let imp = &src[src.find("InsertHost for MacInsertHost").expect("the impl")..];
        let start = imp
            .find(&format!("fn {name}("))
            .unwrap_or_else(|| panic!("fn {name}"));
        let rest = &imp[start + 3..];
        &rest[..rest.find("\n    fn ").unwrap_or(rest.len())]
    }

    #[test]
    fn mog_06c_mac_pasteboard_work_runs_in_an_autorelease_pool() {
        // `insert_text` runs on a reused worker thread with no pool of its own, so an AppKit
        // object that is autoreleased there lives until the thread ends: a leak on every paste.
        let src = include_str!("macos.rs");
        for name in [
            "snapshot_clipboard",
            "write_concealed",
            "clipboard_mark",
            "restore_clipboard",
        ] {
            assert!(
                body_of(src, name).contains("autoreleasepool("),
                "{name} must run its AppKit calls in objc2::rc::autoreleasepool"
            );
        }
    }

    #[test]
    fn mog_06c_mac_host_sees_a_password_manager_secret() {
        let host: MacInsertHost = MacInsertHost::default();
        let item = |types: &[&str]| -> Vec<(String, Vec<u8>)> {
            types
                .iter()
                .map(|t| ((*t).to_owned(), Vec::new()))
                .collect()
        };
        let secret = vec![item(&[
            "public.utf8-plain-text",
            "org.nspasteboard.ConcealedType",
        ])];
        assert!(host.is_concealed(&secret));
        let transient = vec![
            item(&["public.png"]),
            item(&["public.utf8-plain-text", "org.nspasteboard.TransientType"]),
        ];
        assert!(host.is_concealed(&transient));
        let plain = vec![item(&["public.utf8-plain-text", "public.html"])];
        assert!(!host.is_concealed(&plain));
        assert!(!host.is_concealed(&Vec::new()));
    }
}
