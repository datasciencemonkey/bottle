//! #11 MOG-02: make a real NSWindow, apply the MOG panel step and read it back from AppKit.
//!
//! AppKit windows must be made on the main thread, and the default test harness runs each
//! test on a worker thread. So this file has its own `main` (`harness = false` in Cargo.toml).
//! The window is never ordered front, so nothing shows on screen.
//! On other OSes the file prints a skip line.

#[cfg(target_os = "macos")]
fn main() {
    use bottle_lib::macos_panel::{
        appkit_collection_behavior, appkit_level, apply_panel, read_panel, PanelReadback,
    };
    use bottle_lib::windows::{PanelLevel, WindowKind};
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize};

    let mtm = MainThreadMarker::new().expect("harness = false runs on the main thread");
    let mut failed = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!("test {name} ... {}", if ok { "ok" } else { "FAILED" });
        if !ok {
            println!("  {detail}");
            failed += 1;
        }
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

    let rect = NSRect::new(NSPoint::new(-4000.0, -4000.0), NSSize::new(160.0, 160.0));
    // SAFETY: main thread; a plain borderless window that is never shown.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            rect,
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            true,
        )
    };
    // SAFETY: we own the window, it is on the main thread and nothing observes it yet.
    unsafe { window.setReleasedWhenClosed(false) };
    let before: PanelReadback = read_panel(&window);
    check(
        "mog_02_plain_window_is_not_a_panel_before_the_step",
        !before.is_panel,
        format!("{before:?}"),
    );
    // SAFETY: as above.
    let applied = unsafe { apply_panel(&window, &spec) };
    check(
        "mog_02_panel_step_accepts_the_mog_spec",
        applied.is_ok(),
        format!("{applied:?}"),
    );
    let mut keyed = spec;
    keyed.can_become_key = true;
    // SAFETY: as above.
    let refused = unsafe { apply_panel(&window, &keyed) };
    check(
        "mog_02_panel_step_refuses_a_key_capable_spec",
        refused.is_err(),
        format!("{refused:?}"),
    );
    let got = read_panel(&window);
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
    check(
        "mog_02_appkit_reads_back_a_status_nspanel_that_never_becomes_key",
        got == want,
        format!("got {got:?}\n  want {want:?}"),
    );
    window.close();

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
