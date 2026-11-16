//! The Bottle Tauri app (ARC-1). One OS process: this crate links `mog-core`.
//!
//! The app only hosts windows. Domain logic runs in the hidden `brain` window (ARC-2);
//! secrets and I/O stay in `mog-core` (section 4.4). The IPC command surface is in `ipc` (#223).

pub mod ipc;
pub mod window_host;
pub mod windows;

use tauri::Manager;
use window_host::TauriWindowHost;
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
        .manage(ipc::BrainChannel::default())
        .invoke_handler(ipc::handler)
        .on_window_event(|window, event| {
            // R11: a window that closes outside the host frees its ledger slot.
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(host) = window.try_state::<TauriWindowHost<tauri::Wry>>() {
                    host.window_destroyed(window.label());
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle();
            app.manage(TauriWindowHost::new(handle.clone()));
            let host = app.state::<TauriWindowHost<tauri::Wry>>();
            for kind in startup_windows(demo) {
                // MOG and card windows go through R11, so the cap and the ledger hold.
                match &kind {
                    WindowKind::Mog(id) => host.open_mog_window(id)?,
                    WindowKind::Card(id) => host.show_card(id)?,
                    _ => {
                        windows::build(handle, &kind.spec())?;
                    }
                }
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running the Bottle app");
}
