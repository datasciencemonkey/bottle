# ADR 0037: Block browser media capture in every WebView

- Status: Accepted
- Date: 2026-11-16
- Issue: #387 (CAP-02b, AT-17; part of #4)

## Context

CAP-02b says that WebView windows must not use browser media APIs. Its test says that the CSP and
the capabilities block `getUserMedia`. The issue did not say which mechanism to use.

These facts limit the choice:

1. CSP has no directive for `getUserMedia` or for other media capture. CSP controls where a page
   loads content from. It does not control device access.
2. Tauri capabilities control only IPC commands. Tauri has no media permission, so a capability
   cannot grant or deny the microphone.
3. Wry 0.57.2 (used by Tauri 2.12.5) grants every WKWebView media request on macOS when the app sets
   no permission handler. See `wry/src/wkwebview/class/wry_web_view_ui_delegate.rs`, method
   `webView:requestMediaCapturePermissionForOrigin:initiatedByFrame:type:decisionHandler:`.
   The `else` branch returns `WKPermissionDecision::Grant`.
4. The Rust core owns the microphone through cpal (#4). The app process holds the OS microphone
   grant for the core. A WebView runs under the same app, so on macOS a WebView `getUserMedia` call
   gets the microphone with no prompt.
5. On Windows, WebView2 shows its own permission prompt when the host app does not handle the
   `PermissionRequested` event.

## Decision

Bottle uses three layers. Layer 1 is the real block. Layers 2 and 3 are defense in depth.

1. **App-wide deny handler.** `run()` calls `tauri::Builder::on_permission_request` with
   `webview_media::on_permission_request`. The handler returns `PermissionResponse::Deny` for every
   permission kind, so it covers microphone, camera and display capture. Tauri applies the app-level
   handler to every WebView that the app creates. Wry maps `Deny` to these platform values:
   - macOS: WKUIDelegate media capture decision `WKPermissionDecision.deny`.
   - Windows: WebView2 `PermissionRequested` state `COREWEBVIEW2_PERMISSION_STATE_DENY`.

   Both values deny the request with no prompt. The handler never returns `Default`, because
   `Default` shows a prompt. Bottle views need no browser permission, so the handler denies all kinds.
   A per-WebView handler runs before the app-level handler and can override it. Thus no other code
   may call `on_permission_request`. A test enforces this rule.
2. **Permissions-Policy header.** `app.security.headers` sets
   `Permissions-Policy: camera=(), microphone=(), display-capture=()`. Tauri adds this header to each
   page that the `tauri://` protocol serves. Chromium (WebView2) then rejects `getUserMedia` with
   `NotAllowedError` before any permission request. WebKit support for this header is partial, so
   layer 1 is still necessary on macOS. In `pnpm tauri dev` the pages come from the Vite server
   without this header. Layer 1 applies in dev too.
3. **CSP and capabilities.** The CSP stays `default-src 'self'` with no remote host and no
   `mediastream:` or `blob:` source, so no remote script can run in a WebView. The capabilities hold
   no plugin permission and no remote URL (ARC-4). A test fails if a capability mentions media,
   microphone or camera.

The native side does not change. The Info.plist `NSMicrophoneUsageDescription` and the hardened
runtime entitlement `com.apple.security.device.audio-input` belong to the core capture work (#4) and
the packaging work. This ADR does not add or remove them. The WebView block does not depend on them.

## Tests

- `src-tauri/tests/cap_02b_media_handler.rs`: the handler returns `Deny` for every kind, in a
  MockRuntime WebView double for each window class.
- `src-tauri/tests/cap_02b_media_config.rs`: the app installs one app-wide handler, the header denies
  the three media features, and the CSP and capabilities allow no media source.
- Device steps (a person runs them): `docs/tests/webview-media-block.md`.

## Consequences

- A future view feature that needs a browser permission (for example clipboard read) needs a new
  ADR. It must allow that one kind only, and never microphone, camera or display capture.
- If a user allowed a permission in an old WebView2 profile, WebView2 can reuse that saved choice
  without a call to the handler. Bottle never allowed one, so a new install has no saved choice.

## References

- Tauri 2 `Builder::on_permission_request`: <https://docs.rs/tauri/2.12.5/tauri/struct.Builder.html#method.on_permission_request>
- Tauri 2 security headers: <https://v2.tauri.app/security/http-headers/>
- Apple WKUIDelegate media capture decision: <https://developer.apple.com/documentation/webkit/wkuidelegate/webview(_:requestmediacapturepermissionfor:initiatedbyframe:type:decisionhandler:)>
- WebView2 `PermissionRequested`: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2#add_permissionrequested>
- MDN Permissions-Policy: <https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Permissions-Policy>
- MDN CSP directives (no media capture directive): <https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Content-Security-Policy>
