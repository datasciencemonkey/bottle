//! SP-01 macOS-only experiments (objc2), each behind an env switch:
//! - `SP01_TRACK=1`: an `.activeAlways` NSTrackingArea on the content view whose
//!   owner (an NSResponder subclass) forwards mouseMoved:/mouseEntered:/mouseExited:
//!   into the page as synthetic DOM pointer events, and logs each native call.
//! - `SP01_PANEL_SWAP=1`: swap the tao window class to an NSPanel subclass and add
//!   `NSWindowStyleMaskNonactivatingPanel` (what `tauri-nspanel` does).
//! - `SP01_SNAP=<path>`: write a WKWebView `takeSnapshot` TIFF of the Tauri webview.

use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{define_class, msg_send, AllocAnyThread, ClassType, MainThreadOnly};
use objc2_app_kit::{
    NSEvent, NSPanel, NSResponder, NSTrackingArea, NSTrackingAreaOptions, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSObjectProtocol, NSRect};

use crate::{env_or, sp01_log};

static WIN: OnceLock<tauri::WebviewWindow> = OnceLock::new();
static HEIGHT: OnceLock<f64> = OnceLock::new();

fn forward(ev: &NSEvent, kind: &str) {
    let p = ev.locationInWindow();
    let h = *HEIGHT.get().unwrap_or(&0.0);
    let (x, y) = (p.x, h - p.y);
    let _ =
        sp01_log(serde_json::json!({ "kind": "native", "type": kind, "x": x, "y": y }).to_string());
    if let Some(w) = WIN.get() {
        let _ = w.eval(format!(
            "window.sp01Forward && window.sp01Forward('{kind}', {x}, {y})"
        ));
    }
}

define_class!(
    #[unsafe(super(NSResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "SP01Tracker"]
    struct Tracker;

    impl Tracker {
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, ev: &NSEvent) {
            forward(ev, "pointermove");
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, ev: &NSEvent) {
            forward(ev, "pointerenter");
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, ev: &NSEvent) {
            forward(ev, "pointerleave");
        }
    }
);

define_class!(
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "SP01Panel"]
    struct Panel;

    impl Panel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main(&self) -> bool {
            false
        }
    }
);

/// Log whether the tracker answers the AppKit selectors (proof for finding 1).
pub fn log_tracker_selectors(mtm: MainThreadMarker) {
    let t: Retained<Tracker> = unsafe { msg_send![Tracker::alloc(mtm), init] };
    let obj: &AnyObject = &t;
    let responds = |sel: &str| -> bool {
        let sel = objc2::runtime::Sel::register(&std::ffi::CString::new(sel).unwrap());
        unsafe { msg_send![obj, respondsToSelector: sel] }
    };
    let line = serde_json::json!({
        "kind": "info", "msg": "tracker selectors",
        "mouseMoved:": responds("mouseMoved:"),
        "mouseEntered:": responds("mouseEntered:"),
        "mouseExited:": responds("mouseExited:"),
    });
    let _ = sp01_log(line.to_string());
}

/// Install the always-active tracking area (SP01_TRACK=1).
pub fn install_tracker(win: &tauri::WebviewWindow, ns: &NSWindow, mtm: MainThreadMarker) {
    let _ = WIN.set(win.clone());
    let Some(view) = ns.contentView() else {
        return;
    };
    let _ = HEIGHT.set(view.bounds().size.height);
    let tracker: Retained<Tracker> = unsafe { msg_send![Tracker::alloc(mtm), init] };
    let owner: &AnyObject = &tracker;
    let opts = NSTrackingAreaOptions::ActiveAlways
        | NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::InVisibleRect;
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            opts,
            Some(owner),
            None,
        )
    };
    view.addTrackingArea(&area);
    // A tracking area does not retain its owner: keep the tracker for the app lifetime.
    std::mem::forget(tracker);
    let _ = sp01_log(r#"{"kind":"info","msg":"tracker installed"}"#.to_string());
}

/// Swap the window class to an NSPanel subclass, then add the non-activating
/// style mask (SP01_PANEL_SWAP=1).
pub fn panel_swap(ns: &NSWindow) {
    let cls: &AnyClass = Panel::class();
    let ptr = ns as *const NSWindow as *mut AnyObject;
    // SAFETY: throwaway spike; the same class swap that tauri-nspanel performs.
    unsafe {
        objc2::ffi::object_setClass(ptr, cls);
    }
    let mask = ns.styleMask() | NSWindowStyleMask::NonactivatingPanel;
    ns.setStyleMask(mask);
    let line = serde_json::json!({
        "kind": "info", "msg": "panel swap",
        "is_panel": ns.isKindOfClass(NSPanel::class()),
        "nonactivating": ns.styleMask().contains(NSWindowStyleMask::NonactivatingPanel),
    });
    let _ = sp01_log(line.to_string());
}

/// Write a WKWebView snapshot of the Tauri webview as TIFF (SP01_SNAP=<path>).
pub fn snapshot(win: &tauri::WebviewWindow) {
    let Ok(path) = std::env::var("SP01_SNAP") else {
        return;
    };
    let _ = sp01_log(r#"{"kind":"info","msg":"tauri snapshot requested"}"#.to_string());
    let r = win.with_webview(move |pv| {
        let _ = sp01_log(r#"{"kind":"info","msg":"tauri snapshot in with_webview"}"#.to_string());
        let wk = pv.inner() as *mut objc2_web_kit::WKWebView;
        // SAFETY: wry hands out its live WKWebView on the main thread.
        let wk: &objc2_web_kit::WKWebView = unsafe { &*wk };
        let path = path.clone();
        let block = block2::RcBlock::new(
            move |img: *mut objc2_app_kit::NSImage, _err: *mut objc2_foundation::NSError| {
                let ok = if img.is_null() {
                    false
                } else {
                    // SAFETY: non-null NSImage from WebKit, valid during the callback.
                    let img = unsafe { &*img };
                    match img.TIFFRepresentation() {
                        Some(d) => std::fs::write(&path, d.to_vec()).is_ok(),
                        None => false,
                    }
                };
                let _ = sp01_log(
                    serde_json::json!({ "kind": "info", "msg": "tauri snapshot", "ok": ok })
                        .to_string(),
                );
            },
        );
        unsafe { wk.takeSnapshotWithConfiguration_completionHandler(None, &block) };
    });
    if let Err(e) = r {
        let _ = sp01_log(serde_json::json!({ "kind": "error", "msg": e.to_string() }).to_string());
    }
}

pub fn want(key: &str) -> bool {
    env_or(key, "0") == "1"
}
