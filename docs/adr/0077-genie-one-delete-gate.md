# ADR 0077: The Genie One API delete utility is gated on SP-11, with a "Delete in Genie One" fallback

- Status: accepted
- Issue: #122 (G-45a, G-45b, AT-16; rows 4.6 route IDs, 9.2 "Genie conversations created by Bottle", SP-11, D76, ADM-14, U9)
- Date: 2026-11-17
- Owner: impl-122 (security-lead)

## Context

G-45a lets the bridge use the Genie One API list and delete endpoints only as a utility, and
only after SP-11 passes. G-45b requires the "Delete in Genie One" fallback when SP-11 fails, and
the UI must disclose when deletion is not possible. SP-11 (#121) needs a person, a U2M profile
and an admin (`needs:human`), so it has no result yet. The route IDs
`genie_one.list_conversations` and `genie_one.delete_conversation` are already in the contract
(`RouteId`) and R7 already sends them (`http_transport/routes.rs`). The issue left these points
open:

1. Where the SP-11 gate is, and what its default is.
2. Which conversations the utility deletes, and on which workspace.
3. What "use list as a utility" means.
4. How the disclosure and the link reach the Forget (#123) and Wipe (#124) dialogs.

## Decisions

1. The gate is the constant `SP11_GATE` in `src/brain/components/genie-one-delete.ts`. Its value
   is `'off'`. With the gate off, `GenieOneDeleteUtility.deleteThreads` sends no call. A test
   checks that the default utility sends no `dbx_http`. A second test checks that no other brain
   or view source names the two `genie_one.*` route IDs. Thus no call happens before the SP-11
   pass (AT-16). The flip to `'on'` is a reviewed code change. It happens only when
   `docs/spikes/SP-11.md` records a PASS (`TODO(#121)`).
2. The utility deletes only the threads that the caller gives it. These are the threads that a
   MOG created, from the local exchange rows. It never deletes a conversation that it finds in a
   list. Each delete goes to the profile that made the thread, so a conversation stays in its
   workspace (antigravity-bricks-plugin pattern, AGENTS.md). Deletes run one at a time, so they
   share the R7 rate bucket with other calls. A 404 counts as gone.
3. After the deletes, the utility lists each profile once, at most 5 pages of 100 (the SP-11
   probe limit). A thread that still lists is not reported as deleted. A failed list does not
   undo a delete that the API confirmed. The utility reads conversation IDs only, never a title.
4. The utility runs only on a user action: `'forget'` or `'wipe'` (U9). Any other trigger
   throws before a call. There is no timer.
5. Each thread that is not deleted gets the "Delete in Genie One" link
   `https://<host>/one/chat/threads/<cid>?o=<wsid>` (the G-19a thread link). The link needs an
   https or bare host, a numeric workspace ID and a slug conversation ID. Else there is no link.
   `deleteNotice()` builds the dialog notice: the disclosure text, one link for each thread, and
   a second text for the threads with no link. The shared type is
   `src/shared/genie-one-delete-view.ts`. The thin view `src/views/GenieOneDeleteNotice.tsx`
   shows it (ARC-2). A click sends `view_emit` with the conversation ID only. The URL also shows
   as text. `TODO(#87)`: open the safe thread link in the system browser.
6. The module is a new file. `genie-bridge.ts` does not change, because open PRs #480 and #483
   edit it. `MogBackend.deleteConversation` (4.7) can call the utility later. Forget and Wipe
   call the utility directly.

## Consequences

- Before SP-11 passes, Forget and Wipe always show the disclosure and the links. They never
  call the Genie One API.
- If SP-11 passes, one line changes the gate. The admin must also turn on the Genie One API
  preview (ADM-14). A delete that fails then falls back to the link with the disclosure.
- If the recorded list reply shape differs from `{conversations: [{conversation_id}],
  next_page_token}`, `createDbxGenieOneApi` changes with the gate flip (`TODO(#121)`).
