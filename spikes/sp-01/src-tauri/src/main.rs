//! SP-01 throwaway spike: Rive in a transparent, frameless, always-on-top,
//! non-activating Tauri window. Not production code.
//!
//! Env:
//! - `SP01_RIV`     path to the .riv file (never committed)
//! - `SP01_RUNTIME` `webgl2` (default) or `canvas`
//! - `SP01_MODE`    `animating` (default) or `idle`
//! - `SP01_LOG`     JSONL log path (default `/tmp/sp01-events.jsonl`)
//! - `SP01_PANEL`   `1` (default) to apply the objc2 overlay step, `0` to use `show()`
//! - `SP01_POLICY`  `accessory` (default), `prohibited` or `regular`
//! - `SP01_AIOA`    `1` (default, tao behaviour) or `0`: `activate_ignoring_other_apps`
//! - `SP01_DEACTIVATE` `1` to call `NSApp.deactivate()` on Ready and after samples
//! - `SP01_FOCUSABLE` `1` to keep the window focusable (key-window control)
//! - `SP01_KEY`     `1` to make the window key and activate the app (control)
//! - `SP01_TRACK`, `SP01_PANEL_SWAP`, `SP01_SNAP`: see `macos.rs`

use std::{fs::OpenOptions, io::Write, path::PathBuf, time::SystemTime};

use base64::Engine as _;
use tauri::Manager;

#[cfg(target_os = "macos")]
mod macos;

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

#[tauri::command]
fn sp01_config() -> serde_json::Value {
    serde_json::json!({
        "runtime": env_or("SP01_RUNTIME", "webgl2"),
        "mode": env_or("SP01_MODE", "animating"),
    })
}

/// Return a named blob as base64 so that no `fetch` is needed (ARC-4 CSP).
#[tauri::command]
fn sp01_blob(app: tauri::AppHandle, name: String) -> Result<String, String> {
    let path: PathBuf = match name.as_str() {
        "riv" => PathBuf::from(std::env::var("SP01_RIV").map_err(|_| "SP01_RIV unset")?),
        "wasm-webgl2" | "wasm-canvas" => {
            let rt = name.trim_start_matches("wasm-");
            app.path()
                .resource_dir()
                .map_err(|e| e.to_string())?
                .join(format!("vendor/{rt}/rive.wasm"))
        }
        _ => return Err(format!("unknown blob {name}")),
    };
    let bytes = std::fs::read(&path)
        .or_else(|_| {
            // Dev run: read from the frontend folder next to src-tauri.
            let rt = name.trim_start_matches("wasm-");
            std::fs::read(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../web/vendor/{rt}/rive.wasm")),
            )
        })
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Append one JSONL line from the WebView with the Rust receive time.
#[tauri::command]
fn sp01_log(line: String) -> Result<(), String> {
    let path = env_or("SP01_LOG", "/tmp/sp01-events.jsonl");
    let mut rec: serde_json::Value =
        serde_json::from_str(&line).unwrap_or(serde_json::json!({ "raw": line }));
    rec["rust_t"] = serde_json::json!(now_ms());
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{rec}").map_err(|e| e.to_string())
}

/// Fallback step 2 ("own objc2 fix"): make the window behave like a
/// non-activating overlay without `macOSPrivateApi`.
#[cfg(target_os = "macos")]
fn macos_overlay(win: &tauri::WebviewWindow) -> Result<(), String> {
    use objc2_app_kit::{NSColor, NSWindow, NSWindowCollectionBehavior};
    let ptr = win.ns_window().map_err(|e| e.to_string())? as *mut NSWindow;
    // SAFETY: tao owns the NSWindow for the app lifetime; setup runs on the main thread.
    let ns: &NSWindow = unsafe { &*ptr };
    ns.setHidesOnDeactivate(false);
    ns.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
    );
    ns.setAcceptsMouseMovedEvents(true);
    ns.setOpaque(false);
    ns.setBackgroundColor(Some(&NSColor::clearColor()));
    ns.setHasShadow(false);
    if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
        macos::log_tracker_selectors(mtm);
        if macos::want("SP01_PANEL_SWAP") {
            macos::panel_swap(ns);
        }
        if macos::want("SP01_TRACK") {
            macos::install_tracker(win, ns, mtm);
        }
    }
    if macos::want("SP01_KEY") {
        // Key-window control: activate and make key (steals focus on purpose).
        win.set_focus().map_err(|e| e.to_string())?;
        ns.makeKeyAndOrderFront(None);
    } else {
        // Show without activating the app or making the window key.
        ns.orderFrontRegardless();
    }
    Ok(())
}

/// tao activates the app when launching finishes (after `setup`), so give
/// activation back once the run loop is ready (`SP01_DEACTIVATE=1`, default).
fn deactivate_app() {
    #[cfg(target_os = "macos")]
    if env_or("SP01_DEACTIVATE", "0") == "1" {
        if let Some(mtm) = objc2_foundation::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(mtm).deactivate();
        }
    }
}

/// Log whether the mog window is key and whether the app is active.
fn log_focus(app: &tauri::AppHandle, after: f64) {
    let key = app
        .get_webview_window("mog-spike")
        .and_then(|w| w.is_focused().ok())
        .unwrap_or(false);
    #[cfg(target_os = "macos")]
    let active = objc2_foundation::MainThreadMarker::new()
        .map(|mtm| objc2_app_kit::NSApplication::sharedApplication(mtm).isActive())
        .unwrap_or(false);
    #[cfg(not(target_os = "macos"))]
    let active = false;
    #[cfg(target_os = "macos")]
    let visible = app
        .get_webview_window("mog-spike")
        .and_then(|w| w.ns_window().ok())
        .map(|p| {
            // SAFETY: tao's live NSWindow, read on the main thread.
            let ns: &objc2_app_kit::NSWindow = unsafe { &*(p as *mut objc2_app_kit::NSWindow) };
            ns.occlusionState()
                .contains(objc2_app_kit::NSWindowOcclusionState::Visible)
        })
        .unwrap_or(false);
    #[cfg(not(target_os = "macos"))]
    let visible = true;
    let line = serde_json::json!({
        "kind": "focus", "seq": after, "is_key": key, "app_active": active,
        "occlusion_visible": visible,
    });
    let _ = sp01_log(line.to_string());
    if active {
        // Something activated the app after launch: give activation back again.
        deactivate_app();
    }
}

fn main() {
    tauri::Builder::default()
        .activate_ignoring_other_apps(env_or("SP01_AIOA", "1") == "1")
        .invoke_handler(tauri::generate_handler![sp01_config, sp01_blob, sp01_log])
        .setup(|app| {
            // Activation policy: `accessory` (no Dock icon, like a mog) or `regular`.
            #[cfg(target_os = "macos")]
            match env_or("SP01_POLICY", "accessory").as_str() {
                "accessory" => app.set_activation_policy(tauri::ActivationPolicy::Accessory),
                "prohibited" => app.set_activation_policy(tauri::ActivationPolicy::Prohibited),
                _ => app.set_activation_policy(tauri::ActivationPolicy::Regular),
            }
            let win = app
                .get_webview_window("mog-spike")
                .ok_or("mog-spike window missing")?;
            // Small window in the bottom-right corner (defaults fit a 1728x1117 display).
            let size: f64 = env_or("SP01_SIZE", "240").parse().unwrap_or(240.0);
            let x: f64 = env_or("SP01_X", "1468").parse().unwrap_or(1468.0);
            let y: f64 = env_or("SP01_Y", "857").parse().unwrap_or(857.0);
            if env_or("SP01_NOMOVE", "0") != "1" {
                win.set_size(tauri::LogicalSize::new(size, size))?;
                win.set_position(tauri::LogicalPosition::new(x, y))?;
            }
            // Step 1: Tauri-only flags (focus:false in the config, plus not focusable).
            if env_or("SP01_FOCUSABLE", "0") != "1" {
                win.set_focusable(false)?;
            }
            #[cfg(target_os = "macos")]
            {
                if env_or("SP01_PANEL", "1") == "1" {
                    macos_overlay(&win)?;
                } else {
                    win.show()?;
                }
            }
            #[cfg(not(target_os = "macos"))]
            win.show()?;
            sp01_log(r#"{"kind":"info","msg":"window shown"}"#.to_string())?;
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut prev = 0.0_f64;
                for after in [0.5_f64, 2.0, 5.0] {
                    std::thread::sleep(std::time::Duration::from_secs_f64(after - prev));
                    prev = after;
                    let h = handle.clone();
                    let _ = handle.run_on_main_thread(move || log_focus(&h, after));
                }
            });
            #[cfg(target_os = "macos")]
            let snap_win = win.clone();
            #[cfg(target_os = "macos")]
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                let w = snap_win.clone();
                let _ = snap_win.run_on_main_thread(move || macos::snapshot(&w));
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building SP-01 spike")
        .run(|_app, event| {
            if let tauri::RunEvent::Ready = event {
                deactivate_app();
            }
        });
}
