# ADR 0040: Host routing through the hardened gateway

- Status: Accepted
- Date: 2026-11-16
- Issue: #285, #331, #333 (contract PR A)
- Owner: contract-lead (implemented by impl-285)

## Context

The Tauri host called `mog_core::ipc::handle` directly. So the hardened gateway of ADR 0019 had no production caller (#333). The `window_*` commands and `hotkeys_set` passed the checks and then returned `not_implemented` (#285, #331). The brain-to-view payloads were view-local types (ADR 0004, decision 8). Errors held the received command name and the window label. The task named this ADR 0032. That number was taken by the router step ADR, so this ADR is 0040.

## Decisions

1. **One entry.** The invoke handler in `src-tauri/src/ipc.rs` sends every command to `hardened_gateway_with(NoLog, HostComponents)`. No production code calls `ipc::handle` directly. A source test checks this (`sec_03_the_host_dispatches_only_through_hardened_gateway_with`).
2. **The seam does not change.** The host makes one `HostComponents` value for each call. That value holds the label of the calling window, which the app sets. So `GatewayComponents::call` needs no label and no `Handled` value. `hardened_gateway_with(log, components)` keeps its signature, and the C ABI (#176, PR #277) can use it with no change.
3. **Host actions.** `HostComponents` relays `view_emit` to the brain as `view:input`. It keeps the `brain_subscribe` Channel. It runs the four `window_*` ops on R11 and `hotkeys_set` on the R1 settings. Every other command returns `not_implemented`. A missing host part (no R11 host, no settings, no brain) returns `host_failed`.
4. **Content-free errors.** A rejection is `{ "error": "<code>" }` and nothing else. The code is in `ipc_gateway::ERROR_CODES`. A component code outside that list becomes `host_failed`. `IpcError::UnknownCommand` has no field. `IpcError::WindowNotAllowed` keeps only the table command name, never the label. TS `IpcError` is `{ error: IpcErrorCode }`. The schema document lists the codes, and a TS test compares the two lists.
5. **New codes.** New R11 codes: `mog_cap_reached`, `unknown_mog`, `window_closing`. The code `config_storage_failed` (`IpcError::ConfigStorageFailed`) was added here for #379. #379 then added the other 12 config codes (ADR 0040 update below). `WindowError` maps as follows: `InvalidId` to `invalid_args`, `CapReached` to `mog_cap_reached`, `UnknownMog` to `unknown_mog`, `Closing` to `window_closing`, `HostFailed` and `MainThread` to `host_failed`.
6. **R11 off the main thread.** The four `window_*` commands run on `tauri::async_runtime::spawn_blocking` and resolve later (ADR 0022, decision 6). All other commands run inline, so `view_emit` keeps its order. `ipc::schedule` makes this choice, and a test checks which thread runs each command. The window ops have no order between them: each op is its own worker task, and the host `ops` lock is not first-in, first-out. So the brain must await each window op before it sends the next one. For example, it awaits `window_mog_open` before `window_card_show`.
7. **Core trait.** `mog_core::window_host::WindowHost` gets `hide_card`. `open_mog_window` takes an optional `Geometry`. `TauriWindowHost` implements the trait. It applies the geometry (logical size, then logical position) before the native style and the show. A geometry with a width or height that is not positive gets `invalid_args`.
8. **hotkeys_set.** The route calls `HotkeySettings::set` through the `HotkeysRoute` trait. `unknown_hotkey_action` becomes `invalid_args`. Every other error (for example a failed save) becomes `host_failed`. The reply is `{ conflicts }`. `hotkeys_set` runs inline on the IPC thread (the main thread). A new talk key restarts the platform service. On macOS that restart joins the watchdog thread, so the main thread can wait for up to one watchdog tick (about 100 ms). It cannot deadlock, because the restart does not wait for the main thread. A move to the worker would change when the new key starts against the next inline command, so it stays inline.
9. **Brain Channel scrub.** `BrainChannel::send` scrubs each payload with `token_guard::scrub_json` (SEC-01).
10. **View payloads in the contract.** `MogViewState`, `MogBadgeView`, `CardBlurbView`, `CardUpdate`, `ViewTrace`, `MicLevelFrame` and the expanded card `CardSection` (with `CardUpdate.overflowChips` and `sections`, from #89) and the Pages views `DomainPickerView` and `PagesChecklistView` (`MogViewState.homeDomain`, `CardUpdate.pageDomain` and `pagesChecklist`, from #205), all added on rebase, are in `src/shared/ipc.ts`. The Rust mirror of `CardSection` is an enum tagged by `id`. `view-ipc.ts` keeps the shape checks and re-exports the types. `MogViewState.level` is gone (the view computes the level, ADR 0027). `mog_core::types` has a serde mirror, and shared fixtures check both sides.
11. **The `mog.level` event.** `mog.level` (Tauri `mog:level`, `events::MOG_LEVEL`) is a brain-to-view event with a `MicLevelFrame`. The brain sends it to `mog-<owner>` with `sendMicLevel`. When the mic closes, the brain sends one frame with `owner: null` to the MOG that owned the mic (option (a) of the #12 comment). The MOG window reads it with `viewMicLevels()`. This event is not in the 4.5 list of the requirements. No test reads that list, so `research/` does not change.

## Consequences

- SEC-03 now holds on the Tauri host path, not only in tests. The C ABI gap stays with #176.
- The two concurrent `open_mog_window` calls of the PR #292 review cannot race: the `ops` lock of `TauriWindowHost` makes each open one critical section. Only the call that made the reservation rolls it back.
- The app has no log sink yet, so the gateway log lines go to `NoLog` (ADR 0019, decision 3).

## Deviations

- **`config_*` was not routed here.** The #333 comments ask the routing to pass `MogConfigComponent` (PR #372) for `config_*`, and to answer `config_storage_failed` when no store is managed. PR #372 was still open, so this PR could not wire it. This PR added the code `config_storage_failed`.
- **Update (#379):** the host now routes `config_*` to `MogConfigComponent` through `HostComponents::call`, after the window and schema checks. It reads the store with `try_state`. With no managed store, all four commands answer `config_storage_failed`. The 13 config codes are in `ERROR_CODES`: the 9 store codes and the 4 G-03 backend codes of #28 (`genie_beta_mcp_rejected`, `genie_space_mcp_rejected`, `genie_one_sse_rejected`, `mcp_url_not_canonical`). A guard test lists every `ConfigError`.
- **Update (#479):** the 16 R8 `mcp_call` failure codes are in `ERROR_CODES` and `IPC_ERROR_CODES`: `genie_one_off`, `auth_expired`, `rate_limited`, `non_json_reply`, `timeout`, `offline`, `host_refused`, `redirect_refused`, `reply_too_large`, `bad_reply`, `protocol_unsupported`, `tool_not_allowed`, `invalid_tool_args`, `invalid_meta`, `profile_invalid` and `poll_inflight` (ARC-F1, G-07b). Each is an `IpcError` variant. `rejection` does not change: an R8 `CoreError::Unavailable { reason }` with one of these codes crosses as itself. The other R8 and R6 codes (for example `transport_failed`, `token_unusable`, `login_required`) still cross as `host_failed`. The brain GL-5 mapper (ADR 0072) maps each code.

## Open work

- `TODO(#374)`: the brain `view_emit` handler (card Esc, card stack input, badge actions) and the badges in `mog.view_state`.
- `TODO(#4)`: the brain feeds `sendMicLevel` from the CaptureService level stream.
