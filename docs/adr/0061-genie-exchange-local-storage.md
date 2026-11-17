# ADR 0061: Local storage of Genie exchanges

- Status: Accepted
- Date: 2026-11-17
- Issue: #38 (MEM-25a, MEM-26, with U8, U9, D60, MEM-01 and MEM-32a)

## Context

MEM-25a says the local store must keep each Genie exchange with its summary, key numbers and
pointer. MEM-26 says the full `final_answer_md` must stay until the user selects Wipe, and that
"older than 30 d" removes it. Migration 0001 has the `genie_exchange` table, and #37 (ADR 0054)
added `mem.exchange.upsert` and `mem.exchange.get`. The issue does not say who makes the
summary, how a Wipe ages an exchange, or how Forget finds the rows that cite an exchange. The
contract type `GenieExchange` has no summary, session, workspace id or space id.

## Decision

1. T10 `LocalMemoryService.recordExchange` writes the row with `mem.exchange.upsert`. It takes a
   clock, an id source and a device id as options, so tests use a simulated clock.
2. The summary is distilled in T10 (`exchange-summary.ts`, "writer distils it" in the data
   table). The MVP distiller is local and deterministic: Markdown to plain text, then 600
   characters or less, cut at a word. It computes and rounds nothing, so numbers stay verbatim.
   An SLM distiller can replace it later with no schema change.
3. The key numbers are stored as the bridge gives them, at most 32. The pointer is the deep
   link plus backend, profile, host, conversation and response ids.
4. The sent question is stored only as its SHA-256 (`question_sent_sha256`), because it can hold
   a context preamble. `question_user` is stored for the home workspace only.
5. A non-home exchange stores the pointer and the full answer only. The table CHECK refuses a
   summary, key numbers or the question for it (D60).
6. The upsert keeps the answer parts once written. A later write for the same `request_id` with
   no answer (a late event, a resume) never clears `final_answer_md`, the summary, the key
   numbers, the deep link or `completed_at`. `row_hlc` never goes back. No statement sets
   `expires_at`, and no trigger touches the table (U9).
7. Wipe (`mem.wipe.exchanges_before`, Core) ages an exchange by `asked_at`, the time the user
   asked, not by the local insert time `created_at`. `asked_at` comes from the T10 clock, so the
   "400 d, then Wipe older than 30 d" test runs on a simulated clock, and the `ge_mog_time` index
   serves the delete. R14 WipeService (#124) owns the rest of Wipe.
8. Two brain reads help Forget (#123) collect dependents before a delete, because the links are
   `SET NULL`: `mem.exchange.dependents` (memory items that cite an exchange) and
   `mem.exchange.by_session` (the exchanges of a session, also for SEC-14a). Both filter by
   `mog_id` (MEM-01). `mem.exchange.get` also returns `sessionId`, `splitGroupId` and
   `questionUser`.
9. `GenieExchange` gets optional `sessionId`, `workspaceId` and `spaceId` through
   contract-change #449. Until then T10 accepts an internal `ExchangeRecord` type
   (`TODO(#449)`).

## Consequences

- The caller registers the MOG row (`mem.mog.upsert`) before the first exchange, because
  `genie_exchange.mog_id` references `mog`.
- The host routing of `mem_local_exec` is still contract-change #426 (`TODO(#426)`), so the app
  answers `not_implemented` until it lands. The tests run on the Rust store and a TS double.
- AT-16 is a live test. Its store part (400 d, then Wipe) is covered here on a simulated clock.
