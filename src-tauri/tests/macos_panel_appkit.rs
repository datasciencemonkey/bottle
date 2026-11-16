//! #11 MOG-02 / MOG-01c: make real AppKit windows, apply the MOG panel step and read it back,
//! and feed synthetic mouse events to the drag decision.
//! #78 MOG-03a / UX-19a: apply the card panel step (key-capable, Floating level) the same way.
//!
//! AppKit windows must be made on the main thread, and the default test harness runs each
//! test on a worker thread. So this file has its own `main` (`harness = false` in Cargo.toml).
//! No window is ordered front, so nothing shows on screen, and no event is posted.
//! On other OSes the file prints a skip line.

#[cfg(target_os = "macos")]
fn main() {
    use bottle_lib::macos_panel::{
        appkit_collection_behavior, appkit_level, apply_panel, decide, focus_web_view, read_panel,
        PanelReadback,
    };
    use bottle_lib::mog_drag::{DragGesture, Verdict};
    use bottle_lib::windows::{PanelLevel, WindowKind};
    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSBackingStoreType, NSEvent, NSEventModifierFlags, NSEventType, NSView, NSWindow,
        NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use objc2_web_kit::{WKWebView, WKWebViewConfiguration};

    let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
    let mut failed = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            println!("  {detail}");
            failed += 1;
        }
    };
    let class_name = |w: &NSWindow| w.class().name().to_string_lossy().into_owned();

    let rect = NSRect::new(NSPoint::new(-4000.0, -4000.0), NSSize::new(160.0, 160.0));
    let new_window = || {
        // SAFETY: main thread; a plain borderless window that is never shown.
        let w = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                rect,
                NSWindowStyleMask::Borderless,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        // SAFETY: we own the window and it is on the main thread.
        unsafe { w.setReleasedWhenClosed(false) };
        // Start from the opposite of every value the panel step sets, so each setter is proved.
        w.setHidesOnDeactivate(true);
        w.setHasShadow(true);
        w.setOpaque(true);
        w
    };
    let want = PanelReadback {
        is_panel: true,
        can_become_key: false,
        can_become_main: false,
        nonactivating: true,
        level: 25,
        collection_behavior: 0x151,
        hides_on_deactivate: false,
        opaque: false,
        has_shadow: false,
    };

    // AppKit constants (NSWindow.h): NSFloatingWindowLevel = 3, NSStatusWindowLevel = 25.
    check(
        "mog_02_appkit_status_level_is_25",
        appkit_level(PanelLevel::Status) == 25 && appkit_level(PanelLevel::Floating) == 3,
        format!(
            "{} {}",
            appkit_level(PanelLevel::Status),
            appkit_level(PanelLevel::Floating)
        ),
    );
    let spec = WindowKind::mog("m1").unwrap().panel().unwrap();
    // canJoinAllSpaces 1<<0 | stationary 1<<4 | ignoresCycle 1<<6 | fullScreenAuxiliary 1<<8.
    check(
        "mog_02_appkit_collection_behavior_bits",
        appkit_collection_behavior(&spec) == 0x151,
        format!("{:#x}", appkit_collection_behavior(&spec)),
    );

    // ---- A plain window ----
    let window = new_window();
    let before: PanelReadback = read_panel(&window);
    check(
        "mog_02_plain_window_is_not_a_panel_before_the_step",
        !before.is_panel && before.has_shadow && before.hides_on_deactivate && before.opaque,
        format!("{before:?}"),
    );
    // SAFETY: main thread, our own window.
    let applied = unsafe { apply_panel(&window, &spec) };
    check(
        "mog_02_panel_step_accepts_the_mog_spec",
        applied.is_ok(),
        format!("{applied:?}"),
    );
    let mut main_capable = spec;
    main_capable.can_become_main = true;
    // SAFETY: as above.
    let refused = unsafe { apply_panel(&new_window(), &main_capable) };
    check(
        "mog_02_panel_step_refuses_a_main_capable_spec",
        refused.is_err(),
        format!("{refused:?}"),
    );
    let card_spec = WindowKind::card("m1").unwrap().panel().unwrap();
    // SAFETY: as above.
    let refused = unsafe { apply_panel(&window, &card_spec) };
    check(
        "ux_19a_panel_step_never_turns_a_mog_panel_into_a_key_panel",
        refused.is_err() && !read_panel(&window).can_become_key,
        format!("{refused:?}"),
    );
    let got = read_panel(&window);
    check(
        "mog_02_appkit_reads_back_a_status_nspanel_that_never_becomes_key",
        got == want,
        format!("got {got:?}\n  want {want:?}"),
    );

    // ---- A window with a WKWebView, as wry builds it: WebKit observes the window (KVO) ----
    let web_window = new_window();
    let content = NSView::initWithFrame(NSView::alloc(mtm), rect);
    web_window.setContentView(Some(&content));
    // SAFETY: main thread; a default configuration, no page is loaded.
    let web: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(
            WKWebView::alloc(mtm),
            content.bounds(),
            &WKWebViewConfiguration::new(mtm),
        )
    };
    content.addSubview(&web);
    let observed = class_name(&web_window);
    check(
        "mog_02_webkit_observes_the_window_before_the_step",
        observed.starts_with("NSKVONotifying_"),
        observed.clone(),
    );
    // SAFETY: main thread, our own window.
    let applied = unsafe { apply_panel(&web_window, &spec) };
    let got = read_panel(&web_window);
    check(
        "mog_02_panel_step_makes_a_webview_window_a_status_nspanel",
        applied.is_ok() && got == want,
        format!(
            "{applied:?} {observed} -> {}\n  got {got:?}",
            class_name(&web_window)
        ),
    );
    let still_in = web
        .window()
        .is_some_and(|w| std::ptr::eq(&*w, &*web_window))
        && web_window
            .contentView()
            .is_some_and(|c| std::ptr::eq(&*c, &*content));
    check(
        "mog_02_panel_step_keeps_the_webview_in_the_window",
        still_in,
        format!("webview window: {:?}", web.window().map(|w| class_name(&w))),
    );
    check(
        "mog_02_webkit_observes_the_panel_again_after_the_step",
        class_name(&web_window) == "NSKVONotifying_BottleMogPanel",
        class_name(&web_window),
    );

    // ---- #78 MOG-03a: the card panel, on a window with a WKWebView ----
    let card_window = new_window();
    let card_content = NSView::initWithFrame(NSView::alloc(mtm), rect);
    card_window.setContentView(Some(&card_content));
    // SAFETY: main thread; a default configuration, no page is loaded.
    let card_web: Retained<WKWebView> = unsafe {
        WKWebView::initWithFrame_configuration(
            WKWebView::alloc(mtm),
            card_content.bounds(),
            &WKWebViewConfiguration::new(mtm),
        )
    };
    card_content.addSubview(&card_web);
    // SAFETY: main thread, our own window.
    let applied = unsafe { apply_panel(&card_window, &card_spec) };
    let got = read_panel(&card_window);
    let want_card = PanelReadback {
        is_panel: true,
        can_become_key: true,
        can_become_main: false,
        nonactivating: true,
        level: 3,
        collection_behavior: appkit_collection_behavior(&card_spec),
        hides_on_deactivate: false,
        // The card is not transparent: the step keeps the window's own opacity and shadow.
        opaque: true,
        has_shadow: true,
    };
    check(
        "mog_03a_appkit_reads_back_a_key_capable_floating_nspanel",
        applied.is_ok() && got == want_card,
        format!("{applied:?}\n  got {got:?}\n  want {want_card:?}"),
    );
    check(
        "mog_03a_webkit_observes_the_card_panel_after_the_step",
        class_name(&card_window) == "NSKVONotifying_BottleCardPanel"
            && card_web
                .window()
                .is_some_and(|w| std::ptr::eq(&*w, &*card_window)),
        class_name(&card_window),
    );
    // SAFETY: main thread, our own window.
    let refused = unsafe { apply_panel(&card_window, &spec) };
    check(
        "ux_19a_panel_step_never_turns_a_card_panel_into_a_mog_panel",
        refused.is_err() && read_panel(&card_window).can_become_key,
        format!("{refused:?}"),
    );
    let focused = focus_web_view(&card_window);
    let responder_is_web = card_window.firstResponder().is_some_and(|r| {
        std::ptr::eq(
            &*r as *const _ as *const u8,
            &*card_web as *const WKWebView as *const u8,
        )
    });
    check(
        "mog_03a_card_makes_its_web_view_the_first_responder",
        focused && responder_is_web,
        format!("focused={focused} responder_is_web={responder_is_web}"),
    );
    check(
        "mog_03a_focus_finds_no_web_view_in_a_plain_window",
        !focus_web_view(&new_window()),
        String::new(),
    );

    // ---- MOG-01c: the drag decision for real AppKit events ----
    let event = |t: NSEventType, x: f64, y: f64| -> Retained<NSEvent> {
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
    };
    let mut g = DragGesture::default();
    let steps = [
        decide(
            &mut g,
            &event(NSEventType::LeftMouseDown, 20.0, 20.0),
            &window,
        ),
        decide(
            &mut g,
            &event(NSEventType::LeftMouseDragged, 22.0, 21.0),
            &window,
        ),
        decide(
            &mut g,
            &event(NSEventType::LeftMouseDragged, 30.0, 20.0),
            &window,
        ),
        decide(
            &mut g,
            &event(NSEventType::LeftMouseDragged, 60.0, 20.0),
            &window,
        ),
        decide(
            &mut g,
            &event(NSEventType::LeftMouseUp, 60.0, 20.0),
            &window,
        ),
    ];
    check(
        "mog_01c_appkit_events_on_a_mog_panel_drag_and_swallow_the_mouse_up",
        steps
            == [
                Some(Verdict::Pass),
                Some(Verdict::Pass),
                Some(Verdict::StartDrag),
                Some(Verdict::Swallow),
                Some(Verdict::Swallow),
            ],
        format!("{steps:?}"),
    );
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
    check(
        "mog_01c_appkit_click_on_a_mog_panel_passes_to_the_page",
        click == [Some(Verdict::Pass), Some(Verdict::Pass)],
        format!("{click:?}"),
    );
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
    let mut g = DragGesture::default();
    let on_card = [
        decide(
            &mut g,
            &event(NSEventType::LeftMouseDown, 20.0, 20.0),
            &card_window,
        ),
        decide(
            &mut g,
            &event(NSEventType::LeftMouseDragged, 90.0, 20.0),
            &card_window,
        ),
    ];
    check(
        "ux_19a_appkit_events_on_a_card_panel_are_not_dragged",
        on_card == [None, None],
        format!("{on_card:?}"),
    );
    check(
        "mog_01c_appkit_events_on_other_windows_or_buttons_are_not_touched",
        other == [None, None, None] && !g.is_dragging(),
        format!("{other:?}"),
    );

    window.close();
    web_window.close();
    card_window.close();
    plain.close();
    println!(
        "\ntest result: {}. {failed} failed",
        if failed == 0 { "ok" } else { "FAILED" }
    );
    if failed > 0 {
        std::process::exit(101);
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("macos_panel_appkit: skipped (not macOS)");
}
