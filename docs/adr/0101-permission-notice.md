# ADR 0101: Bottle shows a missing OS permission and asks for it after one click

- Status: accepted
- Issue: refs #43, #552 (CAP-36, CAP-37, MOG-08a, UX-37b)
- Date: 2026-11-19
- Owner: impl-perm

## Context

A new macOS build has a new code signature, so macOS treats it as a new app. Without Input
Monitoring, the listen-only tap does not start. The talk key then does nothing, and Bottle
showed nothing. The host never asks on its own (CAP-36). But no view and no brain code read
the permission state or called `permission_request`. Views call only `view_*` commands
(MOG-08a, the ARC-2 test), and `permission_request` was a hub-only row, so no code path could
call it.

## Decisions

1. **`permission_status` (new row, brain and hub, MVP-D).** The arguments are
   `{ permission: 'input_monitoring' | 'accessibility' }`, the same schema as
   `permission_request`. The reply is `{ status: 'granted' | 'denied' | 'unknown' }`. The host
   reads `PermissionApi::status`. It never shows a prompt. This is the read of #552.
2. **`permission_request` windows: brain and hub.** The brain sends it only for a click on the
   control of that permission in the hub or in the pop-up. The hub view does not call it
   (MOG-08a). The capability `allow-permission-request` is now in `brain.json` too.
3. **One click, then Settings.** The OS shows its prompt only once. So the first click in a run
   shows the prompt. A later click, or a click for a denied permission, opens the System
   Settings pane (`x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent`
   or `?Privacy_Accessibility`) through `NSWorkspace openURL`. The URL is fixed.
4. **A grant in Settings.** When Input Monitoring reads granted and the talk key waits for it,
   `permission_status` starts the talk key again (no prompt). If the talk key still does not
   start, Input Monitoring reads `unknown` until Bottle reopens. The texts tell the user to
   reopen Bottle after the Input Monitoring grant.
5. **Where it shows.** At launch the brain reads both permissions. The hub shows each missing
   permission with its text, the System Settings path and one control. The first MOG that
   opens shows the first missing permission in its pop-up, with one control. The texts are
   "Bottle needs Input Monitoring to hear the talk key" and "Bottle needs Accessibility to
   paste". The hub event `hub:permissions` is not a contract row yet (TODO(#525)).
6. **Both commands run on a worker.** A request can wait for the prompt. A status read can start
   the talk key.

## Consequences

- A talk-key press without Input Monitoring still gives no event, so the notice shows at launch
  and in the hub, not at the press.
- The contract files change: `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs`,
  `src-tauri/src/ipc.rs`, `src-tauri/schemas/ipc-commands.json` and the capabilities.
