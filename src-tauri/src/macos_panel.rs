// Adapted from cjpais/Handy src-tauri/src/overlay.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! macOS: each `mog-<id>` window becomes a non-activating NSPanel (MOG-02) that a native drag
//! moves (MOG-01c). See `docs/adr/0009-macos-mog-panel.md`.
//!
//! The panel settings (Status level, `NSWindowStyleMaskNonactivatingPanel`, join all Spaces,
//! full screen auxiliary, no shadow, show without activation) follow Handy's
//! `create_recording_overlay`. Handy gets them from `tauri-nspanel`; Bottle does the same class
//! swap with objc2 directly, as SP-01 (#10) tested, so it adds no crate and no git dependency.
//! TODO(#10): the signed-DMG checks of SP-01 (alpha over the desktop, white flash, halo).

use crate::mog_drag::{DragGesture, MouseInput, Verdict};
use crate::window_host::{NativeWindowStyle, WindowError};
use crate::windows::{PanelLevel, PanelSpec, WindowKind};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
use objc2::{define_class, ClassType, MainThreadMarker, MainThreadOnly, Message};
use objc2_app_kit::{
    NSColor, NSEvent, NSEventMask, NSEventType, NSFloatingWindowLevel, NSPanel,
    NSStatusWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowLevel, NSWindowStyleMask,
};
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;

define_class!(
    /// The class of a MOG panel: an NSPanel that never becomes key or main (MOG-02).
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "BottleMogPanel"]
    struct MogPanel;

    impl MogPanel {
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            false
        }

        #[unsafe(method(canBecomeMainWindow))]
        fn can_become_main_window(&self) -> bool {
            false
        }
    }
);

/// The AppKit window level of a panel level.
pub fn appkit_level(level: PanelLevel) -> NSWindowLevel {
    match level {
        PanelLevel::Floating => NSFloatingWindowLevel,
        PanelLevel::Status => NSStatusWindowLevel,
    }
}

/// The raw `NSWindowCollectionBehavior` bits of a panel spec.
pub fn appkit_collection_behavior(spec: &PanelSpec) -> usize {
    let flags = [
        (
            spec.join_all_spaces,
            NSWindowCollectionBehavior::CanJoinAllSpaces,
        ),
        (spec.stationary, NSWindowCollectionBehavior::Stationary),
        (spec.ignores_cycle, NSWindowCollectionBehavior::IgnoresCycle),
        (
            spec.full_screen_auxiliary,
            NSWindowCollectionBehavior::FullScreenAuxiliary,
        ),
    ];
    flags
        .iter()
        .filter(|(on, _)| *on)
        .fold(0, |bits, (_, flag)| bits | flag.0)
}

/// What AppKit reports for a window, to check the panel step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PanelReadback {
    pub is_panel: bool,
    pub can_become_key: bool,
    pub can_become_main: bool,
    pub nonactivating: bool,
    pub level: NSWindowLevel,
    pub collection_behavior: usize,
    pub hides_on_deactivate: bool,
    pub opaque: bool,
    pub has_shadow: bool,
}

/// Read the panel attributes of a window from AppKit.
pub fn read_panel(window: &NSWindow) -> PanelReadback {
    PanelReadback {
        is_panel: window.isKindOfClass(NSPanel::class()),
        can_become_key: window.canBecomeKeyWindow(),
        can_become_main: window.canBecomeMainWindow(),
        nonactivating: window
            .styleMask()
            .contains(NSWindowStyleMask::NonactivatingPanel),
        level: window.level(),
        collection_behavior: window.collectionBehavior().0,
        hides_on_deactivate: window.hidesOnDeactivate(),
        opaque: window.isOpaque(),
        has_shadow: window.hasShadow(),
    }
}

fn class_name(cls: &AnyClass) -> String {
    cls.name().to_string_lossy().into_owned()
}

/// Turn a window into a MOG panel: swap its class to `MogPanel` (an NSPanel subclass), then set
/// the style mask, level, collection behavior and transparency of `spec`.
///
/// Refuses a key-capable spec (TODO(#78): the card panel needs its own class) and a window that
/// key-value observing has subclassed, because a class swap would break the observer.
///
/// # Safety
/// Call on the main thread with a live window that no other code swaps or observes. The window
/// keeps its memory: the old class's instance variables stay in place, and tao reads them only
/// through its own typed pointer (the same swap that `tauri-nspanel` does).
pub unsafe fn apply_panel(window: &NSWindow, spec: &PanelSpec) -> Result<(), WindowError> {
    if spec.can_become_key || spec.can_become_main {
        return Err(WindowError::HostFailed);
    }
    let target: &AnyClass = MogPanel::class();
    let current = window.class();
    if !std::ptr::eq(current, target) {
        if class_name(current).starts_with("NSKVONotifying_") {
            return Err(WindowError::HostFailed);
        }
        let obj = window as *const NSWindow as *mut AnyObject;
        // SAFETY: the caller's contract. `MogPanel` adds no instance variables, so the object
        // stays large enough for the new class.
        unsafe { objc2::ffi::object_setClass(obj, target) };
    }
    let mut mask = window.styleMask();
    if spec.nonactivating {
        mask |= NSWindowStyleMask::NonactivatingPanel;
    }
    window.setStyleMask(mask);
    window.setLevel(appkit_level(spec.level));
    window.setCollectionBehavior(NSWindowCollectionBehavior(appkit_collection_behavior(spec)));
    window.setHidesOnDeactivate(spec.hides_on_deactivate);
    // MOG-01a: the window itself paints nothing; the page has a transparent background.
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(false);
    Ok(())
}

/// Run `f` with the NSWindow of `window` on the main thread. If this is the main thread, it runs
/// now and its result is returned. Otherwise it is queued; on failure there, `window` is
/// destroyed, so a half-made MOG window never stays (the Destroyed event frees its slot).
fn on_main<R, F>(window: &tauri::WebviewWindow<R>, f: F) -> Result<(), WindowError>
where
    R: tauri::Runtime,
    F: FnOnce(&NSWindow) -> Result<(), WindowError> + Send + 'static,
{
    fn run<R: tauri::Runtime>(
        w: &tauri::WebviewWindow<R>,
        f: impl FnOnce(&NSWindow) -> Result<(), WindowError>,
    ) -> Result<(), WindowError> {
        let ptr = w.ns_window().map_err(|_| WindowError::HostFailed)?;
        // SAFETY: `ns_window` gives the live NSWindow of a real (wry) window; we are on the
        // main thread.
        let ns: &NSWindow = unsafe { &*(ptr as *const NSWindow) };
        f(ns)
    }
    if MainThreadMarker::new().is_some() {
        return run(window, f);
    }
    let w = window.clone();
    window
        .run_on_main_thread(move || {
            if run(&w, f).is_err() {
                let _ = w.destroy();
            }
        })
        .map_err(|_| WindowError::HostFailed)
}

/// The macOS native style of the R11 host. Use it only with the wry runtime: the Tauri mock
/// runtime has no real NSWindow.
pub struct MacPanelStyle;

impl<R: tauri::Runtime> NativeWindowStyle<R> for MacPanelStyle {
    fn style(
        &self,
        window: &tauri::WebviewWindow<R>,
        kind: &WindowKind,
    ) -> Result<(), WindowError> {
        let Some(spec) = kind.panel() else {
            return Ok(());
        };
        on_main(window, move |ns| {
            // SAFETY: main thread (`on_main`); the window was just built and is not shown yet.
            unsafe { apply_panel(ns, &spec) }
        })
    }

    fn show(&self, window: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        if kind.panel().is_none() {
            return window.show().map_err(|_| WindowError::HostFailed);
        }
        // `orderFrontRegardless` shows the panel without activating Bottle or making it key.
        on_main(window, |ns| {
            ns.orderFrontRegardless();
            Ok(())
        })
    }
}

thread_local! {
    static GESTURE: Cell<DragGesture> = Cell::new(DragGesture::default());
    /// The mouse-down that began the gesture: AppKit starts a window drag from it.
    static PRESS: RefCell<Option<Retained<NSEvent>>> = const { RefCell::new(None) };
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
}

/// Install the app-wide event monitor that drags MOG panels (MOG-01c). It sees only the
/// left-mouse events of `MogPanel` windows; every other event passes unchanged. Idempotent.
pub fn install_drag_monitor(_mtm: MainThreadMarker) {
    if INSTALLED.with(|i| i.replace(true)) {
        return;
    }
    let mask =
        NSEventMask::LeftMouseDown | NSEventMask::LeftMouseDragged | NSEventMask::LeftMouseUp;
    let block = block2::RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit gives a live event for the duration of the call.
        let ev: &NSEvent = unsafe { event.as_ref() };
        handle(ev).map_or(event.as_ptr(), |()| std::ptr::null_mut())
    });
    // SAFETY: the block returns the event it got or null.
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    // The monitor lives as long as the app.
    std::mem::forget(monitor);
}

/// The drag decision for one AppKit event on `window` (MOG-01c). `None` means the event is not
/// ours: the window is not a MOG panel, or it is not a left-mouse event. The position is the
/// event's own position in screen points, not where the pointer is now.
pub fn decide(gesture: &mut DragGesture, ev: &NSEvent, window: &NSWindow) -> Option<Verdict> {
    if !window.isKindOfClass(MogPanel::class()) {
        return None;
    }
    let input = match ev.r#type() {
        NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged => {
            let p = window.convertPointToScreen(ev.locationInWindow());
            if ev.r#type() == NSEventType::LeftMouseDown {
                MouseInput::Down { x: p.x, y: p.y }
            } else {
                MouseInput::Dragged { x: p.x, y: p.y }
            }
        }
        NSEventType::LeftMouseUp => MouseInput::Up,
        _ => return None,
    };
    Some(gesture.on(input))
}

/// `Some(())` when the event is consumed (a drag), `None` to pass it on.
fn handle(ev: &NSEvent) -> Option<()> {
    let mtm = MainThreadMarker::new()?;
    let window = ev.window(mtm)?;
    let verdict = GESTURE.with(|g| {
        let mut gesture = g.get();
        let v = decide(&mut gesture, ev, &window);
        g.set(gesture);
        v
    })?;
    match ev.r#type() {
        NSEventType::LeftMouseDown => PRESS.with(|p| *p.borrow_mut() = Some(ev.retain())),
        NSEventType::LeftMouseUp => PRESS.with(|p| *p.borrow_mut() = None),
        _ => {}
    }
    match verdict {
        Verdict::Pass => None,
        Verdict::Swallow => Some(()),
        Verdict::StartDrag => {
            // AppKit starts the drag from the press, so the MOG keeps its offset to the pointer.
            let press = PRESS.with(|p| p.borrow().clone());
            window.performWindowDragWithEvent(press.as_deref().unwrap_or(ev));
            Some(())
        }
    }
}
