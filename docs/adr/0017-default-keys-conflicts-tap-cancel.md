# ADR 0017: Default global keys, the first-run conflict check, and the tap and cancel rules

- Status: Accepted
- Date: 2026-11-16
- Issue: #24 (depends on #6 and #7; host hook-up #322)
- Owner: impl-24 (capture-lead)
- Amends: ADR 0011, decision 1 (macOS events of interest)

## Context

The default global keys must follow D66, and no default may use Ctrl+Option (CAP-43a). The first-run check must flag Dictation, Fn/Globe, VoiceOver, AltGr layouts, the Copilot key and known voice apps, and each conflict must show with a fix (CAP-43b). A talk-key hold shorter than 250 ms must count as a tap (CAP-44a). If another key goes down in the first 1 s of a hold, R1 must cancel the PTT (CAP-44c). `hotkeys_set {bindings}` returns `{conflicts}` (section 4.5). The issue leaves the conflict rules, the fix shape, the tap event and the macOS other-key source open.

## Decision

1. **Pure module `hotkey_bindings.rs`.** It holds the D66 defaults, the talk-key ladder, the conflict check, `first_run_check` and the logic of `hotkeys_set`. It has no OS calls. The host reads a `KeyboardEnv` with prompt-free OS reads and passes it in (#322).
2. **Defaults.** macOS: Right-Option, panic Cmd+Shift+Esc, session Ctrl+Shift+M, open card Ctrl+Shift+O. Windows: Right-Alt, panic Ctrl+Win+Alt+K, session Ctrl+Win+Alt+R, open card Ctrl+Win+Alt+O. Both: fallback Ctrl+Shift+Space. The keys are the Appendix F settings `app.hotkeys.*`. Per-MOG chords (CAP-46a) are v1 and are not in this map.
3. **"No Ctrl+Option".** On macOS no default chord holds Ctrl and Option. On Windows D66 itself uses Ctrl+Win+Alt chords. The Win key stops the AltGr meaning, so the Windows rule is "no Ctrl+Alt chord without Win".
4. **Ladder.** macOS: Right-Option, then Right-Control. Windows: Right-Alt, then Right-Ctrl, then Ctrl+Win. The first-run check takes the first step with no environment conflict: an AltGr layout skips Right-Alt, and a Copilot key (in place of Right-Ctrl) skips Right-Ctrl.
5. **Conflict rules.** Dictation: the macOS Dictation shortcut presses the talk key twice (Control twice with Right-Control, Command twice with Right-Command), or a Windows binding is Win+H (voice typing). Fn/Globe: any binding with Fn or Globe. VoiceOver: any macOS chord with Ctrl+Option (even with VoiceOver off), or Right-Control as the talk key with VoiceOver on (a Control press stops speech). AltGr: Right-Alt as the talk key, or a Ctrl+Alt chord without Win, on an AltGr layout. Copilot key: Right-Ctrl as the talk key, or Win+Shift+F23, on a keyboard with a Copilot key. Voice app: one notice per running app of a fixed list (Superwhisper, Wispr Flow, MacWhisper, Handy, Talon, Dragon, Voice Access). Also duplicates and a short list of OS-reserved chords.
6. **Every conflict has one fix.** `use_binding` (the next free ladder step, or the D66 default of the action), `open_system_settings` (when no ladder step is free), or `change_other_app`. Each conflict also has a fixed English message. The JSON item is `{kind, action, binding, fix, message, app?}`. The contract type stays `conflicts: unknown[]`, so this is not a contract change.
7. **`hotkeys_set`.** An unknown action key is an error (`unknown_hotkey_action`). A string that Bottle cannot read, or that does not fit the action (the talk key is modifier-only, the other keys are chords), keeps the old binding and adds an `invalid` conflict. Valid bindings apply even when they have conflicts; the caller shows the conflicts.
8. **Tap event.** `HotkeyEngine` times each hold on an injected `MonotonicClock`. A key-up less than 250 ms after its key-down gives `Ptt(Cancel)` and then `HotkeyOutput::Tap`. Section 4.5 has no `tap` phase. `cancel` tells the brain to drop the audio, and nothing calls STT (CAP-44b stays with CaptureService). The double-tap rule (CAP-14a) and the hands-free stop (CAP-14b) can read `Tap` later. The rule applies to the talk key and to the Secure-Input chord. A gap re-read that ends a PTT is not a tap.
9. **Cancel on another key.** `HotkeyEngine::on_other_key_down()` takes no key identity. In the first 1 s of a talk-key hold it ends the PTT with `cancel`; the later key-up gives nothing. Windows: the hook already reports `HookSignal::OtherKeyDown` (ADR 0010); `forward_to_engine` sends it to the engine, so the host adapter of #314 no longer defers it.
10. **macOS other-key source (amends ADR 0011 decision 1).** CAP-44c needs non-modifier key-downs, and a `FlagsChanged` tap cannot see them. The talk-key tap stays `FlagsChanged` only. A second listen-only session tap asks for `KeyDown` only, never `KeyUp`. It is created off. R1 turns it on only while `other_key_window_open()` is true (the first 1 s of a talk-key hold; never for a Secure-Input chord hold, because taps get no `KeyDown` then), and the watchdog tick turns it off at 1 s. Only the tap thread and the watchdog thread turn it on or off, with the engine lock held, and the tap thread clears the port under that lock before it releases the port. Its callback uses only the event type: it never reads the key code, the characters or any other field. The pure classifier `other_key_from_event_type(u32)` cannot see the event and returns a field-less marker, and a test checks it. Another modifier that goes down during a hold comes from the `FlagsChanged` tap, by its flag bit. Caps Lock counts on each event, because its flag shows the lock state, not the key state. No new permission is needed (Input Monitoring covers listen-only taps). If the OS refuses the second tap, PTT still works and only the cancel for non-modifier keys is lost.

## Consequences

- PR #317 (`hotkey_host.rs`) matches `HotkeyOutput` with no wildcard, so #24 adds a `Tap` arm that does nothing.
- The SP-03 HID rig holds the key for 50-150 ms. At R1 these holds are taps: AT-30 must count PTT ends (`up` or `cancel`), not only `up`. `docs/tests/hotkey-macos.md` step 11 says so.
- Real-device checks (the tap, the other-key cancel, the first-run reads) need a person: #303 (macOS) and #296 (Windows).
- The host must read the environment and call the logic: #322.
- Handy `src-tauri/src/shortcut/` was read. It has no conflict check of this kind and its defaults differ from D66. No Handy code is adapted.
