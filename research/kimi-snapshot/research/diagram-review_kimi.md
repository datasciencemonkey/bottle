# Motherbricks diagram review

Date: 2026-11-14. From: diagram-specialist. To: requirements director.

## Naming and scope

- Use **Motherbricks** for both the project and the desktop application. Use this exact capitalization.
- A **MOG** is a floating agent that Motherbricks creates and manages.
- Keep the existing `mother-databricks` directory names. They are storage paths, not product names.
- Reviewed diagrams:
  - `mother-databricks/diagrams/01-system_kimi.svg`
  - `mother-databricks/diagrams/02-voice-sequence_kimi.svg`
  - `mother-databricks/diagrams/03-state_kimi.svg`
  - `mother-databricks/diagrams/04-trust_kimi.svg`

All four files parse as valid standalone SVG. Each has one accessible `<title>`, one `<desc>`, and unique element IDs.

## Components the final requirements must name explicitly

The diagrams expose components that were implicit in the product brief. Give each component an owner, interface, failure behavior and acceptance criteria.

1. **Audio Broker:** owns global hotkey routing, microphone and system-audio leases, VAD, separate audio tracks and consumer buffers.
2. **Consent and Policy Gate:** controls meeting start, hosted ASR, context sharing, memory writes, workspace changes and data retention.
3. **STT Provider Interface:** selects local Parakeet or a verified hosted ASR endpoint. Returns text, timing, track ID, language and quality information.
4. **Cleanup Provider Interface:** applies a deterministic pre-pass and a bounded SLM call. Returns raw text with a degraded flag on timeout or failure.
5. **Context Gate and Route Classifier:** distinguishes dictation, a Genie question and context-only input. Exposes the selected route and workspace when policy requires review.
6. **Backend Adapter and Capability Registry:** provides ask, poll, cancel, result, deep-link and health operations. Detects support for Genie One MCP, Genie APIs and custom agents.
7. **Credential Broker:** obtains tokens through approved Databricks CLI profiles, keeps secrets in native process memory, refreshes them and binds each credential to a workspace host.
8. **Response Projector:** creates the short blurb without changing numbers or meaning. Includes the Databricks deep link and an explicit failure state.
9. **Memory Service:** provides local-first recall, provenance, retention, deletion, Lakebase synchronization and conflict handling.
10. **Encrypted Local Store:** holds approved content, memory and recovery IDs. Separates optional meeting audio from text and memory.
11. **Crash Journal and Recovery Manager:** finalizes complete meeting segments, preserves partial status and resumes or reports interrupted Genie work.
12. **Restricted WebView IPC:** allows only declared commands and events. Prevents tokens, Lakebase credentials and raw system audio from entering the WebView.
13. **Workspace and Conversation Pinning:** resolves CLI profiles, prevents ambiguous capture and keeps active work on its original backend and workspace.
14. **Rate-Limit and Cancellation Controller:** prevents overlapping polls, applies backoff, honors cancellation and exposes delayed or failed work.

## Decision register

The director should convert each row into a requirement or an explicit non-goal. The recommendations are intended to prevent unnecessary user escalation.

| ID | Priority | Miss or ambiguity | Recommended requirement decision |
|---|---|---|---|
| D01 | P0 | “Always on” can mean continuous hidden recording. | Define always available, not always recording. Capture only during push-to-talk or an explicitly started meeting. If continuous recording is mandatory, escalate because it changes the product, consent model and threat model. |
| D02 | P0 | The send/review behavior after key-up is undefined. | Define Dictation mode and Genie mode. Dictation returns text only. Genie mode may send after key-up when explicitly enabled. Provide an optional review/edit step. Require approval for meeting-context sharing. |
| D03 | P0 | Genie One MCP has no separate context-upload tool. | Store context-only input locally by default. Include approved relevant context in the next Genie question. Do not create hidden standalone Genie turns for meeting context. |
| D04 | P0 | The diagrams cannot infer which events become agentic memory. | Define typed memory-write rules. Exclude raw audio and query rows by default. Store only approved facts, preferences, summaries and conversation references with source, time, workspace and retention. |
| D05 | P0 | Memory tenancy and workspace switching can leak content. | Namespace memory by user, MOG and home workspace. Do not recall or replicate across workspaces by default. Require a separate export/share action with visible source and destination. |
| D06 | P0 | Multiple MOGs can request the same microphone. | Use one Audio Broker and one microphone owner. Reject a second MOG without interrupting the owner. Do not queue capture silently. Resolve duplicate hotkeys with a chooser before capture. |
| D07 | P0 | Push-to-talk during the same MOG’s meeting is unspecified. | Recommended default: let the meeting owner reuse its existing stream through separate PTT and meeting buffers. Reject other MOGs. Mark the PTT interval in the meeting timeline. Do not reopen or duplicate the audio stream. |
| D08 | P0 | A workspace switch during a meeting or active answer can split data across workspaces. | Bind a meeting and each Genie conversation to the workspace selected at start. A switch affects new work only. Polling, cancellation and results for old work stay on the original workspace. |
| D09 | P0 | Flutter versus Tauri/native remains unresolved. | Keep the shell replaceable around a native core. Recommended desktop MVP: Tauri v2 + Rust core. Treat Flutter as a later shell through FFI if mobile UI becomes primary. List global hotkey, floating-panel, paste and system-audio limits for web/mobile as non-goals. |
| D10 | P0 | The exact MVP operating systems are missing. | Name the reference macOS and Windows versions and required permissions. Mark Linux system audio and Wayland hotkey behavior as capability-detected until verified. Acceptance tests must be platform-specific. |
| D11 | P0 | The default Genie endpoint can be confused with preview or deprecated APIs. | Make GA Genie One MCP the default. Exclude `/api/2.0/mcp/genie`. Keep per-space MCP and Genie SSE APIs behind capability detection. Verify preview/GA status at implementation time. |
| D12 | P0 | CLI authentication and token scopes are not one complete flow. | Support OAuth U2M CLI profiles first. Define profile discovery, ambiguous names, login recovery, token refresh, process argument safety and workspace-host validation. Verify required scopes for Genie MCP and model services. Do not silently rewrite PAT or service-principal profiles. |
| D13 | P0 | “Genie agent” and “custom agent” do not share one API or authentication contract. | Define a capability-based backend interface. Put Genie Agent mode, Databricks Apps and Model Serving in separate adapters. Do not promise identical streaming, cancellation, history or deep-link behavior. |
| D14 | P0 | Lakebase endpoint selection and credentials are unspecified. | Configure a Lakebase endpoint per workspace. Use TLS and short-lived credentials. Rotate pooled connections before expiry. Store only a user/MOG/workspace-scoped schema. Validate least-privilege grants. |
| D15 | P0 | Raw audio retention and deletion are not defined. | Do not retain push-to-talk audio after transcription by default. Make meeting-audio retention explicit and configurable. Define crash-journal cleanup, disk caps, secure deletion and deletion failure reporting. |
| D16 | P0 | Offline behavior can cause unintended delayed sends. | Continue local STT and dictation. Mark hosted cleanup, Genie and Lakebase unavailable. Return raw text when cleanup fails. Do not automatically send queued questions after reconnect. Synchronize only approved memory events. |
| D17 | P0 | Genie polling, cancellation and restart recovery need one owner. | The Backend Adapter owns conversation and response IDs. Never overlap polls for one response. Use bounded backoff and visible elapsed time. Persist recovery IDs. Cancel with `genie_cancel_response`. Never fabricate completion after timeout. |
| D18 | P1 | The blurb has no deterministic construction rule. | Project `final_answer` into a bounded blurb while preserving key numbers, units and caveats. Include the supplied Databricks deep link. Fetch query rows only when needed and permitted. Do not add another unbounded model call by default. |
| D19 | P1 | STT output and meeting chunk contracts are incomplete. | Require segment ID, track ID, start/end time, text, language and quality status. Define overlap, revision and ordering rules for live meeting chunks. Treat “You” and “Others” as track labels, not verified speaker identities. |
| D20 | P1 | Hosted ASR availability and governance are assumptions. | Feature-detect a supported endpoint. Verify audio format, maximum duration, retention and inference logging before release. Local Parakeet remains the offline/default path. Record model version and hardware prerequisites. |
| D21 | P1 | Cleanup can change a data question’s meaning. | Preserve numbers, units, names, code, SQL, negation and self-correction intent. Use a strict deadline and deterministic pre-pass. Show degraded status on fallback. Add evaluation cases for false starts and numeric questions. |
| D22 | P1 | Meeting stop signals need precise semantics and fallbacks. | Use manual stop, call-app mic release when detectable, calendar end with prompt, 15 minutes of dual-track silence, sleep and low disk. Use a 90-minute soft cap only while speech continues and a 4-hour hard ceiling. Always finalize and remove the indicator. |
| D23 | P1 | Local/Lakebase synchronization can duplicate or resurrect memory. | Define local-first commit, idempotency keys, retry, conflict policy, tombstones and retention enforcement. Lakebase is durable storage, not the interactive recall path. Specify behavior when the workspace is unreachable. |
| D24 | P1 | MOG deletion can overpromise remote deletion. | Define deletion of configuration, cache, journals and Lakebase memory. Propagate tombstones. Do not claim Genie conversation deletion unless a supported API exists. Disclose remaining server-side retention. |
| D25 | P1 | WebView and external-link privileges need acceptance criteria. | Use an IPC allowlist, CSP, validated message schemas and no secret access. Open only validated HTTPS Databricks deep links in the system browser. Keep arbitrary remote content out of the privileged UI. |
| D26 | P1 | Local and server-side observability may capture sensitive content. | Redact tokens, audio, transcripts, context and query rows from local diagnostics by default. Verify model-service inference tables and retention. Give administrators and users a clear data-flow inventory. |
| D27 | P1 | Multi-MOG resource use and fairness are unbounded. | Define maximum active MOGs, concurrent recordings, concurrent Genie responses, local STT workers, memory cache size and disk budget. Prevent one MOG from starving others. Keep one global capture owner. |
| D28 | P1 | Performance budgets are not attached to states. | Set reference-device p50/p95 budgets for indicator display, STT, cleanup, local recall and blurb projection. Treat Genie as asynchronous and show progress. Do not advertise a fixed total answer latency. |
| D29 | P2 | Animation states can become the only status signal. | Map idle, listening, recording, processing, answering, success, error, paused and sleeping to text and accessible events. Support reduced motion. Never use color or animation alone for recording or error status. |
| D30 | P2 | Alternate backends can bypass the same privacy controls. | Apply the same context gate, host binding, retention, redaction and blurb rules to every adapter. Make Apps versus Model Serving authentication differences explicit in setup and tests. |

## Suggested requirement invariants

Use these statements as review checks for the final HTML:

1. No audio capture occurs without a current push-to-talk lease or an explicit meeting session.
2. A visible text indicator exists for every listening or recording state.
3. A MOG never sends raw audio to Genie One MCP.
4. A MOG never sends meeting context without the configured approval policy.
5. A bearer token or Lakebase credential never enters the WebView, MOG configuration, audio journal or logs.
6. Every cloud request uses the workspace host bound to the selected profile and credential.
7. A workspace switch never moves an active conversation or meeting.
8. Every memory item has a user, MOG, workspace, source, creation time and retention policy.
9. Local recall failure does not block dictation. Genie failure does not produce a false answer.
10. Every asynchronous response has a terminal, cancelled, failed or recoverable state.
11. A second MOG cannot take the microphone without a visible user action.
12. Motherbricks and MOG naming is consistent in all user-facing requirements and diagrams.

## Escalation guidance

No ordinary clarification needs the user now. Escalate only if the intended product requires continuous ambient recording, automatic cross-workspace memory sharing, or mobile/web feature parity for global hotkeys and system audio. Those requirements would change the core product and threat model.
