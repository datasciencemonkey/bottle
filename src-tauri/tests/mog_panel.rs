//! #11: the MOG window is a transparent, frameless, always-on-top panel that never takes focus
//! (MOG-01a, MOG-01b), is an NSPanel at Status level on macOS (MOG-02), and moves by a native
//! drag that starts no capture (MOG-01c). The AppKit read-back of a real panel is in
//! `tests/macos_panel_appkit.rs`; the on-screen checks (AT-01) are `needs:mac` steps.

use bottle_lib::mog_drag::{DragGesture, MouseInput, Verdict, DRAG_THRESHOLD_PT};
use bottle_lib::window_host::{NativeWindowStyle, TauriWindowHost, WindowError};
use bottle_lib::windows::{PanelLevel, WindowKind};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const ID: &str = "7b0c1f3e-0000-4000-8000-000000000001";

fn mog() -> WindowKind {
    WindowKind::mog(ID).expect("valid id")
}

fn app_file(rel: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)).expect(rel)
}

// ---------- MOG-01a / MOG-01b: the window spec ----------

#[test]
fn mog_01a_mog_spec_is_transparent_frameless_on_top_without_shadow() {
    let spec = mog().spec();
    assert!(spec.transparent, "the desktop must show through");
    assert!(!spec.decorations, "frameless");
    assert!(spec.always_on_top);
    assert!(!spec.shadow, "a shadow draws a halo round the transparent pixels");
}

#[test]
fn mog_01b_mog_spec_never_takes_focus_and_accepts_the_first_click() {
    let spec = mog().spec();
    assert!(!spec.focusable, "a MOG window must not take focus");
    assert!(
        spec.accept_first_mouse,
        "a never-key panel gets every click as a first click"
    );
    for kind in [WindowKind::Brain, WindowKind::Hub] {
        assert!(!kind.spec().accept_first_mouse, "{:?}", kind);
    }
}

#[test]
fn mog_01b_mog_is_built_hidden_and_shown_by_the_native_style() {
    // A visible build would order the window front before it is a non-activating panel.
    assert!(mog().spec().visible, "the MOG shows once it is open");
    assert!(!mog().build_spec().visible, "built hidden, then styled, then shown");
    let card = WindowKind::card(ID).unwrap();
    assert_eq!(card.build_spec(), card.spec(), "no MOG panel step for a card");
    assert_eq!(WindowKind::Hub.build_spec(), WindowKind::Hub.spec());
}

#[test]
fn mog_01b_app_does_not_activate_ignoring_other_apps() {
    // SP-01 section 4.2: the tao default (`true`) activates Bottle at launch and takes the
    // frontmost app away from the user.
    let lib = app_file("src/lib.rs");
    assert!(
        lib.contains(".activate_ignoring_other_apps(false)"),
        "lib.rs must turn off activate_ignoring_other_apps on macOS"
    );
}

// ---------- MOG-02: the macOS panel ----------

#[test]
fn mog_02_mog_kind_is_a_status_level_panel_that_never_becomes_key() {
    let p = mog().panel().expect("a MOG window is a panel");
    assert_eq!(p.level, PanelLevel::Status);
    assert!(!p.can_become_key, "canBecomeKey=false");
    assert!(!p.can_become_main);
    assert!(p.nonactivating, "NSWindowStyleMaskNonactivatingPanel");
    assert!(!p.hides_on_deactivate, "hidesOnDeactivate=false");
}

#[test]
fn mog_02_panel_joins_all_spaces_stationary_fullscreen_aux_and_ignores_cycle() {
    let p = mog().panel().unwrap();
    assert!(p.join_all_spaces, "canJoinAllSpaces");
    assert!(p.stationary, "stationary");
    assert!(p.full_screen_auxiliary, "fullScreenAuxiliary");
    assert!(p.ignores_cycle, "ignoresCycle");
}

#[test]
fn mog_02_only_mog_windows_are_status_panels() {
    assert!(WindowKind::Brain.panel().is_none());
    assert!(WindowKind::Hub.panel().is_none());
    // TODO(#78): the card becomes a key-capable panel at Floating level.
    assert!(WindowKind::card(ID).unwrap().panel().is_none());
}

#[test]
fn mog_02_macos_panel_code_is_gated_to_macos() {
    let lib = app_file("src/lib.rs");
    assert!(
        lib.contains("#[cfg(target_os = \"macos\")]\npub mod macos_panel;"),
        "the objc2 panel module must build only on macOS, so the Windows build holds"
    );
    let cargo = app_file("Cargo.toml");
    let mac = cargo
        .split("[target.'cfg(target_os = \"macos\")'.dependencies]")
        .nth(1)
        .expect("a macOS-only dependency table");
    assert!(mac.contains("objc2-app-kit"));
    let before = cargo
        .split("[target.'cfg(target_os = \"macos\")'.dependencies]")
        .next()
        .unwrap();
    assert!(!before.contains("objc2"), "objc2 must not be a dependency on every OS");
    assert!(!cargo.contains("git ="), "no git dependency (supply chain, #160)");
}

// ---------- MOG-02 / MOG-01b: the host applies the native style ----------

#[derive(Default, Clone)]
struct Recorder(Arc<Mutex<Vec<String>>>);

impl Recorder {
    fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl<R: tauri::Runtime> NativeWindowStyle<R> for Recorder {
    fn style(&self, window: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        assert_eq!(window.label(), kind.label());
        self.0.lock().unwrap().push(format!("style {}", kind.label()));
        Ok(())
    }

    fn show(&self, window: &tauri::WebviewWindow<R>, kind: &WindowKind) -> Result<(), WindowError> {
        assert_eq!(window.label(), kind.label());
        self.0.lock().unwrap().push(format!("show {}", kind.label()));
        Ok(())
    }
}

struct Refuse;

impl<R: tauri::Runtime> NativeWindowStyle<R> for Refuse {
    fn style(&self, _: &tauri::WebviewWindow<R>, _: &WindowKind) -> Result<(), WindowError> {
        Err(WindowError::HostFailed)
    }

    fn show(&self, _: &tauri::WebviewWindow<R>, _: &WindowKind) -> Result<(), WindowError> {
        Ok(())
    }
}

#[test]
fn mog_02_host_styles_a_new_mog_window_before_it_shows() {
    let app = tauri::test::mock_app();
    let rec = Recorder::default();
    let host = TauriWindowHost::with_style(app.handle().clone(), Box::new(rec.clone()));
    host.open_mog_window("m1").unwrap();
    assert_eq!(rec.calls(), ["style mog-m1", "show mog-m1"]);
    // A second open shows the window again and does not style it twice.
    host.open_mog_window("m1").unwrap();
    assert_eq!(rec.calls(), ["style mog-m1", "show mog-m1", "show mog-m1"]);
}

#[test]
fn mog_02_host_frees_the_slot_when_the_panel_step_fails() {
    use tauri::Manager;
    let app = tauri::test::mock_app();
    let host = TauriWindowHost::with_style(app.handle().clone(), Box::new(Refuse));
    assert_eq!(host.open_mog_window("m1"), Err(WindowError::HostFailed));
    assert_eq!(host.active_mogs(), 0, "a MOG that is not a panel must not stay open");
    let left = app.get_webview_window("mog-m1");
    assert!(left.is_none() || left.unwrap().destroy().is_ok());
}

#[test]
fn mog_02_app_host_uses_the_macos_panel_style() {
    let lib = app_file("src/lib.rs");
    assert!(lib.contains("TauriWindowHost::with_style("));
    assert!(lib.contains("macos_panel::MacPanelStyle"));
    assert!(lib.contains("macos_panel::install_drag_monitor("));
}

// ---------- MOG-01c: the native drag gesture ----------

fn run(g: &mut DragGesture, inputs: &[MouseInput]) -> Vec<Verdict> {
    inputs.iter().map(|i| g.on(*i)).collect()
}

#[test]
fn mog_01c_threshold_is_4_points() {
    assert_eq!(DRAG_THRESHOLD_PT, 4.0);
}

#[test]
fn mog_01c_a_small_move_passes_the_click_to_the_view() {
    let mut g = DragGesture::default();
    let v = run(
        &mut g,
        &[
            MouseInput::Down { x: 10.0, y: 10.0 },
            MouseInput::Dragged { x: 12.0, y: 11.0 },
            MouseInput::Dragged { x: 13.0, y: 12.0 },
            MouseInput::Up,
        ],
    );
    assert_eq!(v, [Verdict::Pass; 4], "a move under 4 pt stays a click");
    assert!(!g.is_dragging());
}

#[test]
fn mog_01c_a_4_point_move_starts_exactly_one_native_drag() {
    let mut g = DragGesture::default();
    let v = run(
        &mut g,
        &[
            MouseInput::Down { x: 0.0, y: 0.0 },
            MouseInput::Dragged { x: 3.9, y: 0.0 },
            MouseInput::Dragged { x: 0.0, y: 4.0 },
            MouseInput::Dragged { x: 40.0, y: 30.0 },
        ],
    );
    assert_eq!(
        v,
        [
            Verdict::Pass,
            Verdict::Pass,
            Verdict::StartDrag,
            Verdict::Swallow
        ]
    );
    assert!(g.is_dragging());
}

#[test]
fn mog_01c_the_threshold_is_the_distance_not_one_axis() {
    let mut g = DragGesture::default();
    g.on(MouseInput::Down { x: 0.0, y: 0.0 });
    // 3 and 3 is 4.24 pt.
    assert_eq!(g.on(MouseInput::Dragged { x: 3.0, y: -3.0 }), Verdict::StartDrag);
}

#[test]
fn mog_01c_a_drag_swallows_the_mouse_up_so_no_click_and_no_capture_starts() {
    let mut g = DragGesture::default();
    run(
        &mut g,
        &[
            MouseInput::Down { x: 0.0, y: 0.0 },
            MouseInput::Dragged { x: 10.0, y: 0.0 },
        ],
    );
    assert_eq!(
        g.on(MouseInput::Up),
        Verdict::Swallow,
        "the view must never see the mouse-up of a drag, so it emits no click"
    );
    assert!(!g.is_dragging(), "the gesture ends on mouse-up");
    // The next press is a fresh gesture: a click again.
    assert_eq!(g.on(MouseInput::Down { x: 50.0, y: 50.0 }), Verdict::Pass);
    assert_eq!(g.on(MouseInput::Up), Verdict::Pass);
}

#[test]
fn mog_01c_moves_and_ups_without_a_press_pass_through() {
    let mut g = DragGesture::default();
    assert_eq!(g.on(MouseInput::Dragged { x: 99.0, y: 99.0 }), Verdict::Pass);
    assert_eq!(g.on(MouseInput::Up), Verdict::Pass);
    assert!(!g.is_dragging());
}

#[test]
fn mog_01c_view_capabilities_hold_no_window_drag_permission() {
    // The drag is native (no `startDragging` from the page), so MOG-08a holds.
    for cap in ["mog", "card", "hub"] {
        let text = app_file(&format!("capabilities/{cap}.json"));
        assert!(!text.contains("start-dragging"), "{cap}");
        assert!(!text.contains("core:window"), "{cap}");
    }
}
