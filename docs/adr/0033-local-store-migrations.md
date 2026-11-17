# ADR 0033: Local store migrations (forward-only runner, migration 0001, contract path)

- Status: Accepted
- Date: 2026-11-16
- Issue: #35 (MEM-35a, MEM-08a..f, MEM-01, MEM-21a, MEM-33a, U9)
- Approval: migration writes approved by the user (via the orchestrator session), 2026-11-16.

## Context

#35 builds the encrypted local memory store (B2). Its task list includes the schema, but nothing on
main had migrations yet, and "schema migrations" are contract files (dispatch prompt section 3.6).
Appendix A gives the Lakebase DDL and says the local mirror uses the same tables. Some Appendix A
details conflict with binding decision U9 (no automatic deletion of user content) and with MEM-01
(every statement filters by `mog_id`).

## Decision

1. #35 seeds the migration framework: `crates/mog-core/src/local_store/migrate.rs` (runner) and
   `crates/mog-core/src/local_store/migrations/0001_local_mirror.sql`. From now on the
   `migrations/` folder is a **contract path**. A schema change is a new numbered file
   (`0002_*.sql`, ...), added through a `type:chore` + `contract-change` issue. An applied file is never
   edited.
2. The runner is forward-only (MEM-35a). It creates `schema_migrations (version, applied_at,
   app_version)`, applies each newer migration in its own transaction, and records it. If the database
   has a version above the binary's newest, `open` returns `StoreSchemaNewer` and leaves the file as it
   is: no downgrade and no delete. No file may look like a down migration; a test checks this.
3. Migration 0001 mirrors Appendix A with this mapping: `uuid` to TEXT, `timestamptz` to INTEGER epoch
   ms, `jsonb` and arrays to JSON TEXT with `json_valid` checks, `boolean` to INTEGER 0/1. It adds:
   - FTS5 `memory_item_fts` and `utterance_fts` (external content, `rowid` = the table's `rid`,
     tokenizer `porter unicode61` to match Appendix A's `to_tsvector('english', ...)` stemming)
     with triggers that keep them in step (MEM-08d);
   - vec0 `memory_item_vec_256` and `memory_item_vec_1024`, `int8[d]`, `distance_metric=l2`, with
     `mog_id` and `embedding_model` as partition keys, plus an f16 `embedding` BLOB cache on
     `memory_item` for the exact re-rank (SP-21, MEM-08e, MEM-21a). A trigger removes a deleted item's
     vectors;
   - `outbox` (`kind` in `embed`, `sync`) for embedding requests and v1 sync (MEM-08f);
   - `kv` with a CHECK that refuses keys naming a token, secret, password, credential or cookie;
   - `final_answer_md` on `genie_exchange` (local only, D60).
4. Differences from Appendix A, and why:
   - **U9 wins over the D36 retention columns.** The local mirror has no `*_retention_days`
     columns, no `sync_state.last_retention_run_at`, no expiry indexes and no `CHECK (pinned OR expires_at IS NOT NULL)`. `expires_at` stays
     as a nullable mirror column for the v1 sync, and no local job reads it. Only Settings → Wipe
     (#124) deletes user content. The one expiry index kept is `consent_expiry`: it drives the
     13-month consent re-prompt (#63) and deletes nothing.
   - **MEM-33a: no delete cascades.** Appendix A cascades deletes (`ON DELETE CASCADE`), which on
     Lakebase served the D36 purges. Locally a cascade would let an age-based Wipe of an old exchange
     or session delete a recent memory item, with its FTS5 and vec0 rows ("each option deletes only
     the matching rows"). So every reference to a MOG, and `utterance.session_id`, is `RESTRICT`, and
     the optional pointers `genie_exchange.session_id` and `memory_item.source_exchange_id` are
     `SET NULL`. The WipeService (#124) deletes each table by its own age column, children before
     parents. A session row goes only when none of its utterances remains. SQLite cannot change a
     foreign key without a table rebuild, so this is fixed in 0001 before it ships.
   - **MEM-01:** every table has `mog_id`, and every explicit index starts with it.
     `consent_record` gets a nullable `mog_id` (NULL = a device-wide notice consent).
     `UNIQUE (session_id, seq)` becomes the index `utterance_session_seq (mog_id, session_id, seq)`.
   - Postgres-only parts (HNSW, GIN, `tsvector`, `touch_row`, RLS, `owner_principal DEFAULT
     current_user`) map to FTS5 and vec0 or are left out. `owner_principal` is a nullable column.
     The GIN indexes `memory_item_entities` (`entity_keys`) and `memory_item_sessions`
     (`source_session_ids`) have no local equivalent. A lookup by entity key or source session
     (Forget a session, #123) is a `json_each` scan of one MOG's items. That is acceptable at the
     MVP size (at most 50k items). Add a side table in a later migration if it is too slow.
   - `consent_record.mog_id` is NULL for a device-wide notice consent. A statement that reads those
     rows cannot pass `require_mog_id_filter` (`mog_id = ?`). #37 and #63 must add a named catalog
     statement for device-wide consent, and must not weaken the guard.
   - `memory_item` and `utterance` get `rid INTEGER PRIMARY KEY` for the FTS5 and vec0 rowids; their
     UUID stays `UNIQUE`.
5. The int8 quantizer (`vec_codec::quantize_int8`) clamps to [-90, 90]. In sqlite-vec 0.1.9 the NEON
   `l2_sqr_int8_neon` widens to i16, subtracts, then squares with `vmulq_s16(diff, diff)`, which
   wraps when |diff| >= 182. The distance is then NULL, and such rows rank first. A test pins the
   boundary: a difference of 180 is exact, and 182 is NULL on aarch64. If an upgrade fixes it, that
   test flips and the clamp can return to 127. TODO(#269): SP-21 measured recall at scale 127, so
   measure it again at 90.
6. The in-memory key double `MemoryKeyVault` exists only under `cfg(test)` or the `test-support`
   feature, so a release build can reach only the OS key store. A lost key returns `StoreKeyLost`
   and leaves the file (U9). MEM-09a (start a new empty store) and MEM-09b (tell the user) are
   MemoryService's work in #190; MEM-09a conflicts with U9 and with MEM-08a's one file, and #190
   must settle that.
7. `open` runs the migrations, then one KNN on `memory_item_vec_256` so that the first recall finds
   warm pages (SP-21).

## Consequences

- Downstream memory issues (#37, #38, #63, #102, #114, #123, #124, #199-#202) build on this schema.
- A new column or table needs a contract-change issue and migration 0002 or later.
- The v1 Lakebase sync maps these columns back to Appendix A, including the nullable `expires_at`.
- Windows: the vendored OpenSSL gives LNK4099 under `/WX`. `src-tauri/build.rs` ignores exactly that
  warning (ADR 0036 amendment).
- The requirements text should say that U9 removes the local retention columns and purge indexes
  (comment on #35).

## Amendment (2026-11-17, #449): ids for the exchange columns

The 4.7 `GenieExchange` gave no value for the `genie_exchange.session_id`, `workspace_id` and `space_id`
columns. Then Forget by session (MEM-32a) and "Stop and delete this session" (SEC-14a) cannot find the
exchanges of a session, and D37 recall cannot check the workspace and space of an answer.

- `GenieExchange` gets 3 optional fields: `sessionId` (the R10 session UUID), `workspaceId` (the numeric
  workspace id as a decimal string, the `?o=` value) and `spaceId` (the Genie space id).
- They are added by declaration merging after the 4.7 block in `src/shared/interfaces.ts`, as in ADR 0007,
  so the 4.7 block stays a verbatim copy.
- `workspaceId` is a string on the TS side, the same as `MemoryDraft.workspaceId`. The writer (#38) converts
  it to the `INTEGER` column. No schema change. No Rust type has `GenieExchange`, so no Rust mirror changes.
- The requirements should add the 3 fields to 4.7 `GenieExchange` (comment on #449).
