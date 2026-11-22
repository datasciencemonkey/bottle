# Plan: local file-system memories (capture, consolidate, recall)

- Status: draft plan. The owner decided Q1, Q2, Q4, Q7, the layout (Q11) and auto-save with feedback (Q12) and the quarantine release (Q13) on 2026-11-21 (Section 15.1). The other questions stay open.
- Date: 2026-11-21
- Author: planner agent (branch `chore/plan-memory`)
- Language: drafted to STE100 rules. A script checked the sentence lengths. Verbatim quotes of the owner are exempt. Nobody did the official dictionary check.
- Scope: a plan and requirements. This document changes no app code.
- New ADRs: ADR 0112, ADR 0113, ADR 0114, ADR 0115 (Section 13). The script `adr-next.sh` gave the numbers.
- Live calls: none. No Databricks call and no model call ran for this plan.
- Example names are placeholders: `profile-a`, `MOG A`, `<host-a>`.

## 0. The short version

The owner wants Bottle to keep memories on the device. Bottle forms them from user actions. A background job cleans them up. Later, Bottle can share them.

1. A **memory** is one typed item: a fact, a preference, a definition, a decision, an open question or a kept Genie answer.
2. Bottle uses the OpenClaw layout (decided): `USER.md`, one `MEMORY.md` for each MOG and dated daily notes. The rows stay the source of truth. The files are renderings.
3. The home is the existing encrypted SQLCipher store (decided, Q1). Bottle shows it as a virtual folder `/memories`. Export gives real `.md` files.
4. The user, the hub and the agent memory tool see files. The disk holds only the encrypted store.
5. Slice 1 is small: the store, the "Remember this" pin and a hub Memory list with Delete.
6. Then come recall in the ask path, the consolidation job, automatic capture, the agent memory tool and the forget tool.
7. Every write passes the quarantine check and a new secret check in Rust. Only the secret check refuses a write.
8. AI-written memories auto-save as `active` with a New badge (decided, Q12). Thumbs up keeps a memory. Thumbs down deletes it with Undo.
   The user can also see, edit and delete each memory. Wipe and Forget cover memories.
9. The consolidation job runs in Rust, on idle, with a power policy. It uses rules only at first.
   It can remove stale, unused, unpinned memories. A removed memory stays in "Recently removed" for 30 d, with Restore (decided, Q2).
   The job never removes a pinned memory.
10. A share to Genie (UC Pages) or to another MOG is later. This plan names the hook points only.

The owner chose option (c) for storage (Q1). The next work is slice 0 (contract, ADRs and migration) at the contract gate, then slice 1.

## 1. Glossary

| Term | Definition in this plan |
|---|---|
| Memory | One typed item that Bottle keeps for the user. It has a kind, a title, a summary, a body and frontmatter. |
| Kind | The type of a memory: `fact`, `preference`, `definition`, `decision`, `open_question` or `genie_answer`. |
| Memory record | The fields of one memory (5.2). The store row holds them. The hub shows them as frontmatter. |
| Rendering | A Markdown file that Rust computes from store rows. Nobody stores the file. |
| Entry | The part of a rendering that shows one memory: a heading, a body and one metadata line. |
| `USER.md` | The rendering `/memories/USER.md`. It shows the shared `preference` memories. |
| `MEMORY.md` | The rendering `/memories/<mog-slug>/MEMORY.md`. It shows the curated memories of one MOG. |
| Daily note | The rendering `/memories/<mog-slug>/memory/YYYY-MM-DD.md`. It shows the daily memories of one day. |
| Tier | The layer of a memory: `curated` (in `MEMORY.md` or `USER.md`) or `daily` (in a daily note). |
| Memory flush | One step before Bottle summarizes a long Genie thread or the agent context. The agent proposes memories to save. |
| Dreaming | A step of the consolidation job. It promotes qualified daily memories into `MEMORY.md` with no approval step. |
| OpenClaw | An open-source agent project. Bottle copies its memory layout (3.4). |
| Virtual folder | The path tree `/memories/...` that Bottle computes from store rows. No real folder exists. |
| Local store | R10, the one SQLCipher 4 file `memory.db`. Its key is in the OS keychain (ADR 0105). |
| Store row | One row of the `memory_item` table. One store row is one memory. The rows are the source of truth. |
| MOG folder | The folder of one MOG, `/memories/<mog-slug>/`. One MOG is one CLI profile and one workspace. |
| Shared folder | The memories of the reserved MOG id `shared`. `USER.md` shows them. They belong to no workspace. |
| Status | The state of a memory: `active`, `stale`, `removed`, `superseded`, `retracted` or `quarantined`. |
| Removed | A status. The job took the memory out of use. The hub shows it in "Recently removed" for 30 d. |
| Recently removed | The hub list of `removed` memories, with Restore. |
| Purge | The permanent delete of a `removed` memory after 30 d. Only the job purges. |
| Auto-save | Each writer, also a model or the agent, stores an `active` memory at once. There is no approval queue (Q12). |
| Rating | The user's feedback on one memory: thumbs up, or none. Thumbs down deletes the memory. |
| Unrated | A memory with no thumbs up, no edit and no pin. |
| New badge | The hub and card mark on each unrated memory. |
| Thumbs up | A user click that keeps a memory and raises its recall weight. An edit or a pin counts as a thumbs up. |
| Thumbs down | A user click that deletes a memory after an 8 s Undo. |
| Fade | The change of an unrated daily memory to `stale` after 30 d with no use. |
| Forget tool | A later agent tool. It deletes the memories that the user names in a "forget X" request. |
| Stale | A status. The consolidation job found no use for a long time. Recall does not use it. Bottle keeps it. |
| Pin | A user mark on a memory. A pinned memory never goes stale. Only a user action makes a pin (MEM-28b). |
| Action ticket | A one-time Rust record that proves a user click or a user voice command (#574). |
| Origin | The event that made a memory: `pin`, `kept_answer`, `correction`, `dictation_word`, `call_notes`, `agent`, `flush`, `dream`, `merge`. |
| Source | The speaker or system behind the text: `you`, `others`, `genie`, `agent` or `confirmed` (ADR 0089). |
| Quarantine | The R10 check of injection text (SEC-24a, ADR 0089). A hit sets the status `quarantined`. |
| Secret check | A new R10 check. It refuses a write with a token, key or password pattern. |
| Consolidation job | A Rust worker that dedups, merges, stales, re-scans, caps and rebuilds the index. |
| Recall | The search of memories for a question. Slice 2 uses FTS5 keyword search only. |
| RecallGate | The T10 step that runs recall in parallel with the router step (MEM-16a..e, #200). |
| Provenance chip | The card line "From your memory" with the age and a link to the memory in the hub. |
| Memory tool | A tool for a model loop with 6 commands: view, create, str_replace, insert, delete and rename. |
| Asking MOG | The MOG that the user speaks to. It owns the card (PR #617). |
| Peer MOG | Another MOG of the same user, with its own profile and workspace (PR #617). |
| Brain | The hidden TypeScript window (`src/brain`). It owns flow and UI state. It never holds a token. |
| Hub | The main window of Bottle. This plan adds the Memory area to it. |
| MOG | A small floating assistant window that Bottle creates and manages. |
| Genie One | The Databricks chat assistant that answers questions (ADR 0097). |
| Card | The Bottle window part that shows an answer from Genie One. |
| Frontmatter | The block of YAML fields at the top of a memory file. |
| Slug | The short lower-case name of a memory in its path. |
| Hit | One memory that recall returns for a question. |
| Outbox | The local queue of rows that the v1 SyncWorker sends (MEM-10a, MEM-11a). |
| Slice | One delivery step of this plan (Section 11). |
| ADR | Architecture decision record. Each ADR is a file in `docs/adr/`. |
| Page record | One local row of the hydration plan (PR #611). It links a recording to its Page. |
| Wipe | The Settings action that deletes user content by age (ADR 0087, MEM-33a). |
| Forget | The action that deletes named items, a session or a MOG with the items that cite them (ADR 0081). |
| U9 | The binding rule "Bottle deletes no user content on its own". This plan amends it for removed memories (M13). |
| AI Gateway | The Databricks route `/ai-gateway/mlflow/v1/...` of a workspace. Bottle reaches models only through it. |
| FTS5 | The SQLite full-text search module. The local store already has `memory_item_fts`. |

## 2. What exists today (facts from the code)

| Part | Fact | Where |
|---|---|---|
| Scope | `AGENTS.md` lists "memory recall and embeddings" as out of scope. | `AGENTS.md` section 1 |
| Store | One SQLCipher 4 file, WAL, `secure_delete`, key in the keychain, one key read per launch. | `local_store/store.rs`, `key_vault.rs`, ADR 0105 |
| Hub promise | The hub says "Bottle keeps your history encrypted on this Mac". | ADR 0105 decision 3 |
| Table | `memory_item` has `item_type` (`decision`, `fact`, `entity`, `open_question`, `genie_answer`) and `text` (1 to 600 chars). | `migrations/0001_local_mirror.sql:144` |
| Status | `status` is `active`, `superseded`, `resolved`, `retracted` or `quarantined`. | `0001_local_mirror.sql:156` |
| Provenance | Each item has source, workspace host and id, session, utterance and exchange links. | ADR 0089, MEM-31 |
| Search | `memory_item_fts` (FTS5, porter) and `sqlite-vec` tables exist. Triggers keep them in step. | `0001_local_mirror.sql:271` |
| No expiry | U9: no local job deletes user content. `expires_at` exists, but nothing reads it. | `0001_local_mirror.sql:9-12`, ADR 0087 |
| Writes | All item writes run `mem.item.write`: the item and its embed job in one transaction. | ADR 0068, ADR 0089 |
| Quarantine | Brain and R10 both check the text. Either check quarantines. A quarantined item gets no embed job. | `local_store/quarantine.rs`, ADR 0089 |
| Quarantine gaps | Homoglyph, combining-mark, spaced-letter and phrase forms escape the check. | issue #543 |
| Pin | `rememberThis()` exists. The host refuses every `pinned: true` call because no user-action proof crosses IPC. | `memory-service.ts`, issue #574 |
| Agent write | `bottle-mcp` `memory.write` writes source `agent`, never pins, and has no workspace fields yet. | `bottle_mcp.rs:194`, issue #538 |
| Agent guard | Memory and tool results are data, never instructions. The agent cannot add memory text to a Genie ask. | `agent_guard.rs`, ADR 0085 |
| Recall | `MemoryService.gate()` is a stub. `AgentTurn.recall` is always `[]`. | `memory-service.ts:337`, `ask-gate.ts:144`, #200 |
| History | Each Genie exchange is saved through `recordExchange`. No memory item is written today. | `genie-history.ts` |
| Wipe | R14 deletes items by `first_seen_at` in one transaction, only on the user's Wipe click. | `wipe.rs`, ADR 0087 |
| Forget | Three atomic groups delete citing items first. | ADR 0081 |
| Dictation words | The hub list "heard → write" lives in `localStorage`, not in the store. | `src/views/hub/DictationWords.tsx` |
| Call notes | The recording feature writes `notes.md` to the user's UC volume with a Claude model. | ADR 0103 |
| Diagram | Diagram 6 shows the MemoryWriter, the RecallGate, the score formula and the v1 Lakebase sync. | `research/diagrams/memory-flow.svg` |

Two facts drive this plan:

- Bottle already has a governed, encrypted memory table with quarantine, Forget and Wipe. A second store adds risk with no gain.
- U9 forbids an automatic delete. The owner amends U9 for memories (Q2, M13). The job removes first, and purges only after 30 d.

## 3. External patterns (official documents, fetched 2026-11-21)

### 3.1 Anthropic memory tool (`memory_20250818`)

- The tool entry is `{"type": "memory_20250818", "name": "memory"}`. It has no input schema to define.
- It is client-side. "Claude requests file operations, and your application runs them."
- The commands are `view` (with `view_range`), `create`, `str_replace`, `insert`, `delete` and `rename`.
- "The `/memories` path is a prefix that your handler maps onto real storage, such as a per-user directory or keys in a database."
- The docs ask the app to validate every path against traversal, to cap file sizes and the `view` output, and to expire old files.
- The docs say that `view` truncates files longer than 16,000 characters.
- The API adds a system prompt line: "ALWAYS VIEW YOUR MEMORY DIRECTORY BEFORE DOING ANYTHING ELSE."
- The SDK helpers are in the beta namespace. The tool itself needs no beta header.

Fit for Bottle:

- The backend shape fits a virtual folder on the local store. The docs name "keys in a database" as a valid backend.
- The Anthropic tool type works on the Anthropic Messages API. The MOG-to-MOG plan uses Chat Completions functions (tools) on the AI Gateway (PR #617, Section 4.3).
- So Bottle must define its own `memory` function with the same 6 commands. The model then sees a familiar shape on either route.
- The automatic "always view first" prompt does not suit Bottle. Recall gives the model a small, ranked set instead.

### 3.2 Claude Code and the Claude Agent SDK

- Claude Code auto memory keeps "a `MEMORY.md` index and one topic file per memory" under `~/.claude/projects/<project>/memory/`.
- Each topic file has a `type` field in its frontmatter: `user`, `feedback`, `project` or `reference`.
- Only "the first 200 lines or 25KB of `MEMORY.md`" load at session start. Topic files load on demand.
- The files are plain text on disk. "Auto memory is machine-local."
- Bottle copies the index cap and the type field. The layout comes from OpenClaw (3.4). Bottle does not copy the plain-text storage.

### 3.4 OpenClaw memory (docs.openclaw.ai/concepts/memory)

Source: https://docs.openclaw.ai/concepts/memory, fetched 2026-11-21.

- `MEMORY.md` is the long-term layer "for durable non-profile facts and decisions". It loads at session start. It is "not a raw transcript, daily log, or exhaustive archive."
- `memory/YYYY-MM-DD.md` holds daily notes and raw context. Today's and yesterday's notes load on a new session. Search tools index the older notes.
- `USER.md` holds stable preferences as directives, with a small budget. A changed preference replaces its entry in place.
- The memory flush: "Before compaction summarizes a conversation, OpenClaw runs a silent turn that reminds the agent to save important context to durable memory." The flush writes only to the daily note. A failed flush does not block compaction.
- Dreaming is the background consolidation. It "promotes qualified owner- or agent-derived items into `MEMORY.md`." Items must pass score, recall-frequency and query-diversity gates.
- In OpenClaw, "Untrusted and system-derived candidates never reach the consolidation or promotion path."
- OpenClaw keeps these files as plain files in the agent workspace. Its promotions write to `MEMORY.md` with no user approval step.

What Bottle copies, and what Bottle keeps (owner decision "OpenClaw layout, Bottle safety", Q11):

| OpenClaw part | Bottle |
|---|---|
| `USER.md`, `MEMORY.md`, daily notes | Copied as renderings of store rows. |
| Plain files on disk | Not copied. The encrypted store stays (Q1). |
| Memory flush before compaction | Copied. The flush output is `active` with the New badge (Q12). |
| Dreaming promotion into `MEMORY.md` | Copied. A promotion is automatic. Rated and often-used memories come first (Q12). |
| Untrusted candidates never promoted | Copied. Source `others` never promotes. |
| Agent writes to files directly | Not copied. Each write goes through R10, with the quarantine and secret checks. |

### 3.3 OpenAI Agents SDK

- The SDK has a `Session` interface (`getItems`, `addItems`, `popItem`, `clearSession`).
- It stores conversation history. It is not a long-term store of typed facts.
- Built-in classes are `MemorySession` (process memory) and `OpenAIConversationsSession` (a remote API).
- Bottle needs no part of it. Genie One already keeps the conversation of each ask.

## 4. Storage and privacy options

### 4.1 The rules that each option must keep

- R-1. Bottle tells the user that local history is encrypted (ADR 0105). Memories are history.
- R-2. Bottle never writes a token to disk, logs, test output or an agent transcript (`AGENTS.md` section 6).
- R-3. Every write passes the quarantine check in Rust (ADR 0089). A quarantined item never reaches a model (SEC-24b). Only the user can release it (Q13).
- R-4. Only Wipe, Forget and the memory purge of M13 delete user content. Each delete is atomic with its citing rows (ADR 0081).
- R-5. Every statement filters by `mog_id` (MEM-01). The brain never sends SQL (MEM-34b).
- R-6. Logs carry no content (SEC-16c).

### 4.2 Option (a): plain Markdown files in a user-only folder

Bottle writes one `.md` file for each memory under the app data folder. The folder mode is 0700 and the file mode is 0600.

| For | Against |
|---|---|
| The user can open, read and edit each file in any editor. | It breaks R-1. Any process of the same user can read the files. |
| Simple code. The Anthropic SDK has a ready file backend. | Spotlight, backup tools and cloud folders can copy the plain text. |
| Easy to share or put in git. | Quarantine and the secret check run only on Bottle writes. An outside edit skips them (R-3). |
| | Wipe and Forget must delete files and rows together. A crash can leave a file behind (R-4). |
| | A second store next to `memory.db` needs a sync rule and a conflict rule. |
| | FTS needs a second index or a full scan. |

### 4.3 Option (b): files encrypted at rest with the keychain key

Bottle writes one encrypted file for each memory (AES-256-GCM). Bottle derives the key from the keychain item of ADR 0105. The user views and edits a memory only in the hub, or through an explicit export.

| For | Against |
|---|---|
| It keeps R-1. | The user cannot open a file in an editor. The "file" is only a format. |
| Each memory is a separate file, so a sync tool can move one file. | It is a second encrypted store with its own code, tests and key use. |
| | Wipe, Forget and the outbox must span the files and `memory.db`. No single transaction covers both (R-4). |
| | FTS needs a decrypted index, which is a third copy. |
| | Each new crypto path needs a security review. |

### 4.4 Option (c): memories in the SQLCipher store, shown as a virtual folder (recommended)

Each memory is one store row. The row holds the frontmatter fields as columns and the Markdown body as text. Rust computes the virtual folder `/memories/...` from the rows. The hub, the export and the agent memory tool all use the virtual folder.

| For | Against |
|---|---|
| It keeps R-1 with no new crypto. | The user cannot open the live files in an outside editor. |
| One transaction covers the row, the FTS row, the embed job and the outbox (ADR 0068). | It needs a schema migration (0003) and new catalog statements. |
| Quarantine, the secret check, Forget and Wipe already run in R10 (R-3, R-4). | The virtual folder needs its own path rules and tests. |
| FTS5 recall is already in the schema. | If the keychain key is lost, all memories are lost (MEM-09a). Export is the backup. |
| The path grammar is fixed, so path traversal has no real target. | |
| The v1 Lakebase sync can use the same outbox. | |

How the user views and edits memories in option (c):

- The hub Memory area shows each memory as a file in a folder tree, with its frontmatter and its rendered body.
- The user edits the title, the body, the kind and the tags in the hub. Rust checks each edit as a new write.
- "Export" writes a zip of the real files (`USER.md`, each `MEMORY.md` and the daily notes) to a place that the user picks (MEM-38a).
- The export dialog says that the files have no encryption (MEM-38b).
- An import of edited files is a later item. It runs each file through the same write checks.

### 4.5 Decision (owner, 2026-11-21)

The owner chose option (c). Export gives the real `.md` files of the layout in 5.3 on request.

The reasons for option (c):

Use option (c). It keeps every rule in 4.1 with code that is on main. The "file system" is the shape that the user and the models see, not the bytes on disk. If the owner wants real editable files, the export gives them on request.

## 5. The memory model

### 5.1 Kinds

| Kind | Definition | Example | Default folder |
|---|---|---|---|
| `fact` | A statement about the business or the data. | "The sales mart refreshes at 06:00 UTC." | MOG |
| `preference` | How the user wants Bottle to act or write. | "Show amounts in EUR." | shared |
| `definition` | The meaning of a business term or metric. | "An active user logged in this month." | MOG |
| `decision` | A choice that a team made, with its date. | "Q3 churn uses the new formula." | MOG |
| `open_question` | A question that has no answer yet. | "Which table holds refunds?" | MOG |
| `genie_answer` | A Genie answer that the user kept. It has the exchange link. | "Churn in Q3 was 4.2 %." | MOG |

The kinds `preference` and `definition` are new. The owner dropped the `person` kind for now (Q7).

The kind `entity` goes away. Migration 0003 must handle the old `entity` rows by these rules:

- The migration must change each `entity` row to the kind `fact`.
- The migration must set the status of each such row to `stale`.
- The migration must add the tag `legacy-entity` to each such row.
- A `quarantined` `entity` row must stay `quarantined`.

The reason: an `entity` row can hold data about a person. As a `stale` memory, recall, the renderings and the agent never see it. The hub shows it with the `stale` filter. A thumbs up makes it `active`. With no action, the removal rules of 7.2a apply. No user data is lost at once.

- The `bottle-mcp` list `AGENT_ITEM_TYPES` must drop `entity`.
- A writer must not make a memory with a person as its subject.

### 5.2 Memory record and entry format

Each memory is one store row. The hub shows its fields as frontmatter. The values are placeholders.

```markdown
---
id: 7f0c2e1a-3d4b-4c5e-9f60-1a2b3c4d5e6f
kind: definition
tier: curated
title: Active user
status: active
pinned: true
source: you
origin: pin
mog: mog-a
workspace_host: <host-a>
workspace_id: "1234567890"
profile: profile-a
note_date: null
exchange_id: null
recording_id: null
created: 2026-11-21T10:00:00Z
updated: 2026-11-21T10:00:00Z
last_used: 2026-11-21T10:05:00Z
confidence: 1.0
sensitivity: normal
tags: [metrics, growth]
supersedes: []
---
An active user is a user who logged in at least once in the calendar month.
```

In a rendering, the same memory is one entry:

```markdown
### Active user
<!-- m:7f0c2e1a kind:definition pinned source:you origin:pin updated:2026-11-21 -->
An active user is a user who logged in at least once in the calendar month.
```

Rules for the record and the entry:

- The record must use only the fields of this list.
- Rust must compute the frontmatter and each entry from the store columns. Rust does not store the text of a rendering.
- The metadata line of an entry must hold the short id `m:<first 8 hex chars>`. The memory tool uses it to find the row.
- The `title` must have 1 to 80 characters.
- The `text` column (the summary line) must have 1 to 600 characters, as today.
- The body must have 4,000 characters or less.
- The `tags` list must have 8 tags or less.
- Each tag must have 32 characters or less.
- The `profile` field holds the CLI profile name. It must never hold a token or a host credential.
- The `note_date` field holds the day of a `daily` memory. It is `null` for a `curated` memory.

### 5.3 Virtual folder layout (OpenClaw layout, decided)

```text
/memories/
  USER.md                        shared preferences (MOG id `shared`), tier curated
  <mog-slug>/                    one folder per MOG = one profile = one workspace
    MEMORY.md                    curated facts, definitions, decisions and open questions
    memory/
      2026-11-20.md              daily note: the daily memories of that day
      2026-11-21.md
```

What each rendering shows:

| Rendering | Memories | Order |
|---|---|---|
| `USER.md` | `active` memories of MOG id `shared`, kind `preference`. | Pinned first, then by `last_used`. |
| `MEMORY.md` | `active` `curated` memories of the MOG, kinds `fact`, `definition`, `decision` and `open_question`. | One `##` section per kind. Pinned first, then by `last_used`. |
| Daily note | `active` `daily` memories of the MOG with that `note_date`. Kept Genie answers go here. | By `created`. |

Rules for the layout:

- The store rows must stay the source of truth. Each file must be a rendering of rows.
- Rust makes the `<mog-slug>` from the MOG name. Rust must keep it unique on the device.
- A path must match the fixed grammar `/memories/USER.md`, `/memories/<mog-slug>/MEMORY.md` or `/memories/<mog-slug>/memory/YYYY-MM-DD.md`. Rust must refuse any other path.
- A rendering must never show a `stale`, `removed`, `superseded`, `retracted` or `quarantined` memory.
- A rendering must mark each unrated memory with `new` in its metadata line.
- `USER.md` must hold only `preference` memories in the first slices. Q5 asks the owner about more kinds.
- A Genie answer must never enter `USER.md`. It belongs to one workspace (D60).
- A Genie answer must have the tier `daily`. It ages fast (MEM-17b).
- Each new memory must start in the daily note of its day, except a `preference` memory.
- A `preference` memory must go to `USER.md` at once. A changed preference replaces its entry in place.
- An entry in a daily note must have the anchor `#<slug>`. The `<slug>` comes from the title.
- `MEMORY.md` must have 200 lines or less and 25 KB or less, as the Claude Code index cap. Pinned memories come first.
- If `MEMORY.md` is over its cap, Rust must leave out the oldest unpinned entries. The hub shows "MEMORY.md is full".
- `USER.md` must have 100 lines or less and 10 KB or less.
- A daily note must show 50 entries or less.
- At the start of an agent session, the agent gets `USER.md`, `MEMORY.md`, today's note and yesterday's note, within their caps.

### 5.4 Size caps

| Item | Cap | Effect at the cap |
|---|---|---|
| Body of one memory | 4,000 characters | Rust refuses the write with `body_too_long`. |
| Active memories in one MOG (both tiers) | 2,000 | Automatic capture stops. The hub shows "Memory is full". User writes still work. |
| Memories of the shared MOG id (`USER.md`) | 500 | The same as above. |
| All memory text on the device | 20 MB | The same as above. |
| One `view` result for the memory tool | 16,000 characters | Rust returns the first part and the line count. The model pages with `view_range`. |
| Recall hits on the card | 3 hits, 1,500 characters | Rust returns the top hits only. |

## 6. Writes: which interactions make a memory

### 6.1 Write triggers

| Trigger | Origin | Writer | Status after the write | Slice |
|---|---|---|---|---|
| The user clicks "Remember this" on a card. | `pin` | Deterministic rule | `active`, pinned, confidence 1.0 | 1 |
| The user says "remember this: <text>" on the You track. | `pin` | Deterministic rule (`isRememberThis`) | `active`, pinned, confidence 1.0 | 1 |
| The user adds a memory in the hub. | `pin` | Deterministic rule | `active`, source `you` | 1 |
| The user clicks "Keep" on a Genie answer card. | `kept_answer` | Deterministic rule | `active`, kind `genie_answer`, tier `daily`, rated up | 4 |
| The user edits a memory in the hub. | `correction` | Deterministic rule | `active`, source `confirmed`, rated up; the old version becomes `superseded` | 3 |
| The user clicks "Ask Genie anyway" on a memory hit. | `correction` | Deterministic rule | The memory loses one point of confidence. | 2 |
| The user turns on "Save dictation words as memories". | `dictation_word` | Deterministic rule | `active`, kind `preference`, tag `dictation-word` | 4 |
| A recording finishes and has notes. | `call_notes` | One model call through the AI Gateway | `active`, unrated (New badge) | 4 |
| The agent calls the memory tool. | `agent` | The model, in a tool loop | `active`, unrated (New badge) | 5 |
| A long Genie thread or the agent context is about to be summarized (6.4). | `flush` | One model call, or the agent in its loop | `active`, unrated, tier `daily` | 4, 5 |
| The consolidation job promotes a daily memory (7.2b). | `dream` | Rules | `active`, tier `curated` | 3 |
| The consolidation job merges two memories. | `merge` | Rule, or a model call later | `active` | 3 |

### 6.2 Write rules

- Each write must run through the R10 statement group `mem.item.write`.
- R10 must run the quarantine check on the title, the summary and the body before each write.
- R10 must run the secret check on the title, the summary and the body before each write.
- If the secret check finds a match, R10 must refuse the write with `secret_refused`. R10 must not store the text.
- The secret check must cover Databricks tokens, JWTs, cloud access keys, private key blocks and `password=` forms.
- A pin must need a valid, unused action ticket (#574).
- The agent and a model writer must never pin.
- Each writer must store an `active` memory at once (auto-save, Q12). There is no approval queue and no hold.
- Only the secret check must refuse a write. Each other check must store the memory.
- A quarantine hit must store the memory with the status `quarantined`. It is not a refusal (ADR 0089).
- The quarantine stays (SEC-24a, SEC-24b). Only the release rule of 6.5 can change a `quarantined` status (Q13).
- A memory from a model or the agent must start unrated, with the New badge.
- A memory from a user action must start rated up.
- Each write from a card or a voice command must show "Saved to memory" with Undo for 8 s.
- Undo must run `mem.forget.items` for that one memory. This is a user action, so U9 holds.
- A model writer must use the home profile of the MOG.
- A model writer must use the route `POST /ai-gateway/mlflow/v1/chat/completions`.
- A model writer must use a `json_schema` response format (MEM-29a).
- A model writer must never write special-category data. Others-track text becomes "a participant" (MEM-29c).
- Each write must record the source, the origin and the workspace fields (MEM-31). This closes #538 for agent writes.

### 6.3 The call-notes writer (slice 4)

1. A recording ends and `notes.md` is ready (ADR 0103).
2. Rust sends the notes text, not the transcript, to the notes model of the recording settings.
3. The prompt asks for up to 10 candidates of the kinds `definition`, `decision`, `fact` and `open_question`.
4. Each candidate must quote one line of the notes as evidence.
5. Rust checks each quote against the notes. Rust drops a candidate with no match.
6. Rust writes each candidate as an `active` daily memory, with `recording_id` and confidence 0.6 to 0.8.
7. The hub shows each candidate with the New badge. The user can rate, edit or delete it.

### 6.4 The memory flush (slice 4 for Genie threads, slice 5 for the agent)

The flush copies the OpenClaw "memory flush" (3.4). It saves know-how before a summary drops the detail.

When it runs:

- For a Genie thread: when the thread reaches 8 follow-ups, before the brain shortens the thread for the card.
- For the agent loop: when the context reaches 75 % of its budget, before the loop summarizes it.

Rules:

- The flush must run at most once for each summary.
- For a Genie thread, Rust must send one tool-free `json_schema` call through the AI Gateway with the home profile.
- For the agent loop, the agent gets one silent step. It can call only the memory tool with `insert` on today's daily note.
- The flush must propose 5 memories or less.
- Each flush memory must have the origin `flush`, the tier `daily` and the status `active`.
- Each flush memory must start unrated, with the New badge.
- Each flush memory must pass the R10 quarantine and secret checks.
- The flush must write only to the daily note of the MOG that owns the thread.
- The flush must never write to `MEMORY.md` or `USER.md`.
- A failed or skipped flush must not block the summary.
- The flush must never send Others-track text or a quarantined memory to the model.
- The setting `memory.flush` must turn the flush off. Its default is on.
- The log line of a flush must hold counts only.

### 6.5 The feedback model (decided, Q12)

The owner chose auto-save with user feedback. The store is local, and the user can ask the agent to remove a memory.

- The hub must show the New badge on each unrated memory.
- Each memory hit on a card must also show the New badge if the memory is unrated.
- The hub and each memory hit on a card must show thumbs up and thumbs down.
- Thumbs up must keep the memory, set its rating to `up` and clear the New badge.
- Thumbs up must add 0.15 to the recall score of the memory.
- Thumbs up must set the confidence of the memory to 0.9 or more.
- An edit by the user must count as a thumbs up.
- A pin must count as a thumbs up.
- Thumbs down must show "Memory deleted" with Undo for 8 s.
- After the 8 s, thumbs down must run `mem.forget.items` for that memory.
- Thumbs down is a user action, so it needs no consolidation rule.
- Thumbs down on a pinned memory must need one confirm click.
- The hub must have a "New" filter. It lists the unrated memories, newest first.
- A rating must never leave the device in the MVP.

Release of a quarantined memory (decided, Q13):

- The hub must show each `quarantined` memory with a "Quarantined" mark and its reason code.
- The hub must show thumbs up on a `quarantined` memory.
- A thumbs up on a `quarantined` memory must set the status `active` and the rating `up`.
- This release must need a valid, unused #574 UI action ticket.
- The host must mint this ticket only on a user click in the hub.
- Rust must refuse a release with no valid ticket. A replayed ticket must also fail.
- No brain caller, agent tool or model writer can release a memory.
- The memory tool, the forget tool and `bottle-mcp` must have no rate or release command.
- Rust must record the time of the release in `quarantine_released_at`.
- The hub must show "You released this memory" on a released memory.
- A card must never show a `quarantined` memory, so a card has no release action.

## 7. The consolidation job

### 7.1 Where and when it runs

- The job must run in the Rust core as one background task. It is not a separate process and not a Databricks job.
- The job must start only when Bottle is idle.
- Idle means: no capture, no ask in flight and no user input for 5 min.
- The job must run at most once in 24 h. The user can also click "Tidy up now" in the hub.
- On battery, the job must start only with 50 % charge or more.
- In macOS Low Power Mode, the job must not start.
- The job must stop at once when capture starts or the user presses the talk key. It resumes at the next idle.
- Each run must take 2 s of CPU or less for 2,000 memories. A run that needs more continues at the next idle.
- The job must run each change in its own short transaction, so an ask never waits more than 50 ms.

### 7.2 What it does (rules only, slice 3)

| Step | Rule | Change |
|---|---|---|
| Exact dedup | Two memories in one folder have the same normalized summary. | The older one becomes `superseded` by the newer one. Pins win over age. |
| Near match | Two memories share 90 % or more of their FTS tokens. | Bottle shows a "Merge?" card in the hub. The job changes nothing. |
| Stale-out | An unpinned `active` memory has no recall use and no edit for 90 d. | The status becomes `stale`. |
| Fade | An unrated `daily` memory has no recall use for 30 d. | The status becomes `stale`. |
| Revive | The user gives a `stale` memory a thumbs up. | The status becomes `active` again. |
| Re-scan | The quarantine pattern version changed (for example after #543). | Each memory gets the new check. A hit becomes `quarantined`. A check never lowers a quarantine. A released memory is skipped until its text changes. |
| Remove | See the rules of 7.2a. | The status becomes `removed`. The hub shows it in "Recently removed". |
| Purge | A `removed` memory has had that status for 30 d, with no Restore. | The job deletes the row through a Forget step. |
| Cap | A folder is over its cap (5.4). | Automatic capture stops. The hub shows "Memory is full". Nothing is deleted. |
| Dream | A `daily` memory meets the rules of 7.2b. | The job writes an `active` `curated` copy. `MEMORY.md` shows it at once. |
| Render | Any change in a MOG. | Rust marks its renderings for a new render and runs the FTS5 `optimize` command. |

### 7.2b Dreaming: promote daily memories (decided, Q11 and Q12)

Dreaming copies the OpenClaw promotion (3.4). The promotion is automatic. There is no approval step (Q12).

The job must promote a `daily` memory only if each of these conditions is true:

- The memory must have the status `active`.
- The memory must not have the source `others`. Others-track text is not trusted.
- The memory must not have the kind `genie_answer`. A Genie answer stays daily (MEM-17b).
- The memory must be 3 d old or more.
- No `curated` memory of the MOG must have the same normalized summary.
- The memory must be rated up, or have 2 recall uses or more from 2 different questions or more.

The job must rank the candidates in this order:

1. Pinned memories.
2. Memories with a thumbs up or a user edit.
3. Unrated memories by recall uses, then by the count of different questions.

Further rules:

- A promotion must write a new `active` `curated` memory with the origin `dream`.
- The new memory must keep the rating, the pin and the source of the daily memory.
- The new memory must cite the daily memory in `supersedes`.
- The daily memory must become `superseded` in the same transaction.
- A promotion must pass the R10 quarantine and secret checks.
- Dreaming must never promote a `quarantined`, `stale` or `restricted` memory.
- Dreaming must use rules only in slice 3. A model merge is later (7.3).
- Dreaming must promote 10 memories or less in one run.
- If `MEMORY.md` is at its cap, dreaming must promote only rated or pinned memories.

An unrated daily memory with no use fades to `stale` after 30 d. Then the removal rules of 7.2a apply.

### 7.2a Remove and purge rules (decided, Q2)

The owner said that Bottle must delete irrelevant memories, and must show them to the user to delete or add. So the job removes memories in two steps: remove, then purge.

The job must remove a memory only if each of these conditions is true:

- The memory must not be pinned.
- The memory must have the status `stale`.
- The memory must have had the status `stale` for 90 d or more.
- The memory must have no recall use in the last 180 d.
- The user must not have edited the memory in the last 180 d.
- The memory must not be cited by a Page record of the hydration plan.

Every other memory stays. The job never removes a pinned memory. A pin always overrides each rule of this section.

Further rules:

- A removed memory must keep all of its fields.
- Recall, the renderings and the memory tool must not use a `removed` memory.
- "Restore" must set the status that the memory had before the remove.
- The job must purge a memory only after 30 d with the status `removed`.
- A purge must use the Forget step for items (ADR 0081).
- That step also deletes the FTS rows and the embed jobs.
- A purge must write one `forget_log` row with the scope `purge` and the item ids only.
- The job must purge 200 memories or less in one run.
- Wipe and Forget must also delete `removed` memories, as other memories.

The user stays in control. Each memory that the job removes was `stale` for 90 d first. Then it waits 30 d in "Recently removed" with Restore. The user can also delete a memory at once with thumbs down.

### 7.3 Model use (later, off by default)

- A later setting "Use a model to merge similar memories" must be off by default.
- With the setting on, the job sends near-match pairs to the notes model through the AI Gateway.
- The model output must be an `active` merged memory, unrated, with the New badge.
- The job must make 20 model calls or less in one run.
- The job must send 50,000 input tokens or less in one day.
- The job must never send a `quarantined`, `restricted` or `genie_answer` memory to a model.

### 7.4 Logs

- Each run must write one log line with counts only: for example `event=memory.consolidate deduped=2 staled=5 dreamed=2 removed=3 purged=1 rescanned=40 ms=310`.
- A log line must never hold a title, a summary, a body, a tag or a path.

## 8. Recall

### 8.1 Memory-first in the ask path (slice 2, #200)

- The RecallGate must start FTS5 recall on the pre-pass text in the same 10 ms as the router step (MEM-16d).
- The RecallGate must delay `genie_ask` by 300 ms or less (MEM-16a). The local query must take 50 ms or less at p95 (MEM-14).
- Recall must read the MOG folder and the shared folder in two statements. The MEM-01 guard forbids `OR`.
- Recall must use only `active` memories. It must skip `stale`, `removed`, `superseded`, `retracted` and `quarantined` memories.
- The score must use keyword rank, recency, pin, rating, confidence and kind. Embeddings stay out of scope (AGENTS.md).
- A Genie answer memory must match the workspace and the space of the ask (MEM-17b).
- A Genie answer memory must be 24 h old or less (MEM-17b).
- In slice 2, a hit must never stop the Genie ask. The card shows the hits next to the Genie answer.
- In slice 2, Bottle must not add memory text to the Genie question. A context preamble needs the ContextGate (#152).
- The card must show each hit with the provenance chip "From your memory · <age>".
- A click on the chip must open the memory in the hub.
- A hit that arrives after the gate closes must show as a chip. Such a hit must not stop the ask (MEM-16c).
- Each recall use must update `last_used`.
- Each recall use must add one to `use_count`.

"Answer from memory" (Bottle does not call Genie) stays later. It needs the golden set with precision 0.95 or more (MEM-17a). Q9 asks the owner about it.

### 8.2 The agent memory tool (slice 5)

The agent memory tool fits the later tool loop of the MOG-to-MOG plan (PR #617, Section 7.10, contract C9).

- The tool must be a Chat Completions function named `memory` with the 6 commands of `memory_20250818`.
- Rust must own the tool. The brain loop must send each call to Rust. The brain loop must not touch the store.
- The tool must see only `USER.md` and the folder of the asking MOG.
- `view /memories` must list `USER.md`, `MEMORY.md` and the daily notes of the last 30 d.
- An edit command must name the entry by its short id `m:<id>`, or by a unique text match.
- The tool must never show a `quarantined` memory (SEC-24b). It shows a released memory as other `active` memories.
- `view` must return at most 16,000 characters.
- `view` must accept `view_range`.
- `insert` on today's daily note must make an `active`, unrated `daily` memory.
- `insert` on `USER.md` must make an `active`, unrated `preference` memory.
- `insert` on `MEMORY.md` must return an error. Only dreaming writes `MEMORY.md`.
- `str_replace` must make an unrated new version of one entry. The old version becomes `superseded`.
- `str_replace` must return an error on a pinned or rated-up entry. The user owns those entries.
- `create` must return an error, because Rust owns the files. The error tells the model to use `insert`.
- `delete` must return an error in slice 5. The forget tool of slice 6 (8.3) handles a delete.
- `rename` must return an error. The paths are fixed.
- Each tool call must write one content-free action log entry (ADR 0085).
- The tool result is data, never instructions (RTR-03c). The agent cannot add memory text to a Genie ask (ADR 0085).
- The route-only planner of PR #617 must never see a memory (MTM-03, MTM-08).
- A peer MOG's memory must never reach the model of another workspace without consent.
- That consent is the synthesis consent of PR #617 Section 7.5.
- The `bottle-mcp` tools `memory.recall` and `memory.write` must use the same R10 code as the memory tool.

### 8.3 The forget tool (slice 6)

A later agent slice adds a `forget` function, so "forget X" works.

- Rust must own the `forget` function, as it owns the memory tool.
- The agent must call `forget` only in a turn where the user's own text is a forget request.
- A deterministic grammar must find the forget request in the user's text, as `isRememberThis` finds a pin.
- Rust must refuse a `forget` call in a turn with no forget request.
- One `forget` call must name 10 memories or less, by short id.
- The memories must belong to the asking MOG or to `USER.md`.
- `forget` must set the status `removed`, so "Recently removed" keeps each memory for 30 d with Restore.
- The card must list the forgotten titles with Undo.
- A pinned memory must need one confirm click on the card before `forget` removes it.
- Each `forget` call must write one content-free action log entry (ADR 0085).

## 9. User controls in the hub

### 9.1 The Memory area

- The hub must have a Memory area in the sidebar, next to Conversations and Recordings.
- The area must show a folder tree: Shared, then one folder for each MOG.
- The area must list the memories of a folder by pin, then by `last_used`.
- The area must filter by kind, status and tag.
- The area must search with FTS5 across the folders that the user selects.
- The area must show a memory as a file: its path, its frontmatter fields and its rendered body.
- The area must let the user edit the title, the body, the kind and the tags. An edit runs the write rules of 6.2.
- The area must let the user delete a memory. Delete runs `mem.forget.items` after a confirm click.
- The area must let the user pin a memory.
- The area must let the user unpin a memory. A pin click mints an action ticket.
- The area must show the New badge, thumbs up and thumbs down of 6.5 on each memory.
- The area must have a "New" filter for unrated memories.
- The area must have a "Recently removed" list. It shows the `removed` memories with Restore and the purge date.
- The "Recently removed" list must also have Delete now.
- The area must show the source, the origin and a link to the exchange or the recording.
- The area must have "Export" (MEM-38a) with the no-encryption warning (MEM-38b).
- The area must show "Memory is full" when a cap applies (5.4).

### 9.2 Wipe and Forget

- The Wipe dialog must name memories in its list of content. Wipe already deletes items by `first_seen_at`.
- A pinned memory has no Wipe exception. Q8 asks the owner about this rule.
- Forget of a session, an exchange or a MOG must delete the memories that cite them (ADR 0081, as today).
- Forget of a MOG must not delete the shared folder.
- Wipe "everything" must delete the shared folder too.

## 10. Share to Genie and to other MOGs (later; hook points only)

| Hook | Where | What it does later |
|---|---|---|
| H1. Page candidate | A `definition` or `decision` memory with source `you` or `confirmed` | The hub action "Propose as a Page" sends it to the Term Page review of the hydration plan (PR #611, slice 4). Bottle creates DRAFT Pages only. |
| H2. Evidence link | `recording_id` and `exchange_id` in the frontmatter | A Term Page gets its evidence quotes from the linked recording. |
| H3. Change event | A new content-free event `memory.changed {mogId, count}` | The hub and a later share worker know that memories changed, with no text. |
| H4. Copy to another MOG | A hub action on one memory | The user copies one memory to another MOG folder. The copy has source `confirmed`. No model takes part. |
| H5. Peer recall | The memory tool in synthesis mode (PR #617) | A peer MOG answers from its own memory, under the cross-workspace consent. |
| H6. Lakebase sync | The existing outbox (MEM-10a, MEM-11a) | The v1 SyncWorker sends memory rows by the D60 rules. |
| H7. Genie memories | MEM-32d link to the Genie One memory review | Bottle links its memories to Genie One memories. No API call in the MVP. |

"Shared as they are formed" maps to H3 plus H1. No memory leaves the device without a user action.

## 11. Slices

### Slice 0: contract and ADRs (no behavior)

- ADR 0112 and ADR 0113 (draft).
- Migration 0003, the new catalog ids, the interface types and the action ticket command. Each new command answers `not_implemented`.
- AC-0.1: `cargo test` and `pnpm test` pass. The catalog test lists the new ids.
- AC-0.2: Migration 0003 runs on a store from 0002 and keeps every row. The test checks the count and the FTS rows.
- AC-0.3: The migration changes each `entity` row to kind `fact`, status `stale` and tag `legacy-entity`. A quarantined row stays quarantined.
- AC-0.5: The new table refuses the status `proposed`.
- AC-0.4: The new table refuses the kinds `entity` and `person`.

### Slice 1: local memory store, "Remember this" and the hub list (small and useful)

Scope:

1. The action ticket of #574. The host mints it on a card click or a voice command. `mem.item.write` accepts `pinned: true` only with it.
2. The card button "Remember this". It pins the answer summary as a `genie_answer` or `fact` memory.
3. The voice command "remember this: <text>" on the You track. It pins the text as a `fact` memory.
4. The R10 secret check (6.2).
5. The virtual folder read path: `mem.item.list` and `mem.render.get` for `MEMORY.md` and the daily notes.
6. The hub Memory area with the folder tree (`MEMORY.md`, `memory/`), the list, the file view and Delete. No edit, search or export yet.

Acceptance criteria:

- AC-1.1: A click on "Remember this" stores one pinned, `active` memory with confidence 1.0 and the exchange link.
- AC-1.2: A pin without a ticket gets `invalid_args` from Rust. A replayed ticket gets `invalid_args`.
- AC-1.3: The voice phrase "remember this: the mart refreshes at six" stores one pinned `fact` memory.
- AC-1.4: The phrase "remember this week" stores nothing.
- AC-1.5: A text with a fake Databricks token pattern gets `secret_refused`. The store has no row with the text.
- AC-1.6: A text from the adversarial fixtures stores a `quarantined` memory. The hub list marks it.
- AC-1.7: A pinned memory shows as one entry at `/memories/<mog-slug>/memory/<day>.md#<slug>`, in the daily note of its day.
- AC-1.7a: The rendering is computed from the rows. A row delete removes the entry at the next render.
- AC-1.8: Delete in the hub removes the row, its FTS row and its embed job. `mem.recall.fts` no longer finds it.
- AC-1.9: The card shows "Saved to memory" with Undo. Undo removes the memory.
- AC-1.11: A pinned memory has no New badge, because a pin counts as a thumbs up.
- AC-1.10: No log line has memory text. A log scan test checks this.

Not in slice 1: recall, edit, search, export, the shared folder, automatic capture, the consolidation job and the agent tool.

### Slice 2: recall in the ask path

- The RecallGate of #200 with FTS5 only. The card section "From your memory" with the provenance chip.
- `USER.md` for shared `preference` memories, and the hub search.
- AC-2.1: A trace shows the `genie_ask` delay of 300 ms or less (MEM-16a).
- AC-2.2: A dictation utterance gets no memory verdict (MEM-16e).
- AC-2.3: A `stale`, `removed` or `quarantined` memory never shows on a card.
- AC-2.7: Each card hit shows thumbs up and thumbs down. Thumbs down deletes the memory after the 8 s Undo.
- AC-2.8: A rated-up memory outranks an unrated memory with the same keyword score.
- AC-2.9: A hub thumbs up with a valid ticket moves a `quarantined` memory to `active` and rated up.
- AC-2.10: A release with no ticket, or from a brain or agent caller, gets `invalid_args`. The memory stays `quarantined`.
- AC-2.4: A Genie answer memory from another workspace never shows (MEM-17b).
- AC-2.5: The Genie question that R8 sends equals the user text. No memory text is in it.
- AC-2.6: A late hit shows as a chip and the ask continues (MEM-16c).

### Slice 3: the consolidation job (rules only)

- ADR 0114. The Rust worker, the power policy, the steps of 7.2, the remove and purge rules of 7.2a and "Tidy up now".
- The hub edit (origin `correction`) and the "Merge?" cards.
- The hub list "Recently removed" with Restore, the New filter and the fade rule.
- Dreaming (7.2b).
- AC-3.1: With a simulated clock, a 91 d unused unpinned memory becomes `stale`. A pinned one stays `active`.
- AC-3.1a: A memory `stale` for 91 d with no recall in 180 d becomes `removed`. A pinned memory with the same history stays.
- AC-3.1b: The job purges a `removed` memory after 30 d. Restore before that sets its old status again.
- AC-3.1c: A purge leaves no row, FTS row or embed job. It writes one `forget_log` row with the scope `purge`.
- AC-3.2: The job deletes only rows that meet the purge rule. A row count test before and after a run proves it.
- AC-3.3: The job does not start on battery under 50 % or in Low Power Mode. A test double gives the power state.
- AC-3.4: The job stops within 100 ms of a talk-key press.
- AC-3.5: A pattern version change quarantines a memory that matches the new pattern.
- AC-3.5a: A released memory stays `active` after a pattern version change, until its text changes.
- AC-3.6: The run log line holds counts only.
- AC-3.7: A daily memory with 2 recall uses from 2 questions moves to `MEMORY.md` as one `active` `curated` memory in one run.
- AC-3.7a: A rated-up daily memory moves before an unrated one when the run limit applies.
- AC-3.7b: An unrated daily memory with no use for 31 d becomes `stale`.
- AC-3.8: A daily memory with source `others` never moves to `MEMORY.md`.

### Slice 4: automatic capture

- Slice 4 uses auto-save with the New badge (ADR 0113, decided, Q12).
- "Keep" on Genie answer cards, the dictation words option, the call-notes writer of 6.3 and the Genie thread flush of 6.4.
- The export.
- AC-4.1: A finished recording with notes gives up to 10 `active`, unrated memories, each with a quote that matches the notes.
- AC-4.2: Bottle sends no transcript text to the model. The request body test checks this.
- AC-4.3: The writer request uses the home profile and `/ai-gateway/mlflow/v1/chat/completions`.
- AC-4.4: With the dictation words option off, no dictation word becomes a memory.
- AC-4.5: Export writes a zip with `USER.md`, each `MEMORY.md` and the daily notes. The warning shows first.
- AC-4.6: A Genie thread at 8 follow-ups gives 5 unrated daily memories or less. A failed flush still lets the summary run.

### Slice 5: the agent memory tool

- ADR 0115. The `memory` function of 8.2 in the tool loop of PR #617 (C9). The `bottle-mcp` tools move to the same code.
- AC-5.1: `view /memories` lists `USER.md` and the files of the asking MOG only.
- AC-5.2: The path `/memories/../x` and the path `/memories/<other-mog>/MEMORY.md` get an error.
- AC-5.3: `insert` on today's note stores an `active`, unrated memory with source `agent`. It never pins.
- AC-5.3a: A tool write with a fake token pattern gets `secret_refused`. No other check refuses it.
- AC-5.4: A `quarantined` memory never appears in a tool result.
- AC-5.5: Each call writes one content-free action log entry.
- AC-5.6: The agent flush at 75 % of the context gives only unrated daily memories.

### Slice 6: the forget tool

- The `forget` function of 8.3 and the forget grammar.
- AC-6.1: "forget the refund table note" removes the matching memory. "Recently removed" shows it with Restore.
- AC-6.2: A `forget` call in a turn with no forget request gets an error. Nothing changes.
- AC-6.3: A pinned memory needs a confirm click before `forget` removes it.
- AC-6.4: Each call writes one content-free action log entry.

### Later

- Embeddings and hybrid recall (MEM-19a..21b), the model merge of 7.3, "answer from memory", import of edited files, and the hooks of Section 10.

## 12. Contract changes (each needs the owner's sign-off)

| ID | Flag | File(s) | Change | Slice |
|---|---|---|---|---|
| M1 | CONTRACT + SCHEMA | `local_store/migrations/0003_memory_files.sql` | Rebuild `memory_item` with the new kinds and statuses. Add `title`, `body_md`, `tags`, `origin`, `tier`, `note_date`, `profile`, `recording_id`, `last_used_at`, `use_count`, `recall_query_count`, `stale_at`, `slug`, `rating`, `rated_at`, `quarantine_released_at`. Old rows get the tier `curated`, and `genie_answer` rows get `daily`. Add the status `removed` and the columns `removed_at` and `status_before_removed`. Drop the kinds `entity` and `person`. Change old `entity` rows to `fact`, `stale`, tag `legacy-entity` (5.1). The table has no `proposed` status. Keep the FTS and vec triggers. | 0 |
| M2 | CONTRACT | `local_store/catalog.rs`, `src/brain/components/memory-store.ts` | New brain ids: `mem.item.list`, `mem.item.search`, `mem.item.edit`, `mem.item.set_status`, `mem.render.get`, `mem.item.export`. New Rust-only ids: `mem.consolidate.*`. | 0, 2, 3 |
| M3 | CONTRACT | `src/shared/interfaces.ts` | `MemoryHit['type']` and `MemoryDraft` get the new kinds, `title`, `body`, `tags` and `origin`. `MemoryDraft` gets the #533 link fields. | 0 |
| M4 | CONTRACT | `src-tauri/src/ipc.rs`, `crates/mog-core/src/ipc.rs`, `src/shared/ipc.ts` | The action ticket of #574: a host-minted, single-use ticket bound to one MOG. `mem_local_exec` accepts `pinned: true` only with it. | 0 |
| M5 | REQUIREMENT | `research/final-requirements.md` (MEM-01) | A reserved MOG id `shared` for the shared folder. Each statement still filters by one `mog_id`. | 2 |
| M6 | CONTRACT | `local_store/catalog.rs`, `quarantine.rs` | A new core-computed param `textSecret`, as `textQuarantine` is. New error code `secret_refused`. | 1 |
| M7 | CONTRACT | `src/shared/ipc.ts` (card events) | Card section kind `memory_hits` and card actions `remember_this` and `keep_answer`. | 1, 2 |
| M8 | CONTRACT | `src/shared/ipc.ts` | New content-free event `memory.changed {mogId, count}` for the hub. | 1 |
| M9 | CONTRACT + SCHEMA | `crates/mog-core/src/app_settings.rs`, schema | New settings: `memory.autoCapture`, `memory.dictationWords`, `memory.modelMerge`, `memory.flush`, `memory.dreaming`. No setting adds an approval queue. `deny_unknown_fields` makes this a schema change. | 3, 4 |
| M10 | CONTRACT | `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs` | A Rust handler for the `memory` function inside `agent_model_step` (PR #617 C9). | 5 |
| M14 | CONTRACT | `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs` | Later: a Rust `forget` handler in `agent_model_step`, with the forget grammar check (8.3). | 6 |
| M15 | CONTRACT | `src/shared/ipc.ts`, card events, `local_store/catalog.rs` | Card and hub actions `rate_up` and `rate_down`, and the brain id `mem.item.rate`. | 2 |
| M16 | CONTRACT | `local_store/catalog.rs`, `src-tauri/src/ipc.rs`, `src/shared/ipc.ts` | The hub action `release_quarantine` and the group `mem.item.release`. The group needs the #574 UI action ticket (MEM-59). | 2 |
| M11 | SCOPE | `AGENTS.md` section 1 | Lift "memory recall" (FTS5 only) from the out-of-scope list. Embeddings stay out. | 2 |
| M12 | REQUIREMENT | `research/final-requirements.md` | New rows for this plan: kinds, statuses, caps, secret check, consolidation job and memory tool. Amend MEM-28a and MEM-29a for auto-save and the feedback model (Q12). | 0 |
| M13 | REQUIREMENT (decided, Q2) | `research/final-requirements.md` (U9, MEM-33a, Section 9.2), `AGENTS.md` | Amend U9 and MEM-33a for the memory remove and purge of 7.2a. Add the rows of 12.1. | 0 (text), 3 (code) |

### 12.1 M13: the requirement change for U9 and MEM-33a

The new text of U9:

> Bottle deletes no user content on its own, with one exception. The memory consolidation job can remove and then purge memories by the rules of MEM-42 to MEM-49 and MEM-56. All other user content is deleted only by Wipe or Forget.

The new text of MEM-33a. The first sentence changes, and the rest of the row stays:

> The WipeService must delete user content only when the user selects Wipe in Settings, except the memory purge of MEM-47.

The acceptance text of MEM-33a gets this addition: "A memory purge by MEM-47 is not a Wipe."

The new requirement rows (priority M, milestone MVP-D, use case UC2):

| ID | Requirement | Acceptance |
|---|---|---|
| MEM-42 | The consolidation job must never remove or purge a pinned memory. | A pinned memory with no use for 400 d stays `active`. |
| MEM-43 | The job must set `stale` on an unpinned `active` memory with no recall use and no edit for 90 d. | The simulated clock test passes. |
| MEM-44 | The consolidation job must remove a memory only if it meets each condition of Table MEM-REMOVE. | A memory that fails one condition stays. |
| MEM-45 | A `removed` memory must not appear in recall, in a rendering or in a memory tool result. | The recall, rendering and tool tests find no `removed` memory. |
| MEM-46 | The hub must show each `removed` memory in "Recently removed" with Restore and its purge date. | Restore sets the status from before the remove. |
| MEM-47 | The consolidation job must purge a memory only after 30 d with the status `removed`. | A memory at 29 d stays. A memory at 31 d is gone. |
| MEM-48 | A purge must delete the memory row, its FTS row and its embed job in one transaction. | `mem.recall.fts` no longer finds the text. |
| MEM-49 | A purge must write one `forget_log` row with the scope `purge` and the item ids only. | The row holds no text. |
| MEM-50 | Each writer must store an `active` memory at once, with no approval queue. | A model write is in recall at once. |
| MEM-51 | Only the secret check must refuse a memory write. | A quarantine hit stores a `quarantined` row. |
| MEM-52 | The hub must show the New badge on each unrated memory. | A memory from the agent shows the badge. A pinned one does not. |
| MEM-53 | Thumbs up must keep a memory and add 0.15 to its recall score. | A rated memory outranks an unrated tie. |
| MEM-54 | Thumbs down must delete a memory after an 8 s Undo. | Undo keeps the memory. No Undo removes the row. |
| MEM-55 | A user edit or a pin must count as a thumbs up. | An edited memory has the rating `up`. |
| MEM-56 | The job must set `stale` on an unrated daily memory with no recall use for 30 d. | The simulated clock test passes. |
| MEM-57 | Dreaming must promote daily memories into `MEMORY.md` with no approval step. | A qualified memory moves in one run. |
| MEM-58 | The `forget` tool must run only in a turn with a forget request from the user. | A call with no request gets an error. |
| MEM-59 | A release of a `quarantined` memory must need a #574 UI action ticket from a hub click. | A release from the brain or an agent tool gets `invalid_args`. |
| MEM-60 | The re-scan must skip a released memory until its text changes. | A released memory stays `active` after a pattern update. |

The amended row SEC-24b (Q13):

| ID | Requirement | Acceptance |
|---|---|---|
| SEC-24b | The ContextAssembler and the `memory.recall` tool must not give quarantined items to Genie or to the main agent. Only a user release by MEM-59 makes a quarantined item `active`. | Quarantined items never appear in a preamble or a tool result. A released item appears as other active items. |

**Table MEM-REMOVE (conditions for a remove, MEM-44).**

| Condition | Value |
|---|---|
| Pin | Not pinned. |
| Status | `stale` for 90 d or more. |
| Recall use | None in the last 180 d. |
| User edit | None in the last 180 d. |
| Page record | No Page record cites the memory. |

Section 9.2 of the requirements gets one new row: "Removed memories: purged by the consolidation job 30 d after the remove (MEM-47)."

## 13. New ADRs

- **ADR 0112: Local memories as a virtual folder on the SQLCipher store.** It records option (c), the record and entry format and the OpenClaw layout. It also records the path grammar, the caps, the export and migration 0003.
- **ADR 0113: Memory capture rules.** The write triggers and the writers. It also records auto-save, the feedback model (Q12), the action ticket, the secret check and Undo.
- **ADR 0114: The memory consolidation job.** It records the Rust worker, the idle and power rules and the steps. It also records dreaming (7.2b), the remove and purge rules (M13) and the content-free log.
- **ADR 0115: Memory recall in the ask path and the agent memory tool.** It records the RecallGate with FTS5 and the provenance chip. It also records the `memory` function with the `memory_20250818` command shape on the renderings, the memory flush (6.4) and the forget tool (8.3).

Note on ADR numbers: the hydration plan (PR #611) names ADR 0109 to ADR 0112 for its own work. Those numbers are not free.

- ADR 0109 is on main (recording upload in parts).
- ADR 0110 and ADR 0111 belong to the MOG-to-MOG plan (PR #617).
- ADR 0112 to ADR 0115 belong to this plan.

PR #611 must reserve new numbers with `adr-next.sh` before it merges.

## 14. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Migration 0003 rebuilds a table with triggers. | A bad copy can lose memories. | Use the SQLite table rebuild steps in one transaction. Test on a filled 0002 store. Keep forward-only rules (MEM-35a). |
| The owner wants real files on disk (option a). | Plain text leaks to backups, indexers and other apps. | Keep option (c). Give real files only through Export with the warning. |
| A memory holds injection text. | The agent obeys it. | Quarantine on each write and each re-scan. Tool results are data. Quarantined rows never reach a model. New badges show model writes. Fix #543. |
| A memory holds a token or a password. | A secret is on disk. | The R10 secret check refuses the write. Tests use fake values only. |
| An old `entity` row holds data about a person. | Privacy harm. | The `person` kind is dropped (Q7). Old rows become `stale` with the tag `legacy-entity`, so no recall uses them unless the user gives a thumbs up. |
| Keyword recall gives poor hits. | Noise on the card. | Show 3 hits or less. Never stop the Genie ask in slice 2. Measure on a golden set. |
| The job removes a memory that the user needs. | Lost know-how. | Pins are never removed. 90 d `stale` first, then 30 d in "Recently removed" with Restore. |
| Auto-save stores a wrong or noisy memory. | Poor hits on the card. | The New badge, thumbs down with Undo, the 30 d fade and the forget tool. Rated memories rank first. |
| The purge breaks the U9 promise to users. | Loss of trust. | The hub and the docs say which memories the job can purge, and when. M13 changes the requirement text first. |
| The job uses power or blocks an ask. | Battery drain or a slow ask. | Idle start, power rules, short transactions and a stop on the talk key. |
| The keychain key is lost. | All memories are lost (MEM-09a). | The hub names the loss. Export is the backup. v1 sync can rebuild. |
| #574 is not done. | Slice 1 cannot pin. | The ticket is part of slice 0 and slice 1. |
| A model writer costs money. | Charges in the home workspace. | Off until slice 4, a call cap and a token cap, and the data panel line. |
| A memory crosses workspaces. | Data from workspace A reaches workspace B. | One folder per MOG, Genie answers never shared, and the PR #617 consent rules. |

## 15. Questions for the owner

### 15.1 Decided on 2026-11-21

- **Q1. Storage and privacy: option (c).** Memories live in the encrypted SQLCipher store. Bottle shows them as the virtual folder `/memories`. Export gives real `.md` files.
- **Q2. Cleanup with removal.** The owner said: "yeah we should be able to delete memories that are irrelevant and also show them to the user to delete or add."
  - The job removes only unpinned, stale, unused memories by Table MEM-REMOVE (7.2a).
  - A removed memory stays 30 d in "Recently removed" with Restore. Then the job purges it.
  - Q12 removed the suggestion lists. The New badge and thumbs replace them.
  - The job never removes a pinned memory.
  - This amends U9 and MEM-33a (M13, Section 12.1).
- **Q4. Model writes.** Q12 replaces the first answer. A model-written memory is `active` at once, with the New badge.
- **Q7. The `person` kind.** It is dropped for now. Old `entity` rows become `fact`, `stale`, tag `legacy-entity` (5.1).
- **Q11. Layout: "OpenClaw layout, Bottle safety".** Bottle uses `USER.md`, `MEMORY.md` for each MOG and dated daily notes (5.3).
  - The rows stay the source of truth. The files are renderings.
  - Bottle adds the memory flush (6.4) and dreaming (7.2b).
  - The Bottle safety decisions stay: encryption, the quarantine and secret checks, the #574 ticket and the removal rules.
- **Q12. Auto-save with feedback.** AI-written memories auto-save as `active`. There is no approval queue and no hold.
  - Only the secret check refuses a write. A quarantine hit stores a hidden `quarantined` row.
  - The owner's reason: the store is local, and the user can ask the agent to remove a memory.
  - The feedback model of 6.5: the New badge, thumbs up, thumbs down with Undo, and an edit as a thumbs up.
  - Dreaming promotes daily memories automatically. Rated and often-used memories come first. Unrated daily memories fade after 30 d.
  - A later slice (6) adds the forget tool, so "forget X" works.
  - The OpenClaw layout, the flush, the encryption, the secret check and the user-only pin ticket stay.
- **Q13. Quarantine release.** The quarantine stays (SEC-24a, SEC-24b), with a release rule (6.5).
  - A thumbs up from the user in the hub moves a quarantined memory to `active` and rated up.
  - The thumbs-up action needs a #574 UI action ticket. So no brain or agent caller can release an item.
  - MEM-59 and MEM-60 are new. The SEC-24b row changes (12.1).
- **Next work.** Build slice 0 (contract, ADRs and migration 0003) at the contract gate. Then build slice 1.

### 15.2 Still open

- Q3. Do you lift "memory recall" (keyword only) from the out-of-scope list for slice 2? Embeddings stay out.
- Q5. Do you accept a shared folder across MOGs (a reserved MOG id)? Which kinds can go in it?
- Q6. Can the consolidation job use a model to merge memories later, with the caps of 7.3?
- Q8. Must Wipe delete pinned memories, as it does today?
- Q9. Do you want "answer from memory" (no Genie call) later, after a golden set passes?
- Q10. Must dictation words become memories, or stay a separate setting?
