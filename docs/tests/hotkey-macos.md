# macOS talk key: device check (#6, AT-30, SP-03 #5)

Unit tests cover the logic of R1 HotkeyService with fakes. These steps check the real OS on a Mac. A person runs them. Time: about 25 minutes. The probe prints only PTT phases, pill changes and counts. It never prints key contents, and it records no audio.

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
| 11 | With the SP-03 HID rig (`BOTTLE_TEST_HID_PORT`), run 1,000 Right-Option hold-and-release cycles. | 1,000 `ptt up` lines. | CAP-35b, AT-30 |

Step 11 needs the Pi Pico HID rig of SP-03. Without the rig, report steps 1-10 only.

## Report

Post on #6 (or #5): the macOS version, the Mac model, the step numbers that passed, and the probe output of steps 3, 5 and 6. Do not post anything else from the terminal.
