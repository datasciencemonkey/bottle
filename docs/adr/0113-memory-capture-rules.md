# ADR 0113: Memory capture rules

- Status: accepted (the owner approved slice 1 on 2026-11-22)
- Issue: local memory plan (PR #623), slice 1; #574 (MEM-28b)
- Date: 2026-11-22
- Owner: impl-memory-s1

## Context

The owner wants minimal friction, maximum use, and the agency to change or remove each memory.
Memories save at once, with no approval step: "it is local and the user can ask the agent to
remove memory as well anyway". A memory must still never hold a secret, and instruction-like text
must never reach Genie or the agent. Before this ADR the host refused every pin, because no proof
of a user action crossed from the brain (#574).

## Decisions

1. **Every write saves at once.** This covers the card, the voice command, the hub, AI writers
   and the agent. There is no `proposed` status. Each new item goes into today's note of its MOG
   (ADR 0112, MEM-43).
2. **The secret check refuses the write** (MEM-44).
   - R10 checks the title, the summary, the body and each tag of each write and each edit: the
     7 rule-7 patterns and the memory patterns (password forms, GitHub, Slack, API and cloud
     keys).
   - It also checks the NFKC form without the default-ignorable code points, and the fields run
     together, so a token split across two fields is found.
   - The check is best effort against a hostile brain. It finds the known credential shapes, not
     every way to spell a secret.
   - A match raises `CoreError::SecretRefused` before any SQL runs. The IPC answer is
     `secret_refused`. R10 stores nothing, and the error holds no text. The SQL also checks the
     core param `secretCheck = 0` as a second guard.
3. **Instruction-like text is saved as `quarantined`** (SEC-24a, ADR 0089; owner ruling
   2026-11-21, "Quarantine, thumbs up releases").
   - R10 checks the title, the summary, the body and the tags together. On a hit the item is saved as
     `quarantined`. The core check is the only one on a pin: the brain sends no flag.
   - The hub shows the item with a mark and its body as plain text. Genie and the agent never see
     it (SEC-24b), and it gets no embed job.
   - **Release.** A thumbs up from the user in the hub releases it: status `active`, rating 1,
     `reviewed_at` and `released_at` are set, and the embed job is queued (SEC-24c). The hub then
     shows "You released this".
   - **The release is bound to what the user saw.** The ticket keeps its wall-clock mint time,
     and the core adds it as `mintedAt` (a caller that sends it is refused). The rate changes
     only a row with `updated_at < mintedAt`; every writer sets `updated_at` in ms on the SQLite
     clock. So a write between the click and the release makes the release change nothing.
   - An automatic write never rewrites the words of a quarantined row either.
   - **An edit re-quarantines** (MEM-46). An edit runs the check again. Instruction text sets
     `quarantined` and clears `released_at`, also on an item that the user released before.
     Clean text sets `active`, also on a quarantined or stale item.
   - The later re-scan of the consolidation job (ADR 0114) must not quarantine again an item
     that the user released, unless its text changed after `reviewed_at`.
4. **User-only writes need a host ticket** (#574, MEM-45). The host mints a ticket on the user's
   own action and redeems it before the write runs (`memory_host.rs`, `action_ticket.rs`):

   | Trigger | Minted on | Purpose | Binds |
   |---|---|---|---|
   | Ui | a card `memory.remember` click | Pin | the MOG |
   | Voice | Rust's own transcript holds "remember this: …" | Pin | the MOG of the call's tags, and the text |
   | HubAdd | a hub `memory.add` click | Pin | the MOG, and `["add", kind, title, body]` |
   | HubRate | a hub `memory.rate` click | Rate | the MOG, and the item id |
   | HubEdit | a hub `memory.edit` click | Edit | the MOG, and `["edit", itemId, kind, title, body]` |

   - A binding is the SHA-256 of the compact JSON array. At redeem time the ticket builds the
     array again from the write params, so the write must hold exactly what the user clicked or
     said. A voice pin has no title or body of its own. A hub add has no summary of its own and
     has origin `added`.
   - A ticket is single-use, valid for 60 s, and has one purpose. At most 16 are open.
   - The host takes the ticket out before the write runs, so a replay or a second try after a
     refusal gets `invalid_args`.
   - `mem.item.rate` and `mem.item.edit` are Rust-only groups. Only `exec_ticketed` runs them,
     after a redeem. `mem_local_exec` refuses them with `invalid_args`.
   - A pin with no ticket gets `invalid_args`. A ticket on any other call gets `invalid_args`.
   - An AI or agent write never pins. The table refuses origin `pin` or `added` without a pin.
   - A user pin carries no tags in slice 1; a pin ticket refuses a write that has them.
   - `exec_ticketed` takes the proof that only a redeem makes, and runs only the write that
     fits its purpose.
   - Expiry runs on the host's monotonic clock. A voice ticket needs a UUID MOG in the call's
     tags, or the host mints none.
   - The note day comes from the core clock, never from the caller's `seenAt`.
5. **The user's words win** (MEM-47). A later automatic write never rewrites the title, the
   summary or the body of a pinned or reviewed row, and never quarantines it.
6. **Feedback.**
   - Each item has a rating: 0 at first, 1 after a thumbs up, an edit, a pin or a hub add. -1 is
     kept for later use. An item with rating 0 and no review time shows "New".
   - A thumbs up sets rating 1, `reviewed_at`, and confidence 0.9 or more. A stale item becomes
     `active`.
   - An edit sets rating 1 and `reviewed_at`. A source other than `you` becomes `confirmed`. The
     edit clears the embedding, and an `active` result gets a new embed job.
   - Thumbs down deletes the item by the Delete path below.
7. **Delete and Undo, with no confirm click.**
   - A card or voice write shows "Saved to today's note" with Undo for 8 s. Undo runs
     `mem.forget.items` for that item.
   - A hub Delete or a thumbs down hides the item at once and shows "Memory deleted" with Undo
     for 8 s. Then Bottle runs `mem.forget.items`. Undo makes no write.
   - An edit shows Undo for 8 s. Undo is a new edit click, so it gets a new ticket.
   - These are user actions, so U9 holds.
8. **Logs.** No log line holds a title, a summary, a body, a tag or a path. The slice 1 memory
   code has no log call at all; a test scans for one.

## Later slices

- The agent gets a forget tool, so "forget X" works by voice or in chat. It runs the same
  `mem.forget.items` path.
- Pin and unpin in the hub, tags and "Move to MEMORY.md" (slice 3).

## Consequences

- A card or hub `click` counts as the user's own only because the view code is app-owned and
  the gateway lets only view windows send `view_emit`.
- A rendered body cannot open a comment (`<!--` becomes `&lt;!--`). A body that the user did not
  write (Genie, an agent, a legacy row) is a block quote, so it cannot fake the heading or the
  meta line of a pinned item.
- A card ticket binds the MOG, not the text. A faulty brain could use one real click to pin other
  text in that MOG within 60 s.
- R4 reports no lease owner for a segment, so the voice ticket binds the MOG that the brain named
  in the `stt_transcribe` tags, before it saw the text. It binds the text in full.
- A faulty hub view could send a click that the user did not make. This is the same trust level
  as the card click and the hub's `permission_request`. Each ticket binds its item and its fields,
  so one click changes one item at most.
- A dictation hold that starts with "remember this:" pins and does not paste.
- Code: `action_ticket.rs`, `local_store/secret_check.rs`, `local_store/catalog.rs`,
  `local_store.rs` (`exec_ticketed`), and `src-tauri/src/memory_host.rs`.
