# ADR 0125: the local recording index, and local-first delete

- Status: Accepted (owner authorization, 2026-11-25)
- Issue: owner request in chat (no GitHub issue filed); see `research/message-board.md` for the
  lane note
- Related: ADR 0103 (call recording contract), ADR 0105 (one store, one key, one keychain item),
  ADR 0033 / ADR 0071 (MEM-01 and its one named exemption, `consent_record`)
- Owner: lane `arch/rec-index`

## Context

The owner, in chat on 2026-11-25, asked for Recordings to be examined and fixed on three points:
listing is slow (5-8 s with about 15 recordings), stopping a recording does not show it being
processed at once, and delete does not work for a row the hub shows as failed with no local
audio. The root causes:

1. `recording_list` always calls `CloudAdapter::list`, which lists the volume directory, then
   reads one `job.json` per recording, serially: N+1 network calls on every list, and the brain
   calls `refreshList` after almost every action.
2. `CloudAdapter::delete` made a network read first, refused when any part had ever uploaded, and
   silently answered `Ok(false)` for a name with no local audio — exactly the row the owner could
   not delete. The brain showed no feedback for the `Ok(false)` case.
3. Empty `rec-<id>` temp folders (the per-recording container under the app's temp root; named by
   the capture's internal id, not the recording's volume name) were never swept once their last
   file was gone.

The owner's authorization, verbatim: "It could literally be another table in that same
underlying SQLite encrypted store," and "you should not have a separate key in the keychain
because we already have the keys for the conversations." That authorizes one new `local_store`
migration (0005) and the small IPC shape change `recording_list` needs; it does not authorize
rewriting `CloudAdapter::list`'s already-reviewed overlay logic (live, active, in-flight, ends),
which this change keeps and reuses.

## Decisions

1. **One new table, `recording`, migration `0005_recording_index.sql`, in the same SQLCipher
   file, through the same store handle and the same key as Conversations (ADR 0105).** No new
   keychain item, no second database, no second connection. Columns: `name` (PK), `profile`,
   `started_at_ms`, `duration_s`, `status`, `error_kind`, `error_stage`, `on_device_only`,
   `hidden`, `updated_at_ms`. Metadata only: no transcript text, no notes text, no token, no file
   path.
2. **The table carries no `mog_id`.** A recording belongs to a Databricks profile (a workspace),
   not a MOG: recording predates the MOG split (ADR 0117), and it is the app's own content, the
   same way the already-exempt `consent_record` rows are (ADR 0052/0071). Its statements
   (`local_store::catalog::RECORDING`) are validated by `catalog::check_params`, which carries no
   MEM-01 rule, the same path `catalog::CONSENT` already uses; they are not in
   `catalog::STATEMENTS`, so `LocalStore::exec` and `mem_local_exec` never see them — Rust only,
   reached through `recording_index::SqlRecordingIndex`, which runs them with
   `SqlCipherLocalStore::with_conn` (the exact pattern `consent::ConsentRecords` already uses for
   its own device-wide rows). The repo-wide MEM-01 schema tests (`mem_01_every_table_has_mog_id`,
   `mem_01_every_index_has_mog_id`) gain one named exemption, `recording`, with this same
   reasoning in the test's own comment.
3. **Write-through, not a background indexer.** `CloudAdapter::emit` and `::emit_live` — the two
   existing chokepoints that already notify the brain of every manifest change (start, each part,
   upload progress, transcribing, writing notes, done, failed) — now also upsert the row, right
   before they call `on_job`. No new timer, no new polling loop.
4. **`recording_list` reads the index by default; `reconcile: true` (a new, optional field on
   `RecordingListArgs`, the one small IPC shape change this needs) reads the volume once and
   refreshes the index.** `CloudAdapter::list_fast` is the index's rows, merged through the same
   `merge_overlays` helper `CloudAdapter::list` already used for its live/active/in-flight/ends
   sections (extracted, not rewritten, so none of PR fix-list's review history is lost). No
   network call. `CloudAdapter::reconcile` calls `RecordingCloud::list` (the network path) once,
   upserts every row it returns, tells `on_job` about the ones that changed, and then answers
   only the rows the index still shows as visible (`list` itself knows nothing about `hidden`:
   filtering here is what keeps a removed row from reappearing the moment the volume is read
   again). The brain sets `reconcile: true` on the hub's first open (`recordings.ready`) and once
   more after a recording reaches `done` or `failed` (`onJob`); every other `recording_list` call
   (after Retry, after Upload all, the catch-up poll) is the fast path.
5. **`upsert` never writes `hidden`.** Its `ON CONFLICT` clause does not name the column, so a row
   a user removed stays removed even after a later reconcile re-upserts it from the same
   unchanged volume state. This is the one invariant every index test asserts directly.
6. **Delete is local and instant, for every row the user sees, with no network call at all.**
   `CloudAdapter::delete` no longer reads a job over the network, no longer requires proof that
   nothing uploaded, and no longer special-cases `on_device_only`: it removes the local audio and
   sidecar if they exist (best effort — a file that cannot be removed is not an error), always
   marks the row `hidden`, and refuses only when the take is recording right now
   (`LocalAudio::active`) or a worker is uploading it right now (`CloudAdapter::is_live`), with
   the existing `recording_busy` error. A copy already in the volume is never touched. The view's
   action is renamed Remove; its confirm copy is "Removes it from this Mac and from this list. A
   copy already in Databricks stays." `rowCanDeleteOf` (`src/shared/recording-view.ts`) now shows
   it for every row except one that is `recording` or `uploading` (the two states a busy refusal
   can come from), rather than only a device-only or zero-parts-failed row.
7. **Launch cleanup removes a `rec-*` temp dir with no audio parts and no sidecar outright, and
   one with only a stale sidecar (older than 24 h, with no row in the index for its name) too.**
   (`recording_cloud_host::cleanup_stale_temp_dirs`.) The age gate exists only for the
   sidecar-only case, so a take that just started (sidecar written, first part not flushed yet)
   is never touched; a dir with any audio is never touched either way. It runs synchronously, at
   `install`, before the crash-resume scan's background thread starts, so the two passes never
   race over the same files.
8. **Settings → Wipe clears the table too, in its own write, not inside the per-MOG
   transaction.** `LocalWipeService::with_recording_index` (a new builder method, the same shape
   as `with_expiry`/`with_agent_hook`) wires in the index; `wipe` calls
   `RecordingIndexStore::wipe_before`/`wipe_everything` once per call, device-wide, after the
   MOG-scoped transaction commits (calling the index's own `with_conn` from inside that
   transaction's already-held connection lock would deadlock on the single-connection store).
   `consent_host::install_with` builds `SqlRecordingIndex` once, on the same store, and manages it
   as `Arc<dyn RecordingIndexStore>` so `recording_cloud_host::install` can wire the same instance
   into both `CloudAdapter` and the cleanup pass.
9. **First run (an empty index): the hub's first `reconcile: true` read shows the volume's
   existing rows at once and writes them into the index, so every later open is instant.** No
   separate "first run" code path; decision 4 already does this for free.

## Consequences

- Contract surface touched: one new migration
  (`crates/mog-core/src/local_store/migrations/0005_recording_index.sql`), one new optional field
  on `RecordingListArgs` (`reconcile`, `crates/mog-core/src/ipc.rs` and `src/shared/ipc.ts`), and
  the regenerated `src-tauri/schemas/ipc-commands.json`. `RecordingSummary` itself is unchanged:
  `hidden` is purely a local index concept, never serialized to the brain.
- `RecordingCloud` gains two methods, `list_fast` and `reconcile` (not contract files:
  `crates/mog-core/src/recording/mod.rs` says so in its own header). `CloudAdapter` is the only
  implementor in the repo.
- `recording_cloud::contract::LocalAudio::never_uploaded` is no longer called from
  `CloudAdapter::delete` (decision 6 removed the "prove nothing uploaded" rule it served). The
  trait method and its implementations stay, unused for now, rather than touch three more files
  to remove a still-plausible future seam.
- `CloudAdapter::delete`'s reconciliation with a reconcile that still sees the same row on the
  volume is covered directly
  (`rec_component_delete_hides_a_failed_upload_row_and_a_later_reconcile_does_not_undo_it`,
  `crates/mog-core/tests/recording_cloud_contract.rs`).
- The parallel, capped (about 4) `job.json` fetch for reconcile's volume read is not implemented
  in this change: `reconcile` still reads job files serially, as `CloudAdapter::list` always has.
  It runs rarely (at most once per hub open, plus after a terminal status), so the dominant,
  user-visible win — every other `recording_list` call makes no network call at all — is
  unaffected; a parallel reconcile fetch is a follow-up, not blocked by anything here.
