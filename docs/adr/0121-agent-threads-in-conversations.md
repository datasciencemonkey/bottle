# ADR 0121: Agent threads in Conversations

Status: accepted (drafted to STE100 rules; the official dictionary check was not done).

Amends: plan Amendment 2, A2 "Panel notes", and ADR 0120 (the sentence "the panel holds no
content on disk").

## Context

Before this ADR, the hub Agent panel held the agent turns in memory only (at most 20). The store
held one `genie_exchange` row for each terminal Genie call of a turn. Each row had its own Genie
`conversationId`, and a failed call showed as one more "Failed" thread. So one turn with three
calls became three unrelated threads. The user question and the combined answer ("Note from your
MOG") were stored nowhere, and no path deleted one exchange.

## Decision

### Turn row

- At each turn end, the brain writes one more `genie_exchange` row for the turn:
  - `backend_kind = 'genie_agent'` and `request_id` = the UUID of the turn;
  - `split_group_id` = the thread root, which is the `request_id` of the first turn of the thread;
  - `is_home_workspace = 1`, with the home profile and host;
  - `question_user` = the user question (at most 1,000 characters);
  - `final_answer_md` = the combined answer, or `''` when the summary is hidden or missing;
  - `answer_summary` = NULL, `key_numbers = '[]'` and `conversation_id` = NULL;
  - `status`: answered gives `completed`, stopped gives `cancelled`, timeout gives `abandoned`,
    every other result gives `failed`.
- The call rows stay as they were. They keep `split_group_id` = the UUID of the turn. The
  fallback direct row gets the same value.
- No schema change and no migration. `backend_kind` already allows `genie_agent`.
- `mem.exchange.recent` returns `backendKind` and `splitGroupId`, so the brain can group rows.

### Two meanings of `split_group_id`

The column now has two meanings. The value of `backend_kind` tells which one applies:
- On a turn row (`genie_agent`), the value is the thread root. All turns of one thread share it.
- On any other row, the value is the `request_id` of the turn that made the row (a part of that
  turn). A row with no turn row (a 0.1.5 build) is a group of its own.

A split card (G-50) used the column for the group of one split. That use is the same as the
second meaning, so it does not change.

### Persisted summary, privacy and Wipe

- The combined answer stays on disk in `final_answer_md`, encrypted by SQLCipher. It can join text
  from several workspaces. The call rows already keep the full answer of each workspace,
  including workspaces that are not home. So the summary adds no new kind of data.
- The text stays out of `answer_summary` and the key numbers (D60 fields). So the recall paths
  never use it.
- Wipe needs no change. Age Wipe deletes a turn row by `asked_at`. Wipe "everything" deletes all
  of them. The turn row has `conversation_id` NULL, so no remote delete targets it. A test keeps
  this true.
- This amends Amendment 2 A2 "Panel notes" and ADR 0120. The panel no longer holds the turns
  only in memory: the turn text is on disk. GENIE_HOME stays disposable, and AC-2a.26 does not
  change.

### Local-only delete

- A new brain statement, `mem.exchange.delete` (Write, `Access::Brain`), deletes the named
  exchanges of one MOG: `mogId` and `requestIds`, a list of at most 200 strings of at most 36
  characters. It answers `{changes}`.
- The SQL filters by `mog_id` and passes the MEM-01 guard. A value that is not a request id
  matches no row. An empty list deletes nothing. The list limits come from the `Tags` param type,
  so the check runs before any SQL.
- It writes no `forget_log` row and calls no remote service. A Genie conversation stays on the
  server unless Forget or Wipe "Also delete Genie conversations" removes it.
- `memory_item.source_exchange_id` is `ON DELETE SET NULL`. A citing memory item keeps its row and
  loses its source.
- The brain sends the delete after the 8 s Undo window, like Memory (`memory.delete`).

### HubThread ticket

- A typed follow-up in an agent thread must reach `agent_turn_submit`. That call needs the Ask
  latch or an Ask ticket. The host mints Ask tickets today only for the card chips.
- `Trigger::HubThread` (purpose `Ask`) is new. `mint_for_view` mints it only when all of these
  hold: the window is `hub`, `args.mogId == "conversations"`, `payload.action ==
  "agent.thread_send"`, and `payload.mogId` is a UUID.
- Like the card chip ticket, it binds the MOG only, is single-use and lives 60 s. The host does
  not bind the question text. The session host redeems it with `Purpose::Ask` for
  `agent_turn_submit`, so a wrong MOG, a second use, a late use or another purpose is refused.
- Only a view window can send `view_emit`, so the brain cannot mint this ticket.

## Consequences

- The Conversations list can show one thread for each agent thread, and the Agent panel goes.
- Disk now holds the user question and the combined answer of each turn until the user deletes the
  thread or uses Wipe.
- A delete is local. The Genie conversations stay on the server.
