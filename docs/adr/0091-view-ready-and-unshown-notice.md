# ADR 0091: The view `ready` signal and the hub-only notice for an unshown dictation

- Status: Accepted
- Date: 2026-11-18
- Issues: #507 and #509, for #498 (MOG-18a, MOG-18b, MOG-13b, MOG-05b, MOG-06d, MOG-08a, SEC-02a, SEC-02b, SEC-03, SEC-16c)
- Owner: contract-lead-2 (implemented by impl-507)

## Context

The host makes a card window late (MOG-18a) and destroys it after 5 min hidden (MOG-18b). A `card:update` that the brain sends before the new view listens is lost. The MOG view and the hub have the same race.

When `window_card_show` fails twice, #498 must put the dictated text in a hub notice with Copy. The only hub notice is the consent notice on the Tauri event `hub:notice`. ADR 0003 (decision 3) says that a listener with the default target `Any` gets every `emit_to` event, and that `core:event:allow-listen` has no event-name scope. So every MOG and card window can listen to `hub:notice`. It is not a hub-only path.

## Decisions

1. **`ready` is a generic `view_emit` kind.** `ViewEmitKind` gets `ready` in `src/shared/ipc.ts` (`VIEW_EMIT_KINDS`) and in `mog_core::ipc`. The windows stay `hub`, `mog` and `card`. `ready` has no payload. The gateway refuses a `payload` key with `invalid_args`, also `null`. The brain parser drops it too. The brain keys a replay on `from`, which the host sets. This change does not wire a replay: #498 (card), #439 (MOG) and #393/#407 (hub) do.
2. **The text does not go on `hub:notice`.** A text on a Tauri event can reach every view, so the event stays as it is.
3. **A private hub Channel, like `brain_subscribe`.** There are two new MVP-D rows:
   - `view_notice_subscribe { onNotice: Channel }` is for the `hub` window only. The name starts with `view_` because a view capability may hold only `view_*` commands (MOG-08a). The host keeps the Channel of the calling hub webview in `HubChannel`. A hub reload replaces it.
   - `hub_notice_send { kind: 'dictation_unshown', mogId, text }` is for the `brain` only. The host sends the notice on the hub Channel only. It does not use the brain Channel or a Tauri event. With no hub Channel the answer is `host_failed`, so the brain keeps the text. The host keeps no copy.
   - A MOG or card window cannot take the Channel (`window_not_allowed`), so it never gets a notice. No view can send the notice.
   - The hub Channel payload deliberately skips the rule-7 scrub that the brain Channel gets, so the user can copy the exact text. The card path does not scrub either.
4. **No text in Debug or logs.** `HubNoticeSendArgs` has a hand-written `Debug` that shows `<redacted N chars>`. #505 is the general fix for content in `Debug`. The gateway log line holds only the command, the window class and the outcome. A refusal is only a code. The TS helpers (`sendHubNotice`, `subscribeHubNotices`) log nothing.
5. **The hub only copies or dismisses.** No new command sends the text to another place. The hub dismisses a notice in its own state, so there is no dismiss command. If the brain must know about a dismiss later, the hub sends `view_emit` `click` with an action and no text.

## Consequences

- #498 calls `sendHubNotice(mogId, text)` from `onUnshown`. On `false` it keeps the text and sends it again after the hub sends `ready`.
- The hub view calls `subscribeHubNotices` and shows the text with Copy (#498).
- `src/views/arc2.test.ts` now allows the two `view_*` commands `view_emit` and `view_notice_subscribe`.
