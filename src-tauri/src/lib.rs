//! The Bottle Tauri app (ARC-1). One OS process: this crate links `mog-core`.
//!
//! The app only hosts windows. Domain logic runs in the hidden `brain` window (ARC-2);
//! secrets and I/O stay in `mog-core` (section 4.4). The IPC command surface is in `ipc` (#223).

pub mod hotkey_host;
pub mod ipc;
#[cfg(target_os = "macos")]
pub mod macos_panel;
pub mod mog_drag;
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
    let builder = tauri::Builder::default();
    // MOG-01b: the tao default (`true`) activates Bottle at launch and takes the frontmost app
    // away from the user (SP-01 section 4.2).
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(false);
    builder
        .manage(ipc::BrainChannel::default())
        .invoke_handler(ipc::handler)
        // R1: an opened MOG or card view moves the talk-key target (#305).
        .on_page_load(hotkey_host::on_page_load)
        .on_window_event(|window, event| {
            // R1: a focused card or a closed MOG window moves the talk-key target (#305).
            hotkey_host::on_window_event(window, event);
            // R11: a window that closes outside the host frees its ledger slot.
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(host) = window.try_state::<TauriWindowHost<tauri::Wry>>() {
                    host.window_destroyed(window.label());
                }
            }
        })
        .setup(move |app| {
            let handle = app.handle();
            // MOG-02 / MOG-01c: MOG windows are non-activating NSPanels that a native drag moves.
            #[cfg(target_os = "macos")]
            {
                use objc2::MainThreadMarker;
                if let Some(mtm) = MainThreadMarker::new() {
                    macos_panel::install_drag_monitor(mtm);
                }
                app.manage(TauriWindowHost::with_style(
                    handle.clone(),
                    Box::new(macos_panel::MacPanelStyle),
                ));
            }
            #[cfg(not(target_os = "macos"))]
            app.manage(TauriWindowHost::new(handle.clone()));
            // R1: start the talk key. This never prompts (CAP-36, #305). If the forwarder
            // thread cannot start, the app still runs without the talk key.
            if hotkey_host::install(handle).is_err() {
                // TODO(#297): report the talk key as unavailable once `hotkey.status` exists.
            }
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
        .build(tauri::generate_context!())
        .expect("error while building the Bottle app")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                // R1: remove the key tap on quit (#305).
                hotkey_host::shutdown(app);
            }
        });
}
