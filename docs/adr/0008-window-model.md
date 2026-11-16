# ADR 0008: The window model (R11 WindowHost and brain-to-view routing)

- Status: Accepted
- Date: 2026-11-15
- Issue: #8
- Owner: shell-lead (implemented by impl-8)

## Context

The skeleton (#221, #223, #226) has the four window kinds of ARC-3, the per-label capabilities of ARC-4 and the brain Channel of ADR 0003. Issue #8 adds the run-time window lifecycle, the brain-to-view routing and a build-time bundle check. The issue left five points open.

## Decisions

1. **MOG-08a scope.** MOG-08a says view capabilities hold only `view_*` commands. Section 4.5 gives the hub (V1) the profile, config, policy and hotkey commands. We apply MOG-08a strictly to the `mog-*` and `card-*` capabilities. The hub capability holds `view_emit` and the 4.5 rows that name the hub, and nothing else. The tests are `mog_08a_view_caps_only_view_commands` and `mog_08a_hub_holds_only_view_commands_and_the_4_5_hub_rows`.
2. **R11 lives in the app crate.** `src-tauri/src/window_host.rs` has a pure `WindowLedger` and a `TauriWindowHost`. The ledger enforces the cap of 8 active MOGs, label-safe IDs and one card per open MOG. Closing a MOG closes its card. The ledger slot is freed only after the windows are destroyed. A window that closes outside the host (crash, OS close) frees its slot through the `WindowEvent::Destroyed` handler in `lib.rs`. The host does not hold the ledger lock while it builds a window. The host is managed app state. The debug demo windows go through it too.
3. **Host errors are local.** `WindowError` has content-free codes (`invalid_id`, `mog_cap_reached`, `unknown_mog`, `host_failed`). `mog_core::window_host::WindowHost` returns `CoreError`, which has no variant for these codes. So `TauriWindowHost` does not implement that trait yet. Contract issue #285 routes the `window_*` commands to the host and maps the codes.
4. **Brain-to-view events go to named labels.** The brain sends `mog:view_state` only with `sendMogViewState(mogId, state)`, to `mog-<id>` and to `hub`. The hub (V1) lists every MOG with its mood, so it gets a copy. The brain sends `card:update` only with `sendCardUpdate(mogId, update)`, to `card-<id>`. Both refuse an ID that is not label-safe. A view listens with its own label as the event target, so it does not get the events of another window (Tauri `AnyLabel` match). This is routing hygiene, not a trust boundary. A view with `core:event:allow-listen` can still listen with no target and get every `emitTo` (ADR 0003, decision 3). So these events carry only what views render.
5. **The brain checks view input again.** `onViewInput(bus, handler)` parses `view:input`. It drops input with an unknown kind, and input from a MOG or card window for another MOG. The core does the same check first (ADR 0003, decision 2).
6. **The bundle check runs the real build.** `tests/bundle-graph.test.ts` runs the Vite build in memory (`write: false`) in `pnpm test`. It reads the module graph of each entry and its shared chunks. Each T1-T15 module of the registry must be in the `brain` graph. No `src/brain` module may be in the `hub`, `mog` or `card` graph, and no `src/views` module may be in the `brain` graph.

## Notes

- The payload types `MogViewState` and `CardUpdate` stay in `src/views/view-ipc.ts` until #285 moves them to `src/shared/ipc.ts`. The brain helpers take `unknown` until then.
- The hidden-card reaper and the key-capable card panel stay with #78 (done: ADR 0022). The non-activating MOG panel stays with #11 and #80.
- A person must still check on a Mac that the brain window stays hidden and that the demo windows open (`needs:human` on #8).
