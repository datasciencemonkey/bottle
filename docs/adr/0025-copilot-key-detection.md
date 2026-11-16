# ADR 0025: Detect the Windows Copilot key from the hook

- Status: Accepted
- Date: 2026-11-16
- Issue: #332 (follows #322 and ADR 0020, decision 3; uses ADR 0010 and ADR 0017)
- Owner: impl-332 (capture-lead)

## Context

CAP-43b says that the first-run conflict check must flag the Copilot key. On a keyboard with a Copilot key, the Copilot key is in place of Right-Ctrl, and the key sends Win+Shift+F23. No Microsoft documentation page gives this chord. Microsoft Q&A answers and the PowerToys Keyboard Manager record it as Left-Win + Left-Shift + F23. This is not verified on a device here: #296, step 17 checks it. ADR 0017 flags Right-Ctrl as the talk key, and the chord Win+Shift+F23, when `KeyboardEnv::copilot_key` is `true`. ADR 0020 reads `copilot_key` through `EnvReads::copilot_key`. The Windows read gives `false` until this issue decides the method. The issue lists three options:

1. Settings (#77) and the wizard (#43) ask the user, and save the answer.
2. The hook (#7) sets a flag when it first sees Win+Shift+F23. The hook keeps no other key identity.
3. A device read (a HID usage or a registry value), if one is documented.

The read must not prompt, and it must not log a key.

## Research for option 3

Microsoft documents no prompt-free read that tells if a keyboard has a Copilot key.

- "Microsoft Copilot hardware key providers" (learn.microsoft.com, `windows/apps/develop/windows-integration/microsoft-copilot-key-provider`) documents two registry values: `HKCU\Software\Microsoft\Windows\Shell\BrandedKey\BrandedKeyChoiceType` and `...\BrandedKey\AppAumid`. They tell which target the user selected for "the Microsoft Copilot hardware key or the Windows key + C". The Windows key + C shortcut is on every keyboard. So these values do not prove that the keyboard has the key.
- "Copilot key state" (same site, `copilot-key-state`) documents the events that a registered provider app gets on a press. It needs a packaged provider app and a press. It does not tell if the key exists.
- No Microsoft page documents a HID usage or a device property for the Copilot key. Answers on Microsoft Q&A are not documentation.

So option 3 is rejected.

## Decision

1. **Option 2: the hook sees the key once.** `mog_core::hotkey_windows::CopilotWatch` runs in `HookCore::handle`, after `HookFilter::on_key`. It fires on the first physical F23 key-down while one Win key and one Shift key are down (either side). Then it stops. An injected event (`LLKHF_INJECTED`) never counts, because `SendInput` does not prove a key on the keyboard. The watch reads the key state of the filter and keeps one bool. Each call is O(1) with no allocation, so the CAP-40a budget holds. The watch changes no verdict: Windows still gets the chord, and Copilot still opens.
2. **One signal with no key.** The watch makes `HookSignal::CopilotKeySeen`. It has no data: no key, no time. A hook thread sends it at most once. The host adapter maps it to `DeferredSignal::CopilotKeySeen`.
3. **One flag.** `mog_core::keyboard_env::CopilotKeyFlag` holds one bool: `present()` and `set(bool)`. `set` returns `true` if the value changed. `MemoryCopilotKeyFlag` is the in-memory flag. `WithCopilotFlag` wraps the OS `EnvReads`. Its Copilot read is the device read or the flag. The other reads go to the inner reads. The Windows device read stays `false`, with a comment that cites this ADR.
4. **Host wiring.** `hotkey_host::install` makes one flag. It gives the flag to the key check reads (`WithCopilotFlag`) and to the Windows `platform_service`. The Windows `platform_service` gives the flag to `HookConsumers`, the seam of the hook adapter. On `CopilotKeySeen`, `HookConsumers` sets the flag. `HookConsumers` replaces `PendingHookConsumers`; the other arms keep their `TODO(#14)`, `TODO(#294)` and `TODO(#297)`.
5. **Where the conflict shows.** `HotkeySettings::set` reads a fresh `KeyboardEnv` on each call (ADR 0020, decision 6). So after the flag is set, `hotkeys_set { bindings: {} }` returns the `copilot_key` conflict of a Right-Ctrl talk key or a Win+Shift+F23 chord. Its fix is the next free ladder step (Ctrl+Win). Settings (#77) and the wizard (#43) show it. The host does not change a saved key by itself. Nothing shows the conflict yet when the flag turns on: `HookConsumers` gets the change from `CopilotKeyFlag::set`, and #346 shows the conflict there: `TODO(#346)`.
6. **Option 1 stays a seam.** `CopilotKeyFlag::set` takes `false` too. Settings (#77) and the wizard (#43) can ask "Does your keyboard have a Copilot key?" and set the answer. The last write wins. The UI owners decide if they ask.
7. **Saved flag.** Appendix F has no setting for the flag, so the flag is in memory until #341 (contract change) adds `app.keyboard.copilotKey` and the settings store (#77) saves it: `TODO(#341)`.
8. **No logs.** No new code logs. The signal and the flag carry no key identity.

## Consequences

- CAP-43b is partial for the Copilot key until #346 lands. The first-run check runs at the first launch, before a press can happen. On that launch, the check does not know about the Copilot key. On a keyboard with an AltGr layout and a Copilot key, the first launch picks Right-Ctrl, which the keyboard does not have. After the first Copilot press, `hotkeys_set` flags the conflict with the Ctrl+Win fix. A wizard that runs `hotkeys_set { bindings: {} }` after its "Hold to test" step (UX-37a) shows the conflict on first run. The wizard owner (#43) can also ask the user (decision 6).
- Until #341 and #77 land, the flag lasts until quit. The next launch knows the key only after a new press.
- A keyboard that remaps the Copilot key in firmware to another chord is not detected. A remap tool that uses `SendInput` is not detected.
- The Win32 glue does not change: `HookCore` owns the watch. `mog-core` passes clippy for `x86_64-pc-windows-msvc`. The full `src-tauri` crate does not build for that target on a macOS host (ADR 0015, amendment 1). Nothing ran on Windows. The device steps are on #296 (needs a Copilot+ PC).
- No Handy code is adapted.
