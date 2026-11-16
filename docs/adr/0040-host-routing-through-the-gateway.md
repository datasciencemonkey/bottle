# ADR 0040: Host routing through the hardened gateway

- Status: Accepted
- Date: 2026-11-16
- Issue: #285, #331, #333 (contract PR A)
- Owner: contract-lead (implemented by impl-285)

## Context

The Tauri host called `mog_core::ipc::handle` directly. So the hardened gateway of ADR 0019 had no production caller (#333). The `window_*` commands and `hotkeys_set` passed the checks and then returned `not_implemented` (#285, #331). The brain-to-view payloads were view-local types (ADR 0004, decision 8). Errors held the received command name and the window label.

## Decisions

1. **One entry.** The invoke handler in `src-tauri/src/ipc.rs` sends every command to `hardened_gateway_with(NoLog, HostComponents)`. No production code calls `ipc::handle` directly. A source test checks this (`sec_03_the_host_dispatches_only_through_hardened_gateway_with`).
2. **The seam does not change.** The host makes one `HostComponents` value for each call. That value holds the label of the calling window, which the app sets. So `GatewayComponents::call` needs no label and no `Handled` value. `hardened_gateway_with(log, components)` keeps its signature, and the C ABI (#176, PR #277) can use it with no change.
3. **Host actions.** `HostComponents` relays `view_emit` to the brain as `view:input`. It keeps the `brain_subscribe` Channel. It runs the four `window_*` ops on R11 and `hotkeys_set` on the R1 settings. Every other command returns `not_implemented`. A missing host part (no R11 host, no settings, no brain) returns `host_failed`.
4. **Content-free errors.** A rejection is `{ "error": "<code>" }` and nothing else. The code is in `ipc_gateway::ERROR_CODES`. A component code outside that list becomes `host_failed`. `IpcError::UnknownCommand` has no field. `IpcError::WindowNotAllowed` keeps only the table command name, never the label. TS `IpcError` is `{ error: IpcErrorCode }`. The schema document lists the codes, and a TS test compares the two lists.
5. **R11 codes.** New codes: `mog_cap_reached`, `unknown_mog`, `window_closing`. `WindowError` maps as follows: `InvalidId` to `invalid_args`, `CapReached` to `mog_cap_reached`, `UnknownMog` to `unknown_mog`, `Closing` to `window_closing`, `HostFailed` and `MainThread` to `host_failed`.
6. **R11 off the main thread.** The four `window_*` commands run on `tauri::async_runtime::spawn_blocking` and resolve later (ADR 0022, decision 6). All other commands run inline, so `view_emit` keeps its order.
7. **Core trait.** `mog_core::window_host::WindowHost` gets `hide_card`. `open_mog_window` takes an optional `Geometry`. `TauriWindowHost` implements the trait. It applies the geometry (logical size, then logical position) before the native style and the show. A geometry with a width or height that is not positive gets `invalid_args`.
8. **hotkeys_set.** The route calls `HotkeySettings::set` through the `HotkeysRoute` trait. `unknown_hotkey_action` becomes `invalid_args`. Every other error (for example a failed save) becomes `host_failed`. The reply is `{ conflicts }`.
9. **Brain Channel scrub.** `BrainChannel::send` scrubs each payload with `token_guard::scrub_json` (SEC-01).
10. **View payloads in the contract.** `MogViewState`, `MogBadgeView`, `CardBlurbView`, `CardUpdate`, `ViewTrace` and `MicLevelFrame` are in `src/shared/ipc.ts`. `view-ipc.ts` keeps the shape checks and re-exports the types. `MogViewState.level` is gone (the view computes the level, ADR 0027). `mog_core::types` has a serde mirror, and shared fixtures check both sides.
11. **The `mog.level` event.** `mog.level` (Tauri `mog:level`, `events::MOG_LEVEL`) is a brain-to-view event with a `MicLevelFrame`. The brain sends it to `mog-<owner>` with `sendMicLevel`. When the mic closes, the brain sends one frame with `owner: null` to the MOG that owned the mic (option (a) of the #12 comment). The MOG window reads it with `viewMicLevels()`. This event is not in the 4.5 list of the requirements. No test reads that list, so `research/` does not change.

## Consequences

- SEC-03 now holds on the Tauri host path, not only in tests. The C ABI gap stays with #176.
- The two concurrent `open_mog_window` calls of the PR #292 review cannot race: the `ops` lock of `TauriWindowHost` makes each open one critical section. Only the call that made the reservation rolls it back.
- The app has no log sink yet, so the gateway log lines go to `NoLog` (ADR 0019, decision 3).

## Open work

- `TODO(#374)`: the brain `view_emit` handler (card Esc, card stack input, badge actions) and the badges in `mog.view_state`.
- `TODO(#4)`: the brain feeds `sendMicLevel` from the CaptureService level stream.
