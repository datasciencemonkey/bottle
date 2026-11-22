# ADR 0113: Memory capture rules

- Status: proposed (draft; the contract gate of slice 0 is open)
- Issue: local memory plan (PR #623), slice 1; #574 (MEM-28b)
- Date: 2026-11-21
- Owner: impl-memory-s1

## Context

A memory must come from the user, or wait for the user. A memory must never hold a secret or an
instruction to a model. Before this ADR the host refused every pin, because no proof of a user
action crossed from the brain (#574).

## Decisions

1. **User actions write `active` memories.** "Remember this" on the answer card and the voice
   command "remember this: <text>" each write one pinned, `active` memory at confidence 1.0.
2. **Model writers write `proposed` memories** (owner, Q4). R10 sets `proposed` for each write with
   the source `agent`. A `proposed` memory becomes `active` only after the user approves it.
   Recall and the agent never use a `proposed` memory.
3. **A pin needs a host ticket.** The host mints a single-use ticket bound to one MOG, valid for
   60 s, on one of two user actions:
   - a `remember_this` click from the card window of that MOG;
   - a transcript that the host made of the user's speech, which matches the "remember this"
     grammar. This ticket binds the SHA-256 of the remembered text.
   `mem_local_exec` accepts `pinned: true` only with a ticket that is valid, unused, of the same MOG
   and, for voice, of the same text. The host takes the ticket out before the write runs, so a
   replay gets `invalid_args`.
4. **The secret check.** R10 checks the title, the summary and the body before each write: the 7
   rule-7 patterns and the memory patterns (password forms, GitHub, Slack, API and cloud keys).
   The check also runs on the NFKC form without zero-width code points. A match refuses the whole
   write with `secret_refused`. R10 stores nothing of the text.
5. **The quarantine** (ADR 0089) now checks the title, the summary and the body. A hit stores a
   `quarantined` memory. The hub marks it and shows its body as plain text.
6. **Undo.** Each card or voice write shows "Saved to memory" with Undo for 8 s. Undo runs
   `mem.forget.items` for that memory. A hub Delete waits 8 s with Undo, then runs
   `mem.forget.items`. Both are user actions, so U9 holds.
7. **Logs.** No log line holds a title, a summary, a body, a tag or a path.

## Consequences

- A card ticket binds the MOG, not the text. A faulty brain could use one real click to pin other
  text in that MOG within 60 s. The voice ticket binds the text.
- A dictation hold that starts with "remember this:" pins and does not paste.
- Code: `action_ticket.rs`, `local_store/secret_check.rs`, and the host check in `src-tauri/src/ipc.rs`.
