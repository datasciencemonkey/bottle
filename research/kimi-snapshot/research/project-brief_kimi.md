# Motherbricks project brief

Date: 2026-11-14. Status: input to the requirements director and specialist teams.

## Authoritative product vision

- **Motherbricks** is the project and the local desktop application. Use this exact spelling. Do not use "Mother Databricks" or "Mother Bricks".
- A **MOG** is a small floating agent created and managed by Motherbricks. Motherbricks can run many MOGs.
- A MOG is always available. It captures voice on a global key binding, records meetings when explicitly started, transcribes speech, cleans the utterance, and routes a question or context to Databricks Genie through Genie MCP.
- A question produces a short response blurb near the MOG, with key numbers and a link to the full Databricks result.
- Transcription can use Databricks model serving or an optional local STT model such as NVIDIA Parakeet. A small language model on Databricks cleans fillers, punctuation, false starts and self-corrections.
- A setting controls whether relevant context is sent to Genie.
- Meeting recording was initially described as a 60-minute default with human override. Replace this with a safer signal-based policy if research supports it.
- Every MOG can connect to Genie MCP. The architecture should also permit a Genie agent or a custom agent on Databricks behind the same backend interface.
- A MOG stores agentic memory in Lakebase for fast lookup. Local caching may be required to meet interactive latency.
- A MOG can move dynamically between Genie instances in different workspaces by using Databricks CLI profiles. Sathish Gangichetty's MIT-licensed `antigravity-bricks-plugin` is the reference implementation to borrow from.
- The user's initial stack preference was Flutter for mobile/web portability, or JavaScript. Earlier research recommended Tauri v2 with a Rust core. Treat this as a collision to resolve explicitly.
- The mascot is Genie-inspired and interactive, with idle, listening, processing, answering, success, error and sleeping states. Rive is an asset option, but mascot implementation is not part of this task.
- The user wants a single end-to-end requirements document in ASD-STE100-style simplified English, with inline SVG architecture diagrams. Do not ask nonessential clarification questions.

## Verified or strongly supported findings from earlier research in this thread

These are research inputs, not a substitute for final verification of load-bearing claims.

### Genie and agents

- Default to **Genie One MCP**, GA: `https://<workspace-host>/ai-gateway/mcp-services/system.ai.genie_one_mcp`, streamable HTTP with a bearer token. Tools include `genie_ask`, `genie_poll_response`, `genie_get_query_result` and `genie_cancel_response`.
- The old beta endpoint `/api/2.0/mcp/genie` reaches end of life on **2026-11-30**. Do not build on it.
- A per-space Genie MCP endpoint `/api/2.0/mcp/genie/{space_id}` is Public Preview and requires the Managed MCP Servers preview. Verify current limitations before making it an MVP dependency.
- Genie One MCP supports a polling flow. `genie_ask` returns conversation and response IDs; `genie_poll_response` returns status, final answer, progress steps, query item IDs and a Databricks deep link. Poll calls must not overlap for a response.
- Genie MCP has no separate context-upload tool. Context can only be included in question text or retained as conversation turns. This collides with the phrase "share questions or context with Genie" and needs an explicit design.
- GA Genie space REST remains a useful fallback: `POST /api/2.0/genie/spaces/{space_id}/start-conversation`, follow-up messages, message polling and attachment query-result endpoints.
- Genie One API `POST /api/2.0/genie/one/responses` and Genie Agent mode `POST /api/2.0/genie/agents/{agent_id}/responses` are Beta and use SSE. Keep behind feature detection.
- Custom agents can run on Databricks Apps or Model Serving. Apps invocation and authentication differ from serving endpoints; model this as a separate backend adapter.

### Identity and workspaces

- Reuse the Databricks CLI for user identity: `databricks auth profiles`, `databricks auth token --profile <name>` and `databricks auth login`.
- Only OAuth U2M CLI profiles (`auth_type == databricks-cli`) should be supported initially. Do not rewrite PAT or service-principal profiles.
- Tokens stay in process memory, are refreshed before expiry, and are sent only to the workspace host belonging to the selected profile. Do not put tokens in MOG configuration files or the webview.
- A conversation stays pinned to the workspace and backend where it started. Ambiguous workspace names require a user choice. One-off workspace hopping must not silently change the MOG's home profile.
- The plugin reference is `https://github.com/datasciencemonkey/antigravity-bricks-plugin`, particularly `plugins/databricks-genie/scripts/genie_bridge.py` and the `select-databricks-profile` skill.

### Lakebase memory

- Lakebase database credentials are short-lived (maximum one hour) and can be generated with `databricks postgres generate-database-credential <endpoint-resource>`. Rotate them when opening pooled connections; use TLS.
- `lakebase_vector` provides pgvector-compatible vector search after Lakebase Search is enabled. A self-managed schema is likely preferable because MOG needs custom memory types, provenance, retention and SQL visibility.
- Databricks managed Agent Memory / Agent Sessions should be evaluated as an alternative, not assumed to fit.
- Fast lookup requires a local encrypted cache plus Lakebase as the durable memory store. Network-only memory cannot reliably meet a 50 ms recall target.

### Voice, desktop and meetings

- Earlier research recommended local Parakeet TDT 0.6B v3 with sherpa-onnx and Silero VAD for low-latency dictation, with Databricks-hosted ASR as an alternative. Do not assume a managed Databricks Foundation Model ASR endpoint exists without verification.
- Cleanup can use a low-latency Databricks Foundation Model endpoint with a deterministic pre-pass and a strict deadline. Exact model names must be verified or described as configurable examples.
- Flutter has portability appeal, but earlier research found weak support for global key-release detection and non-activating floating panels. Tauri v2 plus a native Rust core was recommended; a Flutter shell could reuse that core through FFI. Global hotkeys, system audio and paste-into-app cannot port to iOS/web unchanged.
- Recording must never start silently. Show a persistent recording indicator and consent UX. Stop on manual request, call-app mic release, calendar end with prompt, prolonged silence, sleep or low disk. Suggested policy: 15 minutes of silence, a 90-minute soft cap that extends only while speech continues, and a 4-hour hard ceiling. Journaling must be crash-safe.
- Keep microphone and system/loopback audio as separate tracks when supported. Chunk long recordings into ASR-safe segments.

## Expected final content

1. Executive requirements rundown.
2. Product boundaries, actors and glossary.
3. End-to-end architecture with inline SVG diagrams, including a state/lifecycle view and trust boundaries.
4. Numbered requirements with priorities, release phase and testable acceptance criteria.
5. MOG lifecycle: create, configure, sign in, listen, transcribe, clean, route, ask, poll, render, remember, switch workspace, pause, stop and delete.
6. Meeting policy, multi-MOG arbitration, offline behavior, error recovery and security/privacy.
7. Collision register and decision log. Resolve conflicts explicitly; mark only genuinely existential decisions for the user.
8. Administrator prerequisites, nonfunctional budgets, phased roadmap, verification strategy and sources.

## Output constraints

- Final file: `/home/sandbox-agent/workspace/mother-databricks/requirements_kimi.html`.
- One self-contained HTML file; inline CSS and SVG, no external scripts, fonts or stylesheets.
- Use ASD-STE100-style simplified English: short sentences, one instruction per sentence, approved/simple words where practical, and consistent terms. Technical names and API paths remain exact.
- Do not claim full formal ASD-STE100 certification unless a controlled-language checker actually validates it. Label it "STE100-style" or "written using STE100 principles".
- External links are allowed only as citations. All diagrams must be inline SVG with accessible titles/descriptions.
- Do not include secrets or real workspace identifiers.
