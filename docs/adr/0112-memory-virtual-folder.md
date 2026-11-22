# ADR 0112: Local memories as a virtual folder on the SQLCipher store

- Status: proposed (draft; the contract gate of slice 0 is open)
- Issue: local memory plan (PR #623), slices 0 and 1; #574
- Date: 2026-11-21
- Owner: impl-memory-s1

## Context

The owner wants Bottle to keep memories on the device. The plan compared three homes: plain
Markdown files, encrypted files, and the existing SQLCipher store. The owner chose the store
(option c, Q1). The user, the hub, the export and a later agent memory tool must still see
memories as files. Bottle already tells the user that local history is encrypted (ADR 0105).

## Decisions

1. **One row is one memory.** Each memory is one `memory_item` row in `memory.db`. No memory text
   is in a real file. The disk holds only the encrypted store.
2. **The folder is virtual.** Rust computes `/memories/...` from the rows. The grammar is fixed:
   `/memories/(shared|<mog-slug>)/<kind-folder>/<slug>.md` and `/memories/<folder>/index.md`.
   Rust refuses each other path, so no path can point outside the store.
3. **The kinds.** `fact`, `preference`, `definition`, `decision`, `open_question` and
   `genie_answer`. The kind folders are `facts`, `preferences`, `definitions`, `decisions`,
   `open-questions` and `answers`. There is no `person` kind (owner, Q7).
4. **The slugs.** A memory slug is the lower-case ASCII form of its title, with hyphens, 64
   characters or less. A clash in the same MOG and kind adds `-<first 6 hex of the item id>`. A
   unique index on `(mog_id, item_type, slug)` keeps one path for one row. A MOG folder slug is the
   slug of the MOG name. A clash with another MOG, or with `shared`, adds `-<first 6 of the MOG id>`.
5. **The file format.** YAML frontmatter, then the Markdown body. Rust computes the frontmatter
   from the columns and never stores it. Every string value is a JSON string, so user text cannot
   add a key. The field list and its order are those of the plan, section 5.2.
6. **The index.** One line for each memory in use: pinned first, then by the last use. At most 200
   lines and 25 KB. A last line names the memories that it leaves out.
7. **Migration 0003.** A table rebuild in one transaction. It keeps each `rid`, so the FTS5 and the
   vec0 rows stay valid. It adds the columns of the plan (title, body, tags, origin, slug, profile,
   recording link, last use, use count, stale and removed fields). Each old `entity` row becomes a
   `fact` with the tag `legacy-entity`. An `active` one becomes `proposed`.
8. **Export** (a later slice) writes real `.md` files to a place that the user picks, after a
   warning that the files have no encryption (MEM-38a, MEM-38b).

## Consequences

- R-1 to R-6 of the plan hold with no new crypto: one transaction covers the row, the FTS row,
  the embed job and the outbox.
- The user cannot open a live memory in an outside editor. Export is the way out.
- If the keychain key is lost, the memories are lost (MEM-09a). Export is the backup.
- Code: `local_store/memory_files.rs` (grammar and renderers), migration 0003, and the catalog ids
  `mem.item.list` and `mem.index.get`.
