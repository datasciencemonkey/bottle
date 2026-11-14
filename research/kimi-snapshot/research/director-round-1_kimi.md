# Mother Databricks: Round 1 requirements direction

Date: 2026-11-14  
Owner: requirements director  
Status: direction for specialist teams; not the final requirements document  
Final deliverable after team review: `mother-databricks/requirements_kimi.html`

## 1. Purpose

This direction converts the project brief into a controlled product baseline.
It resolves collisions that would otherwise produce incompatible designs.
It assigns verification and requirements work to six teams.
It does not authorize implementation.

The final document must use ASD-STE100-style simplified English.
It must not claim formal ASD-STE100 compliance without a controlled-language check.
It must contain inline SVG diagrams with accessible titles and descriptions.
It must be one self-contained HTML file.

## 2. Product baseline

- **Mother Databricks** is the project name.
- **Mother Bricks** is the desktop application.
- A **MOG** is a configured floating agent instance.
- Mother Bricks can run more than one MOG.
- A MOG is always available for an explicit user action.
- A MOG is not always recording.
- Push-to-talk is the default capture mode.
- Meeting capture requires an explicit start action.
- A MOG can dictate text without contacting Databricks.
- A MOG can send an explicit question to a selected Databricks backend.
- A MOG can add selected context to that question when the user enables context sharing.
- A MOG can store local memories.
- A MOG can synchronize memories to Lakebase when Lakebase is configured.
- A MOG can use a Genie One MCP, a supported Genie agent, or a custom agent through a backend adapter.
- A question response appears as a short blurb.
- The blurb preserves important numbers and units.
- The blurb includes a Databricks link when the backend supplies one.
- The desktop app is the first release.
- Mobile and web are possible companion targets with reduced capabilities.

No existential user question blocks Round 1.

### Conditional user-confirmation items

These items do not block the current requirements work.
The director must raise them only if the associated condition is true.

| ID | Condition that needs user confirmation | Current director decision |
|---|---|---|
| UC-01 | Mobile must be in the first public release. | Desktop is first. Mobile and web are later companions. |
| UC-02 | “Always on” means continuous ambient recording without a user action. | Reject that interpretation. Availability does not imply capture. |
| UC-03 | Lakebase must be mandatory before a user can use dictation or Genie. | Local operation must work without Lakebase. Lakebase is the durable synchronization option. |
| UC-04 | The product must silently route a conversation to a different workspace or agent. | Prohibit silent rerouting. A conversation stays pinned to its origin. |

## 3. Collision decisions

Each team must use these decisions as constraints.
A team can request a decision change only with new primary evidence or a failed technical spike.

### D-01: Flutter versus Tauri

**Decision:** Use Tauri v2 with a React and TypeScript UI for the desktop MVP.
Use a native Rust service for global input, audio, secure storage, token handling, and network transport.
Keep provider interfaces independent of the UI.
Do not make Flutter an MVP dependency.

**Reason:** Global key-release handling, non-activating panels, and system audio are load-bearing desktop features.
The earlier research found weaker Flutter support for these features.
Flutter cannot make global hotkeys, system audio capture, or paste-into-other-apps portable to iOS and web.

**Verification gate:** The voice/desktop team must run a focused Tauri spike on macOS and Windows.
The spike must test key-down, key-up, cancellation, focus behavior, microphone permission denial, and overlay behavior.
The platform team must verify the relevant plugin and operating-system claims.
If the spike fails, the director will compare a native shell and a Flutter shell with the same Rust core.

### D-02: Always available versus always recording

**Decision:** “Always on” means that the app can receive a configured global command while it is running.
It does not mean continuous microphone capture.

The app must have three distinct capture modes:

1. Push-to-talk while the user holds a key.
2. An explicit hands-free dictation session with a visible stop control.
3. An explicit meeting recording session with a persistent recording indicator.

No mode can start from a transcript, memory, Genie response, calendar event, or remote instruction alone.
Pause must stop capture.
Sleep must end capture safely.
The recording indicator must remain visible even when the mascot is hidden or minimized.

### D-03: Genie MCP has no independent context-upload operation

**Decision:** Context sharing creates a bounded context envelope in the next question string.
The app must not claim that it uploads a separate memory store to Genie.

The context envelope must identify:

- The cleaned question.
- Each included context item.
- The source of each item.
- The timestamp of each item.
- Any truncation applied by the app.

Context sharing is off by default.
The user must be able to preview the exact outbound payload.
The app must not send ambient meeting content automatically.
A remembered item is not approved for sharing merely because it is in local memory.
Conversation history can supply context only within the same pinned backend conversation.
The app must label this as conversation context, not permanent Genie memory.

### D-04: Retired beta Genie MCP versus Genie One

**Decision:** Genie One MCP is the default MCP connector.
Do not use `https://<host>/api/2.0/mcp/genie` in any release plan.
That beta endpoint has a documented end of life on 2026-11-30 in the supplied research.

The initial tool contract is:

- `genie_ask`
- `genie_poll_response`
- `genie_get_query_result`
- `genie_cancel_response`

The app must not overlap poll calls for the same response.
The app must preserve conversation and response identifiers.
The app must expose progress without claiming that polling is streaming.
The GA Genie space REST API can be an explicit fallback adapter.
Fallback must not silently change the selected data target.

### D-05: Sixty-minute timer versus signal-based meeting policy

**Decision:** Do not stop a healthy meeting only because 60 minutes elapsed.
Use elapsed time for warnings and review.
Use explicit and environmental signals for stopping.

| Signal | Required behavior |
|---|---|
| User selects Stop | Stop capture immediately. Finalize stored data. |
| 60 minutes elapsed | Show elapsed time. Continue recording. |
| 15 minutes without detected speech | Show a 60-second visible countdown. Stop if silence continues. Reset the countdown when speech resumes. |
| 90 minutes elapsed | Show a review notice. Continue while meeting signals remain active. |
| Calendar end | Prompt the user. Do not stop an active meeting only because the calendar event ended. |
| Call application releases the microphone | Start a 30-second grace period. Stop if the microphone remains released. |
| System sleep | Stop capture. Finalize crash-safe segments. Mark the session interrupted by sleep. |
| Low disk threshold | Warn before the threshold. Stop before data loss occurs. Preserve completed segments. |
| Four hours elapsed | Stop at the hard ceiling. Require a new explicit start for a new session. |

The voice/desktop team can propose evidence-based changes to debounce values.
It must not remove the hard ceiling or the manual stop control.

### D-06: Lakebase durability versus fast local lookup

**Decision:** Use an encrypted local memory store for interactive recall.
Use Lakebase for durable memory, cross-session recovery, and optional synchronization.
Network access must not be on the critical path for local recall.

The local store contains the active working set and an outbound synchronization queue.
Lakebase is the durable store when it is configured.
Every memory has a stable identifier, MOG identifier, workspace scope, type, source, creation time, retention class, and provenance.
Deletion must propagate as a tombstone.
The app must prevent memories from one MOG or workspace from appearing in another scope without explicit user action.
The memory team must evaluate Lakebase vector search and managed Agent Memory.
It must not assume that either service satisfies the complete data model.

### D-07: Databricks CLI identity versus a custom OAuth application

**Decision:** Reuse Databricks CLI profiles in the MVP.
Do not require a custom OAuth application for the MVP.
Do not use PAT or service-principal profiles in the first release unless the security team approves an explicit later requirement.

The app must:

- Discover profiles through supported CLI commands.
- Support OAuth U2M CLI profiles first.
- Start CLI login through a user action when authentication is missing or expired.
- Request tokens through the CLI without a shell command string.
- Keep access tokens in the native process memory.
- Keep tokens out of the webview, logs, crash reports, memory records, and MOG configuration.
- Send a token only to the HTTPS workspace host associated with the selected profile.
- Refresh a token before expiry without changing the selected profile.

The platform team must verify CLI output, token scopes, and Genie One MCP compatibility.
If the CLI token cannot access the required MCP endpoint, the director will reopen the custom OAuth application decision.
The app must not silently broaden scopes or rewrite a CLI profile.

### D-08: Genie One versus per-space Genie and custom agents

**Decision:** Define one backend adapter contract with explicit capabilities.
Provide separate connector configurations for each backend type.

| Backend | Release role | Constraint |
|---|---|---|
| Genie One MCP | Default | GA path in the supplied research. Verify current scope and grants. |
| Genie space REST | Supported fallback | Uses a selected space and polling. |
| Per-space Genie MCP | Optional preview | Do not make it an MVP dependency. Verify preview limitations. |
| Genie One API or Genie Agent API | Later beta adapter | Feature-detect SSE and recovery. Do not describe it as GA. |
| Custom Databricks Apps agent | Separate adapter | Apps invocation and OAuth rules differ from serving. |
| Custom Model Serving agent | Separate adapter | Define the exact invocation schema and streaming support. |

A conversation is pinned to its workspace, profile, backend type, backend identifier, and conversation identifier.
A one-off workspace selection must not change the MOG home profile.
A backend switch starts a new conversation unless the user explicitly exports local context into a new question.
The UI must show the active workspace and backend before submission.

### D-09: Question, dictation, context, and memory commands

**Decision:** Do not send every cleaned utterance to Genie.

The router must support these explicit outcomes:

- Insert dictated text.
- Copy dictated text.
- Save a local memory.
- Ask the selected backend.
- Save context locally without sending it.
- Ask for user disambiguation.

An explicit command such as “Ask Genie” selects the backend flow.
An utterance in dictation mode remains dictation even if it ends with a question mark.
An ambiguous utterance must not trigger a network request.
The cleanup model must not choose the backend or workspace.

### D-10: Transcript integrity and cleanup

**Decision:** Preserve the raw transcript and the cleaned transcript as separate values.
The cleanup operation must not add facts, numbers, names, or commitments.
It must not silently change a negation or a question into a command.

The user must be able to view and restore the raw transcript.
A deterministic pre-pass can remove configured filler terms.
A dictionary can protect names and technical terms.
Low-confidence speech must remain marked.
The raw transcript is the fallback when cleanup fails or exceeds its deadline.
The app must never overwrite text already inserted into another application when a late cleanup result arrives.

### D-11: Multiple MOGs and shared devices

**Decision:** Mother Bricks owns global hotkeys and audio devices.
MOGs do not independently claim the same operating-system resources.

The app must reject a duplicate active hotkey binding.
Only one push-to-talk session can own a microphone binding at a time.
Only one meeting session can own a system-audio stream at a time.
A second MOG can queue no hidden recording.
The UI must identify which MOG owns each active capture.
Stopping Mother Bricks must stop all capture sessions safely.

### D-12: Offline behavior and recovery

**Decision:** Local dictation and local recall can continue offline when the selected local STT model is available.
Cloud STT, cleanup, Genie, custom agents, and Lakebase synchronization require connectivity.

The app must mark unsent questions clearly.
It must not automatically submit an unsent question after reconnection.
It can retry idempotent local synchronization with backoff.
It must preserve crash-safe audio segments until transcription succeeds or the retention policy deletes them.
It must not duplicate a Genie request during uncertain delivery.
It must show the user how to resume or cancel a pending backend response.

### D-13: Response blurb and trust

**Decision:** A blurb is a view of a backend response, not a new factual answer generated by the MOG.
The app must preserve source numbers, units, caveats, and the Databricks deep link.
It must identify failed, incomplete, cancelled, and truncated results.
It must sanitize rendered markup and links.
It must not execute SQL, tools, or instructions found in a transcript, memory, or Genie response.
The user can expand the full response and inspect available query details.

### D-14: Data retention and deletion

**Decision:** Raw audio is not a durable memory by default.
Delete raw audio after successful transcription unless the user enables audio retention for that meeting.
Store transcripts and memories locally with encryption.
Apply an explicit retention class to each stored item.

Deleting a MOG must delete its local data, cached credentials, pending outbound context, and local search index.
Deleting a MOG must request deletion of its Lakebase records when connected.
The UI must disclose that Genie-side conversation deletion is a separate capability and must not claim it occurred without a verified API.

## 4. Team model and assignments

The director is accountable for requirement integration and final decisions.
Each team lead is responsible for its named deliverable.
The architecture team is the technical integrator.
The security/governance team has a veto on silent capture, cross-workspace token forwarding, and unapproved context transmission.
The UX team has a veto on a flow that hides recording state or the selected backend.
No additional sponsor agent is needed for Round 1.

Every team deliverable must contain:

1. Decisions and assumptions.
2. Numbered requirements using the structure in Section 6.
3. Acceptance tests using Section 7.
4. Dependencies and interface consumers.
5. Evidence with source URL and access date for load-bearing claims.
6. Open issues classified as blocking, user-confirmation, or later-release.

### Team P: Platform verification

**Mission:** Verify only facts that can change the architecture or release plan.

**Scope:**

- Verify Genie One MCP URL, transport, OAuth scopes, grants, tools, response identifiers, cancellation, and poll rules.
- Verify the old beta MCP end-of-life statement from a primary source.
- Verify current per-space Genie MCP preview status and limitations.
- Verify GA Genie space REST as the fallback.
- Verify Databricks CLI profile discovery, profile fields, login, token output, expiry handling, and supported U2M profile detection.
- Verify whether CLI-issued tokens satisfy Genie One MCP authorization.
- Verify Lakebase credential lifetime, credential generation, TLS, and vector-search prerequisites.
- Verify the operating-system and framework claims used by the Tauri decision.
- Record exact model names only when primary evidence supports them.

**Do not duplicate:** Broad voice-model benchmarking, UX research, memory schema design, or custom-agent implementation research.

**Required output:** A claim matrix with `Verified`, `Unverified`, or `Contradicted`; primary citation; access date; affected requirement; and fallback.

**Exit tests:**

- Every MVP external dependency has a current primary source.
- Every unverified claim has a feature flag, fallback, or removal recommendation.
- No requirement depends on the retired beta MCP endpoint.
- No exact model or API field is presented as verified from memory alone.

### Team A: Architecture

**Mission:** Produce the integrated system, interface, lifecycle, and failure-model requirements.

**Scope:**

- Define boundaries among the UI, native core, providers, backend adapters, local memory, Lakebase, CLI, and Databricks workspaces.
- Define `SttProvider`, `CleanupProvider`, `BackendAdapter`, `AuthProvider`, `MemoryStore`, and `CaptureSession` contracts.
- Define capability discovery for polling, streaming, cancellation, query results, deep links, and context limits.
- Define the MOG state machine and the capture state machine separately.
- Define conversation affinity and explicit backend switching.
- Define request identifiers, idempotency, timeouts, cancellation, retry, backoff, and uncertain-delivery behavior.
- Define multi-MOG resource arbitration with the voice/desktop team.
- Define offline and crash recovery.
- Consolidate measurable nonfunctional budgets.
- Produce source drawings for four final inline SVGs: system context and trust boundaries; voice-to-answer sequence; MOG and capture state machines; memory and workspace data flow.

**Required output:** Numbered architecture requirements, interface tables, state-transition table, failure matrix, diagram SVG source, and traceability to D-01 through D-14.

**Exit tests:**

- Every external call crosses an identified trust boundary.
- Every state has an entry action, exit action, and recoverable failure behavior.
- Every backend operation has timeout, cancellation, and uncertain-delivery behavior.
- Every diagram element maps to a requirement.
- No arrow shows a token or context item crossing to an unselected workspace.

### Team V: Voice and desktop

**Mission:** Prove the desktop interaction and define capture, transcription, cleanup, insertion, and meeting requirements.

**Scope:**

- Run the focused Tauri macOS and Windows spike required by D-01.
- Test global key-down, key-up, focus loss, stuck-key recovery, Escape cancellation, and duplicate bindings.
- Test a non-activating overlay and a separate keyboard-capable response panel.
- Define push-to-talk, hands-free dictation, and meeting state behavior.
- Define microphone and system-audio tracks, permission handling, silence detection, and signal-based stopping.
- Define crash-safe audio segmentation and transcription chunks.
- Benchmark local Parakeet with the selected runtime on named reference hardware.
- Treat Databricks-hosted ASR as configurable until a supported endpoint is verified.
- Define cleanup deadlines, raw-transcript fallback, dictionaries, and fidelity checks.
- Define text insertion and copy fallback without late overwrite.
- Define resource arbitration and recovery when devices disconnect.

**Required output:** Spike evidence, benchmark method and results, numbered requirements, state tests, and meeting-policy tests.

**Exit tests:**

- Key release ends push-to-talk on each MVP operating system.
- Focus loss cannot leave capture active without a visible state.
- Permission denial does not record or fabricate a transcript.
- A late cleanup result cannot overwrite inserted text.
- Sleep, microphone release, silence, low disk, and the four-hour ceiling preserve completed data.
- Two MOGs cannot secretly capture the same audio stream.

### Team M: Memory

**Mission:** Define local-first agentic memory and optional Lakebase durability.

**Scope:**

- Define memory types for user preference, meeting summary, approved context, question, response reference, and MOG configuration.
- Define stable IDs, provenance, workspace and MOG scope, retention classes, timestamps, and sensitivity labels.
- Define encrypted local storage, local indexing, cache eviction, and an outbound sync queue.
- Define Lakebase schema, TLS connection handling, credential rotation, and reconnect behavior.
- Compare self-managed `lakebase_vector` with managed Agent Memory for this data model.
- Define exact and semantic recall, ranking, filters, and latency measurement.
- Define conflict handling, tombstones, deletion, export, and rebuild from durable storage.
- Define memory write approval and memory recall approval for outbound Genie context.
- Treat recalled text as untrusted data, not executable instructions.

**Required output:** Data dictionary, lifecycle and retention matrix, sync state machine, numbered requirements, and performance tests.

**Exit tests:**

- Local recall meets its agreed latency budget without network access.
- Lakebase unavailability does not block dictation or local recall.
- Credential expiry does not cause token logging or memory loss.
- Deletion removes local data and propagates a tombstone after reconnection.
- Cross-MOG and cross-workspace recall tests return no unauthorized records.
- Sync retries do not create duplicate memories.

### Team S: Security and governance

**Mission:** Define consent, identity, data protection, threat controls, audit, and administrator prerequisites.

**Scope:**

- Produce a threat model for the webview, native core, CLI, local store, Lakebase, MCP, custom agents, and deep links.
- Define capture consent, persistent indicators, participant-notice UX, and enterprise policy controls.
- Define CLI process invocation, profile validation, host canonicalization, token lifetime, and secret redaction.
- Prove that tokens cannot be forwarded to a different workspace host.
- Define context-sharing approval, payload preview, redaction, and audit metadata without content logging by default.
- Define local encryption and OS keychain requirements with the memory team.
- Define prompt-injection boundaries for transcripts, memory, tool output, and rendered responses.
- Define link allowlisting, markup sanitization, and safe external-link opening.
- Define retention, deletion, export, and legal-hold extension points.
- Define signing, update, logging, and organization-managed configuration requirements.

**Required output:** Threat register, data-classification matrix, trust-boundary review, numbered requirements, abuse cases, and release-blocking controls.

**Exit tests:**

- No silent capture path exists.
- No token enters the webview or persistent content storage.
- A malicious profile hostname cannot receive another profile's token.
- Context sharing sends only the approved payload to the displayed target.
- Prompt-injection fixtures cannot trigger a tool call or change capture state.
- Logs and crash reports pass secret and content-leak inspection.
- A user can stop capture and delete local data without network access.

### Team U: UX and accessibility

**Mission:** Define the complete user journey and visible behavior for every state.

**Scope:**

- Define onboarding, CLI sign-in, MOG creation, connector selection, hotkey setup, model setup, and permission setup.
- Define idle, listening, recording, transcribing, cleaning, waiting, answering, success, error, paused, and sleeping presentations.
- Keep the privacy recording indicator independent of mascot animation.
- Define the question blurb, progress, full response, deep link, cancellation, and retry.
- Define raw-versus-clean transcript review and correction.
- Define context preview, inclusion, exclusion, truncation notice, and outbound target confirmation.
- Define explicit one-off workspace selection and home-profile changes.
- Define meeting start, elapsed time, stop, silence countdown, calendar prompt, interruption, and recovery.
- Define multi-MOG identification, capture ownership, and conflict messages.
- Define keyboard-only operation, screen-reader labels, focus order, reduced motion, contrast, and non-audio alerts.
- Define mobile/web companion boundaries without promising desktop-only capabilities.

**Required output:** Flow inventory, wireframe-level descriptions, state-to-message matrix, accessibility requirements, and usability acceptance tests.

**Exit tests:**

- A first-time user can create and configure a MOG without editing a configuration file.
- The active capture state and backend target are visible before submission.
- A user can stop any capture with one visible action and one keyboard action.
- A user can inspect the exact context payload before sending it.
- Workspace switching cannot be confused with changing an existing conversation.
- All critical flows are usable without a pointer and without animation.

## 5. Cross-team integration contracts

The teams must resolve these interfaces jointly:

| Interface | Accountable team | Required reviewers |
|---|---|---|
| Hotkey and capture ownership | Voice/desktop | Architecture, UX, Security |
| STT and cleanup provider contract | Voice/desktop | Architecture |
| Backend adapter and capability contract | Architecture | Platform, Security, UX |
| CLI identity and workspace selection | Platform | Architecture, Security, UX |
| Local and Lakebase memory | Memory | Architecture, Security |
| Context-envelope construction | Architecture | Memory, Security, UX |
| Blurb and full-response rendering | UX | Architecture, Security |
| Retention and deletion | Security | Memory, UX |
| Nonfunctional measurement | Architecture | All teams |

A team must not solve an interface disagreement by adding a hidden fallback.
The team lead must send a timestamped message to the director on the message board.
The director will record the decision and affected requirement IDs.

## 6. Required structure for the final requirements

The final HTML must follow this order:

1. Document control, status, audience, and STE100-style statement.
2. Executive rundown and release recommendation.
3. Product boundaries, non-goals, actors, and controlled glossary.
4. User-confirmation items and director decisions.
5. System context and trust-boundary SVG.
6. MOG lifecycle and capture state-machine SVG.
7. Voice-to-answer sequence SVG.
8. Memory and workspace data-flow SVG.
9. Functional requirements by subsystem.
10. Meeting, multi-MOG, offline, and recovery policies.
11. Security, privacy, governance, and administrator prerequisites.
12. Nonfunctional budgets and measurement environments.
13. Phased roadmap and dependency gates.
14. Acceptance-test matrix and requirement traceability.
15. Collision register, decision log, open issues, and primary sources.

Use these requirement families:

| Prefix | Family |
|---|---|
| `MB` | Mother Bricks application and MOG management |
| `CAP` | Hotkeys, audio capture, and meetings |
| `STT` | Transcription and cleanup |
| `RTE` | Intent routing and context envelopes |
| `GEN` | Genie and agent backend adapters |
| `MEM` | Local memory and Lakebase |
| `WS` | CLI profiles and workspace affinity |
| `UX` | Presentation and accessibility |
| `SEC` | Security, privacy, retention, and governance |
| `REL` | Reliability, offline operation, and recovery |
| `NFR` | Performance and capacity |
| `OPS` | Installation, update, support, and administration |

Every mandatory requirement must have:

- A unique identifier.
- One testable statement.
- A priority: `MUST`, `SHOULD`, or `COULD` for the named release.
- A release phase.
- A rationale or decision reference.
- At least one acceptance-test identifier.
- A verification method: automated test, platform test, inspection, demonstration, or primary-source verification.

Use “shall” for mandatory behavior.
Use “should” for recommended behavior.
Use “may” for permitted optional behavior.
Use one action per sentence.
Use terms from the glossary consistently.
Avoid “fast,” “smart,” “seamless,” “as needed,” “and/or,” and “etc.”
Replace each vague term with a measured condition.
Do not combine two independent obligations in one requirement.
Do not hide an obligation in a diagram, note, or example.

## 7. Acceptance-test expectations

### Test format

Each acceptance test must contain:

1. Test identifier and linked requirement identifiers.
2. Target release and supported platform.
3. Preconditions and test data.
4. Ordered actions.
5. Observable expected results.
6. Performance measurement and percentile when applicable.
7. Evidence to retain.
8. Automation method or manual platform procedure.

Use Given, When, and Then only when each statement remains short and testable.
Include positive, negative, boundary, interruption, and recovery cases.
Do not accept a screenshot alone as proof of privacy, security, timing, or data deletion.

### Minimum end-to-end scenarios

- Create two MOGs with different hotkeys and different workspaces.
- Sign in with an expired CLI profile and recover without losing the MOG configuration.
- Dictate locally while offline and insert the raw transcript when cloud cleanup fails.
- Dictate a question without sending it to a backend.
- Ask Genie with context disabled and verify that no context item leaves the app.
- Preview and send approved context to the displayed workspace.
- Poll a Genie response without overlapping calls.
- Cancel a pending response and recover from an uncertain cancellation.
- Receive a failed, incomplete, truncated, and successful response.
- Preserve numbers, units, caveats, and the deep link in the blurb.
- Attempt cross-workspace conversation reuse and require a new conversation.
- Start a meeting, deny one permission, disconnect a device, resume, and stop.
- Exercise silence, calendar end, microphone release, sleep, low disk, 90-minute review, and four-hour stop.
- Start a second MOG during capture and verify visible arbitration.
- Recall memory with Lakebase unavailable.
- Rotate an expired Lakebase credential without losing queued changes.
- Delete a MOG offline and propagate deletion after reconnection.
- Inject malicious instructions into a transcript, memory, and backend response without triggering a tool or capture action.
- Operate every critical flow with keyboard only, reduced motion, and a screen reader.

### Preliminary measurable budgets

Teams must validate or amend these budgets with a named measurement environment.
They must not remove a budget without supplying a replacement.

| Measure | Preliminary acceptance target |
|---|---|
| Visible response to hotkey | 95th percentile at or below 100 ms while the app is running. |
| Warm local STT after key-up | 95th percentile at or below 1.5 seconds for an utterance of 15 seconds or less on reference hardware. |
| Cleanup deadline | 2.0 seconds. Use the raw transcript after the deadline. Do not overwrite inserted text later. |
| Local structured recall | 95th percentile at or below 50 ms for a 10,000-item local test set on reference hardware. |
| Local semantic recall | Propose a separate measured budget. Do not include remote Lakebase latency in the local target. |
| Backend submission feedback | Show accepted, pending, or failed local state within 300 ms after the transport result is known. |
| Genie answer | Show progress immediately. Do not promise a fixed answer time. Support long-running polling and user cancellation. |
| Reconnection | Never auto-submit an unsent question. Retry safe memory synchronization without duplicates. |
| Capture stop | Stop new audio acquisition within 250 ms after a manual stop on reference hardware. Finalization can continue with visible progress. |

## 8. Round 1 completion gates

The director will not produce the final HTML until these gates pass:

- **G1 Evidence:** Platform verification covers every load-bearing external claim or supplies a fallback.
- **G2 Feasibility:** The desktop spike covers hotkeys, focus, permissions, and overlay behavior.
- **G3 Architecture:** All four diagrams and all state transitions map to requirements.
- **G4 Privacy:** Security approves capture, context, identity, retention, and deletion controls.
- **G5 Testability:** Every `MUST` requirement has at least one acceptance test.
- **G6 Integration:** Cross-team interfaces have one owner and no hidden fallback.
- **G7 Language:** The final text uses short, consistent, testable statements and carries no unsupported formal-compliance claim.
- **G8 Delivery:** The final artifact is one self-contained `requirements_kimi.html` with inline CSS and accessible inline SVG.

## 9. Director instructions for Round 2

Round 2 must integrate the team outputs rather than concatenate them.
It must remove duplicate requirements.
It must resolve contradictory numbers and names.
It must trace each collision decision to requirements and tests.
It must place unsupported features in a later phase or mark them unverified.
It must ask the user only if UC-01 through UC-04 becomes an actual release requirement.
Otherwise, it must proceed with the decisions in this document.
