# Bottle (Mother Databricks): Requirements (draft v0.2, 2026-11-14)

> **Bottle** is the desktop app. You use it to create **MOGs**: small floating agents that are always on. A MOG listens, transcribes, and sends questions or context to **Genie through the Genie MCP**. When it hears a question, it shows a short blurb of Genie's answer.
>
> Each MOG keeps its agent memory in **Lakebase** for fast lookup. It can poll Genie for feedback, and it can move between Genie in different workspaces because it runs on top of the **Databricks CLI**.

This document turns that concept into requirements. Items marked **❓** need a decision from you; they are collected in [§9](#9-open-questions-for-you). The underlying research is in `../MOG_RECOMMENDATIONS.md`.

---

## 1. Terms

| Term | Meaning |
|---|---|
| **Bottle** | The desktop app (the hub). It creates, configures, starts and stops MOGs. It also owns sign-in, the tray icon, settings, history, and the shared local services: audio, speech-to-text and the MCP client. |
| **MOG** | One floating agent window with a mascot. Each MOG has its own *binding* (workspace profile + Genie target), its own *memory namespace*, its own *capture mode*, and its own *persona/instructions*. You can run several at once. |
| **Binding** | Where a MOG sends things: a Databricks CLI profile (which fixes the workspace) plus a Genie target (Genie One across all data, or one Genie space). |
| **Utterance** | One transcribed span of speech, after cleanup. |
| **Context** | Something the MOG hears that is *not* a question, for example a meeting statement or a decision. It is remembered and can be given to Genie later. |
| **Blurb** | A short answer card (2–3 lines plus key numbers) shown next to the MOG, with a link to the full Genie answer. |

## 2. Users and scenarios

1. **Ask out loud.** You hold a hotkey and say "what were EMEA bookings last week?". The MOG shows a blurb with **$4.2M** and a "View in Databricks" link.
2. **Meeting companion.** The MOG records a meeting. Questions people ask ("do we know churn for Q3?") are found, sent to Genie, and the answers appear as blurbs during the meeting. Everything else is kept as context in memory.
3. **Specialist MOGs.** You run a Finance MOG bound to `prod-finance` and a Sales MOG bound to `sales-ws`. Each is a separate window with its own memory and its own Genie.
4. **One question across workspaces.** One MOG splits a question into parts per workspace, asks each workspace's Genie, and merges the answers with where each number came from. This is the plugin's `select-databricks-profile` behaviour.
5. **Recall.** "What did we decide about the pricing table yesterday?" is answered from Lakebase memory, and Genie is never called.

## 3. System shape

```
┌──────────────── Bottle (desktop app) ───────────────────────────────┐
│  MOG manager ── creates/starts/stops MOGs, persists their configs          │
│  Shared services (one each, used by all MOGs):                             │
│   • Audio: mic + optional system audio, VAD, one capture lock              │
│   • STT: local Parakeet (default) | Databricks-served ASR (opt.)           │
│   • Cleanup SLM: Databricks FMAPI endpoint                                 │
│   • Router: is this a question, context, a command, or noise?              │
│   • Genie bridge: MCP client → Genie One MCP, signed per profile           │
│     (port of genie_bridge.py)                                              │
│   • Profile service: wraps `databricks auth profiles/token/login`          │
│   • Memory service: Lakebase pool (credential rotation) + local cache      │
│                                                                            │
│  ┌ MOG "Finance" ┐ ┌ MOG "Sales" ┐   floating windows, mascot + blurbs     │
│  │ profile A     │ │ profile B   │   (Rive art; states: idle/listening/    │
│  │ Genie One     │ │ space 01ef… │    processing/answering/success/error/  │
│  │ mem ns: fin   │ │ mem ns: sal │    sleeping)                            │
│  └───────────────┘ └─────────────┘                                         │
└──────────────┬──────────────────────────────┬──────────────────────────────┘
               │ Bearer token from            │ Postgres (TLS), password from
               │ `databricks auth token`      │ `databricks postgres
               ▼                              │  generate-database-credential`
  https://<ws>/ai-gateway/mcp-services/       ▼
  system.ai.genie_one_mcp                  Lakebase project/branch/endpoint
  (genie_ask / genie_poll_response /       (mog_memory tables, lakebase_vector)
   genie_get_query_result /
   genie_cancel_response)
```

## 4. Functional requirements

Priorities: **M** = must for MVP, **S** = should, **C** = could.

### 4.1 Bottle (hub)

| ID | Requirement | Pri |
|---|---|---|
| MB-1 | Create a MOG with a wizard that asks for: name, mascot variant/colour, Databricks CLI profile, Genie target (Genie One or a specific space), capture mode, memory on/off, and persona instructions. | M |
| MB-2 | List, start, stop, duplicate, edit and delete MOGs. MOG configs are saved locally as JSON or YAML, with **no secrets** in them. | M |
| MB-3 | A tray/menu-bar icon shows global state: recording indicator, how many MOGs are active, and pause all. | M |
| MB-4 | Global hotkeys: hold to talk to the *focused* MOG; one chord per MOG (❓); a panic "stop all recording" key. | M |
| MB-5 | A history view of every utterance, its route, the Genie conversation, and the blurb, filterable by MOG. | S |
| MB-6 | Export or share a MOG definition (no tokens) so a teammate can import it. | C |

### 4.2 MOG (floating agent)

| ID | Requirement | Pri |
|---|---|---|
| MOG-1 | A frameless, always-on-top, non-focus-stealing window you can drag. The mascot is clickable; clicking toggles listening (❓). | M |
| MOG-2 | Mascot states driven by the pipeline: `idle → listening → processing → answering → success / error`, plus `sleeping` when paused. The Rive view model is `mood`, `level` and `poke`. | M |
| MOG-3 | **Capture modes:** (a) push-to-talk, (b) hands-free toggle, (c) **always-on / meeting** with VAD segmentation and the stop policy in §4.6. | M (a, b), M (c) ❓ |
| MOG-4 | Every utterance goes through: STT → cleanup → **router**. | M |
| MOG-5 | **Router.** It classifies each utterance as one of: `question` (send to Genie), `context` (keep in memory, and later possibly give to Genie), `command` (for example "switch to sales workspace", "stop recording"), or `noise`. In always-on mode, a question is only sent to Genie automatically if it meets a confidence threshold *and* the MOG's auto-ask setting is on (❓). | M |
| MOG-6 | **Blurb.** A question shows a card while Genie works, with live progress text from `progress_steps` ("Running SQL on `sales.bookings`…"). When it finishes, the card shows a 1–3 line answer, **bold key numbers**, and a "View in Databricks" link (`deep_link`). Clicking expands the card to the full answer and a table preview. | M |
| MOG-7 | Blurb queue: if several questions are in flight, cards stack (at most 3 visible) and old ones collapse into history. | S |
| MOG-8 | Follow-ups ("and by region?") continue the same Genie `conversation_id` while the topic stays the same (❓ topic-reset rule). | S |
| MOG-9 | Cancel a running question (Esc on the card), which calls `genie_cancel_response`. | S |
| MOG-10 | The MOG shows which workspace and profile it is using: a badge on hover ("prod-finance · Genie One"). This matches the plugin's "Asking Genie in <host> (profile <p>)" rule. | M |

### 4.3 Genie through MCP

| ID | Requirement | Pri |
|---|---|---|
| G-1 | Connect to the **Genie One MCP**: `https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp`, streamable HTTP, `Authorization: Bearer <token>`. **Do not** use the Beta `/api/2.0/mcp/genie` endpoint; it shuts down on **2026-11-30**. | M |
| G-2 | Tools used: `genie_ask{question, conversation_id?}`, `genie_poll_response{conversation_id, response_id}` (polled one at a time, never overlapping), `genie_get_query_result{…, item_id}` (for the expanded table), and `genie_cancel_response`. | M |
| G-3 | **"Poll Genie for feedback"** means narrating progress from `progress_steps` into the blurb and mascot. It also includes re-polling after a client timeout with the *same* IDs, never by asking again. These timeout and duplicate-avoidance rules come from the plugin. | M |
| G-4 | **Sharing context with Genie.** Genie MCP has no "upload context" tool, so context can only reach Genie inside a question's text or as earlier turns in its conversation. Requirement: when sending a question, the MOG may add relevant memory to it, for example "Context: we're discussing EMEA enterprise deals, FY27 plan…". This must stay under a size cap and must be visible in the expanded card (❓ whether you want this at all). | S |
| G-5 | Optional per-space binding: the per-space Genie MCP `/api/2.0/mcp/genie/{space_id}` is **Public Preview**, needs the admin's "Managed MCP Servers" preview, and keeps no history. It can be used when a MOG must be pinned to one curated space. Without it, Genie One routes across all data. (❓ do MOGs need space pinning for MVP?) | C |
| G-6 | Pluggable backends behind one `MogBackend` interface: Genie One MCP (default), Genie space REST (GA), Genie Agent mode (Beta, SSE), and a custom Databricks agent (Apps `/api/invocations` or `/responses`). This covers your earlier "Genie MCP, Genie agent or custom agent" requirement. | S |

### 4.4 Workspaces through the Databricks CLI (borrowed from the antigravity plugin)

| ID | Requirement | Pri |
|---|---|---|
| W-1 | Bottle stores **no tokens**. For each call it runs `databricks auth token --profile <p> --output json`, caches the token in memory until shortly before it expires, and drops it on a 401. This is a direct port of the plugin's `CLI.token()`. | M |
| W-2 | Profile discovery: `databricks auth profiles --skip-validate --output json` lists name, host and auth_type. Only `auth_type == databricks-cli` (OAuth U2M) profiles can be used. PAT and service-principal profiles are shown as unsupported and are **never rewritten**. | M |
| W-3 | First-run and new-workspace setup: ask the user for the workspace URL (never guess it), validate it against the Databricks domains, and run `databricks auth login --host <url> --profile <name>` in the background so the browser opens for SSO. Never point an existing profile at a different host. | M |
| W-4 | Find the CLI on PATH plus the usual folders (`/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, and Windows paths). If it's missing, offer the official installer and only run it after the user agrees. | M |
| W-5 | **Dynamic hopping.** A MOG has a *home* profile. A spoken or typed command ("ask sales workspace…", "use profile B from now on") changes the target for one question or for the session, using the same rules as the plugin's `select-databricks-profile` skill. If more than one profile matches, the MOG asks. A conversation stays in the workspace where it started. | M |
| W-6 | Cross-workspace questions: split them per workspace, ask each part, merge the answers, and attribute each number to its workspace. | S |
| W-7 | Respect only `DATABRICKS_CONFIG_PROFILE` and `DATABRICKS_CONFIG_FILE`. Ignore a stray `DATABRICKS_HOST` or `DATABRICKS_TOKEN`, as the plugin does. | M |
| W-8 | Clear error mapping, reusing the plugin's cases: `unsupported_auth`, `profile_exists_unreadable`, HTTP 403 (IP access list or VPN), 404 (Genie One not enabled), a non-JSON response (proxy or SSO page), and timeouts. | M |

### 4.5 Memory in Lakebase

| ID | Requirement | Pri |
|---|---|---|
| MEM-1 | Each MOG has a memory namespace in **one Lakebase Postgres endpoint** (`projects/<p>/branches/<b>/endpoints/<e>`) chosen at setup. | M ❓ |
| MEM-2 | Auth: get the password from `databricks postgres generate-database-credential <endpoint>` using the same CLI profile. It lasts at most 1 hour, so get a **new one for each new pooled connection**. Connect with TLS (`sslmode=require`), user = the user's Databricks identity. | M |
| MEM-3 | Schema, at minimum: `mog`, `session` (meeting or chat), `utterance` (text, speaker track, time, route), `memory_item` (distilled fact or decision, `embedding VECTOR(n)` through `lakebase_vector`, source utterance IDs), and `genie_exchange` (question, conversation_id, response_id, final answer, deep_link, profile/host). | M |
| MEM-4 | **Fast lookup** has two tiers. A local cache (SQLite) holds recent items so recall works offline in under 50 ms. Lakebase holds the durable store, searched by `lakebase_ann` cosine similarity plus SQL filters (MOG, time, speaker). Recall happens before calling Genie: if memory answers the question with high confidence, Genie isn't called. | M |
| MEM-5 | A memory-writer step (SLM) runs after each session, or every N minutes in always-on mode. It distils utterances into `memory_item`s (decisions, entities, open questions) and embeds them with a Databricks embedding endpoint (❓ which model, which sets the vector dimension). | M |
| MEM-6 | **Genie feedback into memory.** Each completed Genie answer is saved as a `genie_exchange` and summarised into a memory item, so "what did Genie say churn was?" is answered from memory. | S |
| MEM-7 | Evaluate Databricks' managed **Agent Memory / Agent Sessions**, which are backed by Lakebase, against a self-managed schema. The self-managed schema is recommended because we need SQL visibility and our own retrieval (❓). | S |
| MEM-8 | Privacy controls: memory off per MOG, "forget this session", a retention period (❓ default), and raw audio **never** uploaded by default. | M |

### 4.6 Always-on recording and the meeting stop policy

This carries over from the research report §5.

- **Never start silently.** Recording begins only from a user action or an accepted "meeting detected" prompt, and a **visible recording indicator** shows on the MOG and in the tray.
- **Stops on signals:** manual stop, the call app releasing the mic, calendar end time (with a prompt), 15 minutes of silence, the machine sleeping or disk running low, a 90-minute soft cap that extends itself while speech continues, and a 4-hour hard ceiling. These replace a fixed 60-minute limit.
- **Transcript tracks:** "You" for the mic and "Others" for system audio. Chunks of 30 seconds or less are transcribed live, and the journal is crash-safe.
- ❓ **"Always-on"** might mean *all day, listening for a wake word or questions* rather than *during meetings*. That changes privacy, battery and legal requirements a lot. See Q1.

## 5. Non-functional requirements

| Area | Requirement |
|---|---|
| Latency | Push-to-talk: transcribed and cleaned text in ≤ 1.5 s at p95. Genie blurb: first progress text in ≤ 3 s, final answer usually 10–60 s, and the UI must stay responsive throughout. Memory recall: ≤ 50 ms locally, ≤ 300 ms from Lakebase. |
| Security | Tokens stay in memory only; the Databricks CLI owns its token cache. Nothing secret goes in MOG configs. A token is only ever sent to its profile's host. All Genie and Lakebase access runs as the signed-in user, so Unity Catalog permissions apply. |
| Governance | Every request is tagged with the MOG name and ID so admins can attribute usage. Who gets audio and transcripts is clear: local by default, Lakebase only if memory is on, Genie only for routed text. |
| Platforms | macOS 14.4+ first, Windows 11 next, Linux best-effort. Shell: Tauri v2 + Rust core per the report, or Flutter (❓ final call). The Genie bridge and profile service sit behind a small interface so they can run in a Rust or Dart host, or as a Python sidecar reusing `genie_bridge.py` almost unchanged. |
| Reliability | Many MOGs share one audio capture, so there is one owner of the mic at a time. Poll loops back off; poll calls never overlap per conversation. Offline mode keeps transcription and local memory working and queues Genie questions. |

## 6. What to borrow from `antigravity-bricks-plugin`

The plugin is MIT licensed, so keep its license notice. It was reviewed at commit `a31fd2c`, 2026-10-24.

| Plugin piece | Reuse in Bottle |
|---|---|
| `scripts/genie_bridge.py`: CLI token sourcing, Genie One MCP forwarding, timeouts, error mapping | **Port as is** as the Genie bridge, either as a Python sidecar or translated to Rust/TS. Remove the Antigravity-specific 150 s/3 min tool-timeout limits; our UI is async. |
| `genie_connection` / `genie_setup` tools | Become the hub's *Connect workspace* screen and setup flow (W-2, W-3). |
| `select-databricks-profile` skill rules | Become the MOG router's **workspace-hopping policy** (W-5, W-6). |
| `antigravity-databricks-genie` skill and the "always route data questions to Genie" rule | Become the system prompt for the MOG's router and answer-formatter. |
| `genie_tools.json` (cached tool schemas) | Used as a fallback and contract test for the MCP tool shapes. |
| Test suite (`test_genie_bridge.py` etc.) | Used as a starting point for bridge contract tests. |

## 7. Out of scope (for now)

Mobile clients, a web client (it would need a CORS proxy), diarization by named speaker, writing to Genie spaces (instructions or curation), and auto-joining calls as a bot.

## 8. Phasing

- **MVP:** Bottle shell; 1–N MOGs; push-to-talk and toggle; local STT and cleanup; router (question/context/command); Genie One MCP bridge (ported); blurbs with progress; CLI profiles and hopping; Lakebase memory (write and recall) plus local cache.
- **v1:** meeting / always-on mode with the stop policy; cross-workspace split questions; memory-augmented questions (G-4); history UI; per-space Genie MCP; custom-agent backend.
- **v2:** shareable MOG templates, a Genie Agent mode "deep analysis" button, managed Agent Memory option, and mobile.

## 9. Open questions for you

1. **What does "always-on" mean?** (a) Meetings only, started by a click or a prompt. (b) All-day ambient listening that only acts on a wake word or on questions. (c) All-day full transcription. *Recommendation: (a) for MVP and (b) later. (c) brings heavy consent and legal requirements.*
2. **Auto-asking Genie.** In meeting mode, should a detected question go to Genie **automatically**, or appear as a "Ask Genie?" chip that you confirm with one click? *Recommendation: the chip by default, with auto-ask as a per-MOG setting.*
3. **Context to Genie (G-4).** Should a MOG add memory context to questions it sends to Genie, or send only the question you asked?
4. **Where memory lives.** One shared Lakebase endpoint for all of a user's MOGs (one namespace per MOG), or each MOG chooses its own (possibly per workspace)? Also, which workspace hosts it when a MOG moves between workspaces? *Recommendation: one "home" Lakebase per user, so memory follows the MOG across workspaces.*
5. **Is memory personal or shared?** Can teammates read a MOG's memory, for example a shared "Team Finance MOG"?
6. **Space pinning.** Is Genie One (all data) enough for MVP, or must some MOGs be pinned to a specific Genie space?
7. **Shell.** Is Tauri + Rust core (the research pick) acceptable, or is Flutter required? And may the MVP run the plugin's Python bridge as a sidecar to ship faster?
8. **Retention.** Default retention for transcripts and memory (for example 30 days for transcripts, memory items until deleted)? Is there an org policy we must follow?
9. **Hotkeys and multiple MOGs.** Should there be one global push-to-talk key that goes to the last-focused MOG, or a separate key per MOG?
10. **Sathish's involvement.** Should we coordinate with Sathish to pull the bridge into a shared package (for example `databricks-genie-bridge`) used by both the Antigravity plugin and Bottle, rather than forking it?
