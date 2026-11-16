//! CAP-02b / AT-17 (#387): no WebView may use browser media APIs.
//!
//! The Rust core owns the microphone (cpal). The app holds the OS mic grant for the core, and a
//! WebView runs under the same app, so a WebView `getUserMedia` call must be denied here.
//! With no handler, wry grants every WKWebView media request on macOS (ADR 0037).
//!
//! `run()` installs this handler on the app builder, so it covers every WebView. `Deny` maps to
//! `WKPermissionDecision::Deny` (WKUIDelegate `requestMediaCapturePermission`, macOS) and to
//! `COREWEBVIEW2_PERMISSION_STATE_DENY` (WebView2 `PermissionRequested`, Windows). Neither shows
//! a prompt. Bottle views need no browser permission, so every kind is denied.

use tauri::webview::{PermissionKind, PermissionResponse};
use tauri::{Runtime, Webview};

/// The app-wide WebView permission handler: deny every request, never prompt.
pub fn on_permission_request<R: Runtime>(
    _webview: Webview<R>,
    _kind: PermissionKind,
) -> PermissionResponse {
    PermissionResponse::Deny
}
