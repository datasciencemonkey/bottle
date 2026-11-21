# Plan: a MOG can talk to other MOGs (agent across workspaces)

- Status: draft plan for the owner. Not decided. Drafted to STE100 rules.
- Date: 2026-11-21
- Author: architect (research and design only; no app code changes)
- New ADRs (numbers reserved with `adr-next.sh`): ADR 0110, ADR 0111 (section 12).
- Live calls: none. No Databricks call and no model call ran for this plan.

## 0. Summary

The user wants to ask one question and get answers from the Genies of more than one workspace. Each MOG is
bound to one CLI profile. So "a MOG talks to other MOGs" means "one question goes to the Genie One of more
than one profile".

The recommendation: do not add an agent SDK for this step. Build a small **MOG router** in Bottle.

1. One planner model call runs in Rust, through the AI Gateway route that Bottle already uses.
2. The planner sees the question and the list of the user's MOGs. It never sees a Genie answer.
3. The user sees the plan (which MOG gets which question) and clicks Ask.
4. Bottle code (not the model) sends each part to Genie One, in parallel.
5. The card shows each answer verbatim, with its MOG, workspace and Open in Databricks link.

A real tool loop (the model reads answers and asks again) is a later step. For that step, the plan
recommends a manual tool loop in the brain over a new Rust model command. It does not recommend the Claude
Agent SDK or the OpenAI Agents SDK.

## 1. Purpose and scope

This plan answers these questions:

- Which agent technology fits Bottle's trust rules.
- How a MOG asks other MOGs: routing, split, merge, consent, guards, budgets, failures and UI.
- How the new path fits the direct ⌥G path.
- Which first PR proves it, and which contract changes it needs.

**Scope conflict (must go to the user).** `AGENTS.md` section 1 puts "workspace hop and split
questions" out of scope. It also puts "the main agent and `bottle-mcp` tools" out of scope. This plan touches the split part. It does
not start the R16 main agent. The user must lift the split item from the out-of-scope list before slice 1
starts. See open question Q1.

## 2. Glossary

| Term | Meaning in this plan |
|---|---|
| MOG | A small floating assistant window. Each MOG has one home profile (`mog.binding.home`). |
| Asking MOG | The MOG that the user speaks to. It owns the card and the plan. |
| Peer MOG | Another MOG of the same user. It has its own home profile, so its own workspace and Genie One. |
| Profile | One Databricks CLI profile. One profile is one workspace. The user picks it; Bottle never picks one. |
| Brain | The hidden TypeScript window (`src/brain`). It owns flow and UI state. It never holds a token. |
| View | A React window (MOG, card, hub). A view only listens and sends `view_emit` (ARC-4). |
| Genie One | The Databricks Genie One MCP server of one workspace. |
| Direct path | Today's ⌥G flow: the exact text goes to Genie One of the MOG's home profile (ADR 0097). |
| MOG router | The new flow in this plan: one planner call, a user approval, then parallel Genie asks. |
| Planner | The one model call of the MOG router. It returns a route plan as strict JSON. |
| Route plan | The planner output: 1 to 3 parts. Each part has one peer MOG ID and one question text. |
| Part | One question for one MOG in a route plan. |
| Split | A route plan with 2 or 3 parts. |
| Catalog | The list of MOGs that the planner may name: MOG ID, MOG name and workspace label only. |
| Route-only mode | The planner sees the question and the catalog only. It never sees an answer. |
| Synthesis mode | A later mode. A model reads the answers and writes one combined text. Not in slice 1. |
| Plan ticket | A Rust record of one approved route plan. It holds a SHA-256 of each part, never the text. |
| Workspace label | The host of a profile, as the hub shows it. In this document: `<host-a>`, `<host-b>`. |
| Provenance | The data that says which MOG, profile and workspace gave each answer, with its link. |
| Tool loop | A model loop that asks for tool calls, gets results, and calls the model again. |
| AI Gateway | The Databricks route `/ai-gateway/...` of a workspace. Bottle reaches models only through it. |

Example names in this plan are placeholders: `profile-a`, `profile-b`, `MOG A`, `MOG B`.

## 3. What exists today (facts from the code)

| Part | Fact | Where |
|---|---|---|
| Direct path | ⌥G sends the exact text to T8 GenieBridge `submit` on the MOG's home profile. No agent takes part. | `src/brain/genie-direct.ts:74`, ADR 0097 |
| Token | R6 gets tokens only from `databricks auth token --profile <p>`. Other code lends a token for one call through `with_token`. | `crates/mog-core/src/token_broker.rs:1-24`, `:150-174` |
| Genie allowlist | R8 calls 4 tools only: `genie_ask`, `genie_poll_response`, `genie_get_query_result`, `genie_cancel_response`. | `crates/mog-core/src/mcp_transport/tools.rs:24-45` |
| Profiles | T7 lists only `databricks-cli` (U2M) profiles as supported. It never picks a profile. | `crates/mog-core/src/profile_service.rs:1-21` |
| R16 AgentHost | One shared harness sidecar per harness kind, one session per MOG, loopback model proxy, no token in the child. The runtime program is not chosen (SP-19, #111). | `crates/mog-core/src/agent_host.rs:1-46`, `:249`, ADR 0014 |
| Model proxy | Claude harness goes to `/ai-gateway/anthropic`. Codex harness goes to `/ai-gateway/mlflow/v1`. The real upstream is a stub (502). | `crates/mog-core/src/agent_host/adapter.rs:69`, `:92`, `agent_host/model_proxy.rs:1-17` |
| `bottle-mcp` | 7 tools. No tool starts capture, approves context, publishes or switches the workspace. | `crates/mog-core/src/bottle_mcp.rs:57-65` |
| agent_guard | A data-only system prompt, `builtinTools: none`, a content-free action log, and `OutboundApprovals`: the agent may send only the exact approved question, once. | `crates/mog-core/src/agent_guard.rs:38-65`, `:162-210`, ADR 0085 |
| Hop chips | Only a click on "Switch to <profile>?" calls `profile_select`. An agent call never does. | `src/brain/components/agent-hop.ts:1-11`, `:69` |
| Router dispatch | A split result is refused with `split_not_supported` (v1). | `src/brain/components/router-dispatch.ts:136-137` |
| Router model | `gpt-oss-20b` or Llama 3.1 8B, eval-gated (NFR-22a). | `src/brain/components/router-model.ts:9-40` |
| Tool guard | `model_invoke` refuses any request with `tools`, `tool_choice` or `functions` (RTR-20a). | `crates/mog-core/src/ipc_gateway.rs:352`, `model_request_guard.rs:10-36` |
| Model route | Recording calls `POST /ai-gateway/mlflow/v1/chat/completions` from Rust, with the default endpoint `databricks-claude-sonnet-5-5`. | `crates/mog-core/src/recording_cloud/cloud.rs:43`, `types.rs:805`, `http_transport/routes.rs:141` |
| CSP | No WebView has an external `connect-src`. Only `ipc:` is allowed. | `research/final-requirements.md:290` (ARC-4), SEC-07a |
| Config | `mog.binding.home`, `mog.binding.backend.profile`, `mog.genie.shareContextCrossWorkspace` (default false), `mog.agent.harness` (default claude). The schema denies unknown fields. | `crates/mog-core/src/mog_config.rs:109`, `:256-269`, `:307-326`, `:340-346` |
| Local store | `genie_exchange` already has a `split_group_id` column. | `crates/mog-core/src/local_store/migrations/0001_local_mirror.sql:126` |
| Requirements | Split is v1: RTR-11b, W-13a..c, G-50, SEC-29a, SEC-29b, UX-11b, SC-6, F6. W-21a keeps model calls on the home profile. | `research/final-requirements.md:874`, `:1062-1064`, `:1017`, `:1271-1272`, `:1082`, `:1607-1614` |

Two facts drive the design:

- The CSP blocks every direct network call from the brain. So any SDK in the brain must send its HTTP
  through a Rust IPC command. A plain `baseURL` change does not work.
- `OutboundApprovals` already binds an agent send to the exact user-approved text. The MOG router reuses
  this idea for each part.

## 4. SDK findings (official documents, fetched 2026-11-21)

### 4.1 OpenAI Agents SDK for TypeScript (`@openai/agents`)

- **Handoffs and agents as tools.** A handoff gives part of a conversation to another agent. The model sees
  each handoff as a tool. Handoffs stay inside one run. For a specialist that stays behind the first agent,
  the docs say to use agents as tools (`agent.asTool()`). [OA-1]
- **Model layer.** The SDK has a `Model` interface (`getResponse(request): Promise<ModelResponse>` and
  `getStreamedResponse(request): AsyncIterable<StreamEvent>`). A `ModelProvider` maps a name to a `Model`
  (`getModel(modelName?)`). You pass it with `new Runner({ modelProvider })` or `setDefaultModelProvider`.
  [OA-2], [OA-3]
- **OpenAI-compatible endpoint.** `OpenAIProvider` takes a `baseURL`. `setOpenAIAPI('chat_completions')`
  selects Chat Completions; the default is `'responses'`. [OA-2]
- **Fit for the Rust IPC rule.** A `baseURL` at the Databricks route needs a token in the brain. The CSP
  also blocks it. A custom `Model` that calls a Rust IPC command keeps the token in Rust. This is possible by
  the interface. Bottle then writes the request and reply mapping itself.
- **Runtime.** The README lists Node.js 22 or later, Deno and Bun. Cloudflare Workers is experimental.
  Browsers are not in the supported list. Browser use is shown only for realtime voice. [OA-4]
- **Tracing.** Tracing is on by default in server runtimes. It exports to the OpenAI backend through
  `OpenAITracingExporter`. It is off by default in browsers. It turns off with
  `OPENAI_AGENTS_DISABLE_TRACING=1` or `RunConfig.tracingDisabled: true`. [OA-5]
- **Conclusion.** Bottle can make it comply with a custom `Model` and tracing off. But the WebView is a
  runtime that the README does not list. Its main value (handoffs between model agents) is a
  poor match: Bottle wants code, not the model, to decide each send.

### 4.2 Claude Agent SDK (`@anthropic-ai/claude-agent-sdk`)

- **Process.** The SDK "runs the Claude Code binary". It bundles a native binary as an optional
  dependency and spawns it as a subprocess. Options include `pathToClaudeCodeExecutable`, `executable`
  (`bun`, `deno`, `node`) and `spawnClaudeCodeProcess`. [CA-1], [CA-2], [CA-3]
- **In Tauri.** It needs a sidecar process. Tauri ships a sidecar through `bundle.externalBin` with a
  target-triple suffix. [TA-1] This is the R16 shape of ADR 0014. It adds a second large binary and a
  second signing and notarization item.
- **Model endpoint.** Claude Code speaks the Anthropic Messages format (`/v1/messages`), chosen by
  `ANTHROPIC_BASE_URL`. Bedrock and Vertex formats exist. No OpenAI Chat Completions format is listed.
  [CA-4] Databricks serves the Anthropic Messages API at `/ai-gateway/anthropic/v1/messages` for
  pay-per-token Claude models, with a bearer token. [DB-3]
- **Auth.** The quickstart uses `ANTHROPIC_API_KEY`, or the Bedrock, Vertex, Foundry and AWS variables.
  A gateway gets `ANTHROPIC_AUTH_TOKEN` or an `apiKeyHelper` value as a bearer. [CA-2], [CA-4]
  For Bottle, `ANTHROPIC_BASE_URL` must point at the R16 loopback model proxy, with the per-session
  secret as the credential. The proxy adds the Databricks token in Rust (ADR 0014 item 7).
- **Lock-down.** `tools` sets the built-in tool list. `disallowedTools` removes a tool definition.
  `allowedTools` only pre-approves; other tools still exist. `permissionMode: "dontAsk"` turns each
  prompt into a deny. `settingSources: []` stops the load of user and project settings. `persistSession:
  false` stops the session files on disk. [CA-3], [CA-5]
- **Fit.** It is a coding agent with file, shell and web tools. Bottle must turn all of them off, and
  ADR 0014 still lists a shell residual risk. For a router that only sends questions to Genies, it is
  heavy. It brings a subprocess, a cold start, settings on disk and the Commercial Terms. [CA-1]

### 4.3 Anthropic TypeScript SDK: Tool Runner or a manual loop

- **Runtimes.** Node 20+, Deno, Bun, Workers, Edge. Browsers need `dangerouslyAllowBrowser: true`. [AN-1]
- **Custom fetch.** The client takes a `fetch` function and `fetchOptions`. [AN-1] So a fetch shim can
  send each request to a Rust IPC command. The brain then holds only a dummy key. Rust adds the token.
- **Tool Runner.** `client.beta.messages.toolRunner()` runs the tool loop with `betaZodTool()` or
  `betaTool()`. It is beta. The docs say: for human-in-the-loop approval, use the manual loop. [AN-2]
- **Databricks routes.** (a) Native Anthropic Messages at `/ai-gateway/anthropic`, for Claude
  pay-per-token models. The page does not mention tool use. [DB-3] (b) OpenAI-compatible Chat Completions
  at `/ai-gateway/mlflow/v1/chat/completions`. [DB-1] Databricks function calling is OpenAI-compatible.
  It lists Claude Sonnet 5.5 and Opus 5.5. Parallel function calling is not supported. The limit is 32
  functions. [DB-2]
- **Fit.** A manual loop is about 150 lines. It needs no SDK at all if it uses Chat Completions through
  Rust. NFR-21a requires the `/ai-gateway/mlflow/v1` route for model calls, so route (b) also fits the
  current requirement text.

### 4.4 The in-repo R16 AgentHost and `bottle-mcp`

- The Rust guard parts are built and tested (ADR 0083, ADR 0085). They are the action log,
  `OutboundApprovals`, the Genie One proxy with its ask ledger, and the 7-tool server.
- The harness is not chosen (SP-19, #111). The model proxy upstream is a 502 stub. The host wiring is
  `TODO(#563)`.
- It assumes a harness process. The MOG router does not need one.

## 5. Comparison table

Legend: Yes = meets the rule as is. Work = meets it with Bottle code. No = conflicts.

| Rule or need | A. OpenAI Agents SDK (TS) | B. Claude Agent SDK | C. Anthropic SDK Tool Runner / manual loop | D. R16 AgentHost + `bottle-mcp` | E. MOG router (recommended) |
|---|---|---|---|---|---|
| Token only in Rust | Work: custom `Model` over IPC | Work: loopback proxy (ADR 0014) | Work: custom `fetch` over IPC, or no SDK | Yes: built (ADR 0014, 0083) | Yes: planner call runs in Rust |
| All network through Rust | Work: custom `Model`; CSP blocks `baseURL` | Yes: proxy; child has loopback only | Work: custom `fetch` | Yes | Yes |
| Views only listen and `view_emit` | Yes | Yes | Yes | Yes | Yes |
| Model through the workspace (no Anthropic or OpenAI key) | Work: Chat Completions mapping | Work: `/ai-gateway/anthropic` via proxy | Yes: either Databricks route | Work: proxy upstream is a stub | Yes: `/ai-gateway/mlflow/v1/chat/completions` (NFR-21a) |
| Runs in the brain WebView | Unclear: browsers not listed [OA-4] | No: needs a subprocess sidecar | Yes: with `dangerouslyAllowBrowser` | No: needs a harness process | Yes: brain is flow only |
| Telemetry off | Work: must set `tracingDisabled` | Work: settings and env to set | Yes: no export by default | Depends on the harness | Yes: nothing to turn off |
| Never picks a profile | Work: code check | Work: code check | Work: code check | Yes: proxy drops `profile` | Yes: catalog holds only user-bound MOGs |
| agent_guard (no capture, publish or switch without the user) | Work | Work: plus shell residual risk | Work | Yes | Yes: model has no tools at all |
| User approves each cross-workspace send | Work: hard with handoffs | Work: `canUseTool` | Yes: manual loop fits [AN-2] | Work: one approved text only | Yes: plan ticket per part |
| Split and merge with provenance | Work | Work | Work | Work | Yes: deterministic merge (W-13c) |
| Latency (one routed question) | 1+ model round trips | Cold start + 1+ round trips | 1+ round trips | Cold start + 1+ round trips | 1 round trip, then parallel Genie |
| New dependencies | `@openai/agents`, `zod` | SDK + native binary per platform | `@anthropic-ai/sdk` (optional) | None new | None |
| Size of the first PR | Large | Large | Medium | Large (SP-19 first) | Small to medium |
| Later tool loop (synthesis, chains) | Yes | Yes | Yes | Yes | Work: add the manual loop (ADR 0110) |

## 6. Recommendation

1. **Slice 1 uses option E, the MOG router.** One planner call in Rust, route-only, strict JSON, no tools.
2. **No agent SDK in slice 1.** The route-only planner needs one call, not a loop.
3. **Later, a manual tool loop (option C without the SDK).** It runs in the brain and calls a new Rust
   command for each model step. Rust holds the token and checks the tool schema.
4. **Do not adopt the Claude Agent SDK for this use.** It is a coding agent. It needs a sidecar. Bottle
   must turn off all of its tools.
5. **Do not adopt the OpenAI Agents SDK for this use.** The README does not list a browser runtime.
   Tracing needs an explicit off. Handoffs give the send decision to the model.

Reasons:

- Route-only gives a structural privacy rule. The model never sees an answer, so it cannot move data from
  workspace A to workspace B. No prompt rule is necessary for this.
- The user sees and approves the exact text of each part. This keeps B1: nothing goes to Genie without a
  user action.
- It reuses parts on main: GenieBridge, PollActor, R8, R6, T7 and the local store with its
  `split_group_id`. It also reuses the Rust chat-completions route of the recording feature.
- It matches the v1 split requirements (RTR-11b, W-13a..c, G-50, SEC-29a, SEC-29b) with few changes.

What it gives up: the planner cannot ask a follow-up on the basis of an answer. Section 7.10 adds that
later.

## 7. Architecture

### 7.1 Flow

```
 user: hold Right Option, speak, tap the route key, release          (key: see 7.9 and Q2)
        │
 brain: utterance-route ─► mog-router.plan(question, catalog)
        │                     │ IPC mog_route_plan (new, C1)
        │              Rust: R6 token of the ASKING MOG's home profile (W-21a)
        │                    POST /ai-gateway/mlflow/v1/chat/completions (NFR-21a)
        │                    fixed prompt + json_schema, no tools (RTR-20a)
        │                    validate: 1..3 parts, mogId in catalog, size caps
        │                    store plan ticket: SHA-256 per part ─► reply {planId, parts}
        │
 card: "Ask 2 MOGs?"  MOG A · <host-a> · "question 1"   MOG B · <host-b> · "question 2"
        │  [Ask] [Ask only MOG A] [Cancel]          (view_emit click ─► brain)
        │
 brain: IPC mog_route_approve {planId, parts} (C3) ─► Rust marks the ticket approved
        │
 brain: SplitCoordinator: for each approved part, in parallel:
        │   GenieBridge.submit(profile of that MOG, part text, planPart) ─► mcp_call genie_ask
        │   Rust R8: ticket check (hash, approved, once) ─► R6 token of THAT profile ─► Genie One
        │   PollActor polls each part in its own conversation and rate bucket (G-09a)
        │
 card: one row per part: MOG name · workspace label · answer verbatim · key numbers · Open in Databricks
 local store: one genie_exchange per part, shared split_group_id (G-50), flag cross_workspace (SEC-29a)
```

### 7.2 Roles

- **Rust** owns the planner prompt, the JSON schema, the token, the HTTP call, the output check and the
  plan ticket. The prompt is a Rust constant, as `MAIN_AGENT_SYSTEM_PROMPT` is.
- **The brain** owns the catalog, the card state, the SplitCoordinator and the Genie asks. It sends each
  part through the existing GenieBridge and PollActor.
- **The views** show the plan and the parts. They send clicks through `view_emit` only.
- **Peer MOGs** are not separate agents. A peer is an identity (name, profile, Genie One). A peer MOG lamp
  shows its "working" mood while its part runs. So the user sees the MOGs "talk".

### 7.3 Routing: which workspaces get the question

1. The brain builds the catalog from the MOG configs and `profiles_list`.
2. The catalog holds only MOGs with a supported home profile. It holds the MOG ID, the MOG name and the
   workspace label. It holds no profile name, token, memory or answer.
3. The catalog marks each MOG as `ready`, `signed_out` or `unknown`. The planner gets only `ready` MOGs.
4. The planner returns 1 to 3 parts. A part names one catalog MOG ID and one question text.
5. Rust refuses a part with a MOG ID that is not in the catalog of that call. Nothing is sent then.
6. A plan with one part to the asking MOG is the same as the direct path. The card then sends it with no
   plan card. See Q4.
7. A plan with no part ("none of your MOGs fits") shows "No MOG fits this question." with [Ask MOG A].
8. Bottle never adds a MOG by itself. A MOG that the user did not create is never in the catalog.
9. A MOG name in the question ("ask the sales MOG") is data for the planner. It is not an order to code.

### 7.4 Split and merge with provenance

- The planner may split one question into 2 or 3 parts (W-13a). A 4th part shows as a chip.
- Each part goes to its own conversation in its own workspace (G-27a, W-12a).
- The merge is code, not a model, in route-only mode. The card shows each part answer verbatim (W-13c).
- Each part row shows the MOG name, the workspace label, the state and the answer. It also shows up to
  3 key numbers, copied verbatim (UX-14a), and Open in Databricks. The link comes from the `deep_link` of that part's
  poll reply. The card trusts a link only on that part's own host (#523).
- A failed part shows "Part 2 (MOG B): <reason>" (W-13b). The other parts still show.
- The card counts a split as one blurb (UX-11b).
- The local store keeps one `genie_exchange` per part with one shared `split_group_id` (G-50).
- The merged view stays local, with the flag `cross_workspace` (SEC-29a). It never goes into a preamble
  (SEC-29b).
- A follow-up on one part row goes to that part's pinned conversation and profile (G-27b).

### 7.5 Cross-workspace consent

The data flows in route-only mode are these:

| Data | From | To | Consent |
|---|---|---|---|
| The user's question | device | the asking MOG's home workspace (planner model) | The key press. W-21a already sends router calls to the home profile. |
| A part question | device | the part's workspace (Genie One) | A click on [Ask] on the plan card. The click covers the exact text shown. |
| A part answer | the part's workspace | device (card, local store) | None more. It does not leave the device. |
| A part answer | device | any model or other workspace | Not allowed in route-only mode. |

Rules:

- The plan card must show the exact text of each part before the send.
- The user must click once to send. One click may approve all parts on the card.
- [Ask only MOG A] must send only the asking MOG's part, with no cross-workspace send.
- An edit of a part text must make a new plan ticket. The old ticket must stop.
- `mog.genie.shareContextCrossWorkspace` stays false by default. In route-only mode Bottle does not read
  it, because no context moves between workspaces.
- Synthesis mode (7.10) must need `shareContextCrossWorkspace = true` on the asking MOG. It must also need
  a one-time confirmation for each pair of workspaces. The text: "Answers from <host-b> will go to the
  model in <host-a>." MDM `CrossWorkspaceContextAllowed = false` must hide it (SEC-28b).

### 7.6 agent_guard rules for the MOG router

- The planner has no tools. Its request has no `tools` field, so RTR-20a holds.
- The planner cannot start capture, publish a Page or switch a workspace. It has no path to them.
- A part is not a workspace switch. The asking MOG's binding does not change. `profile_select` is not
  called. The hop badge and banner do not change (W-20 stays for real hops).
- The planner output is data, and code checks it. Each MOG ID must be in the catalog. A plan must have
  1 to 3 parts. Each text must pass the G-47 sanitizer and the G-30a cap.
- The #117 injection guard runs on each part. A flagged part shows the warning chip and is not checked
  by default on the plan card.
- Rust writes one content-free audit entry for each plan and each sent part. The entry holds the MOG ID,
  the kind, an opId, the time and sizes only (RTR-30). It holds no text and no hash.
- The R8 ask caps apply to each part: 2 per MOG (G-11a), 4 per profile (G-11b).
- Nothing is sent again by itself (G-15b). [Try again] on a part row sends that part once more.

### 7.7 Latency and cost budgets

| Step | Budget | Why |
|---|---|---|
| Catalog build | 50 ms or less (local) | Config and profile lists are in memory. |
| Token status probe of catalog MOGs | 300 ms or less, in parallel | R6 cache. A miss marks the MOG `unknown`, not an error. |
| Planner call | 2.5 s or less at p95; hard stop 6 s | One call, `max_tokens` 512, small input. |
| Plan card to the user | at once after the planner reply | No other wait. |
| Genie parts | parallel; soft deadline 4 min (F6 step 5) | The merge shows the parts that are done; late parts update in place. |
| Part give-up | 10 min (G-12b) | Existing PollActor rule. |
| Planner input | 4,000 tokens or less | Question (G-30a cap) plus a catalog of 12 MOGs or less. |
| Planner calls | 1 per route; 0 on retry of a part | A part retry does not call the planner. |
| Planner rate | 6 per minute per asking MOG | Stops a key-repeat loop. Uses the R7 bucket of the home profile. |

Cost: one planner call per routed question, on the user's own workspace, billed as model serving there.
The plan card shows no cost. The hub data panel says "Questions to other MOGs are planned in your home
workspace." (extends W-21b).

### 7.8 Failure states

| State | Detected by | Card text | Effect |
|---|---|---|---|
| Peer MOG signed out | R6 `login_required` on the probe or on the part | "Sign in to <host-b> to ask MOG B." [Sign in] | The planner does not get that MOG. On a send, only that part fails. |
| Peer offline or off VPN | R7/R8 connect error or timeout | "Can't reach <host-b>. Check your network or VPN." [Try again] | Only that part fails. |
| Genie One off in a peer | R8 404 with no session (G-05b) | "Genie One isn't available in <host-b>." | Only that part fails. |
| Rate limit | R8 `rate_limited` | "MOG B is busy. Try again soon." | No auto retry. |
| Planner model has no access | `model_access` probe or HTTP 403 | "Can't plan across MOGs. Ask MOG A only?" [Ask MOG A] | Falls back to the direct path after a click. |
| Planner reply not valid | Rust schema check fails twice | Same as above. | Nothing is sent. |
| Home profile signed out | R6 `login_required` | "Sign in to <host-a> to plan across MOGs." | Nothing is sent. |
| No home profile | config | The pick card (ADR 0092). | Nothing is sent. |
| Plan ticket expired (120 s) | Rust ticket check | "This plan expired. Plan again?" | Nothing is sent. |
| User cancels | click | The plan card closes. | Nothing is sent. |
| A part is cancelled while it runs | click on the row | "Cancelled." | `genie_cancel_response` for that part only. |

### 7.9 Fit with the direct ⌥G path (proposal; the user decides, Q2)

Options:

| Option | What it does | For | Against |
|---|---|---|---|
| K1. ⌥G stays direct. A new key in the hold starts the MOG router (for example ⌥M, "MOGs"). | Two keys, two promises. | ADR 0097 and ADR 0098 do not change. The user always knows which path runs. | One more key to learn. Windows has no ask key yet. |
| K2. ⌥G becomes agentic. Every ask goes through the planner. | One key. | Simple to explain. | Adds a model call and about 1 to 3 s to every ask. It breaks "the exact text goes to Genie". |
| K3. ⌥G stays direct. The answer card gets [Ask other MOGs]. | A click after the answer. | No new key. Works on Windows. | Slower for the cross-workspace case. Needs a first answer. |

Proposal: **K1 plus K3.** ⌥G stays exactly as ADR 0098 says. A second key in the hold starts the MOG
router. The answer card of a direct ask also offers [Ask other MOGs], which sends the same text to the
planner. The key letter and the button text need the user's decision.

### 7.10 Later: synthesis mode and the manual tool loop (not slice 1)

- A brain loop calls a new Rust command `agent_model_step` (C9) once per step.
- Rust allows only a fixed tool set: `ask_mog(mogId, question)` and `show_parts()`. Rust refuses any
  other tool field.
- Databricks function calling does not run tools in parallel [DB-2]. So the loop asks one MOG per step.
  The coordinator still runs the approved asks in parallel.
- Each `ask_mog` call from the model goes to the plan card for a click before any send.
- The model may read part answers only with the synthesis consent of 7.5.
- The step cap is 4 model steps per route. The input cap is 16,000 tokens per step.
- If the owner wants an SDK here, the Anthropic TypeScript SDK with a custom `fetch` over IPC is the
  best fit [AN-1]. The plan still prefers no SDK, because the docs point to a manual loop for approvals.
  [AN-2]

## 8. Requirements (new rows, MTM-)

| ID | Requirement | Check |
|---|---|---|
| MTM-01 | The MOG router must start only from a user action: the route key or [Ask other MOGs]. | No other trigger calls `mog_route_plan`. |
| MTM-02 | The brain must build the catalog only from MOGs that the user created. | A test catalog has no MOG that the config does not list. |
| MTM-03 | The catalog must hold the MOG ID, the MOG name and the workspace label only. | The request body has no profile name, memory or answer. |
| MTM-04 | Rust must send the planner call with the token of the asking MOG's home profile. | The trace shows the home host (W-21a). |
| MTM-05 | Rust must send the planner call to `/ai-gateway/mlflow/v1/chat/completions`. | No planner call goes to `/serving-endpoints` (NFR-21b). |
| MTM-06 | The planner request must not contain a tool field. | RTR-20a inspection. |
| MTM-07 | Rust must refuse a plan with 0 parts, more than 3 parts or an unknown MOG ID. | The reply is a fixed code; nothing is sent. |
| MTM-08 | The planner must not receive a Genie answer in route-only mode. | The request body has no answer text. |
| MTM-09 | The card must show the exact text of each part before the send. | The sent text equals the shown text. |
| MTM-10 | Bottle must send a part only after a user click on the plan card. | R8 refuses a part with no approved ticket. |
| MTM-11 | R8 must send each part text once, and only if its SHA-256 matches the approved ticket. | A changed text gets `plan_not_approved`. |
| MTM-12 | Each part must use the token and Genie One of its own MOG's home profile. | A part for MOG B uses `<host-b>` only. |
| MTM-13 | The card must show the MOG name, the workspace label and the link for each part. | Each row has all three. |
| MTM-14 | The card must copy numbers verbatim and do no arithmetic across parts. | W-13c test. |
| MTM-15 | The local store must write one exchange per part with one shared `split_group_id`. | G-50 test. |
| MTM-16 | A failed part must not stop the other parts. | One signed-out part; the others answer. |
| MTM-17 | Bottle must not send a part again by itself. | G-15b test. |
| MTM-18 | Rust must write one content-free audit entry for each plan and each sent part. | RTR-30 test. |
| MTM-19 | The planner prompt and schema must be Rust constants with no user text inside. | Inspection. |
| MTM-20 | ⌥G must keep the direct path of ADR 0097 and ADR 0098. | AT for ⌥G passes unchanged. |

## 9. Slice 1: two MOGs, one routed question

Goal: prove the full path with 2 MOGs, with no split. The planner picks 1 MOG from a catalog of 2.

In the PR:

1. Rust: `mog_router` module with the planner prompt, the JSON schema, the output check and the plan
   ticket store. Use the existing `mlflow.chat_completions` route and R6.
2. Rust: `mog_route_plan` and `mog_route_approve` commands, brain-only in `ipc_gateway` (C1, C3).
3. Rust: an optional `planPart` on `mcp_call` `genie_ask`; R8 checks it against the ticket (C2).
4. Brain: `src/brain/mog-router.ts`: catalog build, plan call, plan card state, send through GenieBridge.
5. Brain: the [Ask other MOGs] button on a direct answer card (K3). No new key in slice 1.
6. Card view: a plan section (1 row) and a part row with provenance. Reuse `card-sections.ts`.
7. Tests with doubles: a fake planner reply, a fake R8, a fake R6 with one signed-out profile.

Not in slice 1: splits of 2 or 3 parts, the new hold key, synthesis mode, the tool loop, Windows key.

Acceptance criteria:

- AC-1: The test has MOG A (`profile-a`) and MOG B (`profile-b`). A click on [Ask other MOGs] on a MOG A
  card shows a plan card. The plan card has 1 part for MOG B, with its exact text.
- AC-2: Before the click, R8 gets no `genie_ask` for `profile-b`.
- AC-3: After the click, R8 gets 1 `genie_ask` for `profile-b` with the shown text, and none other.
- AC-4: A test that changes the text after approval gets `plan_not_approved`, and nothing is sent.
- AC-5: The part row shows "MOG B · <host-b>", the answer and Open in Databricks on `<host-b>`.
- AC-6: With `profile-b` signed out, the planner request has no MOG B, and the card says "No MOG fits".
- AC-7: The planner request body has no `tools` field and no answer text.
- AC-8: The audit log has 1 plan entry and 1 part entry, with no text.
- AC-9: The ⌥G tests on main pass with no change.
- AC-10: One local exchange row is written for the part, with a `split_group_id`.

Size: about 900 to 1,300 changed lines with tests. Owner: contract lead sign-off for C1 to C3 first.

## 10. Later slices

| Slice | Content |
|---|---|
| 2 | Split of 2 or 3 parts, parallel asks, merge rows, 4th part as a chip, soft deadline. |
| 3 | The route key in the hold (K1) on macOS, after the user picks the key (Q2). |
| 4 | Synthesis mode behind the consent of 7.5, with the manual tool loop (ADR 0110, C9). |
| 5 | Optional MOG descriptions for better routing (C8). Planner eval record (NFR-22a). |

## 11. Contract changes (each needs the owner's sign-off)

| ID | Flag | File(s) | Change |
|---|---|---|---|
| C1 | CONTRACT | `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs`, `src-tauri/src/ipc.rs`, `src-tauri/schemas/` | New command `mog_route_plan {mogId, question, catalog[{mogId, name, hostLabel}], opId, tags}` gives `{planId, parts[{index, mogId, question}], expiresAtMs}` or a fixed code. Brain only. |
| C2 | CONTRACT | `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs` | `mcp_call` gets an optional `planPart {planId, index}` for `genie_ask`. R8 checks the ticket when it is there. |
| C3 | CONTRACT | same as C1 | New command `mog_route_approve {planId, indexes[]}` after the user click. Brain only. |
| C4 | CONTRACT | `crates/mog-core/src/types.rs`, `src/shared/ipc.ts` | `Tags` gets an optional `group` (the `split_group_id`) so the trace joins the parts. |
| C5 | CONTRACT-ADJACENT | `crates/mog-core/src/audit.rs` | Two new audit kinds: `agent_action:mog_route_plan` and `agent_action:ask_mog`. The kind set is closed. |
| C6 | CONTRACT | `src/shared/interfaces.ts` | `DispatchOutcome` and `ProfileService.resolveTarget` get the split result that requirements section 4.7 already shows (`kind: 'split'`, 3 parts or less). |
| C7 | CONTRACT | `src/shared/ipc.ts` (card events) | New card section kinds: `route_plan` and `route_part` with provenance fields. |
| C8 | CONTRACT + SCHEMA | `crates/mog-core/src/mog_config.rs`, schema, fixtures | Later: optional `mog.binding.description` (120 chars or less) for the catalog. `deny_unknown_fields` makes this a schema change. |
| C9 | CONTRACT | `src/shared/ipc.ts`, `crates/mog-core/src/ipc.rs` | Later: `agent_model_step` for the tool loop, with a Rust-fixed tool set. It does not change the `model_invoke` tool guard. |
| C10 | CONTRACT | `hotkey.ptt` payload in `src/shared/ipc.ts`, `hotkey_core.rs` | Only for K1: the up event carries `route: true` for the new key. |
| C11 | SCOPE | `AGENTS.md`, `research/final-requirements.md` | Move RTR-11b, W-13a..c, G-50, SEC-29a, SEC-29b, UX-11b from v1 into this feature. Add the MTM rows. |

No migration is necessary: `split_group_id` is already in `0001_local_mirror.sql:126`.

## 12. New ADRs

- **ADR 0110: Agent runtime for cross-MOG work.** A Rust planner call now, a manual brain loop later and
  no agent SDK. It records the comparison of section 5. It amends ADR 0014 for this use: the R16 harness
  stays for open-ended main-agent work, if the owner keeps it.
- **ADR 0111: MOG-to-MOG asks: catalog, route-only planner, plan tickets and provenance.** It records
  sections 7.3 to 7.8. It extends ADR 0085 `OutboundApprovals` from one text per MOG to one ticket per
  plan part.

## 13. Risks

| Risk | Effect | Mitigation |
|---|---|---|
| The planner picks the wrong MOG. | The user gets an answer from the wrong workspace. | The plan card shows the MOG and text before the send. Add MOG descriptions later (C8). Run an eval (NFR-22a). |
| The planner rewrites the question badly. | A wrong number from Genie. | The card shows the exact part text. [Edit] lets the user fix it. |
| A user clicks Ask with no reading. | Data question goes to a workspace the user did not want. | The workspace label is large on each row. [Ask only MOG A] is next to [Ask]. |
| Tool use on `/ai-gateway/anthropic` is not documented. | The later loop may fail on that route. | Use Chat Completions function calling, which is documented [DB-2]. |
| Databricks has no parallel function calls. | A tool loop is slower. | Route-only plan returns all parts in one JSON reply. |
| The planner endpoint name differs per workspace. | `databricks-claude-sonnet-5-5` may not exist in a home workspace. | Use the `model_access` probe (ADM-04a) and a hub setting, as the recording feature does. |
| More model cost per routed question. | A small charge in the home workspace. | One call, 512 output tokens, rate cap. Show the data panel line. |
| The scope conflict with `AGENTS.md`. | Work starts on an out-of-scope item. | Q1 goes to the user first. |
| Prompt injection in the user's own utterance. | A bad plan. | Code checks, sanitizer, injection guard, and the user's click. The model has no tools. |

## 14. Open questions for the user

- Q1. Do you lift "workspace hop and split questions" from the out-of-scope list for this feature?
- Q2. Which entry do you want? The options are K1 (a new key in the hold), K2 (⌥G becomes agentic), K3
  (a card button), or K1 plus K3 (the proposal). If K1, which letter?
- Q3. Do you accept route-only for the first release? The model then never reads an answer.
- Q4. Must a one-part plan to the asking MOG skip the plan card, as the direct path does?
- Q5. Must one click approve all parts, or must each part have its own checkbox?
- Q6. Which endpoint do you want for the planner: `databricks-claude-sonnet-5-5`, a smaller model, or
  the router model of RTR-10?
- Q7. Must a peer MOG be reachable from other MOGs by default, or only after a per-MOG opt-in?
- Q8. Do you want synthesis mode at all? If yes, is the per-pair confirmation of 7.5 enough?

## 15. References

Official documents (fetched 2026-11-21):

- [OA-1] OpenAI Agents SDK, Handoffs: https://openai.github.io/openai-agents-js/guides/handoffs/
- [OA-2] OpenAI Agents SDK, Models: https://openai.github.io/openai-agents-js/guides/models/
- [OA-3] OpenAI Agents SDK, `Model` interface: https://openai.github.io/openai-agents-js/openai/agents/interfaces/model/
- [OA-4] OpenAI Agents SDK README: https://github.com/openai/openai-agents-js
- [OA-5] OpenAI Agents SDK, Tracing: https://openai.github.io/openai-agents-js/guides/tracing/
- [CA-1] Claude Agent SDK overview: https://code.claude.com/docs/en/agent-sdk/overview
- [CA-2] Claude Agent SDK quickstart: https://code.claude.com/docs/en/agent-sdk/quickstart
- [CA-3] Claude Agent SDK TypeScript reference: https://code.claude.com/docs/en/agent-sdk/typescript
- [CA-4] Claude Code gateway compatibility guide: https://code.claude.com/docs/en/llm-gateway-protocol
- [CA-5] Claude Agent SDK permissions: https://code.claude.com/docs/en/agent-sdk/permissions
- [AN-1] Anthropic TypeScript SDK: https://platform.claude.com/docs/en/api/sdks/typescript
- [AN-2] Tool runner: https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-runner
- [DB-1] Databricks, Query model services: https://docs.databricks.com/aws/en/ai-gateway/query-model-services
- [DB-2] Databricks, Function calling: https://docs.databricks.com/aws/en/machine-learning/model-serving/function-calling
- [DB-3] Databricks, Anthropic Messages API: https://docs.databricks.com/aws/en/machine-learning/model-serving/query-anthropic-messages
- [TA-1] Tauri v2, Embedding external binaries: https://v2.tauri.app/develop/sidecar/

Repository:

- `AGENTS.md` sections 1, 2 and 5; ADR 0014, 0083, 0085, 0092, 0097, 0098.
- `research/final-requirements.md`: RTR-11a, RTR-11b, RTR-20a, RTR-21a..RTR-30, W-11a..W-13c, W-20,
  W-21a, W-21b, G-26a..G-27b, G-50, SEC-28b, SEC-29a, SEC-29b, UX-11b, UX-14a, NFR-21a, NFR-21b, SC-6, F5, F6.
- `research/connector-spec-2026-11-14.md`: Genie One MCP tools and `deep_link`.
- `research/diagrams/workspace-hopping.svg`: the hop and the v1 split.
