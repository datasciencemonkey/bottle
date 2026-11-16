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
3. **Secure field.** macOS: the focused AX element has the subrole `AXSecureTextField`, or a process holds Secure Event Input. Windows: Bottle reads the class name of the focused control into a 256-unit buffer. `Edit`, `RichEdit20W`, `RichEdit50W`, `RICHEDIT60W` and `WindowsForms10.EDIT.*` are edit controls. Two signals mark a password box: `ES_PASSWORD` in `GWL_STYLE`, and a nonzero `EM_GETPASSWORDCHAR` reply (`SendMessageTimeoutW`, 50 ms). An edit control with either signal is `secure_field`. A control of an unknown class with either signal is `secure_field` too, because a superclassed or owner-drawn edit can have any class name. The rule is the pure function `windows_control_focus`, so its tests run on every CI OS. Browser and UWP password fields on Windows need UI Automation (#409).
4. **Fail open on unknown focus.** Only "nothing has focus" is `no_focus` (macOS `kAXErrorNoValue`, Windows no foreground window). Any other AX or Win32 error counts as a field, so an app with no AX tree still gets the paste.
5. **Fixed delays (Handy's clipboard paste), not the receipt paste.** Write, wait 60 ms, chord with a 100 ms modifier hold, wait 250 ms, restore. The restore ends about 0.35 s after the chord, inside the 1 s of MOG-06c. The receipt paste needs the AppKit main run loop and a clipboard owner window. It can also hold the text up to 8 s, which breaks MOG-06c. 
6. **A newer copy wins.** If the clipboard mark (macOS `changeCount`, Windows sequence number) changed after the write, Bottle does not restore.
7. **Chord only.** The only key input is `paste_chord`: Command+V (macOS `kVK_ANSI_V`) or Ctrl+V (Windows `VK_V`). There is no auto-submit and no added text. A test checks that no Enter key is sent. On macOS the events have explicit flags, so a held Shift or Option does not change the chord. Handy's layout-aware V key needs TIS on the main thread, so Bottle uses Handy's fallback key code.
8. **Concealed write.** The text goes on the clipboard only through `ClipboardWriter` (SEC-30, #213). This PR adds the Windows backend `WindowsClipboard` (#371): all formats are set in one open and close. The opt-out formats are set first and the text last. Thus a failed opt-out cannot publish the text without its flags.
9. **Full snapshot.** macOS saves every pasteboard item with every type. Windows saves every memory format; Windows makes `CF_DIB` from a bitmap, so images come back.
10. **No log.** The module logs nothing. A host error crosses only as `host_failed`.
11. **Blocking call.** `insert_text` blocks for about 0.4 s. The host runs it off the main thread.
12. **No restore of a password manager secret.** If the saved clipboard is concealed or transient, Bottle does not restore it. It leaves the clipboard empty after the paste. macOS: an item has the type `org.nspasteboard.ConcealedType` or `org.nspasteboard.TransientType`. Windows: the format `ExcludeClipboardContentFromMonitorProcessing` is present, or `CanIncludeInClipboardHistory` or `CanUploadToCloudClipboard` is 0. Reason: 1Password, Bitwarden and KeePass clear the clipboard after some seconds, but only if the clipboard still holds their write. They compare the `changeCount` or the sequence number. A restore writes the secret again with a new mark, so the auto-clear never fires and the secret stays without a time limit. This rule has priority over MOG-06c: for a secret, "the earlier content" is not put back. The same rule applies when the text write fails.
13. **The restore is not marked transient.** nspasteboard.org suggests `TransientType` for a restore write, and Windows has the history opt-outs. Bottle does not add them. With decision 12, a marked restore would look like a secret at the next dictation, and the user's own copy would then be lost. A clipboard manager can thus record the restored item as a new copy.

## Consequences

- The host route (`HostComponents` in `src-tauri/src/ipc.rs`, ADR 0040) sends `insert_text` to `os_component()`, and `schedule` runs it on a worker (`runs_off_main_thread`). On an OS with no paste host the reply is `host_failed`.
- A very slow app that reads the clipboard more than 0.35 s after the chord pastes the earlier content. The device run (#408) measures this.
- On a non-QWERTY macOS layout where Command+V is not the ANSI V key, the paste can fail. The device run checks Dvorak.
- After a paste over a password manager secret, the clipboard is empty, not restored (decision 12).
- A copy that the user makes between the mark check and the restore (`clearContents` or `EmptyClipboard`) is lost. The window is some microseconds, and Bottle accepts this race.
- On Windows the chord does not release modifiers that the user holds. A held Shift or Alt makes the chord Ctrl+Shift+V or Ctrl+Alt+V. This is never Enter, but the target can run a different command. macOS sets explicit flags and has no such effect.
- The snapshot reads every type or format, so the source app renders each delayed or promised format. A large Excel range or image can add latency before the paste (NFR-01d). The device run (#408) measures this.
