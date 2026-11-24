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
  including workspaces that are not home. So the answer adds no new kind of data.
- The question is new on disk in one case. A call row for a workspace that is not home keeps no
  question text (a table CHECK). The turn row keeps the question and counts as a home row, also
  when every part of the turn went to a workspace that is not home. This ADR accepts that. The
  user can delete the thread, and Wipe removes the row.
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
  exchanges of one MOG: `mogId`, `now` and `requestIds`, a list of at most 200 strings of at most
  36 characters. It answers `{changes}`.
- It is a tombstone, not a row delete. It sets `deleted_at` to `now`, sets `question_user`,
  `answer_summary` and `final_answer_md` to NULL, sets `key_numbers` to `'[]'`, and keeps the ids
  (`conversation_id`, `request_id`, `space_id` and the host). The list and get statements skip a
  row with `deleted_at`. A row that is already a tombstone matches no change.
- `mem.exchange.upsert` never brings a tombstone back. On a conflict with a tombstone it changes
  nothing: the row keeps `deleted_at` and the erased text.
- The SQL filters by `mog_id` and passes the MEM-01 guard. A value that is not a request id
  matches no row. An empty list deletes nothing. The list limits come from the `Tags` param type,
  so the check runs before any SQL.
- It writes no `forget_log` row and calls no remote service. A Genie conversation stays on the
  server until Forget or Wipe "Also delete Genie conversations" removes it. These reads take the
  `conversation_id` from the tombstone, so a local delete does not put the conversation out of
  reach. A test deletes a thread, runs Wipe "everything", and checks that the report still has
  the conversation id.
- Wipe and Forget delete the tombstone rows like any other row.
- A citing `memory_item` keeps its row and its `source_exchange_id`: the tombstone row stays.
  A pinned `genie_answer` item also keeps its text. The user pinned it, so a local delete of the
  thread does not erase it. The user deletes the item itself in Memory.
- The tombstone sets `question_sent_sha256` to the empty string (the column is NOT NULL), so the
  hash of the erased question does not stay on disk. No reader uses the column.
- `mem.item.upsert` never links an item to a tombstone. A new item that cites a tombstone is saved
  with no `source_exchange_id`, and a conflict never sets one.
- The brain sends the delete after the 8 s Undo window, like Memory (`memory.delete`).

### Answers first

- The workspace answers must be visible. When a turn has no combined note, the answers show open
  and first, each under "Answer from <workspace>". The note of the MOG then comes after them as
  one small muted block. This holds when the summary is missing, when the summary is only the
  note of the agent (the cross-workspace answer is off), and when a part was never shown to the
  agent. A stored turn does not say whether the agent saw every part, so it always shows the
  answers first.
- The one-line hint of a finished thread in the list is the first line of the first answered
  part, never the note.

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

### Limit: a signed-out turn after a restart

- A turn that ended `signed_out` shows as "needs you" while the client holds it in memory, and
  Clear failed skips it. The turn row on disk says `failed`, and the stored thread (`AgentThreadTurn`
  status is `answered`, `failed` or `stopped`) cannot say more without a contract change. After a
  restart the thread counts as failed, so Clear failed can delete it. The question stays in the
  thread until then. A fix needs a new status in `src/shared/hub-conversations.ts`. This ADR does
  not make that change.

## Consequences

- The Conversations list can show one thread for each agent thread, and the Agent panel goes.
- Disk now holds the user question and the combined answer of each turn until the user deletes the
  thread or uses Wipe.
- A delete is local and leaves a tombstone. The Genie conversations stay on the server until
  Wipe or Forget with "Also delete Genie conversations" removes them.
- A Wipe leaves no turn row. The brain sets a write fence when the user confirms a Wipe, before the
  core wipe and the remote deletes run, and lifts it when the Wipe fails. It sets the fence again
  at each Wipe. A row write that began
  before it saves nothing, and a turn that the session end cut off writes its row only when the
  `agent:session` event arrives with a reason other than `wipe`.
- A turn row that waits behind the fence is written again at the lift. A Wipe whose local part
  worked ends the fence for good, before the remote deletes. The flow runs one Wipe at a time, and
  a fence that holds for 2 min is lifted with a logged fault. Known limit: a turn that the panel
  forgot at a session end while the fence was up is still replayed when the local wipe failed.
- The 120 s fence timeout has a trade-off. If the local `mem_wipe` hangs longer, the fence lifts and
  a late row can survive the Wipe. This keeps the history from being blocked for good.
