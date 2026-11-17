//! The Bottle Tauri app (ARC-1). One OS process: this crate links `mog-core`.
//!
//! The app only hosts windows. Domain logic runs in the hidden `brain` window (ARC-2);
//! secrets and I/O stay in `mog-core` (section 4.4). The IPC command surface is in `ipc` (#223).

pub mod hotkey_host;
pub mod hotkey_settings;
pub mod ipc;
#[cfg(target_os = "macos")]
pub mod macos_panel;
pub mod mog_config_host;
pub mod mog_drag;
pub mod webview_media;
pub mod window_host;
pub mod windows;

use tauri::Manager;
use window_host::TauriWindowHost;
use windows::WindowKind;

/// The id of the demo MOG and card that a debug build opens when `BOTTLE_DEMO_WINDOWS=1`.
pub const DEMO_ID: &str = "demo";

/// MOG-01b: macOS `activateIgnoringOtherApps` at launch. The tao default (`true`) activates Bottle
/// and takes the frontmost app away from the user (SP-01 section 4.2).
pub const ACTIVATE_IGNORING_OTHER_APPS: bool = false;

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

/// Debug demo only: hide and show the demo card `DEMO_CARD_OPENS` times, 1 s apart, then leave
/// it hidden, so the Mac check can count opens and watch the reaper (MOG-03b, MOG-18b).
pub const DEMO_CARD_OPENS: usize = 20;

fn demo_card_cycle(app: tauri::AppHandle<tauri::Wry>) {
    let _ = std::thread::Builder::new()
        .name("bottle-demo-card-cycle".into())
        .spawn(move || {
            let step = std::time::Duration::from_secs(1);
            let Some(host) = app.try_state::<TauriWindowHost<tauri::Wry>>() else {
                return;
            };
            // Time to start `swift tools/mac/card-check.swift` in a terminal.
            std::thread::sleep(step * 20);
            for _ in 0..DEMO_CARD_OPENS {
                let _ = host.hide_card(DEMO_ID);
                std::thread::sleep(step);
                let _ = host.show_card(DEMO_ID);
                std::thread::sleep(step);
            }
            let _ = host.hide_card(DEMO_ID);
        });
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
    #[cfg(target_os = "macos")]
    let builder = builder.activate_ignoring_other_apps(ACTIVATE_IGNORING_OTHER_APPS);
    builder
        .manage(ipc::BrainChannel::default())
        .invoke_handler(ipc::handler)
        // CAP-02b: every WebView denies mic, camera and other permission requests (#387).
        .on_permission_request(webview_media::on_permission_request)
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
                // `setup` runs on the main thread; without it the native drag (MOG-01c) is lost.
                let mtm = objc2::MainThreadMarker::new()
                    .ok_or("Bottle setup is not on the main thread")?;
                macos_panel::install_drag_monitor(mtm);
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
                // The brain subscribes later; `hotkey:status` is sticky, so it still gets this.
                hotkey_host::report_unavailable(handle);
            }
            // MB-02: the MOG config store in the app config dir (#25). If the OS gives no app
            // config dir, the app still runs; `config_*` then fail with a code (#333).
            let _ = mog_config_host::install(handle);
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
            // After setup the host refuses main-thread calls, which could deadlock (ADR 0022).
            host.seal_main_thread();
            // MOG-18b: destroy cards that are hidden for 5 min.
            window_host::spawn_card_reaper(handle.clone())?;
            if demo && std::env::var("BOTTLE_DEMO_CARD_CYCLE").is_ok_and(|v| v == "1") {
                // The Mac check of #78 (tools/mac/card-check.swift): 20 card opens.
                demo_card_cycle(handle.clone());
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
