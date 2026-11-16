# ADR 0004: Brain component stubs and thin views

- Status: Accepted
- Date: 2026-11-16
- Issue: #226
- Owner: foundation-lead (implemented by impl-226)

## Context

Section 4.3 lists the hidden TS brain components T1-T15 and the views V1-V3. ARC-2 puts all domain logic in the `brain` window. Section 4.7 gives interfaces for six components only. Section 4.5 names the events `mog.view_state` and `card.update`, but it does not give their payloads. The issue left these points open.

## Decisions

1. **One module per component.** Each component has one kebab-case module in `src/brain/components/` (for example `mog-manager.ts` for T1). The doc comment of each module copies its 4.3 row. A test checks this.
2. **4.7 interfaces first.** T7, T8, T10, T14 and T15 implement the 4.7 interface from `src/shared/interfaces.ts`. The other components export a small interface in their module, with 1-3 methods from the 4.3 row. The feature issue can change that local interface.
3. **Stubs fail loudly.** Each stub method throws `NotImplementedError` (`src/brain/errors.ts`). The error names the component, the method and the feature issue. The same issue is in a `TODO(#n)` comment. ESLint allows unused parameters that start with `_`, so the stubs keep the parameter names.
4. **One registry.** `BRAIN_COMPONENTS` in `src/brain/components/index.ts` lists T1-T15 with an ID, a name and a factory. `startBrain()` constructs each component once. `src/brain/main.ts` calls it. T9 queues its polls on the one brain actor (`getBrainActor()`), with the key `poll:<conversationId>`.
5. **T11 is real.** `createMoodMapper(now)` returns the 4.7 `MoodMapper`. The priority is the 4.8 order: paused > capture > error > success > answering > processing > dozing > idle. A paused or dozing MOG is `sleeping`. The error and success windows end at `errorUntil` and `successUntil`. Hold times, the overlay rules and the agent `set_mood` tool stay with #9.
6. **Views are thin.** `src/views/view-ipc.ts` is the only IPC module of the views. It subscribes to the events by their `EVENTS` name and sends input only with `call('view_emit', ...)`. The MOG and card views read the MOG ID from the window label (`mog-<id>`, `card-<id>`). V2 keeps the one-instance `MascotRenderer` seam (ADR 0002). A static test checks that no view imports `src/brain` and that views call only `view_emit`.
7. **View payloads are view-local for now.** `MogViewState` and `CardUpdate` are in `src/views/view-ipc.ts`, not in the contract files. The views drop a payload that does not have the expected shape. When the brain starts to send these events (#8, #85), the foundation lead moves the payload types into `src/shared/ipc.ts`.

## Notes

- The test tags `[T1]`..`[T15]` and `[V1]`..`[V3]` have no hyphen, so `scripts/req_coverage.py` does not count them. The tests also carry the hyphenated IDs `ARC-2`, `MOG-08a`, `MOG-08b` and `UX-01a`, which the script counts.
