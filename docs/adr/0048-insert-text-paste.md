# ADR 0048: insert_text pastes through the clipboard with a fixed restore delay

- Status: Accepted
- Date: 2026-11-16
- Issue: #58 (MOG-06a, MOG-06b, MOG-06c, AT-43; builds on #213 / PR #267)
- Owner: shell-lead (implemented by mog-b2)

## Context

UC1 must paste the cleaned dictation text into the focused app with no Enter, and put the earlier clipboard back (MOG-06a..c). The `insert_text` reply is `inserted`, `no_permission`, `no_focus` or `secure_field` (Section 4.5). The issue does not say how Bottle sends the paste, when it restores the clipboard, or how it finds a secure field. Handy (MIT) has two paths: a fixed-delay clipboard paste, and a "receipt" paste that waits for the target to read a lazy clipboard promise.

## Decisions

1. **Module.** `mog_core::insertion` holds the sequence (`insert_text`), the `InsertHost` trait, and `InsertTextComponent` (a `GatewayComponents` for `insert_text`). `MacInsertHost` and `WindowsInsertHost` are the OS hosts. `os_component()` gives the component for this build.
2. **Checks first.** The host checks the input permission, then the focus. A refusal returns its code and does not touch the clipboard or the keyboard. macOS: `AXIsProcessTrusted`; Windows: Handy's integrity check (an elevated foreground window drops input).
3. **Secure field.** macOS: the focused AX element has the subrole `AXSecureTextField`, or a process holds Secure Event Input. Windows: an `Edit` control with `ES_PASSWORD`. Browser and UWP password fields on Windows need UI Automation (#409).
4. **Fail open on unknown focus.** Only "nothing has focus" is `no_focus` (macOS `kAXErrorNoValue`, Windows no foreground window). Any other AX or Win32 error counts as a field, so an app with no AX tree still gets the paste.
5. **Fixed delays (Handy's clipboard paste), not the receipt paste.** Write, wait 60 ms, chord with a 100 ms modifier hold, wait 250 ms, restore. The restore ends about 0.35 s after the chord, inside the 1 s of MOG-06c. The receipt paste needs the AppKit main run loop and a clipboard owner window. It can also hold the text up to 8 s, which breaks MOG-06c. 
6. **A newer copy wins.** If the clipboard mark (macOS `changeCount`, Windows sequence number) changed after the write, Bottle does not restore.
7. **Chord only.** The only key input is `paste_chord`: Command+V (macOS `kVK_ANSI_V`) or Ctrl+V (Windows `VK_V`). There is no auto-submit and no added text. A test checks that no Enter key is sent. On macOS the events have explicit flags, so a held Shift or Option does not change the chord. Handy's layout-aware V key needs TIS on the main thread, so Bottle uses Handy's fallback key code.
8. **Concealed write.** The text goes on the clipboard only through `ClipboardWriter` (SEC-30, #213). This PR adds the Windows backend `WindowsClipboard` (#371): all formats are set in one open and close.
9. **Full snapshot.** macOS saves every pasteboard item with every type. Windows saves every memory format; Windows makes `CF_DIB` from a bitmap, so images come back.
10. **No log.** The module logs nothing. A host error crosses only as `host_failed`.
11. **Blocking call.** `insert_text` blocks for about 0.4 s. The host must run it off the main thread.

## Consequences

- The host route (`HostComponents` in `src-tauri/src/ipc.rs`, ADR 0040) must send `insert_text` to `os_component()` on a worker thread. `src-tauri/src/ipc.rs` is a contract file, and ADR 0040 is still open in PR #376, so that one-line route is a follow-up.
- A very slow app that reads the clipboard more than 0.35 s after the chord pastes the earlier content. The device run (#408) measures this.
- On a non-QWERTY macOS layout where Command+V is not the ANSI V key, the paste can fail. The device run checks Dvorak.
