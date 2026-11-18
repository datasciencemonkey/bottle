# ADR 0081: Forget runs as three atomic groups that delete citing items first

- Status: Accepted
- Date: 2026-11-18
- Issue: #123 (MEM-32a, MEM-32c, MEM-32d, MEM-01, G-45a, G-45b, AT-16; rows 9.2, D36, D61, D76, U9)
- Owner: impl-123 (security-lead)

## Context

MEM-32a says Forget must delete the local rows of a scope and each item that cites them. ADR 0033 makes
the optional pointers `genie_exchange.session_id` and `memory_item.source_exchange_id` `ON DELETE SET
NULL`. The #35 review (note on #123) found the risk: if Forget deletes a parent first, the pointer
becomes NULL, and Forget can no longer find the items that cite it. ADR 0054 runs one catalog statement
for each `mem_local_exec` call, so a Forget of several brain calls is not one transaction. The issue left
these points open: the shape of the Forget statements, the delete set of each scope, what happens to
consent records and the MOG row, and the Genie One memory review link.

## Decisions

1. **Three atomic groups** (ADR 0068 `Group`): `mem.forget.items`, `mem.forget.session` and
   `mem.forget.mog`. Each group is `Access::Brain`, because T10 runs Forget on a user action (ADR 0054).
   Each step is `Access::Core`, so the brain cannot run a partial Forget. The single statements
   `mem.forget.item`, `mem.forget.exchange` and `mem.forget.log` are removed: a lone exchange delete
   leaves citing items with a NULL pointer.
2. **Order.** In one transaction:
   - `mem.forget.session`: the items that cite the session in `source_session_ids`, the items that cite
     one of its utterances in `source_utterance_ids`, the items that cite one of its exchanges in
     `source_exchange_id`; then the embed jobs that point at no item; then the exchanges, the
     utterances and the session row.
   - `mem.forget.items`: the named items, then the embed jobs that point at no item.
   - `mem.forget.mog`: every item, the embed jobs, every exchange, utterance, session, `kv` and
     `sync_state` row of the MOG, then the `mog` row.
   - The last step of each group writes one `forget_log` row with a fixed scope (`item`, `session`,
     `mog`) and the target ids. A failed step rolls back every step and writes no log row.
   The MEM-01 guard forbids `OR`, so each kind of citation is its own step. FTS5 and vec0 rows go
   with their rows through the 0001 triggers. A Rust test checks that `mem.recall.fts` no longer finds
   a forgotten text.
3. **Conversations are read first.** The Brain queries `mem.forget.session_conversations` and
   `mem.forget.mog_conversations` return the distinct conversation ids with their backend, profile,
   host, workspace id and space id. `ForgetFlow` reads them before it calls the group. Forget of items
   deletes no exchange, so it reads no conversation.
4. **Genie delete (MEM-32c).** `genie_one_mcp` threads go to `GenieOneDeleteUtility.deleteThreads
   ('forget', ...)` (ADR 0077). Its SP-11 gate is off (`TODO(#121)`), so the dialog shows the
   "Delete in Genie One" notice. `genie_agent` conversations go to GA `DELETE` on the fixed route
   `genie.delete_conversation`, with the profile and space that made them (9.2). A failed delete or a
   conversation with no API is disclosed. A Genie failure never undoes the local delete, so Forget
   works offline (AT-16).
5. **Kept rows.** Consent records stay. They are content-free security data with their own 13-month
   expiry (SEC-15b), and they are the evidence of consent. Old `forget_log` rows stay for the v1 sync
   (MEM-11c, MEM-12b). Embed jobs hold no text (ADR 0068), and the sweep removes each job that points
   at no item. The MVP writes no `sync` outbox rows. TODO(#195): the v1 SyncWorker queues the remote
   deletes first (MEM-32b), and decides what happens to the `sync` rows of a forgotten scope.
6. **The MOG row goes** with `mem.forget.mog`, so no row of the scope is left. A caller registers the
   MOG again with `mem.mog.upsert` before a new write, as the `recordExchange` contract already says.
7. **Memory review link (MEM-32d, D61).** No source names a Genie One memory page. The public docs
   describe how to review memories inside Genie One. The link is the Genie One home of the MOG's home
   workspace, `https://<host>/one?o=<wsid>`, with the same host checks as the G-45b link.
   `TODO(#512)` (`needs:human`): confirm the exact page path.
8. **Audit.** SEC-23 lists asks, context sends, Page calls, agent actions and sync. It does not list
   Forget, and `AUDIT_KINDS` is a closed set. So Forget writes no audit entry; `forget_log` is the
   content-free record.
9. **Dialog.** `src/shared/forget-view.ts` is the model. The brain builds it in
   `src/brain/components/forget-flow.ts`. The thin view `src/views/ForgetDialog.tsx` shows the confirm
   text, the memory review link and, after the delete, the `GenieOneDeleteNotice` (ARC-2). A click
   sends `view_emit` with the scope kind or the link target only. `TODO(#87)`: open the link in the
   system browser.

## Consequences

- Wipe (#124) can reuse `mem.forget.embed_orphans` and the order rule: delete the citing items before
  their parents.
- "Stop and delete this session" (SEC-14a) can call `mem.forget.session`.
- The item lookup by session or utterance is a `json_each` scan of one MOG's items (ADR 0033 accepts
  this at the MVP size).
