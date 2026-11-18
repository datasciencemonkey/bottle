# ADR 0053: The utterance route and the Shift-forced ask route

- Status: Accepted
- Date: 2026-11-16
- Issue: #59 (RTR-01b, RTR-01c, RTR-17a, RTR-17b; AT-43, AT-06). Depends on #57 and #58 (ADR 0048).
- Owner: agent-lead-2 (implemented by mog-b2; review fixes by impl-59)

## Context

After the router step (#57), the brain must end a `dictation` at `insert_text` with no main agent and no Genie (RTR-01c). Shift held with the talk key must force the ask route, skip only the intent gate, and keep the 1.5 s preview (RTR-17a, RTR-17b). The issue does not say where Bottle reads Shift, how the brain learns it, or what "skips the intent gate only" means for each router intent.

## Decisions

1. **One route function.** `src/brain/utterance-route.ts` has `routeUtterance(utterance, deps)`. It calls the router step, then returns one route: `inserted`, `insert_failed` (with the reason and the text for Copy), `copy` (insertion off), `preview` (1.5 s, `forced` true or false), `chip` (with an optional warning), or `other`. It calls only `model_invoke` (through the router step) and, for a dictation, `insert_text`. It never calls `agent_*` or `mcp_call`.
2. **Dictation.** The brain inserts `cleaned_text`, or the STT text when the cleaned text is empty. With `app.dictation.insert` off, the route is `copy` and `insert_text` is not called. A rejected `insert_text` call is `insert_failed` with `host_failed`.
3. **Questions.** Confidence 0.7 or more gives `preview`; less gives `chip` (RTR-04a, D30). The question is `genie_question`, else `cleaned_text`, else the STT text.
4. **Forced ask.** With Shift, the router step still runs, so the cleaned text and the hints stay. The route is `preview` with `forced: true`, for every intent and every confidence. It never inserts. The preview still guards the send (B1), so the 1.5 s preview shows (RTR-17b). "Skips the intent gate only" (RTR-17a) means that the forced route also skips the D30 confidence gate: the user asked on purpose, and the preview still lets the user cancel. A router failure falls back to `context` (ADR 0032), so a forced ask then previews the STT text.
5. **Guards before any preview.** These rules apply first, also to a forced ask:
   - An ask with no text (all of `genie_question`, `cleaned_text` and the STT text are blank) gives `other`, not an empty preview.
   - An ask that the router marks `injection_suspected` gives `chip` with the warning `injection_suspected`, never a preview (RTR-14: no send, a chip with a warning).
   - A `session` utterance never gets a preview, because a preview sends by itself after 1.5 s (RTR-07, B1). A question or a forced ask in a session gives `chip`. Any other session intent gives `other`, so a session is never inserted into the focused app.
6. **Where Shift is read.** The host reads the OS Shift state when the forwarder thread handles a talk-key `down` (`PttForwarder::with_shift_probe`).
   - macOS: the HID key state of Left-Shift (`kVK_Shift`, 56) and Right-Shift (`kVK_RightShift`, 60), through `CGEventSourceKeyState`, the same read that CAP-45 uses for the talk key. A source-state flags read may not carry the device-dependent bits, so Bottle does not use them. Right-Shift counts unless it is the talk key.
   - Windows: `GetAsyncKeyState` for Left-Shift and Right-Shift. No Windows talk key is a Shift key.
   - The probe reads the talk key now (`TalkNow`), so a new talk key from `hotkeys_set` (CAP-43a) reaches it.
   - **The fallback chord.** Under Secure Event Input the talk key is Ctrl+Shift+Space (CAP-39b). Shift is part of that chord, so the forwarder does not probe Shift in chord mode: from `TalkKeyPaused` or `FallbackChordUnavailable` until `TalkKeyRestored`. A chord PTT is never a forced ask.
   - A Shift pressed after the talk key goes down is an "other key" and still cancels the PTT (CAP-44c). So the user presses Shift first.
   - The read happens on the forwarder thread, not in the OS callback. The queue hand-off is short, but a Shift released at almost the same moment as the talk-key press can be missed. That press then gives a normal route, and the user can repeat it. Bottle accepts this: the R1 output has no field for the modifier state, and a change to `HotkeyOutput` touches every R1 backend.
7. **Payload.** The `hotkey:ptt` down item gets `shift: true` only when Shift is held. All other items keep `{ mogId, phase }`. `asPttEvent` keeps `shift` only on `down` and only when it is `true`. Section 4.5 lists the optional field (`hotkey.ptt {mogId, phase, shift?}`); old readers ignore it.
8. **The lifecycle hook.** `src/brain/utterance-wiring.ts` has `createUtteranceHandler`, the `onUtterance` hook of the #14 lifecycle in `startBrain`. It calls `begin(mogId)` on the request owner (#73, MOG-13a) with the MOG of the key-down, and routes the utterance with the lifecycle mode as the capture mode. A route that is not an ask releases the owner. A preview or a chip goes to `onAsk` with the live owner, and the callee releases it. A MOG that is no longer open, a missing router input, or a route that throws routes nothing and inserts nothing.

## Consequences

- `TODO(#112, #163)`: the router input needs the home profile of the MOG and the profile list. Bottle never picks a profile, so until then `startBrain` routes no utterance.
- `TODO(#142)`: T3 CapturePolicy runs the preview timer, the chip and the agent hand-off, and it calls `stampRequest` and `release` on the request owner. Until then `onAsk` only releases the owner, so nothing is sent.
- `TODO(#60)`: the dictation card with Copy and the "Inserted" pill, and the read of `app.dictation.insert`. Until then a dictation in `startBrain` goes to Copy.
- The macOS Shift read on a device is an AT-43 person step (`docs/tests/AT-DICTATION-STT-OFFLINE.md`).
