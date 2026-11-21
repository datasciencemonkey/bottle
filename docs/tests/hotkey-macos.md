# macOS talk key: device check (#6, AT-30, SP-03 #5)

Unit tests cover the logic of R1 HotkeyService with fakes. These steps check the real OS on a Mac. A person runs them. Time: about 30 minutes. The probe prints only PTT phases, pill changes and counts. It never prints key contents, and it records no audio.

## Before you start

- macOS 14 or later. Use a Mac that has never given Input Monitoring to the terminal app, if you can.
- Build: `cargo build -p mog-core --example hotkey_probe`.
- The probe runs in your terminal app, so macOS asks for the permission for that app.

## Steps

| # | Do this | Expected result | Requirement |
|---|---|---|---|
| 1 | Run `cargo run -p mog-core --example hotkey_probe`. | Only the Input Monitoring prompt shows. No Accessibility prompt shows. The probe prints `Input Monitoring: ...` and exits. | CAP-36, CAP-37 |
| 2 | Grant Input Monitoring to the terminal app in System Settings > Privacy & Security. Run the probe again. | `Listening.` shows. No prompt shows. | CAP-36 |
| 3 | Hold Right-Option for 1 s, then release it. Do this 20 times. | 20 `ptt down` lines and 20 `ptt up` lines. The counts are equal. | CAP-35b |
| 4 | Hold Left-Option, then Right-Option, then release Right-Option first. | `ptt down`, then `ptt up` when Right-Option comes up. | CAP-35b |
| 5 | In Terminal, select Terminal > Secure Keyboard Entry while you hold Right-Option. | `ptt cancel` and `TalkKeyPaused Talk key paused: another app protects typing. Use ⌃⇧Space.` in 250 ms or less (compare the ms values). | CAP-39a, CAP-39c |
| 6 | With Secure Keyboard Entry on, hold Ctrl+Shift+Space for 1 s, then release it. | `ptt down`, then `ptt up`. Right-Option does nothing. | CAP-39b |
| 7 | Turn Secure Keyboard Entry off. | `TalkKeyRestored`. Right-Option works again. | CAP-45 |
| 8 | Hold Right-Option. Close the lid (or Apple menu > Sleep). Release the key. Wake the Mac and unlock it. | `ptt up` shows after wake. Right-Option works again. | CAP-38b, CAP-45 |
| 9 | Hold Right-Option. Press Ctrl+Cmd+Q to lock. Release the key. Unlock. | `ptt up` after unlock. Right-Option works again. | CAP-45 |
| 10 | In a second terminal, run `kill -STOP $(pgrep -n hotkey_probe)`. Press Right-Option 5 times. Wait 5 s. Run `kill -CONT $(pgrep -n hotkey_probe)`. Then hold and release Right-Option. | The macOS tap timeout turns the tap off; the watchdog turns it on again. The last hold gives `ptt down` and `ptt up`. No PTT stays active. | CAP-38a |
| 11 | With the SP-03 HID rig (`BOTTLE_TEST_HID_PORT`), run 1,000 Right-Option hold-and-release cycles. | 1,000 PTT ends: a hold of 250 ms or more gives `ptt up`; a shorter hold gives `ptt cancel` and `Tap` (CAP-44a). The SP-03 rig holds for 50-150 ms, so expect 1,000 `ptt cancel` lines and 1,000 `Tap` lines. | CAP-35b, CAP-44a, AT-30 |
| 12 | Hold Right-Option for 10 s while you run step 10 (`kill -STOP`, wait, `kill -CONT`) a second time. | One `ptt down` before and one `ptt up` only when you release the key. No early `ptt up`. | CAP-45 |
| 13 | Turn Secure Keyboard Entry on. Hold Ctrl+Shift+Space for 5 s, then release it. | Exactly one `ptt down` and one `ptt up`. | CAP-39b, CAP-45 |
| 14 | Optional: start an app that owns Ctrl+Shift+Space as a global key (for example a launcher set to it). Turn Secure Keyboard Entry on and look at the probe. | With such an app: `FallbackChordUnavailable`. Without one: no such line. | CAP-39b |
| 15 | Tap Right-Option quickly (about 0.1 s). Do this 10 times. | Each tap: `ptt down`, `ptt cancel`, then `Tap`. No `ptt up`. | CAP-44a, AT-03 |
| 16 | Hold Right-Option. In the first 0.5 s, press another key (for example `e`, or Left-Shift). Release both. | `ptt down`, then `ptt cancel` when the other key goes down. No `ptt up` on release. The probe prints no key name. | CAP-44c, AT-03 |
| 17 | Hold Right-Option for 2 s, then press `e`, then release both. | `ptt down`, then `ptt up` on release. No `ptt cancel`. | CAP-44c |
| 18 | Click into TextEdit. Hold Right-Option for 2 s, tap G, then release Right-Option. | `ptt down`, then `PttAskUp` on release. TextEdit gets no "g" and no "©". | RTR-17a |
| 19 | Hold Right-Option and tap G in the first 0.2 s, then release after 1 s. | `ptt down`, then `PttAskUp`. No `ptt cancel`. | RTR-17a, CAP-44c |
| 20 | After step 18, press Option+G in TextEdit with no hold. | TextEdit gets "©": the hot key is released after the hold. | RTR-17a |

Step 11 needs the Pi Pico HID rig of SP-03. Without the rig, report steps 1-10 only.

## In the app (#305)

These steps check the host wiring of `src-tauri/src/hotkey_host.rs`. Use a debug build, so the brain WebView can be inspected. Time: about 10 minutes.

- Remove Bottle (and your terminal app, if it runs the build) from System Settings > Privacy & Security > Input Monitoring before step H1.
- Run `BOTTLE_DEMO_WINDOWS=1 pnpm tauri dev`. The demo MOG window opens.

| # | Do this | Expected result | Requirement |
|---|---|---|---|
| H1 | Start the app with no Input Monitoring. Wait 10 s. | No Input Monitoring prompt and no Accessibility prompt show. | CAP-36, CAP-37 |
| H2 | Grant Input Monitoring to the app that runs the build. Quit and start the app again. | No prompt shows. | CAP-36 |
| H3 | In Safari > Develop, open the Web Inspector of the `brain` WebView. In `src/brain/bus.ts`, add a logpoint in `BrainBus.dispatch` that logs `msg`. Hold and release Right-Option 5 times. The demo MOG is the target because it opened last. | 5 `{event: "hotkey:ptt", payload: {mogId: "demo", phase: "down"}}` items and 5 with `phase: "up"`. | CAP-35a, MB-06a |
| H4 | Close the demo MOG window. Hold and release Right-Option. | No `hotkey:ptt` item: no MOG is the target. | MB-06a |
| H5 | Quit the app (Cmd+Q). Run `pgrep -fl bottle`. | No Bottle process is left. | AT-30 |
| H6 | Keep the Call recording knob off. Click into TextEdit. Hold Right-Option for 2 s, tap R, then release. Then press Option+R with no hold. | The hold gives a plain `phase: "up"` (dictation). TextEdit gets the R as usual: "®" for Option+R with no hold. | ADR 0103 |
| H7 | Turn the Call recording knob on. Click into TextEdit. Hold Right-Option for 2 s, tap R, then release. | At the R press: a `phase: "cancel"` item, then one `{event: "hotkey:record_toggle", payload: {mogId: "demo"}}` item. Nothing on release. TextEdit gets no "r" and no "®". Nothing is pasted. A recording starts. | ADR 0103 |
| H8 | While the recording of H7 runs, hold Right-Option for 2 s and release. | The MOG shows "Recording in progress". No dictation and no paste. | ADR 0103 |
| H9 | While the recording runs, hold Right-Option, tap R, then release. Then press Option+R in TextEdit with no hold. | The recording stops. TextEdit gets "®" for the press with no hold. | ADR 0103 |
| H10 | Keep the Call recording knob on, with no recording running (after H9). Click into TextEdit. Hold Right-Option for 2 s, tap R, tap R again, then tap G. Then hold R for 2 s (past the key-repeat delay). Release R, then Right-Option. Then press Option+R with no hold, and Left-Option+R. | One `hotkey:record_toggle` item only, at the first R. A recording starts. TextEdit gets no "r", no "®", no "g" and no "©" while Right-Option is down. After the release, TextEdit gets "®" for Option+R and for Left-Option+R. Then stop the recording (H9). | ADR 0103, #604 |

Report H1-H10 on #303 with the macOS version. Do not post key contents or other terminal output.

## First-run key reads (#322)

These steps check the prompt-free reads of `crates/mog-core/src/keyboard_env.rs` (ADR 0020). Time: about 10 minutes. Use Terminal. Post only the numbers that the commands print.

| # | Do this | Expected result | Requirement |
|---|---|---|---|
| K1 | In System Settings > Keyboard > Dictation, turn Dictation on. Set the shortcut to "Press Control Key Twice". Run `defaults read com.apple.HIToolbox AppleDictationAutoEnable` and `defaults read com.apple.symbolichotkeys AppleSymbolicHotKeys \| sed -n '/164 =/,/};/p'`. | `1`. Entry 164 has `enabled = 1` and the parameters `65535, 65535, 262144`. | CAP-43b |
| K2 | Set the shortcut to "Press Right Command Key Twice", then "Press Left Command Key Twice", then "Press Either Command Key Twice". Run the entry 164 command after each one. | The third parameter is `1048592`, then `1048584`, then `1048576`. | CAP-43b |
| K3 | Set the shortcut to "Press 🌐". Run `defaults read com.apple.HIToolbox AppleFnUsageType`. | `3`. | CAP-43b |
| K4 | Turn VoiceOver on (Cmd+F5). Run `defaults read com.apple.universalaccess voiceOverOnOffKey`. Turn VoiceOver off. | `1` while VoiceOver is on. | CAP-43b |
| K5 | Quit Bottle. Start it again with `pnpm tauri dev`. Wait 10 s. | No prompt shows. The hub window opens. | CAP-36, CAP-43b |

If a value differs, post the value and the macOS version on #303. The decoder in `decode_dictation` must change to match.

## Report

Post on #6 (or #5): the macOS version, the Mac model, the step numbers that passed, and the probe output of steps 3, 5 and 6. Do not post anything else from the terminal.
