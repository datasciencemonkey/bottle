# ADR 0009: The macOS MOG panel and its native drag

- Status: Accepted
- Date: 2026-11-16
- Issue: #11 (soft dependency #10, SP-01)
- Owner: shell-lead (implemented by impl-11)

## Context

MOG-01a, MOG-01b, MOG-01c and MOG-02 need each `mog-<id>` window to be transparent, never take focus, move by a drag that starts no capture, and be an NSPanel on macOS. The panel needs `canBecomeKey=false`, Status level, `canJoinAllSpaces|stationary|fullScreenAuxiliary|ignoresCycle` and `hidesOnDeactivate=false`. The issue left four points open. SP-01 (#10, `docs/spikes/SP-01.md`) is PARTIAL. Its unsigned runs answer the activation question and test the NSPanel class swap. The signed-DMG checks are still open.

## Decisions

1. **An objc2 class swap, not `tauri-nspanel`.** `src-tauri/src/macos_panel.rs` swaps the class of the tao window to `BottleMogPanel`, an `NSPanel` subclass whose `canBecomeKeyWindow` and `canBecomeMainWindow` return `false`. Then it sets the style mask, level, collection behavior and transparency of the `PanelSpec` in `windows.rs`. This is the mechanism that `tauri-nspanel` 2.1 uses and that SP-01 tested ("panel swap", section 4.2). `tauri-nspanel` has no crates.io release for Tauri 2.12, so it would be a git dependency. `objc2 0.6.5`, `objc2-app-kit 0.3.2` and `block2 0.6.2` are already in `Cargo.lock` through tao and wry. So the swap adds no crate and no git dependency (#160). The macOS-only dependency table keeps the Windows build free of them. The panel settings follow Handy `src-tauri/src/overlay.rs`, so `macos_panel.rs` has the Handy MIT header.
2. **Class-swap safety.** `MogPanel` adds no instance variables. The tao window keeps its memory, and tao reads its own instance variables only through its typed pointer. The swap drops tao's `sendEvent:` override, which only serves `movableByWindowBackground`. Bottle does not use that. `apply_panel` refuses a window that key-value observing has subclassed (`NSKVONotifying_*`). It also refuses a key-capable spec, because the card panel (#78) needs its own class. On a failure the host destroys the window, so a MOG window that is not a panel never stays open.
3. **Build hidden, style, then show without activation.** `WindowKind::build_spec` builds a panel kind hidden. The R11 host calls a `NativeWindowStyle` seam (`style`, then `show`). On macOS `MacPanelStyle::show` calls `orderFrontRegardless`, which does not activate Bottle or make the panel key. The seam runs the AppKit step on the main thread: now if the caller is on it, or else queued. Tests use a recording double, because the Tauri mock runtime has no real NSWindow. `tests/macos_panel_appkit.rs` (`harness = false`, main thread) applies the step to a real NSWindow that is never shown, and reads it back from AppKit.
4. **No activation at launch.** The app sets `tauri::Builder::activate_ignoring_other_apps(false)` on macOS. SP-01 (section 4.2, finding 4) found that the tao default (`true`) takes the frontmost app away at launch. The activation policy stays Regular, because the hub is a normal window with a Dock icon. SP-01 shows that Regular with `false` does not change the frontmost app.
5. **A native drag, with no page permission.** The page cannot call `startDragging`, because MOG-08a gives view windows `view_emit` only. So an app-wide `NSEvent` local monitor handles the left-mouse events of `BottleMogPanel` windows. The pure `DragGesture` (`src-tauri/src/mog_drag.rs`) passes events to the page until the pointer is 4 points or more from the press (UX-27a). Then it starts `performWindowDragWithEvent:` with the press event, and swallows the rest of the gesture, including the mouse-up. So the page never gets a `click` for a drag, and no capture starts (MOG-01c). A smaller move stays a click for the page (MOG-12a, #83). The threshold is in points, the unit that AppKit gives.
6. **Transparency.** The window and WebView are transparent through Tauri (ADR 0002). The panel step also sets `opaque=false`, a clear background and no shadow. The MOG page has `src/views/mog/mog.css` with a transparent `html`, `body` and `#root`. It is a file, because the ARC-4 CSP blocks inline styles.

## Consequences

- A person must check on a Mac that the desktop shows through, that 100 clicks steal no focus, that a drag moves the MOG, and that the panel shows over a fullscreen Space. The script is `tools/mac/mog-check.swift`; the steps are on #11 (`needs:human`, `needs:mac`). The signed-DMG checks stay with SP-01 (`TODO(#10)`).
- The brain cannot open MOG windows yet (`TODO(#285)`). The device check uses the debug demo MOG (`BOTTLE_DEMO_WINDOWS=1`).
- Not in this ADR: host-forwarded hover (UX-08a, an SP-01 follow-up), the card panel (#78), Windows `WS_EX_NOACTIVATE` (#80), click-through (#81), and the poke and click rules (#83). The NFR-14b / COL-36 owner decision of SP-01 (wry always uses the private `drawsBackground` key) is not changed here.
