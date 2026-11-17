# ADR 0039: hotkey.status, permission_request and capture_resume

- Status: Accepted
- Date: 2026-11-16
- Issue: #297, #294, #312, #279 (contract PR B)
- Owner: contract-lead (implemented by impl-297)

## Context

R1 detects three talk-key states that the brain cannot see: Secure Event Input (CAP-39c), an elevated foreground window (CAP-42) and a failed hook. Section 4.5 has no core-to-brain event for them (#297, #294). The host has `request_ptt_permission` and `request_insertion_permission`, but no command reaches them (#312). R2 has `resume(lease_id)`, but 4.5 has no `capture_resume` row (#279).

The task named this ADR 0042. PR #361 holds 0042, so `adr-next.sh` gave 0039.

## Decisions

1. **One event `hotkey.status`.** The Tauri name is `hotkey:status` (`events::HOTKEY_STATUS`). The direction is core-to-brain, on the brain Channel. The payload is `TalkKeyStatus { talkKey, reason }`. `talkKey` is `active`, `paused` or `unavailable`. `reason` is `secure_input`, `elevated_foreground`, `fallback_chord_unavailable`, `hook_failed` or `null`. `reason` is required and nullable, like `MicLevelFrame.owner`. The payload has no window title, process name, key or failure text. The Rust mirror is in `mog_core::types` with `deny_unknown_fields`. Shared fixtures check both sides. #294 closes into this event.
2. **Mapping.** The host maps each R1 signal as follows:

   | Source | `talkKey` | `reason` |
   |---|---|---|
   | `HotkeyOutput::TalkKeyPaused` | `paused` | `secure_input` |
   | `HotkeyOutput::TalkKeyRestored` | `active` | `null` |
   | `HotkeyOutput::FallbackChordUnavailable` | `paused` | `fallback_chord_unavailable` |
   | Windows `DeferredSignal::TalkKeyBlocked` | `paused` | `elevated_foreground` |
   | Windows `DeferredSignal::HookFailed(_)` | `unavailable` | `hook_failed` |
   | Windows `DeferredSignal::HookReinstalled` | `active` | `null` |
   | `hotkey_host::install` fails in `lib.rs` | `unavailable` | `hook_failed` |

   The core sends `TalkKeyBlocked` once per elevated process, so the CAP-42 tooltip shows on the first failure. The fixed `HookFailed` reason string stays in the host. The brain picks the pill or the tooltip text by `reason`.
3. **Sticky delivery.** The brain can subscribe after the status changed, for example after an install failure at setup. `events::STICKY` lists the sticky events. Only `hotkey:status` is sticky. The pure type `ipc::StickyEvents` keeps the last payload of each sticky event, scrubbed (SEC-01). `BrainChannel::send_sticky` keeps the item and sends it. `BrainChannel::set` sends the kept items to each new Channel, then uses it. One lock covers the keep, the send and the replay, so a new Channel cannot miss an item. `BrainSink::deliver_sticky` defaults to `deliver`.
4. **Status sink.** `BrainStatus<B: BrainSink>` replaces `PendingStatusEvent`. It sends each status with `deliver_sticky`, except `elevated_foreground`. R1 sends that one once per elevated process, and no signal ends it. So it goes out with plain `deliver`: a reloaded brain must not show the CAP-42 tooltip again or think that the talk key is paused (CAP-42: first failure only). The kept sticky state stays the last real state. **Brain rule:** a `hotkey:status` with `reason: "elevated_foreground"` is a one-shot tooltip notice, not a talk-key state. The brain shows the CAP-42 tooltip and keeps its last other status (the payload says `paused` only to tell the tooltip why). So a live brain and a reloaded brain agree on the talk key. The TS `EVENTS['hotkey.status']` doc states this rule for #7. The forwarder uses it for the R1 pill outputs. `HookConsumers::with_status(..)` gives the Windows hook seam the same sink. `hotkey_host::report_unavailable` sends the install failure.
5. **`permission_request`.** The windows are `hub` only. The phase is MVP-D. The arguments are `{ permission: 'input_monitoring' | 'accessibility' }` with `deny_unknown_fields`. The reply is `{ status: 'granted' | 'denied' | 'unknown' }`. The gateway lets only the hub call it, so the call is the user click (CAP-36, CAP-37). The handler makes `UserAction::from_user_click()`. `input_monitoring` calls `HotkeyHost::request_ptt_permission`, which starts R1 again when granted. `accessibility` calls `request_insertion_permission`. The route is the `PermissionRoute` seam on `HostComponents`. The managed `HotkeyRuntime` implements it. With no runtime, the reply is `host_failed`. The capability `allow-permission-request` is in `hub.json` only. This PR adds no `permission_status` read.
6. **`permission_request` runs on a worker.** The macOS calls `IOHIDRequestAccess` and `AXIsProcessTrustedWithOptions` can wait while the system prompt shows. The IPC handler runs on the main thread, so an inline call can freeze every window. The calls do not need the main thread. A restart after a grant starts the tap on its own threads. The chord uses `dispatch_async` to the main queue and does not wait. So `ipc::schedule` sends `permission_request` to `spawn_blocking`. The order against other commands does not matter: the wizard awaits the reply. A test checks the thread of each command.
7. **`capture_resume`.** The windows are `brain` only. The phase is MVP-D. The arguments are `CaptureLeaseArgs { mogId, leaseId }`. The reply is `CaptureState`. The refusal codes are the same as `capture_pause`. It is a new row, not a `capture_pause` toggle, so a lost reply cannot flip the state. It answers `not_implemented` until the capture IPC wiring of #4.
8. **Mirrors move together.** The Rust table, TS `COMMANDS`, `CommandArgs`, `CommandReturns`, `EVENTS`, the capability files, the generated schema and the generated permission files change in one PR.

## Rows for the requirements owner

This PR does not edit the 4.5 table. Add these rows to 4.5:

| Name | Kind | Windows | Phase | Shape |
|---|---|---|---|---|
| `hotkey.status` | event, core to brain | brain | MVP-D | `TalkKeyStatus { talkKey, reason }`, sticky (not `elevated_foreground`) |
| `permission_request` | command | hub | MVP-D | `{ permission }` to `{ status }` |
| `capture_resume` | command | brain | MVP-D | `{ mogId, leaseId }` to `CaptureState` |

## Consequences

- The brain gets the talk-key status even when it subscribes late or reloads.
- The brain must handle `hotkey:status` on its Channel. The pill and the tooltip UI are the brain and view issues (#6, #7).
- `capture_resume` needs no contract change when #4 wires R2.

## Open work

- Done in #425 (ADR 0055): `capture_resume` goes to R2 `resume(lease_id)` with the other `capture_*` routes.
- An `elevated_foreground` notice that comes before the brain subscribes is lost for that elevated process. This fits CAP-42 ("first failure"), because the brain has no window to show it in yet.
- The pill text for `fallback_chord_unavailable` and the CAP-42 tooltip text are UX decisions for the brain issues.
- The host does not send a status for `HostState::NeedsPermission` or for a start that returns `Unavailable` after install. A later issue can add that.
