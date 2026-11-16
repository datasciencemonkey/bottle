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
//!   write is [`MacPasteboard`] with the concealed and transient types (SEC-30, #213).
//! - Keys: Handy's chord (`send_paste_ctrl_v`): Command down, V click, hold, Command up, posted
//!   with `CGEventPost` and explicit flags, so a still-held Shift or Option does not change the
//!   chord. Handy resolves the layout key of V with TIS, which must run on the main thread;
//!   Bottle uses its fallback, the ANSI V key code (`kVK_ANSI_V`).
//!
//! None of these calls needs the main thread, so `insert_text` can run on a worker.

use super::{mac_keys, Focus, InsertHost, KeyStroke};
use crate::error::{CoreError, CoreResult};
use crate::window_host::{ClipboardPlatform, ClipboardWriter, MacPasteboard};
use objc2::rc::Retained;
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
}

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
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

/// The macOS [`InsertHost`].
#[derive(Debug, Default, Clone, Copy)]
pub struct MacInsertHost;

impl InsertHost for MacInsertHost {
    type Snapshot = MacSnapshot;

    fn input_permitted(&self) -> bool {
        // SAFETY: no arguments; reads the TCC state of this process.
        unsafe { AXIsProcessTrusted() }
    }

    fn focus(&self) -> Focus {
        // SAFETY: returns a new AXUIElement that `Owned` releases.
        let system = Owned(unsafe { AXUIElementCreateSystemWide() });
        if system.0.is_null() {
            return Focus::Field;
        }
        let focused = match ax_attribute(system.0, "AXFocusedUIElement") {
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

    fn snapshot_clipboard(&self) -> CoreResult<MacSnapshot> {
        let pasteboard = NSPasteboard::generalPasteboard();
        let Some(items) = pasteboard.pasteboardItems() else {
            return Ok(Vec::new());
        };
        Ok(items
            .to_vec()
            .iter()
            .map(|item| {
                item.types()
                    .to_vec()
                    .iter()
                    .filter_map(|ty| item.dataForType(ty).map(|d| (ty.to_string(), d.to_vec())))
                    .collect()
            })
            .collect())
    }

    fn write_concealed(&self, text: &str) -> CoreResult<()> {
        ClipboardWriter::new(MacPasteboard).write_text_for(ClipboardPlatform::MacOs, text)
    }

    fn clipboard_mark(&self) -> u64 {
        NSPasteboard::generalPasteboard().changeCount() as u64
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

    fn sleep(&self, d: Duration) {
        std::thread::sleep(d);
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
