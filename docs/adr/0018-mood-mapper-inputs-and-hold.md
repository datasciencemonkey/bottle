# ADR 0018: MoodMapper inputs, the agent set_mood slot and the 400 ms hold

- Status: Accepted
- Date: 2026-11-16
- Issue: #9
- Owner: shell-lead (implemented by impl-9)

## Context

ADR 0004 (decision 5) made T11 real with the 4.8 priority order. Issue #9 adds the 4.8 table, the 400 ms hold (UX-05a), owner-only `listening` (UX-07) and the agent `set_mood` rule (UX-41). The 4.7 `MoodMapper` input has only flags (`paused`, `ownsCapture`, `errorUntil`, ...). It has no field for overlays, the agent mood or the hold. The contract files do not change in this issue. The issue left four points open.

## Decisions

1. **Two pure functions next to the 4.7 form.** `createMoodMapper(now)` keeps the 4.7 signature. `mapMood(mogId, sources, now)` takes typed sources: the MOG status (region A, with `since`, `hop` and `localOnly`), the 4.7 `CaptureState` and the request phases (region C, each with the time it started). It derives the 4.7 flags and the overlays, and then uses the same priority code. The error and success windows come from the phase start: error 3 s (Failed, LoginRequired), success 2.5 s (Completed, MemoryAnswer) and 1.2 s (Inserted). `holdMood(prev, target, now)` is a pure reducer for the hold. The brain keeps one `HeldMood` per MOG and calls again at `recheckAt`. The lifecycle and phase types are local until #98 and #39 add their owners' types (`TODO(#98)`, `TODO(#39)`).
2. **The agent mood sits below success and above answering.** UX-41 says the agent mood applies only when no capture, error or success state has priority. Paused is above capture, so paused also blocks it. The agent mood overrides answering, processing, dozing and idle. The agent can never set `listening`, because only the capture owner shows `listening` (UX-07). The agent mood also ends. Each `set_mood` carries its time (`AgentMood.at`). A request phase or a lifecycle state that starts after that time replaces it. An agent `error` lasts 3 s and an agent `success` lasts 2.5 s, the same as the pipeline moods. Any other agent mood ends after 10 s (`AGENT_MOOD_MS`). So one `set_mood` cannot hide the later moods of a MOG. The mood comes from `set_mood` (`TODO(#113)`).
3. **`listening` skips the hold.** UX-05a says each mood shows for 400 ms or more. UX-05b says `listening` shows less than 100 ms after key-down. If a mood started less than 400 ms before key-down, both rules cannot hold. We let `listening` show at once, because key-down is a direct user action and UX-05b has a measured limit. Every other change waits until the hold ends, and `listening` itself holds 400 ms. (Amended by ADR 0031, decision 2: `listening` -> `processing`, the user ending the capture, also shows at once.)
4. **Listening and recording need an open capture of the right mode.** `listening` needs `owner === mogId`, `recording`, not `paused`, and mode `ptt` or `hands_free`. A meeting session shows `idle` (or the request mood) and the `recording` overlay for its owner only. A paused session has no `recording` overlay. The "PAUSED" pill and the dashed ring have no field in the 4.7 overlays. They are MVP-M view work (`TODO(#100)`).

## Notes

- Region C phases with no 4.8 row map to no mood condition: Cancelled, GaveUp, LateFinished and QueuedOffline. GaveUp is not a failure, because its Appendix E card says "I will check again later". Utterance maps to processing.
- A blurb reveal is `answering` for 1.2 s or less (4.8). Partial is `answering` while the request stays in that phase.
- A chip offered with confidence below 0.7 (or no confidence) sets `attention`. A chip at 0.7 or more does not.
- UX-01b is an inspection test, `src/views/mood-write.test.ts`. It flags the mapper names, `set_mood`, a mood in `emitView`, a `setMood` call or a `data-mood` attribute write that does not pass `<state>.mood`, and a literal mood write. The one allowed literal is the UX-02b first frame (`mood=idle`) in `mog/mascot.ts`.
- The brain does not send the mapped mood to `mog:view_state` yet. That wiring needs the producers above and the typed payload of #285.
