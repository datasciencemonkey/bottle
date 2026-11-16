//! The Bottle Tauri app (ARC-1). One OS process: this crate links `mog-core`.
//!
//! The app only hosts windows. Domain logic runs in the hidden `brain` window (ARC-2);
//! secrets and I/O stay in `mog-core` (section 4.4). The IPC command surface lands in #223.

pub mod windows;

use windows::WindowKind;

/// The id of the demo MOG and card that a debug build opens when `BOTTLE_DEMO_WINDOWS=1`.
pub const DEMO_ID: &str = "demo";

/// The windows Bottle opens at start: the hidden brain and the hub.
/// MOG and card windows open on demand (ARC-3).
pub fn startup_windows(demo: bool) -> Vec<WindowKind> {
    let mut kinds = vec![WindowKind::Brain, WindowKind::Hub];
    if demo {
        kinds.push(WindowKind::Mog(DEMO_ID.to_owned()));
        kinds.push(WindowKind::Card(DEMO_ID.to_owned()));
    }
    kinds
}

/// The core version that this app links (ARC-1).
pub fn linked_core_version() -> &'static str {
    mog_core::core_version()
}

/// Start the app.
pub fn run() {
    let demo =
        cfg!(debug_assertions) && std::env::var("BOTTLE_DEMO_WINDOWS").is_ok_and(|v| v == "1");
    tauri::Builder::default()
        .setup(move |app| {
            let handle = app.handle();
            for kind in startup_windows(demo) {
                windows::build(handle, &kind.spec())?;
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Bottle app");
}
