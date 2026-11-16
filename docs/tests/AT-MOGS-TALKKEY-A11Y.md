# AT-01, AT-03, AT-20, AT-30: MOGs, talk key and accessibility (#230)

These acceptance tests run the scripts of section 15 against the shipped code, with the #229
harness (`SimClock`, `mog_core::testkit`, `src/test-support/fixtures.ts`). The parts that need a
feature that is not built are pending tests. The parts that need a device or a person skip with the
reason, or are steps in this file. A pending or skipped test is never a pass. The gate is the
evidence file (`docs/tests/EVIDENCE-TEMPLATE.md`, `scripts/check_gate_evidence.py`).

## Run

```
cargo test -p mog-core --test at_30_global_keys --test at_03_talk_key
cargo test -p bottle --test at_01_mogs
pnpm vitest run src/acceptance src/test-support/a11y.test.ts
```

## Files

| AT | File | Runs now on shipped code |
|---|---|---|
| AT-01 | `src-tauri/tests/at_01_mogs.rs` | Two MOG windows with their own labels, each a transparent never-key Status panel (MOG-01a, MOG-01b, MOG-02). The card is the only key window (MOG-03a, UX-19a). A drag of each MOG never reaches the page as a click (MOG-01c, UX-27a, UX-27b). The 9th MOG window is refused. A duplicate binding is a conflict (MB-14, core part). |
| AT-01 | `src/acceptance/at-01-mogs.test.tsx` | Nothing yet: the config tests are pending. A self-test checks the secret pattern that they use. |
| AT-03 | `crates/mog-core/tests/at_03_talk_key.rs` | Hold and release; a hold under 250 ms is a tap and its PTT is cancelled (CAP-44a); another key in the first 1 s cancels the PTT (CAP-44c); the Windows hook swallows Esc during a hold and hands-free. |
| AT-20 | `src/acceptance/at-20-a11y.test.tsx` | The mood mapper gives only the 7 moods. One Esc in the card does one thing, by keyboard (UX-19b). Every chip has a text label, and hover sends nothing (UX-20b, UX-20c). The REC, offline and attention overlays show text. DOM events drive hover and click. |
| AT-30 | `crates/mog-core/tests/at_30_global_keys.rs` | Listen-only taps (CAP-35a). 1,000 simulated hold cycles give 1,000 key-ups on macOS and Windows (CAP-35b). Secure Input ends a PTT in one watchdog tick, 100 ms (CAP-39a), the Carbon chord drives PTT (CAP-39b), the pill text (CAP-39c). Tap watchdog (CAP-38a, CAP-38b). Key-state re-read (CAP-45). Input Monitoring only, after a user action (CAP-36); Accessibility only for insertion (CAP-37). Hook callback body p99 under 1 ms (CAP-40a); reinstall every 10 min and on resume in an 8 h soak (CAP-40b); swallow (CAP-41); elevated window (CAP-42). D66 defaults with no Ctrl+Option (CAP-43a). AltGr, Copilot (seen by the hook, #332), Dictation, VoiceOver, Fn and voice-app conflicts, each with a fix (CAP-43b). |
| AT-20 | `src/views/mog/rive-mascot.test.ts` (#12) | A poke never changes the mood (UX-02a). The first frame is idle (UX-02b). The Rive listeners are off (UX-08b). This PR does not copy these tests. |
| AT-30 | `src/acceptance/at-30-wizard.test.tsx` | Nothing yet: wizard step 7 is pending. |
| AT-30 | `crates/mog-core/tests/latency_bench.rs` (#171) | The HID numbers NFR-08a and NFR-08b: `at_30_nfr_08a_nfr_08b_hid_rig_1000_cycles`. This PR does not copy it. |

The simulated 1,000 cycles prove the R1 logic only. NFR-08a and NFR-08b need the HID rig.

## Pending tests

Rust pending tests are `#[ignore = "pending TODO(#n) ..."]` and end in `todo!` with the assertion
that is still to write. TS pending tests are `it.skip` with `TODO(#n)` in the title. They compile.
When the feature issue lands, its implementer removes the mark and finishes the test.

| AT | Requirement IDs | Feature issue |
|---|---|---|
| AT-01 | MB-01, MB-03a, MB-03b (the 9th-start text) | #26 |
| AT-01 | MB-07 | #43 |
| AT-01 | MB-14 (the hub does not save the duplicate) | #77 |
| AT-01 | MOG-15a, MOG-15b, MOG-21 | #82 |
| AT-01 | MOG-16a, MOG-16b, MOG-17 | #81 |
| AT-01 | MOG-04 | #80 |
| AT-01 | MOG-03b (20 card opens, device only) | device step AT-01, step 3 |
| AT-03 | CAP-04a, CAP-04b | #4 |
| AT-03 | CAP-13a, CAP-13b | #21 |
| AT-03 | CAP-14a, CAP-14b, CAP-14c, CAP-44b | #14 |
| AT-03 | CAP-17a, CAP-17b, CAP-22 | #22 |
| AT-03 | MOG-09, MOG-12a, MOG-12b | #83, #98 |
| AT-20 | UX-08a (poke, click and drag rules) | #83 |
| AT-20 | UX-30, UX-31a, UX-31b, UX-32a, UX-32b | #96 |
| AT-20 | UX-20a | #89 |
| AT-20 | UX-23a, UX-23b (MVP-M) | #101 |
| AT-20 | UX-28 | #73 |
| AT-20 | UX-33, UX-34a, UX-34b, UX-34c, UX-35 | #97 |
| AT-30 | MB-05 | #66 |
| AT-30 | UX-37a, UX-37b | #43 |
| AT-30 | NFR-08a, NFR-08b from the shipped HotkeyService (not the SP-03 probe) | #323 |

Not in this PR: MB-09 (v1, #103) and MOG-20a, MOG-20b (v1 Linux, #106).

## Device and person steps

A person runs these steps. Each part takes less than 30 minutes. They need no Databricks profile.
Never put a token, a transcript or a profile secret in a log or an evidence file. Write each result
to `docs/evidence/<AT>-<part>.md` in the format of `docs/tests/EVIDENCE-TEMPLATE.md`, with
`Scope | part: <IDs> only`. Use the device IDs of `docs/tests/README.md`.

### AT-30 part A: HID rig, 1,000 cycles (macOS, about 20 min)

Do Part A of `docs/tests/LATENCY-BENCH.md`. It gives NFR-08a and NFR-08b. The result is `partial`
until the HotkeyService log exists (TODO(#323)).

### AT-30 part B: Secure Keyboard Entry during a hold (macOS, about 10 min)

1. Build the SP-03 probe as in LATENCY-BENCH Part A (`cd spikes/sp-03 && cargo build --release`).
2. Start the probe with Secure Input on (no `--no-sei`):
   `./target/release/sp03-probe listen --seconds 180 --no-clamshell > ~/at30-sei.log`
3. Open Terminal. Do this 3 times: hold Right-Option, select Terminal > Secure Keyboard Entry with the
   mouse (this turns it on), release Right-Option, press and release Ctrl+Shift+Space, then select
   Terminal > Secure Keyboard Entry again to turn it off. Each round gives one PTT end by Secure
   Input and one chord press.
4. Run the device test in your Bottle checkout:
   ```
   BOTTLE_TEST_DEVICE=mac-m1-current BOTTLE_BENCH_SP03_SEI_LOG=~/at30-sei.log \
     cargo test -p mog-core --test at_30_global_keys at_30_cap_39a_cap_39b_device -- --nocapture
   ```
   Expected: `ok`, and no `skipped` line. The test reads `upper_bound_sei_to_end_ms` (each value
   must be 250 ms or less) and the Carbon `EV` lines (each press needs its release). The verdict is
   `WithinProbeOnly`: the log comes from the SP-03 probe, not the shipped HotkeyService, so record
   the CAP-39a and CAP-39b part as `partial` (TODO(#323)), the same rule as Part A.
5. Do steps 5 to 7 of `docs/tests/hotkey-macos.md` with the shipped probe
   (`cargo run -p mog-core --example hotkey_probe`). Expected: `ptt cancel` and the pill in 250 ms or
   less, then the chord PTT, then `TalkKeyRestored`.

### AT-30 part C: AltGr layout and Copilot key (Windows 11, about 15 min)

1. Add the German (Germany) keyboard in Settings > Time & language > Language & region, and make it
   the active layout.
2. Run:
   ```
   $env:BOTTLE_TEST_DEVICE="win11-i5-16gb"; $env:BOTTLE_TEST_LAYOUT="altgr"
   cargo test -p mog-core --test at_30_global_keys at_30_cap_43b_device -- --nocapture
   ```
   Expected: `ok`. With a US layout, use `plain`.
3. Run `cargo run -p mog-core --example hotkey_probe_windows` with the German layout. Hold Right-Alt
   in Notepad. Expected: no `@` or other character is typed, and no menu bar activates in 100
   presses (CAP-41).
4. The Copilot key is not a layout read: the hook sees its chord Win+Shift+F23 (#332, ADR 0025).
   On a Copilot+ PC, do step 17 of #296. Showing the conflict in the app waits for #346.

### AT-30 part D: panic without Input Monitoring (blocked)

The panic key is not built: TODO(#66). When #66 lands, remove Input Monitoring for Bottle, start a
PTT and a hands-free capture on two MOGs, and press the panic key 100 times. Expected: every capture
stops in 200 ms or less (p95).

### AT-01: two MOGs (macOS, about 25 min; partial until #26, #81, #82)

1. Run `BOTTLE_DEMO_WINDOWS=1 pnpm tauri dev`. The demo MOG opens.
2. Give your terminal Accessibility (System Settings > Privacy & Security). Run
   `swift tools/mac/mog-check.swift` (#11). Expected: every check line passes and the script prints
   `done`: 0 focus steals in 100 clicks, the panel level and the drag. Quit Bottle.
3. Quit Safari and every other app that uses WebKit. Run
   `BOTTLE_DEMO_WINDOWS=1 BOTTLE_DEMO_CARD_CYCLE=1 pnpm tauri dev`, and at once run
   `swift tools/mac/card-check.swift` (#78). Do not touch the mouse or keyboard until it prints
   `done` (about 7 min). Expected: every check passes; the frontmost app does not change in 20 card
   opens (MOG-03b).
4. Two MOGs with their own chords, profiles and colours, the restart and the display removal need
   #26 and #82. Record them as `not run` with the reason.

### AT-03: talk key and permissions (macOS, about 15 min; partial until #4, #14, #21, #22)

1. Do steps 3, 15, 16 and 17 of `docs/tests/hotkey-macos.md`. Expected: hold and release give
   `ptt down` and `ptt up`; a tap gives `ptt cancel` and `Tap`; another key in the first 1 s gives
   `ptt cancel`.
2. Esc, focus loss, the 120 s stop, the denied mic and click-to-listen need R2 and the click rules
   (#4, #14, #21, #22, #83). Record them as `not run` with the reason.

### AT-20: keyboard only, screen reader, reduced motion, greyscale (blocked)

The accessible names, live regions, reduced motion and greyscale glyphs are not built (#96, #97). When
they land, do the whole run with VoiceOver (macOS) and Narrator (Windows), with no mouse, with
Reduce motion on, and with the Color Filters greyscale on. Expected: every critical flow ends, every
state has text, and Stop shows and works by keyboard.
