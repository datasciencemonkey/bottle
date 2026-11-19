# ADR 0102: Ask Genie shows its work on the MOG, and the answer card shows by itself

Status: accepted, 2026-11-19. Issue: #573 (and #88). Amends ADR 0022 (decision 2) and ADR 0097.

## Context

A user ran Ask Genie (⌥G) on a debug build of main. Genie got the question, but the user saw no
progress and no answer. Genie One does not stream tokens: `genie_poll_response` gives `status`,
`progress_steps` and, at the end, `final_answer`.

A live check on 2026-11-19 (profile `DEFAULT`, "What tables can you see?") gave these shapes:

- `genie_ask` (4.5 s): `structuredContent` `{conversation_id, response_id, status: "in_progress"}`.
- `genie_poll_response`: the keys `status`, `progress_steps`, `final_answer`, `query_items`,
  `deep_link`, `narration_instruction`, `conversation_id`, `response_id`. The first polls have
  `progress_steps: null`. Later polls have a growing list of steps. Each step is one string that
  starts with `Running SQL:`, `Query result:` or `Thinking:`, followed by raw SQL or table rows.
- The terminal poll (44 s): `status: "completed"`, `progress_steps: null`, a markdown
  `final_answer` (headings, bold numbers) and two `query_items` `{item_id, sql}`.

The brain parsing (`ask-events.ts`, `poll-actor.ts`) matches these shapes. The cause of "nothing
shown" was in the UI path:

1. The pop-up fades after 5 s and sends `cancel`, so the brain hid the card window. The answer
   came about 40 s later into the hidden window, and nothing showed the window again.
2. The card window is 340 x 64 px. The answer panel did not fit in it.
3. The MOG mood did not change during an ask. Only `capture:state` drove it, so the MOG was idle.
4. The card panel became key when it showed, so a card that shows by itself would take the keys
   from the app the user types in.

## Decision

1. **The MOG shows the work.** `capture-mood.ts` takes the phase of the request: `processing` from
   the ⌥G key-up until the end, then `answering` (1.2 s) and `success` (2.5 s), or `error` (3 s).
2. **The status pill shows the stage.** The card shows "Asking Genie", then one fixed stage line
   from the prefix of the newest Genie step (`src/shared/genie-status.ts`), then the timer. The
   step text never shows (G-17b, UX-16a): it holds SQL, rows and names.
3. **The answer card shows by itself.** At the end the brain puts the outcome on the card and then
   calls `window_card_show`. The pill grows into the card. A failure shows the GL-5 text and
   Try again; only a click sends again (G-15b).
4. **The card is transient.** It stays 4 s + 0.25 s per word (6 s to 20 s), a failure 8 s. The
   pointer on the card or on the MOG holds it. Then it fades and sends `click {target:
   'card_done'}`. The brain hides it and clears that answer. A fade is not `cancel`, so it never
   stops hands-free listening.
5. **The card fits its window.** The card view sets its own window height (`fit-window.ts`). R11
   keeps the height of a card that is on screen when it shows it again, and puts a hidden card back
   to one line. The card capability gets only `set-size`, `set-position`, `outer-position`,
   `scale-factor` and `current-monitor`.
6. **A shown card is not key** (amends ADR 0022, decision 2). `MacPanelStyle::show` calls only
   `orderFrontRegardless`. A card can still become key: a click makes it key, and
   `accept_first_mouse` is now true for the card, so that click also acts. This also applies to
   the dictation pop-up.
7. **A double-click opens the hub.** A double-click on the MOG or the card sends `click {target:
   'open_conversations'}`. The brain shows and focuses the hub (`hub-window.ts`) and sends
   `hub:conversations.open {requestId}` to the hub. A close of the hub now only hides it. The brain
   capability gets `get-all-windows`, `show`, `unminimize` and `set-focus`. A single mascot click
   waits 250 ms, so a double-click never starts listening.

No contract file changes. The new signals are `click` and `hover` payloads and brain-to-view
events outside `EVENTS`.

## Consequences

- The ⌥G echo of the question no longer shows by itself; the status pill follows it in the same
  task.
- A single mascot click starts hands-free listening 250 ms later than before.
- Tauri window commands take a window label, so the card could size another Bottle window. The
  card page is bundled code under the CSP, and the answer goes through `safe-markdown.tsx`.
- Not done: the card does not pick the side of the MOG with more room, and the tail is always on
  the left.
