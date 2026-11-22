# ADR 0113: Memory capture rules

- Status: proposed (draft; the contract gate of slice 0 is open)
- Issue: local memory plan (PR #623), slice 1; #574 (MEM-28b)
- Date: 2026-11-21
- Owner: impl-memory-s1

## Context

The owner decided that memories save at once, with no approval step: "it is local and the user
can ask the agent to remove memory as well anyway". A memory must still never hold a secret. Before
this ADR the host refused every pin, because no proof of a user action crossed from the brain
(#574).

## Decisions

1. **Every write saves at once as `active`.** This covers the user, the card, the voice command, AI
   writers and the agent. There is no `proposed` status. Each new item goes into today's note of
   its MOG (ADR 0112).
2. **The secret check refuses the write.**
   - R10 checks the title, the summary and the body before each write: the 7 rule-7 patterns and
     the memory patterns (password forms, GitHub, Slack, API and cloud keys).
   - It also checks the NFKC form without zero-width code points.
   - A match refuses the whole write with `secret_refused`. R10 stores nothing of the text.
3. **Instruction-like text** (SEC-24a, ADR 0089). This rule waits for the owner's ruling Q-a or Q-b:
   - Q-a: R10 saves such text as `quarantined`. The user sees it in the hub with a mark and can
     delete it. It never reaches Genie or the agent (SEC-24b).
   - Q-b: the check goes. SEC-24a, SEC-24b and ADR 0089 decision 6 are then amended.
4. **Only the user pins.** A pin needs a host ticket. The ticket is single-use, bound to one MOG
   and valid for 60 s. The host mints it on one of two user actions:
   - a `remember_this` click from the card window of that MOG;
   - the host's own transcript of the user's speech, when it matches the "remember this" grammar.
     This ticket also binds the SHA-256 of the remembered text.
   `mem_local_exec` accepts `pinned: true` only with a valid ticket. The host takes the ticket out
   before the write runs, so a replay gets `invalid_args`. An AI or agent write never pins.
5. **Feedback.**
   - Each item has a rating: 0 at first, 1 after a thumbs up or an edit. -1 is kept for later use.
   - An item with rating 0 and no review time shows "New" in the hub.
   - Thumbs down deletes the item by the Delete path below.
6. **Undo.**
   - A card or voice write shows "Saved to today's note" with Undo for 8 s. Undo runs
     `mem.forget.items` for that item.
   - A hub Delete or a thumbs down hides the item and shows Undo for 8 s. Then Bottle runs
     `mem.forget.items`.
   - These are user actions, so U9 holds.
7. **Logs.** No log line holds a title, a summary, a body, a tag or a path.

## Later slices

- The agent gets a forget tool, so "forget X" works by voice or in chat. It runs the same
  `mem.forget.items` path.
- An edit in the hub sets the rating to 1.

## Consequences

- A card ticket binds the MOG, not the text. A faulty brain could use one real click to pin other
  text in that MOG within 60 s. The voice ticket binds the text.
- A dictation hold that starts with "remember this:" pins and does not paste.
- Code: `action_ticket.rs`, `local_store/secret_check.rs`, and the host check in
  `src-tauri/src/ipc.rs`.
