# ADR 0033: Local store migrations (forward-only runner, migration 0001, contract path)

- Status: Accepted
- Date: 2026-11-16
- Issue: #35 (MEM-35a, MEM-08a..f, MEM-01, MEM-21a, U9)
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
   - FTS5 `memory_item_fts` and `utterance_fts` (external content, `rowid` = the table's `rid`)
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
     columns, no expiry indexes and no `CHECK (pinned OR expires_at IS NOT NULL)`. `expires_at` stays
     as a nullable mirror column for the v1 sync, and no local job reads it. Only Settings → Wipe
     (#124) deletes user content. The one expiry index kept is `consent_expiry`: it drives the
     13-month consent re-prompt (#63) and deletes nothing.
   - **MEM-01:** every table has `mog_id`, and every explicit index starts with it.
     `consent_record` gets a nullable `mog_id` (NULL = a device-wide notice consent).
     `UNIQUE (session_id, seq)` becomes the index `utterance_session_seq (mog_id, session_id, seq)`.
   - Postgres-only parts (HNSW, GIN, `tsvector`, `touch_row`, RLS, `owner_principal DEFAULT
     current_user`) map to FTS5 and vec0 or are left out. `owner_principal` is a nullable column.
   - `memory_item` and `utterance` get `rid INTEGER PRIMARY KEY` for the FTS5 and vec0 rowids; their
     UUID stays `UNIQUE`.
5. The int8 quantizer (`vec_codec::quantize_int8`) clamps to [-63, 63]. sqlite-vec 0.1.9 built with
   NEON returns a NULL L2 distance when a component difference leaves the i8 range, and such rows
   rank first. TODO(#269): measure SP-21 recall again with this scale.
6. `open` runs the migrations, then one KNN on `memory_item_vec_256` so that the first recall finds
   warm pages (SP-21).

## Consequences

- Downstream memory issues (#37, #38, #63, #102, #114, #123, #124, #199-#202) build on this schema.
- A new column or table needs a contract-change issue and migration 0002 or later.
- The v1 Lakebase sync maps these columns back to Appendix A, including the nullable `expires_at`.
- The requirements text should say that U9 removes the local retention columns and purge indexes
  (comment on #35).
