# ADR 0021: WebView lockdown and the offline Rive loader

- Status: Accepted
- Date: 2026-11-16
- Issue: #208
- Owner: security-lead (implemented by impl-208)

## Context

Issue #208 asks for a CSP with no external `connect-src` in each WebView (SEC-07a), no `http`, `shell` or `fs` plugin in any capability (SEC-07b), and Rive with `enableRiveAssetCDN=false` and bundled assets, so the mascot loads offline (SEC-20a, 4.6 rule 4). Issue #223 already set one CSP in `tauri.conf.json` and one capability file per window kind. The Rive mascot belongs to #3, and no `.riv` file may be committed. The issue left the points below open.

## Decisions

1. **One CSP for every window.** Tauri v2 applies `app.security.csp` to every page it serves from the bundle. Every window uses `WebviewUrl::App`, so one policy covers the brain, hub, MOG and card windows. A test fails on `WebviewUrl::External`, `WebviewUrl::CustomProtocol`, a WebView `navigate` call, a window URL with a scheme, `dangerousDisableAssetCspModification`, `dangerousRemoteDomainIpcAccess`, or a capability with a `remote` scope.
2. **`connect-src` adds `'self'`.** The Rive runtime fetches `rive.wasm` and the `.riv` file. Without `'self'`, the bundled files cannot load. The release CSP is `default-src 'self'; connect-src 'self' ipc: http://ipc.localhost; script-src 'self' 'wasm-unsafe-eval'`. `'self'` is the app origin (`tauri://localhost` or `http://tauri.localhost`), not an external host.
3. **The CSP check is an allowlist.** Each source of each directive must be a keyword or hash, `ipc:` or `http://ipc.localhost`. `devCsp` may also use the loopback dev server on port 1420. Any other source fails, including `data:` and `blob:`. A small CSP matcher also proves that no fetch directive reaches unpkg, jsdelivr, the Rive asset CDN or any https host.
4. **SEC-07b is checked at every place a plugin can enter.** These are capability permissions (`http:`, `shell:`, `fs:`), `plugins` in `tauri.conf.json`, `Cargo.lock` and every workspace `Cargo.toml`, `package.json`, and the Rust sources. The older ARC-4 tests in `skeleton.rs` and `tests/config.test.ts` stay. #36 owns the view-command rule (`sec_02a_*`).
5. **One Rive loader.** `src/views/mog/rive-loader.ts` is the only `src/` file that imports `@rive-app`. `createOfflineRive` sets `enableRiveAssetCDN: false` last, so no option turns it back on. It sets the runtime wasm to the bundled `rive.wasm` (Vite `?url`), and it sets the fallback wasm and the threads wasm to `null`. It throws on a `.riv` URL with a scheme or a host. The CSP is the second guard.
6. **The inspection functions are test code.** They live in `tests/webview-lockdown.ts`, with the same pattern as `tests/packaging-pins.ts`, and each one has a bad-fixture self-test.

## Gaps

- `TODO(#3)`: the mascot renderer builds its Rive instance through `createOfflineRive` with the bundled `.riv` URL. Until then, the MOG view shows `PlaceholderMascot`, and the bundle has no Rive code. The `.riv` must embed its assets. If it uses referenced assets, #3 adds a bundled `assetLoader` in the loader.
- The real offline run (AT-37) needs a built app on a Mac with the network off. It is on #208 as `needs:human` + `needs:mac`, and #208 stays open for it.
