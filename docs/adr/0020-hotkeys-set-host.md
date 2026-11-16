# ADR 0020: Host side of hotkeys_set, the keyboard reads and the talk-key restart

- Status: Accepted
- Date: 2026-11-16
- Issue: #322 (follows #24 and ADR 0017; amends ADR 0015)
- Owner: impl-322 (capture-lead)

## Context

ADR 0017 put the pure key logic in `crates/mog-core/src/hotkey_bindings.rs`: the D66 defaults, the ladder, `first_run_check` and `hotkeys_set`. The host must do four things. It must read the `KeyboardEnv` with prompt-free OS reads. It must save the keys to the `app.hotkeys.*` settings (Appendix F). It must give the talk key to the platform HotkeyService. It must answer `hotkeys_set {bindings} -> {conflicts}` (section 4.5). Main has no settings store, and `mog_core::ipc::handle` does not route `hotkeys_set` to the host. The issue leaves the read methods, the first-run behavior and the save rule open.

## Decision

1. **Reads behind one trait.** `mog_core::keyboard_env::EnvReads` has one method for each OS read: Dictation, VoiceOver, AltGr layout, Copilot key and a process-name visitor. `read_keyboard_env(os, reads)` calls only the reads of `os`. It keeps only `VoiceApp` matches, once each, in enum order. A process name lives only for one visitor call. Tests use doubles and never call the OS.
2. **macOS reads.** All reads are `CFPreferencesCopyAppValue` (read-only) and `proc_listallpids` / `proc_name`. None needs Accessibility or Input Monitoring, and none prompts.
   - Dictation is on if `com.apple.HIToolbox AppleDictationAutoEnable` or `com.apple.assistant.support "Dictation Enabled"` is 1.
   - The Globe key starts Dictation if `com.apple.HIToolbox AppleFnUsageType` is 3.
   - A double-press shortcut is entry 164 of `com.apple.symbolichotkeys AppleSymbolicHotKeys`, enabled, with parameters `65535, 65535, <mask>`. The mask `0x40000` is Control twice. `0x100000` is either Command twice. `0x100008` is Left Command twice. `0x100010` is Right Command twice. Any other value gives no shortcut. A person checks these values on a Mac (#303, steps K1-K4).
   - VoiceOver is on if `com.apple.universalaccess voiceOverOnOffKey` is 1.
3. **Windows reads.** The AltGr read calls `VkKeyScanExW` on the active input layout for 10 probe characters (`@ € { } [ ] \ | ~ #`). The active layout is the one of the foreground window's thread (`GetWindowThreadProcessId(GetForegroundWindow())`). If any character needs Ctrl and Alt together, the layout has AltGr. The process names come from a Toolhelp32 snapshot. Windows has no documented prompt-free read for the Copilot key, so the read gives `false` until #332 decides the method.
4. **Settings seam.** `HotkeySettingsStore` loads and saves the `app.hotkeys.*` values, keyed by the setting key. The values are the binding strings of `Binding::format`. A missing or bad value gives the D66 default. The app uses `MemorySettingsStore` until #77 adds the settings store: `TODO(#77)`.
5. **First run.** If the store has no values, `HotkeySettings::load` reads the `KeyboardEnv` and runs `first_run_check`. It saves the suggested keys: the D66 defaults with the first free ladder step. So an AltGr layout starts with Right-Ctrl, not Right-Alt. The host keeps the report. `first_run_conflicts()` gives its `{conflicts}`, each with a fix, for the first-run screen (#43). If the save fails, the keys apply until quit, and the next launch runs the check again.
6. **`hotkeys_set`.** `HotkeySettings::set(bindings)` reads a fresh `KeyboardEnv` on each call, because apps start and stop. It returns `outcome.result_json(os)` of `hotkey_bindings::hotkeys_set` with no change. An empty `bindings` object changes nothing and returns the conflicts of the current keys. So the first-run screen and Settings can read the conflicts with the same command. An unknown action is the error `unknown_hotkey_action`, and nothing is saved.
7. **Save rule (MB-14).** The host checks the final bindings, not the conflict list, because a conflict of its own hides a duplicate. A changed action that shares its binding with another action keeps its old binding. In a pair, the later action in Appendix F order is "the second binding" of MB-14, and it is not saved. A revert can make a new pair, so the check repeats until no changed action shares a binding. The other changes are saved. When an action is not saved, the reply describes the saved keys: the input errors, one `duplicate` item for each action that was not saved (its fix is the key it keeps), then the conflicts of the saved keys. With no revert, the reply is exactly the result of `hotkey_bindings::hotkeys_set`. ADR 0017, decision 7 still holds for the pure logic. A save failure returns `settings_save_failed` and changes nothing.
8. **Talk-key restart (amends ADR 0015, decisions 6 and 7).** The platform factory takes a `TalkBinding`, so Windows can get Ctrl+Win. On macOS, Ctrl+Win gives the D66 default. When `app.hotkeys.ptt` changes, `TalkKeySink::apply` calls `HotkeyRuntime::set_talk_key`. It makes a new service with the same sink, and `HotkeyHost::replace_service` stops the old service. The new service starts with no prompt (CAP-36), unless the host stopped on quit. The permission API stays the same. If the old service does not stop (Windows `stop_post_failed`), the host keeps it, does not start the new one, and returns `Unavailable` with the stop code, so two hooks never run. When the Windows hook adapter ends (quit or restart), it resets its engine, so a held PTT ends with `cancel` (CAP-45). The settings lock spans the restart, so two calls cannot apply their talk keys out of order. The other keys (panic, session, open card, fallback) are saved, but their consumers belong to their own issues.
9. **IPC route.** `mog_core::ipc::handle` answers `not_implemented` for `hotkeys_set`. The route to `HotkeySettings::set` is contract change #331, done in ADR 0040. The result shape `{ conflicts: unknown[] }` does not change.
10. **No logs.** The new files log nothing. A static test checks for print and log macros, and for the prompt APIs in `keyboard_env.rs`.

## Consequences

- Until #331 lands, no window can call `hotkeys_set`, and the saved talk key is the first-run pick.
- Until #77 lands, a changed key lasts until quit.
- The first-run screen (#43) shows `first_run_conflicts()` or the result of `hotkeys_set { bindings: {} }`.
- The real reads need a person: macOS steps K1-K4 in `docs/tests/hotkey-macos.md` (#303), and Windows steps on #296.
- The full `src-tauri` crate does not build for `x86_64-pc-windows-msvc` on a macOS host (ADR 0015, amendment 1). `mog-core` with the Windows reads passes clippy for that target.
- No Handy code is adapted.
