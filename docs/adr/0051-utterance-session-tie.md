# ADR 0051: Migration 0002 ties an utterance to the MOG of its session

- Status: Accepted
- Date: 2026-11-16
- Issue: #410 (for #384, item 2)
- Owner: contract-lead

## Context

MEM-01 asks for one namespace per MOG. In migration 0001, nothing stops an utterance of MOG A from pointing at a session of MOG B. Appendix A asks for `UNIQUE (session_id, seq)` on `utterance`. Migration 0001 has `UNIQUE (mog_id, session_id, seq)`, because every explicit index starts with `mog_id` (the MEM-01 index test). Migrations are forward only (MEM-35a) and never delete user content (U9).

## Decisions

1. **Triggers, not a composite foreign key.** SQLite cannot add a foreign key to a table that exists. A foreign key from `utterance (session_id, mog_id)` to `session` needs a table rebuild and a copy of all utterances. Migration 0002 adds four `BEFORE` triggers instead. Each trigger only refuses the write with `RAISE(ABORT)`:
   - `utterance_session_mog_bi`: an insert whose `mog_id` is not the `mog_id` of its session;
   - `utterance_session_mog_bu`: an update of `session_id` or `mog_id` that breaks the tie;
   - `session_mog_with_utterances_bu` (`UPDATE OF session_id, mog_id`): a change of a session's `mog_id` while the session has utterances, a rename of a session that has utterances, and an update that takes the ID of a session whose utterances have another MOG. The last case covers `UPDATE OR REPLACE` with foreign keys off or deferred (PR #417 second review);
   - `session_mog_with_utterances_bi`: an `INSERT OR REPLACE` of a session that has utterances of another MOG. A REPLACE deletes and inserts, so no UPDATE trigger runs. With foreign keys off or deferred, the delete would pass (PR #417 review).
2. **No new unique index.** With the tie, all utterances of a session have the session's `mog_id`. So the 0001 index `UNIQUE (mog_id, session_id, seq)` already makes `(session_id, seq)` unique. A new index would break the MEM-01 rule that every index starts with `mog_id`. A test inserts the same `(session_id, seq)` with the same MOG and with another MOG; both fail.
3. **Upgrade.** A test opens a store with 0001 and data, and 0002 then applies with the data kept.
4. **Existing rows.** The triggers check new writes only. Bottle has no released database. A row that breaks the tie in a test database stays readable. The store never deletes it (U9).

## Consequences

- A refused write surfaces as the store's content-free error. The trigger messages are fixed strings with no row content.
