# ADR 0098: ⌥G during the talk-key hold asks Genie; a plain hold is dictation

- Status: accepted (user decision, 2026-11-19)
- Issue: #577
- Date: 2026-11-19
- Owner: agent-lead-5 (implemented by impl-optg)
- Supersedes: the Shift trigger of ADR 0053 (decisions 4 to 7) and its 1.5 s preview for a forced ask
- Refers to: ADR 0096 (dictation first, Ask Genie press), ADR 0097 (direct Genie One send), ADR 0017 (other-key tap), ADR 0011 (listen-only tap)

## Context

The user decided the MVP talk-key flow on 2026-11-19:

- Hold Right-Option, speak, release: dictation. The text pastes at the cursor.
- Hold Right-Option, tap G at any time in the hold, release: Ask Genie. Bottle does not paste and
  does not show the 1.5 s preview. The exact text goes to Genie One through the send seam of ADR 0097.
- The frontmost app must never get a "g" or a "©" from that G.
- The question classifier and the chip-on-question gate leave the MVP flow.
- The pop-up keeps Ask Genie only as a fallback for text that the user already dictated. Its hint is
  "Hold ⌥G to ask Genie".

Before this ADR, Shift held at the key-down forced the ask route (ADR 0053), and the forced ask
kept the 1.5 s preview (RTR-17b). The macOS tap is listen-only (CAP-35a, ADR 0011), so it cannot
stop a key from reaching the app.

## Decisions

1. **Carbon hot key for the G.** R1 grabs the ask key only during a talk-key hold. The ask key is
   the talk-key modifier + `kVK_ANSI_G` (Option+G for the default Right-Option). It uses
   `RegisterEventHotKey` on the main thread, with the same pattern as the fallback chord. The OS
   gives a registered hot key to Bottle only, so the app gets no "g" or "©". The tap stays
   listen-only. Each Carbon handler takes only its own hot-key ID.
2. **Grab only in the hold.** The engine (`hotkey_core.rs`, `AskKeyControl`) grabs the key when a
   talk-key hold starts. It releases the key on each path that ends the hold. These paths are the
   key-up, a tap and a CAP-44c cancel (Esc or any other key in the first 1 s). They are also
   Secure Event Input, a tap disable or a lock with the key up (CAP-45), a service stop and an
   engine drop. A hold that passes 120 s (the R2 hold cap, CAP-04a) also releases the key. Grab and release always pair. Outside a
   hold, Option+G types "©" as usual. A chord hold under Secure Event Input never grabs it.
3. **The flag rides the key-up.** A press of the ask key marks the hold. The key-up then gives
   `HotkeyOutput::PttAskUp`, and the host sends `hotkey:ptt` `{ mogId, phase: 'up', ask: true }`.
   A tap, a cancel and a double-tap never carry `ask`. Bottle removes the `shift` field.
4. **CAP-44c exception for G.** The other-key tap of ADR 0017 compares the key code with
   `kVK_ANSI_G` only. G marks the ask and does not cancel. Every other key cancels as before. The
   tap never stores or logs the code. This is the same rule as the talk-key check.
5. **Race.** A G at the start of the hold can come before the main thread registers the hot key.
   The other-key tap also reports G, so the hold still becomes an ask, and nothing cancels it. Both
   paths set the same flag once. In that short race the app can still get the "©". A listen-only
   tap cannot stop it.
6. **Brain.** A key-up with `ask: true` gives the route `ask` with the exact text. This is the
   text that dictation pastes. The T3 ask gate sends it at once through `send` (ADR 0097). The
   G-47 sanitizer and the G-30a cap still run. With no home profile, the picker opens (ADR 0096).
   The router step still runs for RTR-14. An injection-suspected text gets the warning chip.
7. **No classifier gate.** With no `ask`, every PTT and hands-free utterance is dictation. The
   pop-up shows the dictated text, Ask Genie for that exact text, and the hint. No route makes a
   preview now, so this change deletes the 1.5 s preview code. A meeting session keeps its chips
   (RTR-07).
8. **Windows gap.** Windows does not have the ask key in this build. The Windows hook sends no
   `ask`, so every Windows hold is dictation. The fallback Ask Genie button works on Windows.

## Consequences

- RTR-17a, RTR-17b and CAP-44c change in `research/final-requirements.md`. Section 4.5 shows the
  new `hotkey.ptt` payload.
- B1 holds: ⌥G and the Ask Genie press are user actions. Nothing goes to Genie without one.
- While the talk key is held, the talk-key modifier + G does nothing in the app.
- A person checks the hot key on a Mac (AT-43 step 4 in `docs/tests/AT-DICTATION-STT-OFFLINE.md`).
