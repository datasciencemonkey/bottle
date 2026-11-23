# ADR 0112: Local memories in an OpenClaw-style virtual folder on the SQLCipher store

- Status: accepted (the owner approved slice 1 on 2026-11-22)
- Issue: local memory plan (PR #623), slices 0 and 1
- Date: 2026-11-22
- Owner: impl-memory-s1

## Context

The owner wants Bottle to keep memories on the device. The owner chose the encrypted store
(option c, Q1), then the decision "OpenClaw layout, Bottle safety". The OpenClaw docs describe:

- `USER.md` for stable preferences, replaced in place;
- `MEMORY.md` for curated, durable facts and decisions, loaded at session start;
- `memory/YYYY-MM-DD.md` for daily notes, indexed for search but not put into each turn;
- a "memory flush" that saves key facts before a conversation is summarized;
- "dreaming", a background job that promotes items from the daily notes into `MEMORY.md`;
- hybrid search over the files with a derived SQLite index.

Bottle tells the user that local history is encrypted (ADR 0105). So the files cannot be plain
files on disk.

## Decisions

1. **The layout.**
   - `/memories/USER.md`: the shared preferences across MOGs.
   - `/memories/<mog-slug>/MEMORY.md`: the lasting memories of one MOG (facts, definitions,
     decisions, open questions).
   - `/memories/<mog-slug>/memory/<YYYY-MM-DD>.md`: the daily notes of one MOG. Each item is one
     section of the note of its day.
   - `<file>#<slug>` names one item.
2. **One row is one item.** Each item is one encrypted `memory_item` row. A file is a rendering of
   rows. Recall, Forget, Wipe and Undo work on one item. The disk holds only the encrypted store.
3. **The layer.** Each row has a layer: `user` (USER.md), `durable` (MEMORY.md) or `daily`.
   - A daily row has a `note_day`. The core computes it from the write time and the device time
     zone.
   - A kept Genie answer stays `daily`, because it belongs to one workspace and one day (D60).
   - A preference is always `user`. USER.md needs the reserved shared MOG id (slice 2).
4. **The grammar is fixed.** Rust accepts only these paths: `/memories`, `/memories/USER.md`,
   `/memories/<mog-slug>/MEMORY.md` and `/memories/<mog-slug>/memory/<date>.md`, each with an
   optional `#<slug>`. The date must be a real calendar date. Rust refuses each other path.
5. **The slugs and the day.**
   - The core computes the slug and the `note_day` (the `Slug` and `Day` params). A caller that
     sends either gets `invalid_args`. The day is today's local calendar day on the core's own
     clock and device offset, never the caller's time; tests inject the clock.
   - An item slug is the lower-case ASCII form of its title (else its summary, else its body), 64
     characters or less. An edit computes the slug again from the new title.
   - A clash in the same file adds `-<first 6 hex of the item id>`. If that clashes too, the
     slug is its first 39 characters and the full 32-hex id.
   - A unique index on `(mog_id, layer, note_day, slug)` keeps one address for one row.
   - A MOG slug is the slug of the MOG name. A clash adds `-<first 6 of the MOG id>`.
6. **The renderer.**
   - A file has YAML frontmatter (file, mog, workspace host, profile, day, items, updated). The
     values are JSON strings, so user text cannot add a key.
   - Each item is a section: a heading, a hidden metadata line with codes, ids and times only, a
     visible line (time, source, origin, pin, "New"), the body and the tags.
   - MEMORY.md groups its items by kind.
   - The user audience (hub, export) sees each item in use (`active`, `stale`, `quarantined`),
     with a mark: Pinned, New, Quarantined, Not used lately, Released.
   - The model audience sees only `active` items, and never a quarantined one (SEC-24b).
     MEMORY.md for a model has at most 200 items and 25 KB. One view has at most 16,000
     characters.
7. **Migration 0003.**
   - It rebuilds the table in one transaction and keeps each `rid`, so the FTS5 and vec0 rows
     stay valid.
   - It adds the columns: title, body, tags, origin, layer, note_day, slug, profile, the
     recording link, the last use and use count, `stale_at`, the remove fields, `rating`
     (-1, 0, 1), `reviewed_at` and `released_at` (the user released a quarantine, ADR 0113).
   - FTS5 covers the title, the summary and the body.
   - Each old row becomes `daily`, in the note of the day it was first seen, with origin `legacy`.
   - Each old `entity` row becomes a `fact` with the tag `legacy-entity`. If it is not
     quarantined it becomes `stale`, so recall does not use it until the user rates it up.
   - The table refuses the kinds `entity` and `person`, the status `proposed`, an unknown
     origin, and a user origin (`pin`, `added`) on a row that is not pinned.
8. **The read path.** `mem.item.list` and `mem.file.get` are Rust handlers over fixed
   MOG-scoped statements and this renderer. The MOG folder names come from the device-wide
   `mem.mog.names` (ids and names only). A MOG reads only its own folder and USER.md.
9. **Export** (a later slice) writes real `.md` files in exactly this layout. A warning first says
   that the files have no encryption (MEM-38a, MEM-38b).

## Later slices (recorded here, not built now)

- **Dreaming** is the consolidation job (ADR 0114). It promotes qualified daily items into
  MEMORY.md as `active` memories, with no approval step.
- **Fade.** A daily item that the user did not rate fades after about 30 days. It goes to
  "Recently removed" with Restore, then the purge rules of the plan apply.
- **Flush.** Before a Genie thread is summarized (the thread composer and the agent context),
  Bottle saves its key facts into today's note.
- **Search.** OpenClaw's hybrid search maps to FTS5 first (slice 2). Embeddings come later.

## Consequences

- R-1 to R-6 of the plan hold with no new crypto.
- The user cannot open a live file in an outside editor. Export is the way out and the backup.
- Code: `local_store/memory_files.rs`, migration 0003, `local_store.rs` (the handlers), and the
  catalog ids `mem.item.list` and `mem.file.get`.
