# ADR 0068: The outbox, the atomic item write and the offline embedding queue

- Status: Accepted
- Date: 2026-11-17
- Issue: #102 (MEM-08f, MEM-10a, MEM-19d, with ARC-F7, AT-04, AT-18, SP-21, MEM-01 and U9)

## Context

MEM-10a says MemoryService must write the local row and its outbox row in one SQLite
transaction. MEM-19d says embedding requests wait in the outbox when the device is offline and
run when it is online again. MEM-08f says a queued request survives a restart. Migration 0001 has
the `outbox` table. #37 (ADR 0054) added `mem.outbox.*`, but `mem_local_exec` runs only one
statement for each call. So the brain could not write an item and its job in one transaction.
The issue does not say where the drain runs, what it does on a failure, or how it learns that
the device is online. The embedding call itself is #198. The `net.status` event has no payload
type yet.

## Decision

1. **Atomic groups in the fixed catalog.** `catalog::Group` is a fixed list of catalog write
   statements. `SqlCipherLocalStore::exec` runs a group in one `BEGIN IMMEDIATE` transaction.
   The brain sends one `stmtId` and one params object, the same as for a statement, so the IPC
   contract does not change. The params are checked against the group before the transaction
   starts. The answer has one `{changes: n}` row for each step.
2. **`mem.item.write`** is the first group: `mem.item.upsert`, then the Rust-only step
   `mem.outbox.enqueue_item_embed`. The step adds an `embed` job only if the item is in the same
   MOG (MEM-01), has no embedding, and has no embed job waiting. So a second mention adds no
   duplicate (SP-21), and an item id of another MOG adds nothing. The job is a pointer:
   `ref_id` is the item id, and `payload_json` is `{}`. It holds no item text.
3. **`mem.outbox.due` needs a `kind`.** The embed drain reads only `embed` rows, and the v1
   SyncWorker reads only `sync` rows.
4. **Every item write queues its embedding**, offline or online. One path is easier to test,
   and the job runs at once when the device is online. This is the SP-21 design.
5. **The drain runs in T10** (`embedding-queue.ts`), because the embedding call goes through
   `model_invoke` from the brain. It reads FIFO batches of 100 for each MOG. It runs at start,
   when the network goes online, and after each item write. One drain runs at a time.
   - `done` deletes the outbox row.
   - `offline` or `unavailable` stops the drain. The job and the rest stay, with no attempt
     counted.
   - `failed` sets `next_attempt_at` with a backoff of 30 s, doubled, up to 5 min (the MEM-11b
     curve). The error is a fixed code. Free text becomes `embed_failed`. No job is dropped.
   - The drain deletes outbox rows only. It never deletes or changes an item (U9).
6. **Delivery is at least once.** A crash after the embedder stores a vector and before
   `mem.outbox.done` runs the job again. The `ItemEmbedder` contract says a second run must be
   safe. The SP-21 delete-then-insert by rowid does that.
7. **Interfaces with doubles.** `ItemEmbedder` and `NetworkStatus` are interfaces. The defaults
   are `UnwiredItemEmbedder` (`TODO(#198)`), which answers `unavailable`, and
   `UnwiredNetworkStatus` (`TODO(#466)`), which says offline. So until those land, jobs wait and
   nothing leaves the device.
8. **`write()` stays with #114.** `LocalMemoryService.storeItem()` is the one item write path.
   #114 adds the provenance rules and the quarantine, then calls `storeItem()`.
9. **`synchronous = NORMAL` stays** (SP-21). On a power loss, the last commits can be lost. They
   are lost as whole transactions, so an item never loses its job and a job never loses its
   item. A test cuts the WAL at every frame boundary and inside frames to check this.

## Consequences

- A crash or a kill between the two steps loses nothing: the transaction did not commit, and the
  caller got no answer, so it writes again (AT-18 test, with a child process that exits inside
  the transaction).
- If an item's text changes after it has an embedding, `mem.item.write` adds no new job. #198
  owns the embedding columns and must clear the embedding when the text changes.
- The `net.status` payload type is contract change #466. When it lands, the brain wires
  `bus.on('net:status', ...)` to a `NetworkStatus`.
- `mem_local_exec` still has `TODO(#426)`: the app host does not route it yet.
- The live run (network off on a device, a real embedding endpoint) is part of #175 and #198.
