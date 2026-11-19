# ADR 0088: T4 SttProvider chain, hedge, SttPolicy and language routing

- Status: accepted
- Issue: #52 (CAP-47a, CAP-47b, CAP-59a, CAP-59b, CAP-60a, CAP-60b, CAP-64a, CAP-64b, AT-04,
  AT-21, AT-40). Soft dependencies: #46 (R4, PR #520), #49 (ASR client), #51 (probe and
  opt-in), #514 (host route of `stt_transcribe`).
- Follow-ups: #530 (Whisper config key), #531 (R4 language ID and 60 s Retry audio), #550
  (`net.status` to a brain `NetworkStatus`). The Sessions chain is #116.
- Date: 2026-11-18
- Owner: impl-52 (capture-lead-3)

## Context

T4 SttProvider runs in the TS brain. It calls `stt_transcribe` with `{segmentId, provider,
profile?, model?, language?, tags}` and gets a `Transcript`. It must follow the 8.3 chain, hedge a
slow cloud call with a local decode at 1.2 s, remove every cloud step under
`SttPolicy=local_only`, and route a language that Parakeet does not cover. The requirements
leave these points open:

- what "Databricks ASR selected" and "policy permits Databricks ASR" mean in code;
- where the admin Whisper endpoint comes from (Appendix C has no key);
- what a "valid result" is for the hedge;
- where T4 learns the language of an utterance;
- how "Could not transcribe. Retry" and the CAP-64b text reach the MOG.

## Decisions

1. **One config read per utterance** (`SttConfigSource`). The production source
   (`ipcSttConfig`) reads `policy_get` through `modelPolicyFrom` (ADR 0074). A failed read, a
   thrown source, or an unknown `SttPolicy` value is `local_only`. MDM `AsrModelService`
   replaces the default ASR model (CAP-62a).
2. **ASR is permitted only with the opt-in.** Databricks ASR is in the chain only when
   `SttPolicy` is not `local_only` and the opt-in source returns a user-named profile. #51 owns
   the opt-in: the probe pass (CAP-57a, CAP-57b), the user's choice or MDM `cloud_only`
   (CAP-57e), live demotion (CAP-58) and the geography check (CAP-63). Until #51 lands, the source
   returns nothing, so a new install sends no audio off the device (CAP-57c). "Policy permits
   Databricks ASR" in CAP-64a means the same thing. Bottle does not send audio to the cloud only
   because the language is not covered: that is still an opt-in (B5, COL-97).
3. **Whisper only with an admin config** (CAP-47b). The Whisper source returns nothing until
   #530 names the Appendix C key. Whisper is a cloud step, so `local_only` and offline remove it.
4. **Chains** (8.3, CAP-60a). T4 builds the two PTT chains. The 8.3 Sessions chain (local Parakeet →
   Databricks ASR if allowed → queue) belongs to the meeting STT path, #116 (CAP-61), and is not
   in this ADR. Default: `local_parakeet → dbx_whisper (if configured) → failure`.
   With the opt-in: `dbx_gemini (hedged) → local_parakeet → dbx_whisper (if configured) →
   failure`. A rejection, a reply for another segment or provider, or an empty cloud text is a
   failed step. An empty local text is a valid "no speech" result. Local Parakeet runs at most once
   in a chain. Offline (an injected `NetworkStatus`, ARC-F7) removes the cloud steps; the brain passes it once #550 wires the `net.status` source.
5. **Hedge** (CAP-59a, CAP-59b). The ASR call starts at once. If it has not replied in 1.2 s, the
   local decode of the same segment starts. A cloud failure before 1.2 s starts the local decode
   at once. The first valid result wins; a late reply changes nothing. An empty local result ("no speech") wins only when the cloud call fails, so it never hides cloud text. The cost: with the opt-in, a "no speech" clip waits for the
   cloud reply, up to the 8 s backstop. #49 must set a tighter ASR request timeout. The host cannot cancel a
   call, so the loser's result is dropped. Each step has a backstop deadline (cloud 8 s, local
   15 s): a step with no reply by then fails and the chain goes on, so a hung call never stops the
   chain. The ASR client sets its own request timeout (TODO(#49)).
6. **Languages** (CAP-64a, CAP-64b, D59). Parakeet TDT 0.6B v3 covers 25 languages: bg, hr, cs,
   da, nl, en, et, fi, fr, de, el, hu, it, lv, lt, mt, pl, pt, ro, sk, sl, es, sv, ru, uk. T4
   learns the language from a caller hint (`TranscribeOptions.language`) or from
   `Transcript.lang` of the local decode. A local result in an uncovered language is not valid;
   its language goes back to the chain. An uncovered language skips local Parakeet and goes to
   ASR (if permitted) or Whisper (if configured). With neither, T4 rejects with
   `language_not_available`. A permitted provider that fails gives `could_not_transcribe`. R4 sets
   no `lang` today, so production routing by language waits for #531.
7. **Failures** (`SttFailure`). T4 rejects only with `SttFailure`: a fixed code
   (`could_not_transcribe`, `language_not_available`), the fixed user text and the segment ID.
   It holds no transcript text, no audio, no profile and no error detail. `retry()` runs the
   chain again for 60 s after `could_not_transcribe` (at most 32 segments wait). A second Retry while one runs gets the same result and starts no second chain. R4 drops the
   audio at once today, so a real Retry decode waits for #531.
8. **Brain wiring** (`src/brain/stt-wiring.ts`). The capture lifecycle STT step shows the failure
   text as one blurb on the owner MOG's card and routes no text. So no empty transcript ends
   without a message. The Retry control is #60.
9. **Badge source** (UX-38). `sttUse()` gives the provider and model of the last transcript, or
   the selected provider before one, for each MOG. #374 sends it in the badges.

## Consequences

- T4 calls `stt_transcribe` for every final segment. Until #514 adds the host route, the host
  refuses the call, and the MOG shows "Could not transcribe. Retry".
- Once #514 lands, T4 produces routed dictation text. #498 (the Copy card) must be on main first.
- No contract file changed. The Whisper key (#530) and the R4 facts (#531) are follow-ups.
