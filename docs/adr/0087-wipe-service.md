# ADR 0087: R14 WipeService runs the user's Wipe in one Rust transaction, and the security expiries at start

- Status: Accepted
- Date: 2026-11-18
- Issue: #124 (MEM-33a, SEC-15a, SEC-15c; also MEM-26, SEC-15b, MB-15, G-45a, G-45b, G-49, MEM-01, AT-16;
  rows R13, R14, 9.2, U9, COL-101, Appendix C)
- Owner: impl-124 (security-lead)
- Contract change: #525

## Context

U9 is binding: Bottle deletes no user content on its own. Settings → Wipe offers three options (older than 30 d,
older than 90 d, everything), and it runs only on a user action. Section 9.2 keeps an automatic expiry only for
content-free security data: the session audio journal after commit, the local logs, the action log and the consent
records. R14 WipeService owns both. ADR 0033 set the delete rule (each table by its own age, children before
parents, no cascade). ADR 0054 made `mem.wipe.*` Rust-only. ADR 0081 (Forget) gave atomic groups and reusable delete
steps. ADR 0052 and ADR 0059 made the consent purge and the audit rotation functions that only
`run_security_expiries` calls.

The issue left these points open: where Wipe runs, the age column of each table, what "everything" keeps, how a
Wipe across MOGs keeps MEM-01, what triggers the security expiries, how the brain reaches R14, the Genie
conversations, and what the admin lock shows.

## Decisions

1. **Wipe runs in Rust, in one transaction.** `LocalWipeService::wipe(scope)` opens one IMMEDIATE R10
   transaction (`SqlCipherLocalStore::core_transaction`, crate-private). It reads the device-wide MOG list
   `wipe.mog_list`, then for each MOG it reads the Genie conversations and runs one MOG-scoped group. A failed step
   rolls back the whole Wipe of every MOG.
2. **MEM-01.** Each delete statement and group filters by `mog_id`. The MOG list crosses MOGs on purpose. It is the
   `catalog::DeviceStmt` `WIPE_MOG_LIST` with its reason, as the consent purge of #63 is. It reads MOG ids only, from
   `mog` and from every table that holds user content (the outbox has no foreign key to `mog`). It is not in
   `STATEMENTS`, so `exec` and `mem_local_exec` cannot run it.
3. **Age columns (app clock, as ADR 0061).** Items use `first_seen_at`. Exchanges use `asked_at`. Utterances use
   `started_at`; these are the transcripts (raw and cleaned) and the dictation history (`route = 'dictation'`).
   `mem.wipe.items_before` changes from `created_at` (the SQLite clock) to `first_seen_at`, so the simulated clock
   tests the real rule. "Older than" is strict: a row at the cutoff stays. The cutoffs are now − 30 d and now − 90 d.
4. **Order and no cascade (ADR 0033).** `mem.wipe.before` runs: items, the embed jobs that point at no item
   (`mem.forget.embed_orphans`, reused), exchanges, utterances, then the old sessions that have no utterance and no
   exchange left. A recent item that cites a wiped exchange stays, and its pointer becomes NULL. A session with a
   recent utterance or exchange stays, so a kept exchange keeps its session pointer. FTS5 and vec0 rows go with
   their rows through the 0001 triggers, so recall no longer finds a wiped text.
5. **"Everything" keeps the MOG rows.** `mem.wipe.everything` reuses the Forget steps `mem.forget.mog_items`,
   `_outbox`, `_exchanges`, `_utterances` and `_sessions`. Forget of a MOG deletes the `mog` row because the MOG goes
   away. Wipe keeps the MOGs: the `mog` row holds no content, and the writers do not register it again before each
   write. "Everything" also keeps `kv` and `sync_state` (no content, unused in the MVP), the consent records (SEC-32a,
   their own SEC-15b expiry) and `forget_log` (ids only). So the store has no transcript, dictation, answer, memory
   item, embed job, FTS5 or vec0 row, and a new write still saves.
6. **No `forget_log` row for a Wipe.** `forget_log` has no column for an age selection. TODO(#216): the v1 sync
   applies the same selection to Lakebase (MEM-33b) and records it.
7. **Genie conversations (9.2, G-49).** For each MOG, R14 reads the conversations of the exchanges it deletes, before
   the delete. An age Wipe leaves out a conversation that a kept exchange still uses. The brain deletes them only if
   the user also selected "Also delete Genie conversations". It uses the Forget helper
   `deleteGenieConversations('wipe', ...)`, now shared out of `ForgetFlow`. Genie One threads go to the #122
   utility; its SP-11 gate is off (`TODO(#121)`), so the dialog shows one "Delete in Genie One" notice for each MOG
   (G-45a, G-45b). `genie_agent` conversations go to GA `DELETE` in their space. A Genie failure is disclosed and
   never undoes the local Wipe. `TODO(#87)` opens the links.
8. **Security expiries.** `run_security_expiries` runs each registered `SecurityExpiry` in order. A failure does not
   stop the others. The expiries are the consent records (`ConsentRecords`, SEC-15b) and the action log
   (`AuditLog`, ADR 0059). `TODO(#125)` adds the audio journal deadline and `TODO(#517)` adds the local log file.
   `TODO(#448)` adds the host audit log once the host keeps one. 9.2 says that this data "keeps an automatic
   expiry", so the expiries need a trigger that is not a user action. The host runs them once at start
   (`consent_host::install_with`), and `mem_wipe` runs them after each Wipe. There is no timer and no background job.
   COL-88 ("every 6 h") predates U9 and was for user content; the start run is enough for periods of 30 d and more.
   No expiry deletes user content; a test runs them for 400 simulated days and checks every content row.
9. **The brain reaches R14 through `mem_wipe` (#525).** No command can run a Core statement from the brain. #525 adds
   `mem_wipe { scope }` (brain only) with the answer `{ local, conversations }`, ids only. `mog_core::wipe::mem_wipe`
   is the R14 side; it runs the Wipe, then the expiries. The host already manages `Arc<LocalWipeService>` for the
   route. Until #525 lands, the default `WipeCore` refuses before any call, and the dialog says that nothing changed.
10. **One brain caller.** `WipeFlow` is the only brain code that calls the Wipe, and `WipeDialogController` calls it
    only from the confirm click of a dialog it opened. The view sends the option; the brain keeps it, and a confirm
    for another option does nothing. A test checks that no other source calls it and that there is no timer.
11. **Hub panel (interim names, like ADR 0081).** `src/views/hub/WipeSettings.tsx` shows the three options (none
    chosen; the button is off until one is), "Also delete Genie conversations" (off), the Wipe button and the 9.2
    retention rows behind a "Show how long Bottle keeps data" click. The events `hub:wipe_dialog` and `hub:wipe_settings` go to the hub only (ARC-5). The click
    targets are `wipe_open`, `wipe_confirm`, `wipe_cancel` and `wipe_settings`, with the fixed `mogId` `wipe`,
    because `view_emit` needs one. `TODO(#525)` moves them into the contract.
12. **Admin lock (SEC-15c, MB-15).** `adminWins(user, admin)` applies the admin value and locks it whenever the admin
    sets it. The rows show a lock and "Set by your admin". `AudioRetentionDays` (0-30) is the one Appendix C key in
    this panel. Appendix C has no key for the log, action log and consent periods (`TODO(#186)`), so they show the
    9.2 defaults. The user-content keys (`TranscriptRetentionDays`, `DictationHistoryDays`, `MemoryRetentionDays`,
    `GenieExchangeRetentionDays`) stay "Kept until you wipe": U9 forbids an automatic delete, and COL-101 keeps an MDM
    Wipe policy for v1. The policy comes from `policy_get`; R13 answers `not_implemented` until #186, so no row is
    locked yet. `policy_store.rs` does not change (open PR #510 edits it).

## Consequences

- The Wipe works end to end once #525 routes `mem_wipe` and #426 opens the store for memory. Until then the panel and
  the dialog work, and a Wipe fails safe.
- "Stop and delete this session" (SEC-14a, #133) keeps using `mem.forget.session`.
- A user who never restarts the app gets the consent and action-log expiries at the next start or Wipe. If a
  periodic run is needed, it must be a new decision against U9.
- `conversationOf` in `memory-service.ts` is exported for the Wipe rows. G-21a: the views and the memory modules name
  no backend kind; the split by backend stays in `forget-flow.ts`.
