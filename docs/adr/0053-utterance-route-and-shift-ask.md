# ADR 0053: The utterance route and the Shift-forced ask route

- Status: Accepted
- Date: 2026-11-16
- Issue: #59 (RTR-01b, RTR-01c, RTR-17a, RTR-17b; AT-43, AT-06). Depends on #57 and #58 (ADR 0048).
- Owner: agent-lead (implemented by mog-b2)

## Context

After the router step (#57), the brain must end a `dictation` at `insert_text` with no main agent and no Genie (RTR-01c). Shift held with the talk key must force the ask route, skip only the intent gate, and keep the 1.5 s preview (RTR-17a, RTR-17b). The issue does not say where Bottle reads Shift, how the brain learns it, or what "skips the intent gate only" means for each router intent.

## Decisions

1. **One route function.** `src/brain/utterance-route.ts` has `routeUtterance(utterance, deps)`. It calls the router step, then returns one route: `inserted`, `insert_failed` (with the reason and the text for Copy), `copy` (insertion off), `preview` (1.5 s, `forced` true or false), `chip`, or `other`. It calls only `model_invoke` (through the router step) and, for a dictation, `insert_text`. It never calls `agent_*` or `mcp_call`.
2. **Dictation.** The brain inserts `cleaned_text`, or the STT text when the cleaned text is empty. With `app.dictation.insert` off, the route is `copy` and `insert_text` is not called. A rejected `insert_text` call is `insert_failed` with `host_failed`.
3. **Questions.** Confidence 0.7 or more gives `preview`; less gives `chip` (RTR-04a, D30). The question is `genie_question`, else `cleaned_text`, else the STT text.
4. **Forced ask.** With Shift, the router step still runs, so the cleaned text and the hints stay. The route is always `preview` with `forced: true`, for every intent and every confidence. It never inserts. The preview still guards the send (B1), so the 1.5 s preview shows (RTR-17b).
5. **Where Shift is read.** The host reads the OS Shift state when it forwards a talk-key `down` (`PttForwarder::with_shift_probe`). macOS: `CGEventSourceFlagsState` device bits; Left-Shift counts, and Right-Shift counts unless it is the talk key. Windows: `GetAsyncKeyState` for Left-Shift and Right-Shift (no Windows talk key is a Shift key). A Shift pressed after the talk key goes down is an "other key" and still cancels the PTT (CAP-44c).
6. **Payload.** The `hotkey:ptt` down item gets `shift: true` only when Shift is held. All other items keep `{ mogId, phase }`. `asPttEvent` keeps `shift` only on `down` and only when it is `true`. Section 4.5 lists `{mogId, phase}`; the optional field is an addition, and old readers ignore it.

## Consequences

- The capture lifecycle (#14) must call `routeUtterance` at key-up with `forceAsk` from the `down` item: `TODO(#14)`.
- The preview timer, the agent hand-off and the chip belong to T3 CapturePolicy (#142). The dictation card with Copy and the "Inserted" pill belong to #60.
