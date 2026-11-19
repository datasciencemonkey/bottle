//! #11 MOG-02 / MOG-01c: make real AppKit windows, apply the MOG panel step and read it back,
//! and feed synthetic mouse events to the drag decision.
//! #78 MOG-03a / UX-19a: apply the card panel step (key-capable, Floating level) the same way.
//!
//! AppKit windows must be made on the main thread, and the default test harness runs each
//! test on a worker thread. So this file has its own `main` (`harness = false` in Cargo.toml).
//! #382: that `main` follows the libtest list protocol (`support/libtest_protocol.rs`), so
//! cargo-nextest lists each test and runs it alone, still on the main thread. Each test makes
//! its own windows. No window is ordered front, so nothing shows on screen, and no event is
//! posted. On other OSes the file has no tests.

#[path = "support/libtest_protocol.rs"]
mod libtest_protocol;

use libtest_protocol::Test;
use std::process::ExitCode;

fn main() -> ExitCode {
    libtest_protocol::main(TESTS)
}

#[cfg(not(target_os = "macos"))]
const TESTS: &[Test] = &[];

#[cfg(target_os = "macos")]
use appkit::*;

#[cfg(target_os = "macos")]
const TESTS: &[Test] = &[
    Test {
        name: "mog_02_appkit_status_level_is_25",
        run: mog_02_appkit_status_level_is_25,
    },
    Test {
        name: "mog_02_appkit_collection_behavior_bits",
        run: mog_02_appkit_collection_behavior_bits,
    },
    Test {
        name: "mog_02_plain_window_is_not_a_panel_before_the_step",
        run: mog_02_plain_window_is_not_a_panel_before_the_step,
    },
    Test {
        name: "mog_02_panel_step_accepts_the_mog_spec",
        run: mog_02_panel_step_accepts_the_mog_spec,
    },
    Test {
        name: "mog_02_panel_step_refuses_a_main_capable_spec",
        run: mog_02_panel_step_refuses_a_main_capable_spec,
    },
    Test {
        name: "ux_19a_panel_step_never_turns_a_mog_panel_into_a_key_panel",
        run: ux_19a_panel_step_never_turns_a_mog_panel_into_a_key_panel,
    },
    Test {
        name: "mog_02_appkit_reads_back_a_status_nspanel_that_never_becomes_key",
        run: mog_02_appkit_reads_back_a_status_nspanel_that_never_becomes_key,
    },
    Test {
        name: "mog_02_webkit_observes_the_window_before_the_step",
        run: mog_02_webkit_observes_the_window_before_the_step,
    },
    Test {
        name: "mog_02_panel_step_makes_a_webview_window_a_status_nspanel",
        run: mog_02_panel_step_makes_a_webview_window_a_status_nspanel,
    },
    Test {
        name: "mog_02_panel_step_keeps_the_webview_in_the_window",
        run: mog_02_panel_step_keeps_the_webview_in_the_window,
    },
    Test {
        name: "mog_02_webkit_observes_the_panel_again_after_the_step",
        run: mog_02_webkit_observes_the_panel_again_after_the_step,
    },
    Test {
        name: "mog_03a_appkit_reads_back_a_key_capable_floating_nspanel",
        run: mog_03a_appkit_reads_back_a_key_capable_floating_nspanel,
    },
    Test {
        name: "mog_03a_webkit_observes_the_card_panel_after_the_step",
        run: mog_03a_webkit_observes_the_card_panel_after_the_step,
    },
    Test {
        name: "ux_19a_panel_step_never_turns_a_card_panel_into_a_mog_panel",
        run: ux_19a_panel_step_never_turns_a_card_panel_into_a_mog_panel,
    },
    Test {
        name: "mog_03a_card_makes_its_web_view_the_first_responder",
        run: mog_03a_card_makes_its_web_view_the_first_responder,
    },
    Test {
        name: "mog_03a_focus_finds_no_web_view_in_a_plain_window",
        run: mog_03a_focus_finds_no_web_view_in_a_plain_window,
    },
    Test {
        name: "mog_01c_appkit_events_on_a_mog_panel_drag_and_swallow_the_mouse_up",
        run: mog_01c_appkit_events_on_a_mog_panel_drag_and_swallow_the_mouse_up,
    },
    Test {
        name: "mog_01c_appkit_click_on_a_mog_panel_passes_to_the_page",
        run: mog_01c_appkit_click_on_a_mog_panel_passes_to_the_page,
    },
    Test {
        name: "ux_19a_appkit_events_on_a_card_panel_are_not_dragged",
        run: ux_19a_appkit_events_on_a_card_panel_are_not_dragged,
    },
    Test {
        name: "mog_01c_appkit_events_on_other_windows_or_buttons_are_not_touched",
        run: mog_01c_appkit_events_on_other_windows_or_buttons_are_not_touched,
    },
];

#[cfg(target_os = "macos")]
mod appkit {
    use bottle_lib::macos_panel::{
        appkit_collection_behavior, appkit_level, apply_panel, decide, focus_web_view, read_panel,
        PanelReadback,
    };
    use bottle_lib::mog_drag::{DragGesture, Verdict};
    use bottle_lib::windows::{PanelLevel, PanelSpec, WindowKind};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

    type Outcome = Result<(), String>;

    fn ensure(ok: bool, detail: impl FnOnce() -> String) -> Outcome {
        if ok {
            Ok(())
        } else {
            Err(detail())
        }
    }

    fn mtm() -> MainThreadMarker {
        MainThreadMarker::new().expect("harness = false runs each test on the main thread")
    }

    fn rect() -> NSRect {
        NSRect::new(NSPoint::new(-4000.0, -4000.0), NSSize::new(160.0, 160.0))
    }

    fn class_name(w: &NSWindow) -> String {
        w.class().name().to_string_lossy().into_owned()
    }

    fn mog_spec() -> PanelSpec {
        WindowKind::mog("m1").unwrap().panel().unwrap()
    }

    fn card_spec() -> PanelSpec {
        WindowKind::card("m1").unwrap().panel().unwrap()
    }

    fn want_mog() -> PanelReadback {
        PanelReadback {
            is_panel: true,
            can_become_key: false,
            can_become_main: false,
            nonactivating: true,
            level: 25,
            collection_behavior: 0x151,
            hides_on_deactivate: false,
            opaque: false,
            has_shadow: false,
        }
    }

    /// A plain borderless window that is never shown. It starts from the opposite of every
    /// value the panel step sets, so each setter is proved.
    fn new_window() -> Retained<NSWindow> {
        // SAFETY: main thread; a plain borderless window that is never shown.
        let w = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm()),
                rect(),
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        // SAFETY: we own the window and it is on the main thread.
        unsafe { w.setReleasedWhenClosed(false) };
        w.setHidesOnDeactivate(true);
        w.setHasShadow(true);
        w.setOpaque(true);
        w
    }

    /// A window with a WKWebView, as wry builds it: WebKit observes the window (KVO).
    fn web_window() -> (Retained<NSWindow>, Retained<NSView>, Retained<WKWebView>) {
        let window = new_window();
        let content = NSView::initWithFrame(NSView::alloc(mtm()), rect());
        window.setContentView(Some(&content));
        // SAFETY: main thread; a default configuration, no page is loaded.
        let web: Retained<WKWebView> = unsafe {
            WKWebView::initWithFrame_configuration(
                WKWebView::alloc(mtm()),
                content.bounds(),
                &WKWebViewConfiguration::new(mtm()),
            )
        };
        content.addSubview(&web);
        (window, content, web)
    }

    /// A plain window after the MOG panel step.
    fn mog_panel() -> Result<Retained<NSWindow>, String> {
        let window = new_window();
        // SAFETY: main thread, our own window.
        unsafe { apply_panel(&window, &mog_spec()) }.map_err(|e| format!("{e:?}"))?;
        Ok(window)
    }

    /// A web view window after the card panel step.
    fn card_panel() -> Result<(Retained<NSWindow>, Retained<WKWebView>), String> {
        let (window, _content, web) = web_window();
        // SAFETY: main thread, our own window.
        unsafe { apply_panel(&window, &card_spec()) }.map_err(|e| format!("{e:?}"))?;
        Ok((window, web))
    }

    fn event(t: NSEventType, x: f64, y: f64) -> Retained<NSEvent> {
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            t,
            NSPoint::new(x, y),
            NSEventModifierFlags::empty(),
            0.0,
            0,
            None,
            0,
            1,
            1.0,
        )
        .expect("a synthetic mouse event")
    }

    // AppKit constants (NSWindow.h): NSFloatingWindowLevel = 3, NSStatusWindowLevel = 25.
    pub fn mog_02_appkit_status_level_is_25() -> Outcome {
        let (status, floating) = (
            appkit_level(PanelLevel::Status),
            appkit_level(PanelLevel::Floating),
        );
        ensure(status == 25 && floating == 3, || {
            format!("{status} {floating}")
        })
    }

    // canJoinAllSpaces 1<<0 | stationary 1<<4 | ignoresCycle 1<<6 | fullScreenAuxiliary 1<<8.
    pub fn mog_02_appkit_collection_behavior_bits() -> Outcome {
        let bits = appkit_collection_behavior(&mog_spec());
        ensure(bits == 0x151, || format!("{bits:#x}"))
    }

    // ---- A plain window ----
    pub fn mog_02_plain_window_is_not_a_panel_before_the_step() -> Outcome {
        let before = read_panel(&new_window());
        ensure(
            !before.is_panel && before.has_shadow && before.hides_on_deactivate && before.opaque,
            || format!("{before:?}"),
        )
    }

    pub fn mog_02_panel_step_accepts_the_mog_spec() -> Outcome {
        // SAFETY: main thread, our own window.
        let applied = unsafe { apply_panel(&new_window(), &mog_spec()) };
        ensure(applied.is_ok(), || format!("{applied:?}"))
    }

    pub fn mog_02_panel_step_refuses_a_main_capable_spec() -> Outcome {
        let mut main_capable = mog_spec();
        main_capable.can_become_main = true;
        // SAFETY: main thread, our own window.
        let refused = unsafe { apply_panel(&new_window(), &main_capable) };
        ensure(refused.is_err(), || format!("{refused:?}"))
    }

    pub fn ux_19a_panel_step_never_turns_a_mog_panel_into_a_key_panel() -> Outcome {
        let window = mog_panel()?;
        // SAFETY: main thread, our own window.
        let refused = unsafe { apply_panel(&window, &card_spec()) };
        let got = read_panel(&window);
        ensure(refused.is_err() && got == want_mog(), || {
            format!("{refused:?}\n  got {got:?}")
        })
    }

    pub fn mog_02_appkit_reads_back_a_status_nspanel_that_never_becomes_key() -> Outcome {
        let window = mog_panel()?;
        let got = read_panel(&window);
        let want = want_mog();
        ensure(got == want, || format!("got {got:?}\n  want {want:?}"))
    }

    // ---- A window with a WKWebView ----
    pub fn mog_02_webkit_observes_the_window_before_the_step() -> Outcome {
        let (window, _content, _web) = web_window();
        let observed = class_name(&window);
        ensure(observed.starts_with("NSKVONotifying_"), || observed.clone())
    }

    pub fn mog_02_panel_step_makes_a_webview_window_a_status_nspanel() -> Outcome {
        let (window, _content, _web) = web_window();
        let observed = class_name(&window);
        // SAFETY: main thread, our own window.
        let applied = unsafe { apply_panel(&window, &mog_spec()) };
        let got = read_panel(&window);
        ensure(applied.is_ok() && got == want_mog(), || {
            format!(
                "{applied:?} {observed} -> {}\n  got {got:?}",
                class_name(&window)
            )
        })
    }

    pub fn mog_02_panel_step_keeps_the_webview_in_the_window() -> Outcome {
        let (window, content, web) = web_window();
        // SAFETY: main thread, our own window.
        unsafe { apply_panel(&window, &mog_spec()) }.map_err(|e| format!("{e:?}"))?;
        let still_in = web.window().is_some_and(|w| std::ptr::eq(&*w, &*window))
            && window
                .contentView()
                .is_some_and(|c| std::ptr::eq(&*c, &*content));
        ensure(still_in, || {
            format!("webview window: {:?}", web.window().map(|w| class_name(&w)))
        })
    }

    pub fn mog_02_webkit_observes_the_panel_again_after_the_step() -> Outcome {
        let (window, _content, _web) = web_window();
        // SAFETY: main thread, our own window.
        unsafe { apply_panel(&window, &mog_spec()) }.map_err(|e| format!("{e:?}"))?;
        let name = class_name(&window);
        ensure(name == "NSKVONotifying_BottleMogPanel", || name.clone())
    }

    // ---- #78 MOG-03a: the card panel, on a window with a WKWebView ----
    pub fn mog_03a_appkit_reads_back_a_key_capable_floating_nspanel() -> Outcome {
        let (window, _content, _web) = web_window();
        // SAFETY: main thread, our own window.
        let applied = unsafe { apply_panel(&window, &card_spec()) };
        let got = read_panel(&window);
        let want = PanelReadback {
            is_panel: true,
            can_become_key: true,
            can_become_main: false,
            nonactivating: true,
            level: 3,
            collection_behavior: appkit_collection_behavior(&card_spec()),
            hides_on_deactivate: false,
            // #559: the card is transparent like a MOG; the pop-up draws its own shadow.
            opaque: false,
            has_shadow: false,
        };
        ensure(applied.is_ok() && got == want, || {
            format!("{applied:?}\n  got {got:?}\n  want {want:?}")
        })
    }

    pub fn mog_03a_webkit_observes_the_card_panel_after_the_step() -> Outcome {
        let (window, web) = card_panel()?;
        let name = class_name(&window);
        ensure(
            name == "NSKVONotifying_BottleCardPanel"
                && web.window().is_some_and(|w| std::ptr::eq(&*w, &*window)),
            || name.clone(),
        )
    }

    pub fn ux_19a_panel_step_never_turns_a_card_panel_into_a_mog_panel() -> Outcome {
        let (window, _web) = card_panel()?;
        // SAFETY: main thread, our own window.
        let refused = unsafe { apply_panel(&window, &mog_spec()) };
        ensure(
            refused.is_err() && read_panel(&window).can_become_key,
            || format!("{refused:?}"),
        )
    }

    pub fn mog_03a_card_makes_its_web_view_the_first_responder() -> Outcome {
        let (window, web) = card_panel()?;
        let focused = focus_web_view(&window);
        let responder_is_web = window.firstResponder().is_some_and(|r| {
            std::ptr::eq(
                &*r as *const _ as *const u8,
                &*web as *const WKWebView as *const u8,
            )
        });
        ensure(focused && responder_is_web, || {
            format!("focused={focused} responder_is_web={responder_is_web}")
        })
    }

    pub fn mog_03a_focus_finds_no_web_view_in_a_plain_window() -> Outcome {
        ensure(!focus_web_view(&new_window()), String::new)
    }

    // ---- MOG-01c: the drag decision for real AppKit events ----
    pub fn mog_01c_appkit_events_on_a_mog_panel_drag_and_swallow_the_mouse_up() -> Outcome {
        let window = mog_panel()?;
        let mut g = DragGesture::default();
        let steps = [
            (NSEventType::LeftMouseDown, 20.0, 20.0),
            (NSEventType::LeftMouseDragged, 22.0, 21.0),
            (NSEventType::LeftMouseDragged, 30.0, 20.0),
            (NSEventType::LeftMouseDragged, 60.0, 20.0),
            (NSEventType::LeftMouseUp, 60.0, 20.0),
        ];
        let got: Vec<_> = steps
            .iter()
            .map(|&(t, x, y)| decide(&mut g, &event(t, x, y), &window))
            .collect();
        ensure(
            got == [
                Some(Verdict::Pass),
                Some(Verdict::Pass),
                Some(Verdict::StartDrag),
                Some(Verdict::Swallow),
                Some(Verdict::Swallow),
            ],
            || format!("{got:?}"),
        )
    }

    pub fn mog_01c_appkit_click_on_a_mog_panel_passes_to_the_page() -> Outcome {
        let window = mog_panel()?;
        let mut g = DragGesture::default();
        let click = [
            decide(
                &mut g,
                &event(NSEventType::LeftMouseDown, 20.0, 20.0),
                &window,
            ),
            decide(
                &mut g,
                &event(NSEventType::LeftMouseUp, 21.0, 20.0),
                &window,
            ),
        ];
        ensure(click == [Some(Verdict::Pass), Some(Verdict::Pass)], || {
            format!("{click:?}")
        })
    }

    pub fn ux_19a_appkit_events_on_a_card_panel_are_not_dragged() -> Outcome {
        let (window, _web) = card_panel()?;
        let mut g = DragGesture::default();
        let on_card = [
            decide(
                &mut g,
                &event(NSEventType::LeftMouseDown, 20.0, 20.0),
                &window,
            ),
            decide(
                &mut g,
                &event(NSEventType::LeftMouseDragged, 90.0, 20.0),
                &window,
            ),
        ];
        ensure(on_card == [None, None] && !g.is_dragging(), || {
            format!("{on_card:?}")
        })
    }

    pub fn mog_01c_appkit_events_on_other_windows_or_buttons_are_not_touched() -> Outcome {
        let window = mog_panel()?;
        let plain = new_window();
        let mut g = DragGesture::default();
        let other = [
            decide(
                &mut g,
                &event(NSEventType::LeftMouseDown, 20.0, 20.0),
                &plain,
            ),
            decide(
                &mut g,
                &event(NSEventType::LeftMouseDragged, 90.0, 20.0),
                &plain,
            ),
            decide(
                &mut g,
                &event(NSEventType::RightMouseDown, 20.0, 20.0),
                &window,
            ),
        ];
        ensure(other == [None, None, None] && !g.is_dragging(), || {
            format!("{other:?}")
        })
    }
}
