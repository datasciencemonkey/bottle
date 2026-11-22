# Plan: local file-system memories (capture, consolidate, recall)

- Status: draft plan for the owner. The owner accepts nothing in this plan yet.
- Date: 2026-11-21
- Author: planner agent (branch `chore/plan-memory`)
- Language: drafted to STE100 rules. A script checked the sentence lengths. Nobody did the official dictionary check.
- Scope: a plan and requirements. This document changes no app code.
- New ADRs (numbers reserved with `adr-next.sh`): ADR 0112, ADR 0113, ADR 0114, ADR 0115 (Section 12).
- Live calls: none. No Databricks call and no model call ran for this plan.
- Example names are placeholders: `profile-a`, `MOG A`, `<host-a>`.

## 0. The short version

The owner wants Bottle to keep memories on the device. Bottle forms them from user actions. A background job cleans them up. Later, Bottle can share them.

1. A **memory** is one typed item: a fact, a preference, a definition, a decision or a person and context.
2. Each memory has the shape of one Markdown file with frontmatter. An index file lists the memories.
3. The recommended home is the existing encrypted SQLCipher store. Bottle shows it as a virtual folder `/memories`.
4. The user, the hub and the agent memory tool see files. The disk holds only the encrypted store.
5. Slice 1 is small: the store, the "Remember this" pin and a hub Memory list with Delete.
6. Then come recall in the ask path, the consolidation job, automatic capture and the agent memory tool.
7. Every write goes through the quarantine check and a new secret check in Rust.
8. The user can see, edit, undo and delete each memory. Wipe and Forget cover memories.
9. The consolidation job runs in Rust, on idle, with a power policy. It uses rules only at first.
10. A share to Genie (UC Pages) or to another MOG is later. This plan names the hook points only.

The top open question for the owner is the storage and privacy option (Section 4, Q1).

## 1. Glossary

| Term | Definition in this plan |
|---|---|
| Memory | One typed item that Bottle keeps for the user. It has a kind, a title, a summary, a body and frontmatter. |
| Kind | The type of a memory: `fact`, `preference`, `definition`, `decision`, `person`, `open_question` or `genie_answer`. |
| Memory file | The Markdown form of one memory: YAML frontmatter, then a Markdown body. |
| Index file | `index.md` in a folder. It has one line for each active memory in that folder. |
| Virtual folder | The path tree `/memories/...` that Bottle computes from store rows. No real folder exists. |
| Local store | R10, the one SQLCipher 4 file `memory.db`. Its key is in the OS keychain (ADR 0105). |
| Store row | One row of the `memory_item` table. One store row is one memory. |
| MOG folder | The folder of one MOG, `/memories/<mog-slug>/`. One MOG is one CLI profile and one workspace. |
| Shared folder | `/memories/shared/`. It holds memories that belong to no workspace, such as preferences. |
| Status | The state of a memory: `proposed`, `active`, `stale`, `superseded`, `retracted` or `quarantined`. |
| Proposed | A status. A writer made the memory, but the user did not approve it. Recall does not use it. |
| Stale | A status. The consolidation job found no use for a long time. Recall does not use it. Bottle keeps it. |
| Pin | A user mark on a memory. A pinned memory never goes stale. Only a user action makes a pin (MEM-28b). |
| Action ticket | A one-time Rust record that proves a user click or a user voice command (#574). |
| Origin | The event that made a memory: `pin`, `kept_answer`, `correction`, `dictation_word`, `call_notes`, `agent`, `merge`. |
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
| Wipe | The Settings action that deletes user content by age (ADR 0087, MEM-33a). |
| Forget | The action that deletes named items, a session or a MOG with the items that cite them (ADR 0081). |
| U9 | The binding rule "Bottle deletes no user content on its own". Only Wipe and Forget delete. |
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
- U9 forbids an automatic delete. So the consolidation job can change a status, but it cannot delete.

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
- Bottle copies the shape (one file per memory, one index, a type field, an index cap). Bottle does not copy the plain-text storage.

### 3.3 OpenAI Agents SDK

- The SDK has a `Session` interface (`getItems`, `addItems`, `popItem`, `clearSession`).
- It stores conversation history. It is not a long-term store of typed facts.
- Built-in classes are `MemorySession` (process memory) and `OpenAIConversationsSession` (a remote API).
- Bottle needs no part of it. Genie One already keeps the conversation of each ask.

## 4. Storage and privacy options

### 4.1 The rules that each option must keep

- R-1. Bottle tells the user that local history is encrypted (ADR 0105). Memories are history.
- R-2. Bottle never writes a token to disk, logs, test output or an agent transcript (`AGENTS.md` section 6).
- R-3. Every write goes through the quarantine check in Rust (ADR 0089). A quarantined item never reaches a model (SEC-24b).
- R-4. Only Wipe and Forget delete user content (U9, MEM-33a). Each delete is atomic with its citing rows (ADR 0081).
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

Bottle writes one encrypted file for each memory (AES-256-GCM). The key comes from the keychain item of ADR 0105, through a key derivation step. The user views and edits a memory only in the hub, or through an explicit export.

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
- "Export" writes a zip of real `.md` files and the index files to a place that the user picks (MEM-38a).
- The export dialog says that the files have no encryption (MEM-38b).
- An import of edited files is a later item. It runs each file through the same write checks.

### 4.5 Recommendation

Use option (c). It keeps every rule in 4.1 with code that is on main. The "file system" is the shape that the user and the models see, not the bytes on disk. If the owner wants real editable files, the export gives them on request.

## 5. The memory model

### 5.1 Kinds

| Kind | Definition | Example | Default folder |
|---|---|---|---|
| `fact` | A statement about the business or the data. | "The sales mart refreshes at 06:00 UTC." | MOG |
| `preference` | How the user wants Bottle to act or write. | "Show amounts in EUR." | shared |
| `definition` | The meaning of a business term or metric. | "An active user logged in this month." | MOG |
| `decision` | A choice that a team made, with its date. | "Q3 churn uses the new formula." | MOG |
| `person` | A person and their context at work. No special-category data. | "Pat owns the finance space." | MOG |
| `open_question` | A question that has no answer yet. | "Which table holds refunds?" | MOG |
| `genie_answer` | A Genie answer that the user kept. It has the exchange link. | "Churn in Q3 was 4.2 %." | MOG |

The kinds `preference`, `definition` and `person` are new. The current kind `entity` becomes `person` in the migration.

### 5.2 Memory file format

Each memory file has this form. The values are placeholders.

```markdown
---
id: 7f0c2e1a-3d4b-4c5e-9f60-1a2b3c4d5e6f
kind: definition
title: Active user
status: active
pinned: true
source: you
origin: pin
mog: mog-a
workspace_host: <host-a>
workspace_id: "1234567890"
profile: profile-a
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

Rules for the format:

- The frontmatter must use only the fields of this list.
- Rust must compute the frontmatter from the store columns. The frontmatter text is not stored.
- The `title` must have 1 to 80 characters.
- The `text` column (the summary line) must have 1 to 600 characters, as today.
- The body must have 4,000 characters or less.
- The `tags` list must have 8 tags or less, each with 32 characters or less.
- The `profile` field holds the CLI profile name. It must never hold a token or a host credential.

### 5.3 Virtual folder layout

```text
/memories/
  index.md                       one line per folder
  shared/
    index.md
    preferences/<slug>.md
  <mog-slug>/                    one folder per MOG = one profile = one workspace
    index.md
    facts/<slug>.md
    definitions/<slug>.md
    decisions/<slug>.md
    people/<slug>.md
    open-questions/<slug>.md
    answers/<slug>.md
```

- The `<mog-slug>` comes from the MOG name. Rust must make it unique on the device.
- The `<slug>` comes from the title, in lower case with hyphens. Rust must add a short id suffix on a clash.
- A path must match the fixed grammar `/memories/(shared|<mog-slug>)/<kind-folder>/<slug>.md`. Rust must refuse each other path.
- The shared folder must hold only `preference` memories in the first slices. Q5 asks the owner about more kinds.
- A Genie answer must never go to the shared folder. It belongs to one workspace (D60).
- Each line of an index file has the form `- [<title>](<kind-folder>/<slug>.md) — <summary, 80 chars or less>`.
- An index file must have 200 lines or less and 25 KB or less. Pinned memories come first, then by `last_used`.

### 5.4 Size caps

| Item | Cap | Effect at the cap |
|---|---|---|
| Body of one memory | 4,000 characters | Rust refuses the write with `body_too_long`. |
| Active memories in one MOG folder | 2,000 | Automatic capture stops. The hub shows "Memory is full". User writes still work. |
| Memories in the shared folder | 500 | The same as above. |
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
| The user clicks "Keep" on a Genie answer card. | `kept_answer` | Deterministic rule | `active`, kind `genie_answer` | 4 |
| The user edits a memory in the hub. | `correction` | Deterministic rule | `active`, source `confirmed`; the old version becomes `superseded` | 3 |
| The user clicks "Ask Genie anyway" on a memory hit. | `correction` | Deterministic rule | The memory loses one point of confidence. | 2 |
| The user turns on "Save dictation words as memories". | `dictation_word` | Deterministic rule | `active`, kind `preference`, tag `dictation-word` | 4 |
| A recording finishes and has notes. | `call_notes` | One model call through the AI Gateway | `proposed` | 4 |
| The agent calls the memory tool. | `agent` | The model, in a tool loop | `proposed` | 5 |
| The consolidation job merges two memories. | `merge` | Rule, or a model call later | `proposed` | 3 |

### 6.2 Write rules

- Each write must run through the R10 statement group `mem.item.write`.
- R10 must run the quarantine check on the title, the summary and the body before each write.
- R10 must run the secret check on the title, the summary and the body before each write.
- If the secret check finds a match, R10 must refuse the write with `secret_refused`. R10 must not store the text.
- The secret check must cover Databricks tokens, JWTs, cloud access keys, private key blocks and `password=` forms.
- A pin must need a valid, unused action ticket (#574).
- The agent and a model writer must never pin.
- A model writer must write only `proposed` memories. Q4 asks the owner about this default.
- A deterministic write from a user action must write an `active` memory.
- Each write from a card or a voice command must show "Saved to memory" with Undo for 8 s.
- Undo must run `mem.forget.items` for that one memory. This is a user action, so U9 holds.
- A model writer must use the home profile of the MOG and the route `POST /ai-gateway/mlflow/v1/chat/completions`.
- A model writer must use a `json_schema` response format (MEM-29a).
- A model writer must never write special-category data. Others-track text becomes "a participant" (MEM-29c).
- Each write must record the source, the origin and the workspace fields (MEM-31). This closes #538 for agent writes.

### 6.3 The call-notes writer (slice 4)

1. A recording ends and `notes.md` is ready (ADR 0103).
2. Rust sends the notes text, not the transcript, to the notes model of the recording settings.
3. The prompt asks for up to 10 candidates of the kinds `definition`, `decision`, `fact` and `open_question`.
4. Each candidate must quote one line of the notes as evidence.
5. Rust checks each quote against the notes. A candidate with no match is dropped.
6. Rust writes each candidate as `proposed`, with `recording_id` and confidence 0.6 to 0.8.
7. The hub shows the candidates under "Proposed". The user approves, edits or rejects each one.

## 7. The consolidation job

### 7.1 Where and when it runs

- The job must run in the Rust core as one background task. It is not a separate process and not a Databricks job.
- The job must start only when Bottle is idle.
- Idle means: no capture, no ask in flight and no user input for 5 min.
- The job must run at most once in 24 h. The user can also click "Tidy up now" in the hub.
- On battery, the job must start only with 50 % charge or more.
- In macOS Low Power Mode, the job must not start.
- The job must stop at once when capture starts or the talk key goes down. It resumes at the next idle.
- Each run must take 2 s of CPU or less for 2,000 memories. A run that needs more continues at the next idle.
- The job must run each change in its own short transaction, so an ask never waits more than 50 ms.

### 7.2 What it does (rules only, slice 3)

| Step | Rule | Change |
|---|---|---|
| Exact dedup | Two memories in one folder have the same normalized summary. | The older one becomes `superseded` by the newer one. Pins win over age. |
| Near match | Two memories share 90 % or more of their FTS tokens. | Bottle shows a "Merge?" card in the hub. The job changes nothing. |
| Stale-out | An unpinned `active` memory has no recall use and no edit for 90 d. | The status becomes `stale`. |
| Revive | The user opens or searches a `stale` memory and selects "Keep". | The status becomes `active` again. |
| Re-scan | The quarantine pattern version changed (for example after #543). | Each memory gets the new check. A hit becomes `quarantined`. A check never lowers a quarantine. |
| Proposed age | A `proposed` memory has no user action for 30 d. | The status becomes `stale`. |
| Cap | A folder is over its cap (5.4). | Automatic capture stops. The hub shows "Memory is full". Nothing is deleted. |
| Index | Any change in a folder. | Rust rebuilds the index file and runs the FTS5 `optimize` command. |

U9 rule: the job must never delete a row. It changes only `status`, `superseded_by` and the index. Q2 asks the owner if the job can delete stale memories after a long time.

### 7.3 Model use (later, off by default)

- A later setting "Use a model to merge similar memories" must be off by default.
- With the setting on, the job sends near-match pairs to the notes model through the AI Gateway.
- The model output must be a `proposed` merged memory. The user approves it in the hub.
- The job must make 20 model calls or less in one run.
- The job must send 50,000 input tokens or less in one day.
- The job must never send a `quarantined`, `restricted` or `genie_answer` memory to a model.

### 7.4 Logs

- Each run must write one log line with counts only: for example `event=memory.consolidate deduped=2 staled=5 rescanned=40 ms=310`.
- A log line must never hold a title, a summary, a body, a tag or a path.

## 8. Recall

### 8.1 Memory-first in the ask path (slice 2, #200)

- The RecallGate must start FTS5 recall on the pre-pass text in the same 10 ms as the router step (MEM-16d).
- The RecallGate must delay `genie_ask` by 300 ms or less (MEM-16a). The local query must take 50 ms or less at p95 (MEM-14).
- Recall must read the MOG folder and the shared folder in two statements. The MEM-01 guard forbids `OR`.
- Recall must use only `active` memories. It must skip `proposed`, `stale`, `superseded`, `retracted` and `quarantined` memories.
- The score must use keyword rank, recency, pin, confidence and kind. Embeddings stay out of scope (AGENTS.md).
- A Genie answer memory must match the workspace and the space of the ask (MEM-17b).
- A Genie answer memory must be 24 h old or less (MEM-17b).
- In slice 2, a hit must never stop the Genie ask. The card shows the hits next to the Genie answer.
- In slice 2, Bottle must not add memory text to the Genie question. A context preamble needs the ContextGate (#152).
- The card must show each hit with the provenance chip "From your memory · <age>".
- A click on the chip must open the memory in the hub.
- A hit that arrives after the gate closes must show as a chip and must not stop the ask (MEM-16c).
- Each recall use must update `last_used` and add one to `use_count`.

"Answer from memory" (Genie is not called) stays later. It needs the golden set with precision 0.95 or more (MEM-17a). Q9 asks the owner about it.

### 8.2 The agent memory tool (slice 5)

The agent memory tool fits the later tool loop of the MOG-to-MOG plan (PR #617, Section 7.10, contract C9).

- The tool must be a Chat Completions function named `memory` with the 6 commands of `memory_20250818`.
- Rust must own the tool. The brain loop must send each call to Rust and must not touch the store.
- The tool must see only the folder of the asking MOG and the shared folder.
- The tool must never show a `quarantined` memory (SEC-24b).
- `view` must return at most 16,000 characters and must accept `view_range`.
- `create`, `str_replace` and `insert` must make a `proposed` memory, or a `proposed` new version of a memory.
- `delete` must mark the memory `retracted` only if the agent made it and it is still `proposed`. Each other `delete` must return an error.
- `rename` must change only the title and the slug. It must not move a memory to another MOG folder.
- Each tool call must write one content-free action log entry (ADR 0085).
- The tool result is data, never instructions (RTR-03c). The agent cannot add memory text to a Genie ask (ADR 0085).
- The route-only planner of PR #617 must never see a memory (MTM-03, MTM-08).
- A peer MOG's memory must never go to the model of another workspace without consent.
- That consent is the synthesis consent of PR #617 Section 7.5.
- The `bottle-mcp` tools `memory.recall` and `memory.write` must use the same R10 code as the memory tool.

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
- The area must let the user pin and unpin a memory. A pin click mints an action ticket.
- The area must have a "Proposed" list with Approve, Edit and Reject for each memory.
- Reject must run `mem.forget.items` for that memory, because the user asked for the delete.
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
- AC-0.3: The migration maps each `entity` row to `person`.

### Slice 1: local memory store, "Remember this" and the hub list (small and useful)

Scope:

1. The action ticket of #574. The host mints it on a card click or a voice command. `mem.item.write` accepts `pinned: true` only with it.
2. The card button "Remember this". It pins the answer summary as a `genie_answer` or `fact` memory.
3. The voice command "remember this: <text>" on the You track. It pins the text as a `fact` memory.
4. The R10 secret check (6.2).
5. The virtual folder read path: `mem.item.list` and `mem.index.get`.
6. The hub Memory area with the folder tree, the list, the file view and Delete. No edit, search or export yet.

Acceptance criteria:

- AC-1.1: A click on "Remember this" stores one pinned, `active` memory with confidence 1.0 and the exchange link.
- AC-1.2: A pin without a ticket gets `invalid_args` from Rust. A replayed ticket gets `invalid_args`.
- AC-1.3: The voice phrase "remember this: the mart refreshes at six" stores one pinned `fact` memory.
- AC-1.4: The phrase "remember this week" stores nothing.
- AC-1.5: A text with a fake Databricks token pattern gets `secret_refused`. The store has no row with the text.
- AC-1.6: A text from the adversarial fixtures stores a `quarantined` memory. The hub list marks it.
- AC-1.7: The hub shows the new memory under its MOG folder with the path `/memories/<mog-slug>/facts/<slug>.md`.
- AC-1.8: Delete in the hub removes the row, its FTS row and its embed job. `mem.recall.fts` no longer finds it.
- AC-1.9: The card shows "Saved to memory" with Undo. Undo removes the memory.
- AC-1.10: No log line has memory text. A log scan test checks this.

Not in slice 1: recall, edit, search, export, the shared folder, automatic capture, the consolidation job and the agent tool.

### Slice 2: recall in the ask path

- The RecallGate of #200 with FTS5 only. The card section "From your memory" with the provenance chip.
- The shared folder for `preference` memories, and the hub search.
- AC-2.1: A trace shows the `genie_ask` delay of 300 ms or less (MEM-16a).
- AC-2.2: A dictation utterance gets no memory verdict (MEM-16e).
- AC-2.3: A `stale`, `proposed` or `quarantined` memory never shows on a card.
- AC-2.4: A Genie answer memory from another workspace never shows (MEM-17b).
- AC-2.5: The Genie question that R8 sends equals the user text. No memory text is in it.
- AC-2.6: A late hit shows as a chip and the ask continues (MEM-16c).

### Slice 3: the consolidation job (rules only)

- ADR 0114. The Rust worker, the power policy, the steps of 7.2 and "Tidy up now".
- The hub edit (origin `correction`) and the "Merge?" cards.
- AC-3.1: With a simulated clock, a 91 d unused unpinned memory becomes `stale`. A pinned one stays `active`.
- AC-3.2: The job deletes no row. A row count test before and after a run proves it.
- AC-3.3: The job does not start on battery under 50 % or in Low Power Mode. A test double gives the power state.
- AC-3.4: The job stops within 100 ms of a talk-key press.
- AC-3.5: A pattern version change quarantines a memory that matches the new pattern.
- AC-3.6: The run log line holds counts only.

### Slice 4: automatic capture

- ADR 0113 is updated with the final status default (Q4).
- "Keep" on Genie answer cards, the dictation words option and the call-notes writer of 6.3.
- The hub "Proposed" list and the export.
- AC-4.1: A finished recording with notes gives up to 10 `proposed` memories, each with a quote that matches the notes.
- AC-4.2: No transcript text goes to the model. The request body test checks this.
- AC-4.3: The writer request uses the home profile and `/ai-gateway/mlflow/v1/chat/completions`.
- AC-4.4: With the dictation words option off, no dictation word becomes a memory.
- AC-4.5: Export writes a zip with one `.md` file for each memory and the index files. The warning shows first.

### Slice 5: the agent memory tool

- ADR 0115. The `memory` function of 8.2 in the tool loop of PR #617 (C9). The `bottle-mcp` tools move to the same code.
- AC-5.1: `view /memories` returns the index of the asking MOG and the shared folder only.
- AC-5.2: The path `/memories/../x` and the path `/memories/<other-mog>/facts/a.md` get an error.
- AC-5.3: `create` makes a `proposed` memory with source `agent`. It never pins.
- AC-5.4: A `quarantined` memory never appears in a tool result.
- AC-5.5: Each call writes one content-free action log entry.

### Later

- Embeddings and hybrid recall (MEM-19a..21b), the model merge of 7.3, "answer from memory", import of edited files, and the hooks of Section 10.

## 12. Contract changes (each needs the owner's sign-off)

| ID | Flag | File(s) | Change | Slice |
|---|---|---|---|---|
| M1 | CONTRACT + SCHEMA | `local_store/migrations/0003_memory_files.sql` | Rebuild `memory_item` with the new kinds and statuses. Add `title`, `body_md`, `tags`, `origin`, `slug`, `profile`, `recording_id`, `last_used_at`, `use_count`, `stale_at`. Map `entity` to `person`. Keep the FTS and vec triggers. | 0 |
| M2 | CONTRACT | `local_store/catalog.rs`, `src/brain/components/memory-store.ts` | New brain ids: `mem.item.list`, `mem.item.search`, `mem.item.edit`, `mem.item.set_status`, `mem.index.get`, `mem.item.export`. New Rust-only ids: `mem.consolidate.*`. | 0, 2, 3 |
| M3 | CONTRACT | `src/shared/interfaces.ts` | `MemoryHit['type']` and `MemoryDraft` get the new kinds, `title`, `body`, `tags` and `origin`. `MemoryDraft` gets the #533 link fields. | 0 |
| M4 | CONTRACT | `src-tauri/src/ipc.rs`, `crates/mog-core/src/ipc.rs`, `src/shared/ipc.ts` | The action ticket of #574: a host-minted, single-use ticket bound to one MOG. `mem_local_exec` accepts `pinned: true` only with it. | 0 |
| M5 | REQUIREMENT | `research/final-requirements.md` (MEM-01) | A reserved MOG id `shared` for the shared folder. Each statement still filters by one `mog_id`. | 2 |
| M6 | CONTRACT | `local_store/catalog.rs`, `quarantine.rs` | A new core-computed param `textSecret`, as `textQuarantine` is. New error code `secret_refused`. | 1 |
| M7 | CONTRACT | `src/shared/ipc.ts` (card events) | Card section kind `memory_hits` and card actions `remember_this` and `keep_answer`. | 1, 2 |
| M8 | CONTRACT | `src/shared/ipc.ts` | New content-free event `memory.changed {mogId, count}` for the hub. | 1 |
| M9 | CONTRACT + SCHEMA | `crates/mog-core/src/app_settings.rs`, schema | New settings: `memory.autoCapture`, `memory.dictationWords`, `memory.modelMerge`. `deny_unknown_fields` makes this a schema change. | 3, 4 |
| M10 | CONTRACT | `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs` | A Rust handler for the `memory` function inside `agent_model_step` (PR #617 C9). | 5 |
| M11 | SCOPE | `AGENTS.md` section 1 | Lift "memory recall" (FTS5 only) from the out-of-scope list. Embeddings stay out. | 2 |
| M12 | REQUIREMENT | `research/final-requirements.md` | New rows for this plan: kinds, statuses, caps, secret check, consolidation job and memory tool. Amend MEM-28a and MEM-29a for the `proposed` status. | 0 |
| M13 | REQUIREMENT | `research/final-requirements.md` (U9, MEM-33a) | Only if the owner says yes to Q2: let the job delete stale memories after a set time. | 3 |

## 13. New ADRs

- **ADR 0112: Local memories as a virtual folder on the SQLCipher store.** It records option (c) and the file format. It also records the folder layout, the path grammar, the caps, the export and migration 0003.
- **ADR 0113: Memory capture rules.** The write triggers, the deterministic and the model writers, the `proposed` status, the action ticket, the secret check and Undo.
- **ADR 0114: The memory consolidation job.** The Rust worker, idle and power rules, the steps, the U9 rule and the content-free log.
- **ADR 0115: Memory recall in the ask path and the agent memory tool.** It records the RecallGate with FTS5 and the provenance chip. It also records the `memory` function with the `memory_20250818` command shape.

Note: the hydration plan (PR #611) names ADR 0109 to ADR 0112 for its own work. Those numbers are not in the reservation file, and ADR 0109 is on main. That plan must reserve new numbers.

## 14. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| Migration 0003 rebuilds a table with triggers. | A bad copy can lose memories. | Use the SQLite table rebuild steps in one transaction. Test on a filled 0002 store. Keep forward-only rules (MEM-35a). |
| The owner wants real files on disk (option a). | Plain text leaks to backups, indexers and other apps. | Keep option (c). Give real files only through Export with the warning. |
| A memory holds injection text. | The agent obeys it. | Quarantine on each write and each re-scan. Tool results are data. Model writes are `proposed`. Fix #543. |
| A memory holds a token or a password. | A secret is on disk. | The R10 secret check refuses the write. Tests use fake values only. |
| A `person` memory holds personal data. | Privacy harm. | No special-category data (MEM-29c). The hub shows each person memory. Q7 asks the owner. |
| Keyword recall gives poor hits. | Noise on the card. | Show 3 hits or less. Never stop the Genie ask in slice 2. Measure on a golden set. |
| U9 blocks the cleanup that the owner wants. | Memory grows with no limit. | `stale` status, caps and "Memory is full". Q2 asks the owner. |
| The job uses power or blocks an ask. | Battery drain or a slow ask. | Idle start, power rules, short transactions and a stop on the talk key. |
| The keychain key is lost. | All memories are lost (MEM-09a). | The hub names the loss. Export is the backup. v1 sync can rebuild. |
| #574 is not done. | Slice 1 cannot pin. | The ticket is part of slice 0 and slice 1. |
| A model writer costs money. | Charges in the home workspace. | Off until slice 4, a call cap and a token cap, and the data panel line. |
| A memory crosses workspaces. | Data from workspace A reaches workspace B. | One folder per MOG, Genie answers never shared, and the PR #617 consent rules. |

## 15. Open questions for the owner

- **Q1 (top). Storage and privacy.** Do you accept option (c), the encrypted store with a virtual folder? Or do you want (a) plain files or (b) encrypted files?
- Q2. U9 says that only Wipe and Forget delete. Can the consolidation job delete stale memories after a set time, for example 365 d?
- Q3. Do you lift "memory recall" (keyword only) from the out-of-scope list for slice 2? Embeddings stay out.
- Q4. Must model-written memories wait for your approval (`proposed`), or go `active` with Undo?
- Q5. Do you accept a shared folder across MOGs (a reserved MOG id)? Which kinds can go in it?
- Q6. Can the consolidation job use a model to merge memories later, with the caps of 7.3?
- Q7. Do you want the `person` kind at all? It holds data about colleagues.
- Q8. Must Wipe delete pinned memories, as it does today?
- Q9. Do you want "answer from memory" (no Genie call) later, after a golden set passes?
- Q10. Must dictation words become memories, or stay a separate setting?
