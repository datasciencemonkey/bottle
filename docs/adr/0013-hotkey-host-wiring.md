# ADR 0013: Host wiring of the talk key: queue sink, latched target, no prompt at start

- Status: Accepted
- Date: 2026-11-16
- Issue: #305 (follows #6 and ADR 0011)
- Owner: impl-305 (capture-lead)

## Context

R1 HotkeyService on macOS (`MacHotkeyService`, #6) calls its sink from the tap thread with the engine lock held. The Tauri host must create the service with the D66 talk key and forward `HotkeyOutput::Ptt(phase)` to the brain as `hotkey.ptt {mogId, phase}` (section 4.5). The talk key must target the last-interacted MOG (MB-06a). The host must not prompt for a permission on its own (CAP-36, CAP-37). The issue leaves these choices open.

## Decision

1. **One host module.** `src-tauri/src/hotkey_host.rs` holds the wiring. `lib.rs` only calls `install` in setup, `on_window_event`, and `shutdown` on `RunEvent::Exit`.
2. **Queue sink.** The R1 sink only pushes the output on an `mpsc` queue. A forwarder thread drains the queue and sends to the brain Channel. So the tap callback returns fast and never waits for a WebView.
3. **Latched target.** The forwarder reads the target at `down` and keeps it until `up` or `cancel`. One press goes to one MOG, even if the target moves during the hold. With no target, the press sends nothing, and no lone `up` follows.
4. **Interim target input.** A `mog-<id>` window that gets focus becomes the target. Its `Destroyed` event clears it. The brain's `mog_set_active` (#73) replaces this input.
5. **Status seam.** `TalkKeyPaused`, `TalkKeyRestored` and `FallbackChordUnavailable` go to a `StatusSink`. The app sink does nothing until the `hotkey.status` event of #297 exists.
6. **No prompt at start.** `HotkeyHost::start` calls the service `start`, which checks Input Monitoring with no prompt. On `input_monitoring_required` the state is `NeedsPermission`. Only `request_ptt_permission(UserAction)` prompts, and it starts the service again when granted. `request_insertion_permission(UserAction)` asks for Accessibility only. The IPC command for these calls is contract change #312.
7. **Platform seam.** `platform_service` picks the service by `cfg`. macOS: `MacHotkeyService` and `MacPermissions`. Other OS: the R1 stub, so the state is `Unavailable("not_implemented")`. The Windows service of #7 plugs in here with the same sink.
8. **Stop on quit.** `RunEvent::Exit` calls `stop`, which removes the tap and the watchdog.

## Consequences

- `hotkey.ptt` reaches the brain only after the user grants Input Monitoring and the wizard or Settings calls `request_ptt_permission` (#312, #43), or after a restart with the permission granted.
- The talk key comes from the D66 default until Settings stores it (#77).
- Real-device checks of the wiring are steps H1-H5 in `docs/tests/hotkey-macos.md` (#303).
