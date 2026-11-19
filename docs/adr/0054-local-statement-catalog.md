# ADR 0054: The fixed local statement catalog and `mem_local_exec`

- Status: Accepted
- Date: 2026-11-16
- Issue: #37 (MEM-34a, MEM-34b, MEM-39a, MEM-41, MEM-01)

## Context

MEM-34a says the Rust core must run only statements from a fixed catalog. MEM-34b says the brain
must never send SQL. The `mem_local_exec { stmtId, params } -> { rows }` command is in the
contract, but the issue does not say how the catalog binds params, what a write returns, or who
may run deletes. The host routing in `src-tauri/src/ipc.rs` is a contract file.

## Decision

1. The catalog lives in `crates/mog-core/src/local_store/catalog.rs`, in `STATEMENTS`. Each entry
   has an ID, one SQL statement, a typed list of named params, a kind (query or write) and an
   access (`Brain` or `Core`).
2. Params are named: the JSON key `mogId` binds to `:mogId`. `validate` refuses unknown keys,
   missing required keys and values of the wrong type or size. A `Code` param accepts only
   `a-z0-9_.`, so enum fields and error codes hold no content. An `FtsTerms` param keeps only
   the words of the text and quotes each one, so user text never becomes FTS5 syntax.
3. Every entry has `mogId` as a required UUID and passes `require_mog_id_filter` (MEM-01). Each
   upsert has `WHERE <table>.mog_id = excluded.mog_id` in its conflict path, so a row of another
   MOG is never overwritten.
4. A query answers `{ rows: [ {column: value} ] }`, with camelCase column aliases. A write answers
   `{ rows: [ {changes: n} ] }`.
5. `Access::Core` entries run only from Rust. `mem_local_exec` refuses them. The Wipe statements
   (`mem.wipe.*`) are Core, because only R14 WipeService deletes by age (U9). Forget statements
   are Brain, because T10 runs Forget on a user action.
6. `local_store::mem_local_exec(store, args)` is the IPC side. It maps every refusal to a fixed
   code: `invalid_args` for an unknown or Core ID and for bad params, `not_implemented` for a
   missing store, and `host_failed` for a store failure. No error has SQL, a value or content.
7. The IDs follow the remote catalog (4.5) where the local schema supports them. The local recall
   is `mem.recall.fts`; the vector recall comes with #199.
8. The TS side (`src/brain/components/memory-store.ts`) lists the Brain IDs in `LOCAL_STMT_IDS`.
   A Rust test checks that this list equals the Brain entries. `LocalMemoryService` takes a
   `LocalMemoryStore`, so tests pass a double (MEM-39a).
9. The host routing is contract-change #426 (landed). `src-tauri/src/ipc.rs` sends
   `mem_local_exec` to `mog_core::local_store::mem_local_exec` over the one managed
   `Arc<SqlCipherLocalStore>` (opened by `consent_host::install`, shared with R14), on a worker
   thread. The host refuses `pinned: true` with `invalid_args`: no user-action proof crosses
   from the brain yet (MEM-28b, `TODO(#574)`).

## Consequences

- The consent statements of #63 (`consent::STATEMENTS`) can join the catalog later as `Core`
  entries. Their device-wide statements need a scope flag next to `access`, because they have no
  `mog_id`.
- Feature issues #38, #114, #199, #123, #124 and #102 can change their statements here. Each
  change still needs the MEM-01 and parity tests to pass.
- MEM-41's live part (AT-31 and AT-32 on a workspace without Lakebase) is #427 (`needs:human`).

## Amendment (#431, ADR 0071)

- The consent statements joined the catalog as the separate entry set `CONSENT`. Each entry is `Access::Core` and has a `Scope` (`Mog` or `DeviceWide` with a reason). They are not in `STATEMENTS`, so `exec` and `mem_local_exec` never run them.
