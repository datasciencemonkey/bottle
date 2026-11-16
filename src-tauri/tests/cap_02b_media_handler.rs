//! #387: CAP-02b / AT-17. The app-wide WebView permission handler denies every request.
//!
//! A `Deny` maps to `WKPermissionDecision::Deny` (WKUIDelegate, macOS) and to
//! `COREWEBVIEW2_PERMISSION_STATE_DENY` (WebView2 `PermissionRequested`, Windows), so the
//! request fails with no prompt. `Default` would prompt (and, with no handler, wry grants on
//! macOS), so the handler must never return it. The MockRuntime WebView is the test double.

use bottle_lib::webview_media::on_permission_request;
use tauri::webview::{PermissionKind, PermissionResponse};

/// Every kind that Tauri 2.12 can report. A new kind still gets `Deny` (the handler has no match).
const KINDS: [PermissionKind; 16] = [
    PermissionKind::Microphone,
    PermissionKind::Camera,
    PermissionKind::DisplayCapture,
    PermissionKind::Geolocation,
    PermissionKind::Notifications,
    PermissionKind::ClipboardRead,
    PermissionKind::Midi,
    PermissionKind::Sensors,
    PermissionKind::MediaKeySystemAccess,
    PermissionKind::LocalFonts,
    PermissionKind::WindowManagement,
    PermissionKind::PointerLock,
    PermissionKind::AutomaticDownloads,
    PermissionKind::FileSystemAccess,
    PermissionKind::Autoplay,
    PermissionKind::Other,
];

fn webview(label: &str) -> tauri::Webview<tauri::test::MockRuntime> {
    let app = tauri::test::mock_app();
    let window =
        tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App("mog.html".into()))
            .build()
            .expect("mock webview window");
    AsRef::<tauri::Webview<_>>::as_ref(&window).clone()
}

#[test]
fn cap_02b_handler_denies_mic_and_camera_in_every_window() {
    for label in ["brain", "hub", "mog-a", "card-a"] {
        for kind in [
            PermissionKind::Microphone,
            PermissionKind::Camera,
            PermissionKind::DisplayCapture,
        ] {
            assert_eq!(
                on_permission_request(webview(label), kind),
                PermissionResponse::Deny,
                "{label}: {kind:?}"
            );
        }
    }
}

#[test]
fn cap_02b_handler_denies_every_kind_and_never_prompts() {
    for kind in KINDS {
        let got = on_permission_request(webview("mog-a"), kind);
        assert_eq!(got, PermissionResponse::Deny, "{kind:?}");
        assert_ne!(got, PermissionResponse::Default, "{kind:?} must not prompt");
    }
}
