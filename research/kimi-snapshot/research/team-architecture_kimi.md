# Motherbricks: Team A architecture requirements

Date: 2026-11-14  
Owner: Team A, Architecture  
Status: proposed integration baseline for director review  
Naming: **Motherbricks** is the project and desktop application. A **MOG** is a floating agent created by Motherbricks.

## 1. Scope and integration baseline

This report defines the contracts and failure behavior for `SttProvider`, `CleanupProvider`, `BackendAdapter`, `AuthProvider`, `MemoryStore`, and `CaptureSession`.
It integrates D-01 through D-14.
It does not replace the voice, memory, security, UX, or platform deliverables.

### 1.1 Binding decisions

- Motherbricks uses Tauri v2, React, TypeScript, and a native Rust core for the desktop MVP.
- The UI has no direct audio, token, CLI, or Databricks transport access.
- Motherbricks owns shared devices and global hotkeys.
- A MOG is always available for an explicit command while enabled.
- A MOG does not record continuously merely because it is available.
- Local memory serves interactive recall.
- Lakebase provides configured durable synchronization.
- Genie One MCP is the proposed default backend, subject to Platform verification.
- A conversation remains pinned to its origin profile, workspace, backend, and backend identifier.
- No connector can silently fall back to another connector or workspace.
- Context sharing uses the next approved question payload.
- Context sharing does not use a separate Genie memory operation.
- Local dictation and local recall remain available without Lakebase.

### 1.2 Verification boundary

No physical macOS or Windows host is available in this environment.
Team A executed no global-hotkey, overlay, audio-device, audio-capture, STT-latency, capture-stop, sleep, or low-disk spike.
These items remain release-verification gates under G2.
This report does not claim a platform pass or a measured hardware result.

Reference environments `REF-MAC` and `REF-WIN` must be named before performance sign-off.
Each environment record must include the operating-system version, hardware, memory, power mode, application build, model build, and test-tool version.

### 1.3 Release labels

| Label | Meaning |
|---|---|
| Desktop MVP | Required for the first desktop release. |
| Configured MVP | Required when the named optional provider or connector is enabled. |
| Later | Excluded from the desktop MVP acceptance gate. |

## 2. Component and trust boundaries

### 2.1 Component responsibilities

| Component | Owns | Does not own |
|---|---|---|
| React and TypeScript UI | Presentation, explicit user intent, previews, accessible controls | Tokens, device handles, direct CLI invocation, direct Databricks requests |
| Native command API | Input validation, authorization of UI commands, request identifiers, event sequencing | Product policy hidden from the user |
| MOG controller | MOG configuration, foreground job state, backend affinity | Direct audio ownership |
| Capture broker | Hotkey registry, device leases, capture sessions, recording indicator state | Backend routing decisions |
| STT orchestrator | Segment jobs, provider selection, immutable raw transcripts | Backend selection |
| Cleanup orchestrator | Deterministic pre-pass, cleanup deadline, fidelity checks | Intent selection |
| Context gate and router | Explicit route, local recall candidates, context envelope, payload approval hash | Automatic context approval |
| Backend registry | Connector selection, capability discovery, adapter construction | Silent connector fallback |
| Credential broker | CLI processes, native token cache, host binding, Lakebase credentials | Token persistence in content storage |
| Memory service | Encrypted local commits, recall, tombstones, sync outbox | Automatic approval to share recalled content |
| Response projector | Normalized status, source-preserving blurb, deep-link metadata | New factual answers, SQL execution |
| Durable local journal | Capture, request, response, and sync recovery records | Plaintext secrets |

### 2.2 Trust-boundary inventory

| Boundary | Crossing | Required control | Requirement |
|---|---|---|---|
| TB-01 | UI to native command API | Validate schema, MOG identifier, user-action identifier, and current state. | MB-A-001 |
| TB-02 | Native core to operating system | Use explicit permissions, central leases, visible capture state, and fenced callbacks. | CAP-A-001, CAP-A-003 |
| TB-03 | Native core to Databricks CLI | Invoke an executable and argument array without a shell. Validate profile and host. | WS-A-001 |
| TB-04 | Native core to model serving | Bind the request to the selected provider and workspace host. Send audio only for selected hosted STT. | STT-A-003, WS-A-002 |
| TB-05 | Native core to Genie or custom agent | Bind token, payload hash, and conversation affinity to the displayed target. | GEN-A-004, RTE-A-003 |
| TB-06 | Native core to Lakebase | Use TLS, rotated credentials, scoped records, and idempotent sync. | MEM-A-003 |
| TB-07 | Native core to local persistent storage | Encrypt content, exclude tokens, and journal state before dependent side effects. | REL-A-003 |
| TB-08 | Backend response to UI | Treat text and links as untrusted data. Normalize and sanitize before rendering. | GEN-A-008 |

Every external call in the architecture maps to TB-03, TB-04, TB-05, or TB-06.
No token or context item may cross TB-05 to an unselected workspace.

## 3. Common identity, correlation, and error model

### 3.1 Required identifiers

| Identifier | Lifetime and rule |
|---|---|
| `mog_id` | Stable random identifier for one local MOG. It is not a workspace name. |
| `user_action_id` | Unique identifier for one explicit start, stop, approve, submit, or cancel action. Repeated UI delivery reuses this value. |
| `capture_id` | Stable identifier for one capture session. |
| `segment_id` | Stable identifier for one durable audio segment. |
| `utterance_id` | Stable identifier for one raw transcript and all cleanup revisions. |
| `job_id` | Stable identifier for one local dictation, memory, or backend job. |
| `request_id` | Stable UUID for one logical operation across all attempts. |
| `attempt_id` | New UUID for one transport or provider attempt. |
| `idempotency_key` | Stable value sent only when the selected remote contract supports it. It defaults to `request_id` where the contract permits. |
| `resource_epoch` | Monotonic fencing value for one device lease. Callbacks from older epochs are discarded. |
| `conversation_key` | Hash of profile identifier, canonical workspace host, backend type, backend identifier, and remote conversation identifier. |
| `payload_hash` | SHA-256 hash of the exact outbound question string and canonical target. Approval binds to this hash. |
| `memory_id` and `revision` | Stable memory identity and monotonic local revision. |
| `sync_change_id` | Stable identifier for one memory mutation and tombstone. |

### 3.2 Conversation affinity

`BackendTarget` contains:

| Field | Rule |
|---|---|
| `profile_id` | Selected CLI profile. A one-off selection does not change the MOG home profile. |
| `workspace_host` | Canonical HTTPS origin associated with the profile. No user-supplied redirect target is accepted. |
| `backend_type` | Explicit connector type. |
| `backend_id` | Stable connector instance, space, agent, application, or endpoint identifier. |
| `conversation_id` | Remote conversation identifier when one exists. |
| `profile_fingerprint` | Non-secret binding to the authenticated identity when available. |

A new `BackendTarget` is required when any origin field changes.
The adapter for the original target remains attached to pending poll, result, and cancel operations.
A target switch never rewrites an existing `conversation_key`.

### 3.3 Delivery phases

| Phase | Meaning | Automatic retry rule |
|---|---|---|
| `NOT_SENT` | The transport proved that no request bytes were sent. | Retry is permitted within the operation policy. |
| `SENDING` | Transmission began or the phase cannot be proved. | Do not retry a non-idempotent submission. |
| `ACKNOWLEDGED` | A valid remote response supplied stable remote identifiers. | Resume the documented poll or event-recovery operation. |
| `UNCERTAIN` | Timeout, disconnect, crash, or malformed response prevents a delivery decision. | Require reconciliation or explicit user action for a non-idempotent submission. |
| `TERMINAL` | A verified success, failure, cancellation, or local abandonment was recorded. | Do not resubmit. |

### 3.4 Common operation error

Every provider or adapter error returns `code`, `request_id`, `attempt_id`, `delivery_phase`, `retryability`, `safe_message`, and optional `retry_after_ms`.
The error may include a remote status code.
The error must exclude tokens, raw audio, transcript content, memory content, and full request payloads.

Required codes are `UNAVAILABLE`, `OFFLINE`, `AUTH_EXPIRED`, `AUTH_INVALID`, `HOST_MISMATCH`, `PERMISSION_DENIED`, `CAPABILITY_UNKNOWN`, `CAPABILITY_UNSUPPORTED`, `INVALID_INPUT`, `RATE_LIMITED`, `TIMEOUT`, `CANCELLED`, `CANCELLATION_UNCERTAIN`, `DELIVERY_UNCERTAIN`, `RESOURCE_BUSY`, `DEVICE_LOST`, `INTERRUPTED`, `REMOTE_FAILED`, `REMOTE_INCOMPLETE`, `MALFORMED_RESPONSE`, and `STORAGE_FAILED`.

## 4. Provider and service contracts

The tables are language-neutral contracts for native Rust traits and typed UI commands.
All methods accept a cancellation token and an operation context containing the common identifiers.
All callbacks carry the identifier of their originating operation.

### 4.1 `SttProvider`

| Operation or type | Contract | Linked requirement |
|---|---|---|
| `discover_capabilities()` | Returns provider identifier, model version, local or hosted execution, available languages, accepted audio formats, segment limits, timestamp support, confidence support, streaming support, and offline readiness. | STT-A-003 |
| `transcribe(TranscribeRequest)` | Accepts `capture_id`, `segment_id`, immutable audio reference, audio hash, language hint, deadline, and request identity. Returns one immutable `RawTranscript`. | STT-A-001 |
| `RawTranscript` | Contains `utterance_id`, text, ordered segments, optional word timings, optional confidence values, provider and model identity, audio hash, and creation time. Empty audio produces empty text with a status, not fabricated speech. | STT-A-001 |
| `transcribe` caching | A local duplicate key uses capture, segment, audio hash, provider, and model version. A completed local result is reusable without a second model call. | REL-A-002 |
| Hosted submission | The selected workspace host and approved audio-disclosure policy are part of the request. The contract has no alternate-provider fallback parameter. | STT-A-003 |
| Cancellation | Cancellation stops local compute or the hosted request when possible. A late result is stored only as a superseded result and cannot advance the active utterance. | REL-A-006 |

### 4.2 `CleanupProvider`

| Operation or type | Contract | Linked requirement |
|---|---|---|
| `discover_capabilities()` | Returns provider and model identity, hosted or local execution, maximum input, timeout configuration, and structured-output support when known. | STT-A-003 |
| `clean(CleanupRequest)` | Accepts `utterance_id`, `raw_text_hash`, raw text, protected terms, deterministic pre-pass configuration, deadline, and request identity. It has no backend target or routing authority. | STT-A-002, RTE-A-001 |
| `CleanupResult` | Contains cleaned text, raw text hash, warnings, fidelity-check results, provider identity, and completion time. | STT-A-002 |
| Fidelity validation | The orchestrator compares protected terms, numbers, units, negations, and question marks. A failed check selects the raw transcript and records a safe warning code. | STT-A-002 |
| Deadline | The overall deadline is 2.0 seconds. Expiration selects the raw transcript. A late result cannot replace inserted or copied text. | STT-A-002, REL-A-006 |
| Failure | Provider unavailability, invalid output, timeout, and cancellation select the raw transcript without changing the intended route. | STT-A-002 |

### 4.3 `BackendAdapter`

| Operation or type | Contract | Linked requirement |
|---|---|---|
| `discover_capabilities()` | Returns observed and configured capabilities with source and observation time. Discovery uses static manifest data or read-only protocol operations. It never submits a test question. | GEN-A-001 |
| `submit_question(SubmitRequest)` | Accepts the exact approved payload, `BackendTarget`, optional pinned conversation, payload hash, request identity, deadline, and optional idempotency key. Returns remote conversation and response identifiers or a typed error. | GEN-A-004, REL-A-001 |
| `poll_response(ResponseRef)` | Returns normalized progress or a terminal response. A per-response mutex prevents overlapping calls. | GEN-A-003 |
| `get_query_result(QueryRef)` | Returns query rows or a typed unsupported, expired, or malformed-result status. It does not infer an item identifier. | GEN-A-001, GEN-A-008 |
| `cancel_response(ResponseRef)` | Requests cancellation of the original remote response. The result distinguishes request acknowledgement, verified remote cancellation, local stop-waiting, and uncertain cancellation. | GEN-A-009 |
| `recover_response(ResponseRef)` | Uses a documented read or resumed event stream when supported. Unsupported recovery returns `CAPABILITY_UNSUPPORTED`; it never creates a new question. | GEN-A-001, REL-A-004 |
| `normalize_response()` | Produces status, answer text, progress, caveats, truncation, query references, deep link, and raw safe metadata. It does not execute embedded instructions. | GEN-A-008 |
| `BackendCapabilities` | Uses `SUPPORTED`, `UNSUPPORTED`, or `UNKNOWN` for submit idempotency, polling, streaming, cancellation, query results, deep links, follow-up, recovery, and maximum input. | GEN-A-001 |

`UNKNOWN` disables the dependent optional action.
It does not mean `SUPPORTED`.
The UI may submit a plain question when only an optional input limit is unknown, but a configured local payload budget still applies.
A response rejection is shown without fallback.

### 4.4 `AuthProvider`

| Operation or type | Contract | Linked requirement |
|---|---|---|
| `list_profiles()` | Invokes supported CLI discovery and returns profile names, authentication types, canonical hosts, and validity metadata without tokens. | WS-A-001 |
| `start_login(profile_or_host)` | Starts an explicit CLI login action. It does not rewrite another profile or broaden scopes. | WS-A-001 |
| `acquire_token(TokenRequest)` | Accepts profile, canonical host, and intended connector. It returns a native-only token and expiry. Concurrent requests for one profile share one CLI invocation. | WS-A-002 |
| `validate_binding(TokenRequest)` | Rejects a missing, non-HTTPS, or mismatched host before token acquisition. | WS-A-002 |
| `refresh(TokenRef)` | Refreshes before expiry using the same profile and host. A failure retains the selected configuration and requires explicit login when necessary. | WS-A-003 |
| `forget_local_tokens(profile_id)` | Clears native memory cache. CLI-owned refresh-token deletion is a separate explicit operation. | WS-A-003 |

The token value never enters a UI event, persistent MOG configuration, memory record, log, crash report, or renderer-readable environment variable.

### 4.5 `MemoryStore`

| Operation or type | Contract | Linked requirement |
|---|---|---|
| `put_local(MemoryRecord)` | Commits encrypted content and an outbox mutation in one local transaction. `(memory_id, revision)` is the idempotency key. | MEM-A-001, MEM-A-003 |
| `recall_local(RecallQuery)` | Filters by `mog_id` and workspace scope before ranking. Returns records, provenance, freshness, and local index status. | MEM-A-002 |
| `get_local(memory_id)` | Enforces the requested MOG and workspace scope. | MEM-A-002 |
| `delete_local(memory_id)` | Writes a local tombstone and an outbox deletion in one transaction. Network access is not required. | MEM-A-004 |
| `sync_pending(SyncBatch)` | Uploads only configured scopes. Stable `sync_change_id` values make repeated mutations idempotent. | MEM-A-003 |
| `apply_remote(RemoteChange)` | Applies revision and tombstone rules. A deletion wins over an older content revision. | MEM-A-003, MEM-A-004 |
| `rebuild_local(DurableSnapshot)` | Rebuilds an authorized local scope without changing approval labels. | REL-A-004 |
| `MemoryRecord` | Contains stable identity, MOG and workspace scope, type, source, created and updated times, retention class, provenance, sensitivity, revision, and optional tombstone. | MEM-A-002 |
| `RecallResult` | Marks candidates as local data. It contains no `approved_for_context` value inherited from storage. | MEM-A-005 |

Memory synchronization approval and Genie context approval are separate decisions.
Lakebase unavailability never changes a successful local commit into a failed local commit.

### 4.6 `CaptureSession`

Only the capture broker constructs a `CaptureSession`.

| Operation or type | Contract | Linked requirement |
|---|---|---|
| `start(StartCapture)` | Requires explicit `user_action_id`, MOG, mode, requested resources, consent state, retention policy, and capture policy. Duplicate action identifiers return the original session. | CAP-A-002 |
| `stop(StopCapture)` | Requires an explicit or documented environmental stop reason. It stops new acquisition before finalization. | CAP-A-002 |
| `snapshot()` | Returns capture state, owner, resource epochs, elapsed time, active segment, durable segment count, and stop-reason metadata without audio content. | MB-A-002 |
| `events()` | Emits monotonic sequence numbers for state changes and durable segments. Consumers deduplicate by capture and sequence. | REL-A-003 |
| `finalize()` | Flushes the active journal, records checksums, closes completed segments, and releases the remaining session resources. | REL-A-003 |
| `recover(RecoverCapture)` | Reopens durable metadata after a crash, marks interrupted state, fences old callbacks, and preserves segments for transcription or retention deletion. | REL-A-004 |

No transcript, memory record, calendar event, Genie response, or remote instruction can call `start` without a new explicit user action.

## 5. Backend capability and adapter requirements

### 5.1 Capability discovery

| Capability | Behavior when supported | Behavior when unknown or unsupported |
|---|---|---|
| Polling | Use the documented response identifier and one in-flight poll per response. | Use the explicitly configured streaming adapter or reject the pending-wait operation. Do not emulate polling by resubmitting. |
| Streaming | Consume typed events and preserve remote item identifiers. The UI may label the state streaming. | Use polling only when the user selected a polling connector. Do not label polling as streaming. |
| Cancellation | Call the original target and reconcile its status. | Offer local stop-waiting and retain the remote state as unknown. Do not claim remote cancellation. |
| Query results | Fetch a supplied query or item identifier. | Show the available answer and query metadata without inventing rows. |
| Deep links | Validate and display the supplied Databricks link. | Show no link. Do not construct an unverified result URL. |
| Follow-up | Submit to the pinned remote conversation. | Start a new conversation only after the user approves the new conversation and payload. |
| Idempotent submission | Reuse the stable remote idempotency key for a retry. | Do not automatically retry after `SENDING` begins. |
| Input limit | Enforce the discovered limit and the local context budget. | Enforce the configured local budget. Show a remote rejection without rerouting. |
| Response recovery | Resume the documented stream or read operation using original identifiers. | Show `UNCERTAIN` or stale pending state and offer cancel or user-directed reconciliation. |

### 5.2 Explicit adapter matrix

| Adapter | Release role | Required implementation contract | Prohibited hidden behavior |
|---|---|---|---|
| Genie One MCP | Proposed Desktop MVP default | Map `genie_ask`, `genie_poll_response`, `genie_get_query_result`, and `genie_cancel_response`. Preserve conversation, response, and query item identifiers. Serialize polls. | Do not call the retired beta MCP path. Do not switch to REST after uncertain delivery. |
| Genie space REST | Configured MVP fallback | Map start-conversation, follow-up, message polling, and attachment query results to the common response model. Pin the selected space. Report `remote_cancel=false` until a chat-mode cancellation operation is verified. | Do not select another space. Do not convert polling into claimed streaming. Do not reuse agent-mode cancellation for chat mode. |
| Per-space Genie MCP | Later preview | Discover preview availability and map its verified tools before enablement. | Do not assume Genie One tool names or conversation semantics. |
| Genie One API or Genie Agent API | Later beta | Implement typed SSE, output-item replacement, disconnect recovery, and connector-specific cancellation. | Do not describe beta SSE as GA. Do not re-POST to recover a dropped stream. |
| Custom Databricks Apps agent | Configured MVP when explicitly configured | Use the verified application invocation schema, application URL, OAuth rule, application idempotency key when supported, and event recovery contract. | Do not send Apps requests to a serving endpoint. Do not assume PAT support. |
| Custom Model Serving agent | Configured MVP when explicitly configured | Configure the exact endpoint, invocation schema, response parser, streaming support, and cancellation support. | Do not assume the Apps schema or infer streaming from content type alone. |

An adapter returns `CAPABILITY_UNSUPPORTED` for an operation outside its verified contract.
The registry selects one adapter at submission and records its type and version in the job journal.
An operator or user can select a configured fallback for a new question.
The application cannot automatically select that fallback for the current question.

### 5.3 Context-envelope contract

The exact outbound string contains:

1. The cleaned question, or the raw question when cleanup failed.
2. A bounded context section when the user enabled and approved context.
3. A source and timestamp for every included item.
4. A truncation or omission notice for every affected item.
5. No instruction that treats context or memory as executable commands.

The question itself is never silently truncated.
Any edit to the target, question, included item, ordering, or truncation changes `payload_hash` and invalidates approval.
Context-only utterances use the local-save route.
They do not produce a backend request.

## 6. Independent state machines

The MOG state describes configuration and foreground work.
The capture state describes audio ownership.
The UI derives `listening` and `recording` presentations from capture state.
The UI derives `transcribing`, `cleaning`, `waiting`, and `answering` presentations from MOG job state.
The recording privacy indicator derives only from capture state and remains independent of mascot visibility.

### 6.1 MOG state definitions

| State | Entry action | Exit action | Recoverable failure behavior |
|---|---|---|---|
| `DRAFT` | Create a local configuration revision without device or network access. | Validate the revision before transition. | Invalid fields remain in `DRAFT` with field errors. |
| `READY` | Register enabled hotkeys and publish current target summary. | Unregister foreground-job prompts while retaining configuration. | Registration failure enters `DEGRADED`; no capture starts. |
| `PROCESSING_LOCAL` | Create a foreground `job_id` and publish its current STT, cleanup, or route stage. | Cancel stale stage callbacks and persist the job stage. | Provider failure uses raw fallback or enters `DEGRADED`; capture is independent. |
| `AWAITING_USER` | Persist the question, disambiguation, context preview, or uncertain-delivery choices. | Invalidate approval when payload or target changes. | Restart restores the same pending choice without network submission. |
| `SUBMITTING` | Persist approved payload hash, target, adapter version, and delivery phase before transport. | Persist remote identifiers or the uncertain outcome. | Failure follows the delivery-phase rules; it never selects another adapter. |
| `WAITING_BACKEND` | Store original response identifiers and start the adapter-specific wait policy. | Stop the wait timer and serialize any final cancel or result operation. | Offline or timeout enters `DEGRADED` while retaining affinity. |
| `PRESENTING` | Persist normalized response status and render the source-preserving blurb. | Release foreground presentation resources while retaining history. | Malformed content enters `DEGRADED`; no embedded instruction is executed. |
| `DEGRADED` | Publish the failed capability and available local actions. | Clear only the resolved degradation condition. | Authentication, provider, and network failures retain local data and job identity. |
| `SUSPENDED` | Disable hotkeys and stop every owned capture before completion of entry. | Revalidate configuration and permissions before `READY`. | A finalizing capture remains visible until durable finalization finishes. |
| `DELETING` | Disable hotkeys, stop capture, invalidate tokens, and write a deletion journal. | Remove local indexes and queue remote tombstones. | Offline deletion completes locally and retains only the minimum deletion outbox. |
| `DELETED` | Remove active UI state and prevent new commands. | Terminal. | A stale callback is discarded by MOG identity and resource epoch. |

### 6.2 MOG transitions

| From | Event | To | Required guard or action |
|---|---|---|---|
| `DRAFT` | Valid configuration saved | `READY` | Required provider and target fields pass validation. |
| `READY` | Final utterance accepted | `PROCESSING_LOCAL` | A new foreground job is created; meeting capture may continue independently. |
| `PROCESSING_LOCAL` | Route is ambiguous or approval is required | `AWAITING_USER` | No backend call occurred. |
| `PROCESSING_LOCAL` | Dictation, copy, or local save completes | `PRESENTING` | Show local result and no backend status. |
| `PROCESSING_LOCAL` | Explicit ask approved | `SUBMITTING` | Payload and target hash match the preview. |
| `AWAITING_USER` | User approves ask | `SUBMITTING` | Revalidate target, capabilities, and payload hash. |
| `AWAITING_USER` | User cancels | `READY` | Mark the local job cancelled without remote submission. |
| `SUBMITTING` | Remote identifiers returned | `WAITING_BACKEND` | Persist identifiers before publishing the wait state. |
| `SUBMITTING` | Delivery uncertain | `AWAITING_USER` | Offer reconcile, abandon locally, or explicitly create a new request. |
| `WAITING_BACKEND` | Terminal or displayable progress response arrives | `PRESENTING` | Preserve status, caveats, truncation, and source link. |
| `WAITING_BACKEND` | Temporary wait failure | `DEGRADED` | Retain original response identifiers and target. |
| `DEGRADED` | Known response can resume | `WAITING_BACKEND` | Use the original adapter and identifiers. |
| Any nonterminal state | Unrecoverable local command failure | `DEGRADED` | Stop new work and preserve recoverable data. |
| Any nonterminal state | User suspends MOG | `SUSPENDED` | First stop all owned capture. |
| Any state | User deletes MOG | `DELETING` | Deletion has priority over new jobs. |
| `DELETING` | Local deletion committed | `DELETED` | Pending remote deletion remains only in the scoped outbox. |

One foreground dictation or question job is active per MOG.
A second foreground command receives a visible busy response or an explicit visible replacement choice.
Meeting transcription jobs use a separate bounded background queue.

### 6.3 Capture state definitions

| State | Entry action | Exit action | Recoverable failure behavior |
|---|---|---|---|
| `CLOSED` | No capture resources or indicator exist. | Validate explicit start intent. | Invalid start remains closed. |
| `STARTING` | Persist start intent, show the recording indicator, and acquire resource leases. | Fence setup callbacks with the current resource epoch. | Denial or conflict releases leases, closes the session, and shows the reason. |
| `ACTIVE` | Open the selected streams and journal audio frames and segment metadata. | Stop VAD timers and prevent new frames. | Device loss, sleep, or low disk enters `STOPPING` with an interruption reason. |
| `STOPPING` | Stop new acquisition, close stream inputs, and release device ownership. | Detach source callbacks and preserve the last durable journal position. | Finalization continues even when a device-close operation fails. |
| `FINALIZING` | Flush the active segment, persist checksums, and schedule permitted transcription. | Mark the capture terminal only after durable metadata commits. | Storage failure preserves recovery metadata and enters `FAILED`. |
| `COMPLETED` | Publish durable segment inventory and the requested stop reason. | Terminal. | Transcription failure does not reopen capture. |
| `INTERRUPTED` | Publish sleep, device, shutdown, crash, or policy interruption and preserve segments. | Terminal. | Recovery can transcribe preserved audio without restarting capture. |
| `FAILED` | Publish safe failure metadata and preserve any durable segments. | Terminal. | User can retry finalization or delete retained audio. |

### 6.4 Capture transitions and timing

| From | Event | To | Required behavior |
|---|---|---|---|
| `CLOSED` | Explicit push-to-talk, hands-free, or meeting start | `STARTING` | Duplicate `user_action_id` returns the existing session. |
| `STARTING` | Required leases and streams ready | `ACTIVE` | Recording indicator is already visible. |
| `STARTING` | Key released before activation | `STOPPING` | Produce an empty or short capture without fabricating text. |
| `ACTIVE` | Manual stop, pause, Escape, or key release for push-to-talk | `STOPPING` | Stop new audio acquisition within 250 ms at p95 on reference hardware. |
| `ACTIVE` | Silence countdown expires | `STOPPING` | Record `SILENCE_TIMEOUT`; speech before expiry cancels the countdown. |
| `ACTIVE` | Microphone-release grace expires | `STOPPING` | Record `CALL_MIC_RELEASED`; active speech does not override a missing device. |
| `ACTIVE` | Sleep, app shutdown, device loss, low disk, or four-hour ceiling | `STOPPING` | Record the reason and preserve completed segments. |
| `STOPPING` | Inputs closed | `FINALIZING` | Finalization may continue with visible progress. |
| `FINALIZING` | All metadata durable and reason was normal | `COMPLETED` | Raw-audio deletion waits for the separate retention policy. |
| `FINALIZING` | All metadata durable and reason was an interruption | `INTERRUPTED` | Recovery does not start a new capture. |
| `FINALIZING` | Durable finalization failed | `FAILED` | Keep the journal and expose retry or deletion. |

The 60-minute and 90-minute events create notices only.
Calendar end creates a prompt only.
None of these events stops a healthy capture by itself.

## 7. Multi-MOG resource arbitration

### 7.1 Resource lease

A lease contains `resource_id`, `mog_id`, `capture_id`, `resource_epoch`, acquisition time, and owner-visible mode.
The broker persists lease ownership before stream activation.
A stale device callback must match both the active capture and current epoch.

| Requested resource | Conflict | Result |
|---|---|---|
| Global hotkey binding | Same active binding on another MOG | Reject the new binding during configuration. No capture starts. |
| Microphone device stream | Another capture owns the device | Reject or wait for an explicit later user retry. Show the owning MOG. Do not queue hidden capture. |
| System-audio stream | Another meeting owns the stream | Reject the second meeting. Show the owning MOG. |
| Same explicit start action | Broker already processed `user_action_id` | Return the original session without a second lease. |
| MOG foreground job | Another foreground job is active | Show busy or request explicit replacement. Do not create an invisible queue. |
| Motherbricks shutdown | Any active lease | Revoke new starts, stop all captures, and finalize journals. |
| Stale callback | Lease epoch no longer matches | Discard audio and events from that callback. |

For the MVP, each physical microphone stream has one capture owner.
Shared native acquisition and fan-out to multiple MOGs is not permitted until a later consent and arbitration design is approved.

## 8. Timeout, cancellation, retry, and uncertain delivery

### 8.1 Operation policy

The values below are maximum wait defaults, not expected service-answer times.
They are configurable only within validated connector policy.

| Operation | Default deadline or wait | Automatic retry | Cancellation and uncertain result |
|---|---:|---|---|
| CLI profile discovery | 5 s | One retry before process start only. | Terminate a stuck child process without changing profiles. |
| CLI token acquisition | 15 s | One retry when no token was returned and the CLI contract permits. | Keep the selected profile. Require explicit login for expired U2M identity. |
| Capability discovery | 5 s | One read-only retry with backoff. | Mark unknown; disable dependent optional features. |
| Local STT for an utterance of at most 15 s | Performance target in NFR-A-002; configurable compute deadline | Retry from the durable segment only when the job policy permits. | Preserve audio and expose retry or raw-audio deletion. |
| Hosted STT segment | 30 s per approved segment | Retry only before transmission or with verified idempotency. | Mark the segment uncertain; never send it to another provider automatically. |
| Cleanup | 2.0 s overall | No late automatic retry for the same utterance. | Use raw transcript; discard late UI updates. |
| Backend question submission | 30 s transport deadline | Retry `NOT_SENT`; retry after send only with verified idempotency and the same key. | Mark `UNCERTAIN`; do not ask again automatically. |
| Genie poll request | 15 s per request | Retry the read with exponential backoff and jitter. Respect `Retry-After`. | Never overlap a poll for the same response. Preserve remote identifiers. |
| Genie poll interval | Start at 1 s; increase to a 5 s ceiling | Not applicable. | After 10 minutes, show a review notice and continue or cancel by user choice. |
| Query-result fetch | 30 s | Retry the read with bounded backoff. | Preserve the query reference and show expiry or failure. |
| Backend cancellation | 15 s | Reconcile by polling before any repeated cancel. | Report `CANCELLATION_UNCERTAIN` until remote status is known. |
| Local structured recall | NFR-A-004 | No remote retry. | Return local failure independently of Lakebase. |
| Local semantic recall | NFR-A-005 | No remote retry for the local target. | Return structured results or an explicit unavailable status. |
| Lakebase sync batch | 30 s | Retry with the same change identifiers, exponential backoff, and jitter. | Keep the encrypted outbox; credential rotation uses the same changes. |

All retrying operations use bounded exponential backoff with jitter.
The default base is 250 ms, the ceiling is 30 s, and the maximum is five attempts unless a stricter operation policy applies.
A user cancellation stops future attempts.
A rate-limit response supplies the lower bound for the next attempt when `Retry-After` is valid.

### 8.2 Submission decision procedure

1. Persist the approved job, target, adapter, payload hash, and `request_id`.
2. Record `NOT_SENT` before opening the transport.
3. Record `SENDING` immediately before request bytes can be emitted.
4. Accept only a schema-valid response for the same target and request attempt.
5. Persist remote identifiers and `ACKNOWLEDGED` before displaying submission acceptance.
6. Mark `UNCERTAIN` after a timeout, disconnect, crash, or invalid success response when delivery cannot be proved.
7. Reconcile through a verified idempotency lookup or the original remote identifiers when available.
8. Require a new explicit user action and a new `request_id` for a new question after unresolved uncertainty.

The UI must distinguish “not sent”, “delivery unknown”, “accepted”, “waiting”, “cancel requested”, and “cancelled”.

## 9. Offline and crash recovery

### 9.1 Durable journal records

| Record | Minimum persisted content |
|---|---|
| Capture journal | Capture identity, MOG, mode, consent revision, resource epochs, segment offsets, checksums, stop reason, and retention class. |
| STT job | Capture and segment identity, audio hash, provider and model, delivery phase, and result reference. |
| Cleanup job | Utterance identity, raw hash, configuration hash, deadline, result hash, and superseded revision. |
| Backend request | Job, target, adapter version, payload hash, request and attempt identifiers, delivery phase, remote identifiers, and safe error metadata. |
| Memory outbox | Memory identity, revision, sync change identifier, tombstone, retention class, and sync attempt metadata. |
| Deletion journal | MOG or record scope, local deletion status, remote tombstone status, and minimum required provenance. |

Tokens and plaintext audio are not valid journal metadata.
Content encryption and key management follow the Security and Memory contracts.

### 9.2 Restart rules

| Recovered condition | Required recovery |
|---|---|
| Capture was `STARTING` or `ACTIVE` | Fence old epochs, mark capture interrupted, close orphan resources, and preserve every completed segment. |
| Capture was `STOPPING` or `FINALIZING` | Resume finalization from the last durable journal position. Do not resume acquisition. |
| Local STT job was incomplete | Reuse the durable audio reference and idempotency identity. Hosted uncertain jobs follow the no-hidden-fallback rule. |
| Cleanup was incomplete | Recompute locally from the retained raw transcript or select raw. Do not alter inserted text. |
| Backend request was `NOT_SENT` | Restore an unsent question. Do not submit automatically. |
| Backend request was `SENDING` | Mark delivery uncertain. Do not submit automatically. |
| Backend response was `ACKNOWLEDGED` | Resume the original poll or recovery operation when safe. Do not create a new response. |
| Cancellation was uncertain | Reconcile the original response before claiming cancellation. |
| Memory outbox was incomplete | Retry configured Lakebase changes with stable identifiers. Local recall remains available. |
| MOG deletion was incomplete offline | Finish local deletion and retain only the scoped deletion outbox for later propagation. |

Reconnection never approves a payload, changes a target, starts capture, or submits an unsent question.

## 10. Numbered architecture requirements

Verification labels are `Automated`, `Platform`, `Integration`, `Inspection`, and `Primary source`.
`Platform` means a physical macOS or Windows release gate unless the test explicitly names another supported target.

| ID | Priority | Release | Testable requirement | Decision | Verification | Acceptance test |
|---|---|---|---|---|---|---|
| MB-A-001 | MUST | Desktop MVP | Motherbricks shall route device, credential, provider, backend, and memory operations through the native core. | D-01 | Inspection, Automated | AT-ARC-001 |
| MB-A-002 | MUST | Desktop MVP | Motherbricks shall persist MOG job state separately from capture state. | D-02, D-11 | Automated | AT-ARC-010, AT-ARC-013 |
| MB-A-003 | SHOULD | Desktop MVP | Motherbricks should allow one foreground job per MOG while meeting transcription uses a bounded background queue. | D-11 | Automated | AT-ARC-022 |
| CAP-A-001 | MUST | Desktop MVP | The capture broker shall enforce exclusive hotkey and audio-stream leases for all MOGs. | D-11 | Platform | AT-ARC-010 |
| CAP-A-002 | MUST | Desktop MVP | A capture session shall start only from a valid explicit user-action record. | D-02 | Automated, Platform | AT-ARC-011 |
| CAP-A-003 | MUST | Desktop MVP | The capture broker shall reject stale device callbacks by capture identity and resource epoch. | D-11, D-12 | Automated, Platform | AT-ARC-010 |
| CAP-A-004 | MUST | Desktop MVP | Motherbricks shall stop new audio acquisition within the manual-stop budget while finalization continues independently. | D-02, D-05 | Platform | AT-ARC-017 |
| STT-A-001 | MUST | Desktop MVP | The STT contract shall preserve an immutable raw transcript with provider, model, audio-hash, and confidence metadata when available. | D-10 | Automated | AT-ARC-012 |
| STT-A-002 | MUST | Desktop MVP | Cleanup shall select the raw transcript when its deadline or fidelity validation fails. | D-10 | Automated | AT-ARC-012 |
| STT-A-003 | MUST | Configured MVP | Motherbricks shall select an STT or cleanup provider only from discovered or explicitly configured capabilities. | D-01, D-10 | Integration, Primary source | AT-ARC-001 |
| RTE-A-001 | MUST | Desktop MVP | The router shall require an explicit route before any backend request. | D-09 | Automated | AT-ARC-002 |
| RTE-A-002 | MUST | Desktop MVP | The context gate shall bind user approval to the exact payload hash and displayed backend target. | D-03 | Automated | AT-ARC-002 |
| RTE-A-003 | MUST | Desktop MVP | Motherbricks shall invalidate context approval after any payload or target change. | D-03, D-08 | Automated | AT-ARC-002, AT-ARC-003 |
| GEN-A-001 | MUST | Desktop MVP | Each enabled backend adapter shall report tri-state capabilities before dependent operations are offered. | D-04, D-08 | Integration | AT-ARC-001 |
| GEN-A-002 | MUST | Desktop MVP | The Genie One MCP adapter shall use the verified Genie One endpoint and exclude the retired beta MCP endpoint. | D-04 | Primary source, Integration | AT-ARC-006 |
| GEN-A-003 | MUST | Desktop MVP | The backend scheduler shall prevent overlapping poll calls for one remote response. | D-04 | Automated | AT-ARC-006 |
| GEN-A-004 | MUST | Desktop MVP | Every backend operation shall use the conversation affinity persisted at submission. | D-08 | Automated | AT-ARC-003 |
| GEN-A-005 | MUST | Desktop MVP | A backend or workspace switch shall create a new conversation for the next question. | D-08 | Automated | AT-ARC-003 |
| GEN-A-006 | MUST | Desktop MVP | An adapter failure shall not trigger an automatic backend, workspace, or provider fallback. | D-04, D-08, D-12 | Automated | AT-ARC-007, AT-ARC-009 |
| GEN-A-007 | MUST | Configured MVP | A custom Apps or Model Serving connector shall use its own verified invocation and authentication contract. | D-08 | Primary source, Integration | AT-ARC-009 |
| GEN-A-008 | MUST | Desktop MVP | The response projector shall preserve source status, numbers, units, caveats, truncation, and supplied deep links. | D-13 | Automated | AT-ARC-019 |
| GEN-A-009 | MUST | Configured MVP | Cancellation shall distinguish request acknowledgement, verified remote cancellation, local stop-waiting, and uncertain cancellation. | D-12, D-13 | Automated, Integration | AT-ARC-008 |
| WS-A-001 | MUST | Desktop MVP | The authentication provider shall invoke supported CLI commands without a shell command string. | D-07 | Inspection, Platform | AT-ARC-004 |
| WS-A-002 | MUST | Desktop MVP | The credential broker shall acquire and use a token only for the canonical workspace host bound to the selected profile. | D-07 | Automated, Platform | AT-ARC-004 |
| WS-A-003 | MUST | Desktop MVP | Token refresh shall retain the selected profile and workspace binding. | D-07 | Automated, Platform | AT-ARC-005 |
| MEM-A-001 | MUST | Desktop MVP | A memory mutation shall commit local content and its sync outbox entry in one transaction. | D-06, D-12 | Automated | AT-ARC-016 |
| MEM-A-002 | MUST | Desktop MVP | Every recall query shall filter by MOG and workspace scope before ranking. | D-06 | Automated | AT-ARC-015 |
| MEM-A-003 | MUST | Configured MVP | Lakebase synchronization shall use stable memory revisions and sync-change identifiers for every retry. | D-06, D-12 | Integration | AT-ARC-016 |
| MEM-A-004 | MUST | Desktop MVP | Local deletion shall write a tombstone without requiring connectivity. | D-14 | Automated, Integration | AT-ARC-021 |
| MEM-A-005 | MUST | Desktop MVP | Memory recall shall return candidates without granting context-sharing approval. | D-03, D-06 | Automated | AT-ARC-002 |
| REL-A-001 | MUST | Desktop MVP | Every provider and backend operation shall carry a stable request identifier and a unique attempt identifier. | D-12 | Automated | AT-ARC-001, AT-ARC-007 |
| REL-A-002 | MUST | Desktop MVP | Automatic retry shall follow the persisted delivery phase and the verified operation idempotency contract. | D-12 | Automated | AT-ARC-007 |
| REL-A-003 | MUST | Desktop MVP | Motherbricks shall persist recovery metadata before the dependent capture, submission, or synchronization side effect. | D-12 | Automated, Platform | AT-ARC-013, AT-ARC-014 |
| REL-A-004 | MUST | Desktop MVP | Restart recovery shall preserve original identifiers and shall not automatically submit an unsent or uncertain question. | D-12 | Automated | AT-ARC-014 |
| REL-A-005 | MUST | Desktop MVP | Offline mode shall disable unavailable hosted capabilities without disabling available local dictation and local recall. | D-12 | Platform, Automated | AT-ARC-012, AT-ARC-015 |
| REL-A-006 | MUST | Desktop MVP | Motherbricks shall discard late provider callbacks that no longer match the active job revision. | D-10, D-12 | Automated | AT-ARC-012 |
| NFR-A-001 | MUST | Desktop MVP | The visible hotkey response shall have a p95 latency of at most 100 ms while Motherbricks is running. | D-01 | Platform | AT-ARC-017 |
| NFR-A-002 | MUST | Desktop MVP | Warm local STT shall have a p95 latency of at most 1.5 s after key-up for utterances of at most 15 s on reference hardware. | D-01, D-10 | Platform | AT-ARC-018 |
| NFR-A-003 | MUST | Desktop MVP | Cleanup shall have an overall deadline of 2.0 s. | D-10 | Automated, Platform | AT-ARC-012 |
| NFR-A-004 | MUST | Desktop MVP | Local structured recall shall have a p95 latency of at most 50 ms for 10,000 local records on reference hardware. | D-06 | Platform | AT-ARC-015 |
| NFR-A-005 | SHOULD | Desktop MVP | Local semantic recall should have a p95 latency of at most 250 ms for 10,000 indexed records with a local query embedding on reference hardware. | D-06 | Platform | AT-ARC-015 |
| NFR-A-006 | MUST | Desktop MVP | Backend submission feedback shall appear within 300 ms after the transport result is known. | D-13 | Platform, Automated | AT-ARC-019 |
| NFR-A-007 | MUST | Desktop MVP | Manual stop shall end new audio acquisition within 250 ms at p95 on reference hardware. | D-02, D-05 | Platform | AT-ARC-017 |
| NFR-A-008 | MUST | Desktop MVP | Crash recovery shall preserve every completed audio segment and every acknowledged remote response identifier. | D-12 | Platform, Automated | AT-ARC-013, AT-ARC-014 |
| NFR-A-009 | SHOULD | Desktop MVP | The capacity soak should cover 10 configured MOGs, three concurrent backend waits, 10,000 memories per scoped test set, and one four-hour meeting. | D-11, D-12 | Platform | AT-ARC-022 |

## 11. Nonfunctional measurement rules

- Each latency test uses at least 100 samples after the documented warm-up unless the test is a four-hour soak.
- Each latency report includes p50, p95, maximum, sample count, failures, and reference environment.
- STT latency starts at key-up and ends when the raw transcript is available to the route orchestrator.
- Hotkey latency starts at the operating-system callback timestamp and ends at the first visible state commit.
- Capture-stop latency ends when the audio source reports that acquisition stopped, not when finalization ends.
- Local structured recall excludes network access and includes scope filtering and result materialization.
- Local semantic recall includes local query embedding and vector search. Remote embedding latency is reported separately.
- Backend feedback latency starts when a success or failure transport result is available to the native core.
- No fixed Genie answer-time target is permitted. Progress, elapsed time, and cancellation are measured instead.
- A test that cannot run in this environment remains `NOT RUN` with a named release gate.

## 12. Diagram review and integration guidance

Team A reviewed all four SVG files in `mother-databricks/diagrams`.
Team A did not edit any SVG file.
The diagram specialist owns all diagram changes.
All four SVG files parsed as well-formed XML and contained an accessible title and description at review time.

### 12.1 `01-system_kimi.svg` findings

The existing SVG has an accessible title and description.
It shows shared native services, multiple MOGs, selected workspaces, local-first memory, pinned conversations, and alternate adapters.
The following changes are required before G3 sign-off:

| Finding | Required diagram-specialist action | Linked requirement |
|---|---|---|
| Product labels use the superseded two-word naming. | Rename project and application labels to **Motherbricks**. | MB-A-001 |
| The context-only behavior is marked TBD. | Show context-only utterances as local saves. Show context in the next approved question only. | RTE-A-001, RTE-A-002 |
| Cross-workspace sharing is marked TBD. | Show scope isolation by default and explicit export into a new approved question only. | MEM-A-002, GEN-A-005 |
| The optional hosted-ASR arrow starts at the cleanup component. | Start the audio arrow at the STT orchestrator. Keep cleanup text on a separate arrow. | STT-A-003 |
| The device diagram does not separate the WebView from the native trust boundary. | Add the typed native command boundary and prohibit token flow to the WebView. | MB-A-001, WS-A-002 |
| Credential flow to model services, Lakebase, and alternate agents is not explicit. | Show host-bound credential use for every external call or reference the trust-boundary inventory. | WS-A-002, MEM-A-003 |
| Dashed paths can be read as simultaneous delivery to two workspaces. | Label each request as exactly one selected target. Show original-target polling after a home-profile switch. | GEN-A-004, GEN-A-005 |
| The recording indicator is not shown as independent privacy UI. | Add an indicator controlled by capture state, independent of mascot visibility. | MB-A-002, CAP-A-002 |
| The local Parakeet label can be read as verified hardware feasibility. | Mark it as the proposed local provider pending `REF-MAC` and `REF-WIN` tests. | STT-A-003, NFR-A-002 |
| The default Genie path can be read as fully verified. | Mark endpoint, CLI-token scope, and grants as pending Platform verification until G1 passes. | GEN-A-002, WS-A-001 |

### 12.2 Diagram-to-requirement map

| Diagram element or planned view | Requirements that explain it |
|---|---|
| MOG instances and independent configurations | MB-A-002, GEN-A-004, MEM-A-002 |
| Typed UI to native boundary | MB-A-001, WS-A-002 |
| Hotkey and audio broker | CAP-A-001 through CAP-A-004 |
| STT and cleanup orchestration | STT-A-001 through STT-A-003, REL-A-006 |
| Context gate and explicit router | RTE-A-001 through RTE-A-003 |
| Backend registry and selected adapter | GEN-A-001 through GEN-A-009 |
| CLI and credential broker | WS-A-001 through WS-A-003 |
| Local store, outbox, and Lakebase | MEM-A-001 through MEM-A-005 |
| Voice-to-answer sequence | STT-A-001, RTE-A-002, GEN-A-003, REL-A-001 through REL-A-006 |
| Separate MOG and capture state views | MB-A-002, CAP-A-002, REL-A-003 |
| Multi-MOG microphone arbitration | CAP-A-001, CAP-A-003 |
| Memory and workspace data flow | GEN-A-004, MEM-A-002 through MEM-A-005, WS-A-002 |
| Trust boundaries and response rendering | GEN-A-008, WS-A-002, REL-A-003 |

The voice sequence must include the `SENDING` to `UNCERTAIN` branch.
The state view must not combine `recording` with `transcribing` as one mutually exclusive MOG state.
The memory view must distinguish synchronization approval from context-sharing approval.

### 12.3 Voice, state, and trust-diagram findings

| SVG | Finding | Required diagram-specialist or director action | Linked requirement or test |
|---|---|---|---|
| `02-voice-sequence_kimi.svg` | Step 15 shows local hits or direct Lakebase results on the question path. | Keep interactive recall local. Show Lakebase as background durable synchronization or a separate explicit remote-recall operation. | MEM-A-001, REL-A-005, AT-ARC-015 |
| `02-voice-sequence_kimi.svg` | The arrows reach `genie_ask` without a post-recall exact-payload approval step. | Insert exact payload preview, target confirmation, and payload-hash binding after recall and before submission when context is included. | RTE-A-002, RTE-A-003 |
| `02-voice-sequence_kimi.svg` | Submission has no uncertain-delivery branch. | Add `NOT_SENT`, `SENDING`, `ACKNOWLEDGED`, and `UNCERTAIN`. Show no automatic post-send retry without idempotency. | REL-A-001 through REL-A-004, AT-ARC-007 |
| `02-voice-sequence_kimi.svg` | Microphone conflict text permits an unspecified queue. | Use visible rejection and explicit later retry under this architecture. Do not leave queue policy implicit. | CAP-A-001 |
| `02-voice-sequence_kimi.svg` | Cancellation is shown as a direct call without an uncertain outcome. | Add acknowledged, unsupported, and uncertain cancellation branches with original-response reconciliation. | GEN-A-009, AT-ARC-008 |
| `02-voice-sequence_kimi.svg` | One “approved memory event” label can conflate local save approval with Lakebase synchronization. | Label local memory-write policy and durable-sync policy separately. | MEM-A-001, MEM-A-005 |
| `03-state_kimi.svg` | The proposed broker lets the meeting owner reuse its stream for push-to-talk, but this report requires one capture owner per microphone stream. | Director must select the safe MVP rule. This report proposes rejecting a second capture session, including same-MOG push-to-talk, until a same-owner consumer contract is approved. | CAP-A-001, B-07 |
| `03-state_kimi.svg` | The interaction diagram combines listening with interaction state, while the architecture treats capture as independent. | Label listening as a capture-derived presentation and retain the separate capture authority. | MB-A-002 |
| `03-state_kimi.svg` | Cancellation ends after acknowledgement and has no uncertain branch. | Add cancellation-unknown recovery and distinguish local stop-waiting from remote cancellation. | GEN-A-009 |
| `03-state_kimi.svg` | “Delete cancels active work” can imply verified remote cancellation or deletion. | Label local work termination separately from supported remote cancellation and Genie-side deletion. | MEM-A-004, GEN-A-009 |
| `03-state_kimi.svg` | Calendar end appears among stop signals, and the 90-minute label can imply a stop without the silence countdown. | Mark calendar end and 90 minutes as review signals only. Keep stop conditions independent. | CAP-A-002, AT-ARC-011 |
| `04-trust_kimi.svg` | Hosted audio goes directly from capture to model services. | Show the selected hosted STT orchestrator as the egress gate and preserve the selected workspace binding. | STT-A-003, WS-A-002 |
| `04-trust_kimi.svg` | The trust and classification view does not show the local outbox, local-only recall path, or tombstone replay. | Extend the memory and workspace flow with local commit, outbox, retry, deletion, and no-network recall. | MEM-A-001 through MEM-A-004 |
| `04-trust_kimi.svg` | “Approved memory” can conflate synchronization approval with Genie context approval. | Use separate labels for durable-sync policy and outbound-context approval. | MEM-A-005, RTE-A-002 |

## 13. Acceptance tests

All tests use synthetic content and non-production workspace identifiers.
`CI` tests run with fake providers and fault injection.
`Integration` tests require an approved Databricks test workspace.
`Platform` tests require the named physical reference environment.
A platform result is currently **NOT RUN**.

### AT-ARC-001: Contracts and capability discovery

- **Requirements:** MB-A-001, STT-A-003, GEN-A-001, REL-A-001.
- **Release and platform:** Desktop MVP; CI on the supported development platform.
- **Preconditions:** Fake providers expose supported, unsupported, and unknown capabilities. A fake UI attempts a direct transport call.
- **Actions:** Discover capabilities. Request an unknown streaming feature. Submit duplicate logical operations with different attempts. Inspect the UI command surface.
- **Expected:** The unknown feature is disabled. No substitute adapter is called. Request identifiers remain stable and attempt identifiers differ. The UI has no direct transport or token method.
- **Measurement:** Record one in-flight operation for each logical request where single-flight applies.
- **Evidence:** Contract-test results, capability snapshots, and redacted correlation logs.
- **Method:** Automated trait and command-boundary tests.

### AT-ARC-002: Explicit routing and context approval

- **Requirements:** RTE-A-001, RTE-A-002, RTE-A-003, MEM-A-005.
- **Release and platform:** Desktop MVP; CI.
- **Preconditions:** Two local memories contain unique sentinel strings. Context sharing is off. Dictation and Ask modes are configured.
- **Actions:** Dictate a sentence ending in a question mark. Ask without context. Preview approved context. Change one context item and then the workspace target.
- **Expected:** Dictation causes no backend call. The no-context question contains neither sentinel. Approval includes the exact payload hash. Each change invalidates approval and blocks submission.
- **Measurement:** Count outbound requests and compare payload bytes with the approved preview.
- **Evidence:** Route events, payload hashes, fake-transport request count, and approval-state transitions.
- **Method:** Automated router and context-envelope tests.

### AT-ARC-003: Conversation and workspace affinity

- **Requirements:** RTE-A-003, GEN-A-004, GEN-A-005.
- **Release and platform:** Desktop MVP; CI with two fake workspace origins.
- **Preconditions:** MOG home profile is A. Conversation A1 has response R1. Profile B and another backend are configured.
- **Actions:** Submit A1. Switch once to B. Poll and cancel R1. Attempt to reuse A1 on B. Submit a new question on B.
- **Expected:** Poll and cancel use profile A, host A, and original backend. Reuse on B is rejected. The new question creates B conversation B1. The MOG home profile remains A.
- **Measurement:** Compare every outbound target and conversation key with the persisted affinity.
- **Evidence:** Redacted target records, request traces, and conversation identifiers.
- **Method:** Automated affinity and token-binding tests.

### AT-ARC-004: CLI invocation and hostile-host rejection

- **Requirements:** WS-A-001, WS-A-002.
- **Release and platform:** Desktop MVP; CI plus Integration for the real CLI.
- **Preconditions:** A fake CLI records argv. Profiles include valid U2M, unsupported authentication, a non-HTTPS host, and a host that imitates another profile.
- **Actions:** Discover profiles. Acquire a token. Attempt cross-host token use. Invoke the real supported CLI profile and token commands.
- **Expected:** No shell string or shell metacharacter is evaluated. Unsupported and hostile profiles are rejected. A valid token reaches only its bound HTTPS origin. The real command schema matches the Platform claim matrix.
- **Measurement:** Inspect all process arguments and outbound origins. Scan renderer events and logs for token sentinels.
- **Evidence:** Argv captures, outbound fake-server logs, token-leak scan, and CLI version.
- **Method:** Automated security tests plus primary-source and CLI platform verification.

### AT-ARC-005: Expired-profile recovery

- **Requirements:** WS-A-003.
- **Release and platform:** Desktop MVP; physical `REF-MAC` and `REF-WIN`, with an Integration CLI profile.
- **Preconditions:** A configured MOG has an expired U2M CLI profile and saved local configuration.
- **Actions:** Attempt an authenticated operation. Observe the expiry. Complete explicit CLI login. Retry the same operation.
- **Expected:** No automatic profile or scope change occurs. MOG configuration remains unchanged. Refresh retains the original host and profile. No token appears in UI or persistent content.
- **Measurement:** Record recovery steps and compare configuration hashes before and after login.
- **Evidence:** Platform procedure, CLI version, configuration hashes, and redacted auth events.
- **Method:** Manual platform procedure with automated log inspection. **Status: NOT RUN.**

### AT-ARC-006: Serialized polling and verified Genie path

- **Requirements:** GEN-A-002, GEN-A-003.
- **Release and platform:** Desktop MVP; CI and Integration.
- **Preconditions:** A fake Genie adapter delays poll responses and returns a rate limit with `Retry-After`. Platform supplies the verified endpoint matrix.
- **Actions:** Trigger concurrent poll schedulers for one response. Simulate timeout and rate limiting. Inspect all request paths.
- **Expected:** At most one poll is active for the response. Retry follows the read policy and rate-limit lower bound. No request uses the retired beta MCP path. Poll progress is not labeled streaming.
- **Measurement:** Record maximum concurrent polls, retry intervals, and all endpoint paths.
- **Evidence:** Fake-server concurrency trace, scheduler trace, and Platform claim-matrix reference.
- **Method:** Automated scheduler tests plus Integration contract verification.

### AT-ARC-007: Uncertain submission and no duplicate question

- **Requirements:** GEN-A-006, REL-A-001, REL-A-002.
- **Release and platform:** Desktop MVP; CI fault injection.
- **Preconditions:** A fake transport can fail before bytes, after bytes, after remote acceptance, and during a malformed success response.
- **Actions:** Submit one approved question under each fault. Wait through the retry window. Reconcile when supported. Explicitly start a new question after unresolved uncertainty.
- **Expected:** `NOT_SENT` follows its retry policy. Post-send uncertainty creates no automatic second question. A verified idempotent adapter reuses the same key. A new question receives a new request identifier and user-action identifier.
- **Measurement:** Count remote question creations and compare request and idempotency identifiers.
- **Evidence:** Delivery journal, transport fault record, remote creation count, and user-action events.
- **Method:** Automated deterministic fault-injection tests.

### AT-ARC-008: Cancellation uncertainty and reconciliation

- **Requirements:** GEN-A-009.
- **Release and platform:** Configured MVP; CI and Integration for each adapter that advertises cancel.
- **Preconditions:** Response R1 is pending. The fake adapter can acknowledge cancel, reject it, lose its response, or leave R1 running.
- **Actions:** Request cancellation under each outcome. Stop waiting locally in a separate case.
- **Expected:** A cancellation-request acknowledgement is labelled cancellation requested. Only later remote terminal evidence is labelled cancelled. A lost response is labelled uncertain and reconciled by the original response identifier. Local stop-waiting is not labelled remote cancellation. No new question is submitted.
- **Measurement:** Record cancel attempts, poll reconciliation, remote status, and displayed status.
- **Evidence:** Adapter traces, response-status timeline, and UI-state snapshots with redacted content.
- **Method:** Automated adapter tests plus Integration contract verification.

### AT-ARC-009: Alternate adapters without hidden fallback

- **Requirements:** GEN-A-006, GEN-A-007.
- **Release and platform:** Configured MVP for Apps and Serving; Later for beta SSE; CI and Integration.
- **Preconditions:** Contract fixtures exist for Genie REST, Apps, Serving, and SSE. Each adapter has one configured target and a second fallback target.
- **Actions:** Run schema, auth, idempotency, stream-recovery, malformed-response, and unsupported-operation cases. Fail each primary target.
- **Expected:** Each adapter uses only its declared contract. Apps idempotency reuses the original key when supported. SSE recovery does not re-POST. No failure invokes the fallback target.
- **Measurement:** Compare request paths, bodies, authentication route, remote creations, and fallback call count.
- **Evidence:** Contract results and redacted request traces.
- **Method:** Shared adapter contract suite and connector-specific Integration tests.

### AT-ARC-010: Multi-MOG arbitration and stale callbacks

- **Requirements:** MB-A-002, CAP-A-001, CAP-A-003.
- **Release and platform:** Desktop MVP; physical `REF-MAC` and `REF-WIN`.
- **Preconditions:** MOG A and B request the same hotkey and audio device. A owns an active capture.
- **Actions:** Configure a duplicate hotkey. Request capture on B. Inject a delayed A callback after its lease ends. Shut down Motherbricks during the conflict.
- **Expected:** Duplicate binding is rejected. B receives a visible conflict naming A and no queued capture. Stale audio is discarded. Shutdown stops and finalizes all capture.
- **Measurement:** Inspect active owner, resource epochs, acquired audio frames, and final capture states.
- **Evidence:** Broker journal, device traces, and visible conflict logs.
- **Method:** Automated broker tests plus manual device procedure. **Status: NOT RUN.**

### AT-ARC-011: Capture state, consent, and environmental stops

- **Requirements:** CAP-A-002.
- **Release and platform:** Desktop MVP; physical `REF-MAC` and `REF-WIN`.
- **Preconditions:** Microphone and system-audio permissions can be denied. Synthetic calendar, silence, microphone-release, sleep, and low-disk signals are available.
- **Actions:** Attempt starts from calendar, transcript, memory, and backend response. Start explicitly. Exercise manual stop, pause, Escape, permission denial, silence, calendar end, microphone release, sleep, low disk, 90-minute notice, and four-hour ceiling.
- **Expected:** Only explicit actions start capture. The indicator precedes activation and survives mascot hiding. Denial creates no audio. Notices do not stop healthy capture by themselves. Required stops finalize durable segments.
- **Measurement:** Compare capture and MOG state logs. Verify zero frames for denied starts.
- **Evidence:** State timeline, permission state, segment inventory, and indicator event log.
- **Method:** Automated policy tests plus manual platform procedure. **Status: NOT RUN.**

### AT-ARC-012: Offline dictation, cleanup fallback, and late results

- **Requirements:** STT-A-001, STT-A-002, REL-A-005, REL-A-006, NFR-A-003.
- **Release and platform:** Desktop MVP; CI plus physical `REF-MAC` and `REF-WIN` for local STT.
- **Preconditions:** Local STT is ready. Network is unavailable. Hosted cleanup is configured but unreachable. A controllable cleanup provider can return after insertion.
- **Actions:** Dictate text. Force cleanup timeout and fidelity failure. Insert the selected transcript. Deliver the late cleaned result.
- **Expected:** Raw transcript remains immutable and is inserted. No backend request occurs. The late result cannot overwrite inserted or copied text. Local recall remains available.
- **Measurement:** Cleanup fallback occurs by 2.0 s. Compare inserted text with the raw transcript at all later callbacks.
- **Evidence:** Utterance revisions, insertion event, network denial trace, and deadline measurements.
- **Method:** Automated failure tests plus local-provider platform verification.

### AT-ARC-013: Capture crash and finalization recovery

- **Requirements:** MB-A-002, REL-A-003, NFR-A-008.
- **Release and platform:** Desktop MVP; physical `REF-MAC` and `REF-WIN`.
- **Preconditions:** A meeting has multiple completed segments and one active journal. Faults can terminate Motherbricks in `ACTIVE`, `STOPPING`, and `FINALIZING`.
- **Actions:** Inject each crash. Restart. Inspect resource ownership and segment recovery.
- **Expected:** Old epochs are fenced. Acquisition does not resume automatically. Every completed segment and its checksum is preserved. Finalization resumes from the durable position.
- **Measurement:** Compare pre-crash completed-segment inventory with recovered inventory. Require zero lost completed segments.
- **Evidence:** Crash-fault log, journal checksums, recovered inventory, and state transitions.
- **Method:** Automated crash harness on each reference platform. **Status: NOT RUN.**

### AT-ARC-014: Backend-request crash recovery

- **Requirements:** REL-A-003, REL-A-004, NFR-A-008.
- **Release and platform:** Desktop MVP; CI fault injection.
- **Preconditions:** Request journals can crash at `NOT_SENT`, `SENDING`, and `ACKNOWLEDGED`. Remote identifiers are known in the last case.
- **Actions:** Restart after each crash and restore connectivity.
- **Expected:** Unsent and uncertain requests remain unsubmitted. `SENDING` becomes uncertain. Acknowledged work resumes through its original adapter and identifiers. No duplicate remote response is created.
- **Measurement:** Compare remote response count before and after restart.
- **Evidence:** Recovery journal, remote creation count, and resumed poll trace.
- **Method:** Automated crash-state parameterized tests.

### AT-ARC-015: Scoped offline recall and latency

- **Requirements:** MEM-A-002, REL-A-005, NFR-A-004, NFR-A-005.
- **Release and platform:** Desktop MVP; CI for scope, physical reference hardware for latency.
- **Preconditions:** Each of two MOGs and two workspaces has a 10,000-record synthetic test set with unique sentinels. Lakebase and network are unavailable.
- **Actions:** Run structured and local semantic queries for each scope. Attempt cross-scope identifiers. Repeat at least 100 times after warm-up.
- **Expected:** No unauthorized record is returned. Lakebase failure does not block local recall. Structured p95 is at most 50 ms. The proposed semantic p95 is at most 250 ms when local embedding is used.
- **Measurement:** Report p50, p95, maximum, failures, sample count, and reference environment separately for each query type.
- **Evidence:** Query scope logs, result-count assertions, latency files, and environment record.
- **Method:** Automated scope tests and benchmark harness. **Performance status: NOT RUN.**

### AT-ARC-016: Idempotent sync and credential rotation

- **Requirements:** MEM-A-001, MEM-A-003.
- **Release and platform:** Configured MVP; CI plus Lakebase Integration.
- **Preconditions:** Local mutations and one deletion are queued. The sync transport can acknowledge then drop the response. A credential expires during retry.
- **Actions:** Commit a local mutation while Lakebase is unavailable. Retry the same batch. Rotate the credential. Repeat the acknowledged-but-lost case.
- **Expected:** Local commit and recall succeed immediately. Every retry retains stable memory revisions and sync-change identifiers. Rotation loses no queued change. Remote replay creates no duplicate memory or resurrection.
- **Measurement:** Count local and remote records by stable identity and revision.
- **Evidence:** Local transaction log, sync batches, credential-generation metadata without secrets, and remote row counts.
- **Method:** Automated outbox tests plus approved Integration workspace procedure.

### AT-ARC-017: Hotkey response and capture-stop latency

- **Requirements:** CAP-A-004, NFR-A-001, NFR-A-007.
- **Release and platform:** Desktop MVP; physical `REF-MAC` and `REF-WIN`.
- **Preconditions:** Motherbricks is running and warm. A hotkey is valid. At least 100 push-to-talk samples are scripted per platform.
- **Actions:** Press and release the hotkey. Measure visible response and manual-stop acquisition end under idle and meeting-transcription load.
- **Expected:** Visible response p95 is at most 100 ms. New audio acquisition stop p95 is at most 250 ms. Finalization remains visible and does not extend acquisition.
- **Measurement:** Use operating-system callback, UI commit, and audio-source timestamps. Report p50, p95, maximum, and failures.
- **Evidence:** Raw timestamp traces, benchmark build, and environment record.
- **Method:** Platform instrumentation and repeatable input harness. **Status: NOT RUN.**

### AT-ARC-018: Warm local STT latency

- **Requirements:** NFR-A-002.
- **Release and platform:** Desktop MVP; physical `REF-MAC` and `REF-WIN`.
- **Preconditions:** The selected local STT model is warm. A fixed corpus has at least 100 consented synthetic utterances of at most 15 s with expected transcripts.
- **Actions:** Play each utterance through the approved audio test path. Measure key-up to raw-transcript availability. Repeat under the documented background load.
- **Expected:** p95 is at most 1.5 s on each named reference environment. Empty and denied audio produce no fabricated transcript.
- **Measurement:** Report p50, p95, maximum, failures, utterance duration, model version, and transcript accuracy separately.
- **Evidence:** Latency files, corpus manifest, model hash, and environment record.
- **Method:** Platform benchmark. **Status: NOT RUN.**

### AT-ARC-019: Response projection and feedback timing

- **Requirements:** GEN-A-008, NFR-A-006.
- **Release and platform:** Desktop MVP; CI plus UI platform measurement.
- **Preconditions:** Fixtures contain successful, failed, incomplete, cancelled, truncated, and malformed responses with numbers, units, caveats, safe and hostile links.
- **Actions:** Normalize and render every fixture. Measure known-transport-result to feedback commit. Attempt to render an unsanitized payload.
- **Expected:** Each status is explicit. Source numbers, units, caveats, truncation, and valid supplied deep links survive. No link is invented. Unsafe markup and instructions do not execute. Feedback commits within 300 ms after the transport result is known.
- **Measurement:** Compare structured source values with the blurb model and record feedback timestamps.
- **Evidence:** Normalized fixtures, sanitized DOM assertions, and timing trace.
- **Method:** Automated projector and UI tests.

### AT-ARC-020: Untrusted-content isolation

- **Requirements:** MB-A-001, CAP-A-002, GEN-A-008, RTE-A-001.
- **Release and platform:** Desktop MVP; CI.
- **Preconditions:** Transcript, memory, and backend-response fixtures contain instructions to start recording, call tools, change workspace, and reveal tokens.
- **Actions:** Process each fixture through dictation, memory recall, context preview, response projection, and full-response display.
- **Expected:** No capture, tool, backend, workspace, or auth state changes without a separate valid user action. Token sentinels remain absent from output.
- **Measurement:** Count privileged command invocations and compare them with explicit user-action records.
- **Evidence:** Privileged-command audit and content-isolation test results.
- **Method:** Automated prompt-injection and command-authorization tests.

### AT-ARC-021: Offline MOG deletion and later propagation

- **Requirements:** MEM-A-004.
- **Release and platform:** Desktop MVP; CI plus Lakebase Integration for propagation.
- **Preconditions:** A MOG has local audio, transcripts, memories, cached tokens, pending context, and synced records. The network is offline.
- **Actions:** Delete the MOG offline. Inspect local storage. Reconnect with a scoped test identity. Complete deletion synchronization.
- **Expected:** Hotkeys and capture stop. Local content, indexes, cached tokens, and pending context are removed. Tombstones propagate after reconnection. No UI claim states that Genie-side deletion occurred without a verified capability.
- **Measurement:** Inspect records by MOG scope before and after deletion and replay.
- **Evidence:** Deletion journal, local storage inspection, remote tombstone result, and token-cache inspection.
- **Method:** Automated local tests plus Integration verification.

### AT-ARC-022: Capacity and meeting soak

- **Requirements:** MB-A-003, NFR-A-009.
- **Release and platform:** Desktop MVP recommendation; physical `REF-MAC` and `REF-WIN`.
- **Preconditions:** Ten configured MOGs use distinct scopes. Three backend responses remain pending. One explicitly started meeting has synthetic speech and silence phases.
- **Actions:** Run for four hours. Exercise 60-minute and 90-minute notices, background transcription, memory recall, local saves, and a second-MOG capture attempt.
- **Expected:** The four-hour ceiling stops capture. Completed segments remain intact. The second capture has no hidden queue. Foreground and background job limits remain visible. Local recall remains within its agreed budget under load.
- **Measurement:** Record job counts, queue high-water marks, memory use, processor use, dropped frames, segment integrity, and recall latency.
- **Evidence:** Soak metrics, segment inventory, arbitration log, and environment record.
- **Method:** Automated platform soak. **Status: NOT RUN.**

## 14. D-01 through D-14 traceability

| Decision | Architecture requirements | Acceptance coverage | Diagram coverage |
|---|---|---|---|
| D-01 Tauri and native core | MB-A-001, STT-A-003, NFR-A-001, NFR-A-002 | AT-ARC-001, AT-ARC-017, AT-ARC-018 | System native boundary; platform gates |
| D-02 Available, not always recording | MB-A-002, CAP-A-002, CAP-A-004, NFR-A-007 | AT-ARC-010, AT-ARC-011, AT-ARC-017 | Separate capture states and privacy indicator |
| D-03 Context in approved question | RTE-A-001 through RTE-A-003, MEM-A-005 | AT-ARC-002, AT-ARC-003 | Context gate and approved payload sequence |
| D-04 Genie One and no retired beta | GEN-A-001 through GEN-A-003, GEN-A-006 | AT-ARC-006, AT-ARC-007 | Selected Genie adapter and polling sequence |
| D-05 Signal-based meetings | CAP-A-004, NFR-A-007 | AT-ARC-011, AT-ARC-017, AT-ARC-022 | Capture transitions and stop signals |
| D-06 Local memory and Lakebase | MEM-A-001 through MEM-A-005, NFR-A-004, NFR-A-005 | AT-ARC-015, AT-ARC-016, AT-ARC-021 | Local store, outbox, scopes, Lakebase |
| D-07 CLI identity | WS-A-001 through WS-A-003 | AT-ARC-004, AT-ARC-005 | CLI and credential trust boundary |
| D-08 Backend adapters and affinity | RTE-A-003, GEN-A-001, GEN-A-004 through GEN-A-007 | AT-ARC-003, AT-ARC-009 | Selected target and pinned conversation |
| D-09 Explicit routes | RTE-A-001 | AT-ARC-002, AT-ARC-020 | Route branch before network |
| D-10 Raw transcript and cleanup | STT-A-001, STT-A-002, REL-A-006, NFR-A-002, NFR-A-003 | AT-ARC-012, AT-ARC-018 | Raw-versus-clean sequence and late-result fence |
| D-11 Multiple MOGs | MB-A-002, MB-A-003, CAP-A-001, CAP-A-003, NFR-A-009 | AT-ARC-010, AT-ARC-022 | Broker ownership and resource epochs |
| D-12 Offline and recovery | GEN-A-006, GEN-A-009, REL-A-001 through REL-A-006, NFR-A-008 | AT-ARC-007, AT-ARC-008, AT-ARC-012 through AT-ARC-016 | Delivery phases and recovery branches |
| D-13 Trusted response blurb | GEN-A-008, GEN-A-009, NFR-A-006 | AT-ARC-008, AT-ARC-019, AT-ARC-020 | Normalized response and safe UI boundary |
| D-14 Retention and deletion | MEM-A-004 | AT-ARC-013, AT-ARC-021 | Tombstone, deletion outbox, retention gate |

## 15. Dependencies and interface consumers

| Contract | Accountable owner | Consumers | Required reviewers or dependencies |
|---|---|---|---|
| Native command API | Architecture | All UI controls | Security threat model; UX state messages |
| `SttProvider` | Voice/desktop | STT orchestrator | Architecture; Platform model verification |
| `CleanupProvider` | Voice/desktop | Cleanup orchestrator | Architecture; transcript-fidelity tests |
| `BackendAdapter` | Architecture | MOG foreground jobs | Platform endpoint matrix; Security host binding; UX capabilities |
| `AuthProvider` | Platform | Backend, model, and Lakebase credential brokers | Architecture; Security CLI and token controls |
| `MemoryStore` | Memory | Context gate, local history, sync service | Architecture; Security encryption and deletion |
| `CaptureSession` | Voice/desktop | Capture broker, STT orchestrator, privacy indicator | Architecture; UX; Security |
| Context-envelope builder | Architecture | Router and approval UI | Memory provenance; Security; UX exact preview |
| Response projector | Architecture and UX | Blurb and full-response UI | Security sanitization; adapter fixtures |
| Recovery journals | Architecture | Capture, STT, backend, memory, deletion | Memory transaction design; Security storage controls |
| Nonfunctional measurement | Architecture | Release approval | All teams supply fixtures and reference environments |

### Integration inputs required from other teams

- Team P must supply verified Genie endpoints, CLI token compatibility, supported profile detection, custom-agent schemas, and source status.
- Team V must supply the capture-resource model, segment format, audio journal details, model configuration, and physical spike results.
- Team M must supply the encryption format, memory schema, conflict resolution, Lakebase schema, and credential-rotation implementation contract. The Lakebase OAuth contract must use direct endpoints and an application-managed pool as verified by Team P.
- Team S must approve token redaction, consent evidence, payload audit metadata, local key handling, and deletion proof.
- Team U must map every MOG and capture state to visible and accessible presentation without merging their authority.
- The diagram specialist must resolve every finding in Sections 12.1 and 12.3 and map all four views to Section 12.2.

## 16. Evidence register

These are primary sources identified in the supplied research.
Team A did not independently re-verify them in this task.
`Pending` means Team P or the named owner must confirm the claim before the related release gate passes.

| Evidence | Source and access date | Architecture use | Status |
|---|---|---|---|
| Genie One MCP endpoint, tools, transport, scopes, grants, and poll contract | [Genie MCP documentation](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), accessed 2026-11-14; Team P claims P-C-001 through P-C-016 | GEN-A-002, GEN-A-003, WS-A-001 | Primary-source verification complete; live Integration remains |
| Genie space REST conversation and polling contract | [Genie conversation API](https://docs.databricks.com/aws/en/genie/conversation-api), accessed 2026-11-14 in supplied research | Explicit REST adapter | Pending Team P |
| Custom Databricks Apps invocation, idempotency, streaming, recovery, and authentication | [Query agents](https://docs.databricks.com/aws/en/generative-ai/agent-framework/query-agent), accessed 2026-11-14 in supplied research | GEN-A-007, retry policy | Pending Team P |
| OAuth U2M and CLI-to-Genie behavior | [OAuth U2M documentation](https://docs.databricks.com/aws/en/dev-tools/auth/oauth-u2m) and Team P claim P-C-027, accessed 2026-11-14 | WS-A-001 through WS-A-003 | Primary-source verification complete; live supported-environment test remains |
| CLI profile-routing reference behavior | [antigravity-bricks-plugin](https://github.com/datasciencemonkey/antigravity-bricks-plugin), supplied commit `a31fd2cd95645947baf9234c4158332db9031eec`, accessed 2026-11-14 | Reference only, not normative API evidence | Platform comparison required |
| Local STT feasibility and latency | No physical reference result supplied | STT-A-003, NFR-A-002 | Open release gate |
| Tauri hotkey, focus, overlay, permission, and audio feasibility | No physical reference result supplied | MB-A-001, CAP-A-001 through CAP-A-004 | Open release gate |

## 17. Open issues and integration risks

### 17.1 Blocking release issues

| ID | Issue | Required resolution |
|---|---|---|
| B-01 | Primary evidence supports CLI-issued tokens for Genie One MCP, but this environment did not execute the supported-workspace test. | Run Team P test P-AT-012 in each supported cloud and workspace configuration. A failed supported-environment test reopens D-07 through the director. |
| B-02 | Physical Tauri and audio feasibility is unverified here. | Team V must execute G2 on `REF-MAC` and `REF-WIN`. No design document can substitute for this result. |
| B-03 | Exact hosted STT, cleanup model, and local STT performance are not established by this architecture. | Team V and Team P must name supported configurations and benchmark them. Until then, hosted STT is configured capability, not an assumed managed service. |
| B-04 | A generic custom-agent schema cannot cover all Apps and Serving agents. | Enable a custom connector only after its exact invocation, auth, response, and recovery schema passes the adapter suite. |
| B-05 | Capability discovery must not create a remote conversation merely to test a connector. | Team P must identify read-only protocol discovery and static configuration for each enabled adapter. |
| B-06 | Semantic recall depends on the Memory team's local embedding design and reference hardware. | Team M must validate NFR-A-005 or supply a replacement measured budget. |
| B-07 | The state diagram proposes same-MOG microphone sharing during a meeting, while the architecture baseline rejects a second capture session. | Director, Voice, UX, and Security must approve one rule and its consumer contract before G6. No user question is required. |

### 17.2 User-confirmation issues

No existential user-confirmation issue is open.
The naming correction is resolved: Motherbricks is the project and desktop application.
Desktop-first, no ambient recording, local operation without Lakebase, and pinned conversations remain the baseline.

### 17.3 Later-release issues

- Beta Genie One and Genie Agent SSE adapters remain feature-detected later work.
- Per-space Genie MCP remains preview-dependent later work.
- PAT and service-principal profiles remain excluded unless Security approves a later requirement.
- Mobile and web companions require a separate capability statement for global hotkeys, audio capture, and insertion.
- Shared microphone fan-out across MOGs requires a new consent and arbitration design.

### 17.4 Key integration risks

| Risk | Impact | Architecture mitigation |
|---|---|---|
| CLI token and Genie One scope mismatch | Default backend cannot authenticate. | Keep D-07 reopen condition explicit; do not broaden scopes silently. |
| Duplicate question after timeout or crash | Repeated analysis, cost, and user distrust. | Persist delivery phase; no post-send automatic retry without verified idempotency. |
| Silent rerouting during connector failure | Data crosses the wrong workspace or agent. | Bind target and adapter to the job; require a new approved question for fallback. |
| Combined MOG and capture states | UI can hide recording or lose pending answers. | Keep independent persisted state machines and derive presentation separately. |
| WebView or renderer token exposure | Credential compromise. | Native-only CLI and transport boundary with automated token-sentinel scans. |
| Device races during multi-MOG operation | Hidden or duplicated recording. | Central leases, visible conflicts, and resource-epoch fencing. |
| Network-dependent memory | Recall misses the interactive budget offline. | Transactional encrypted local store and idempotent Lakebase outbox. |
| Late cleanup or stale audio callback | Wrong text insertion or capture-state corruption. | Job-revision and resource-epoch fences. |
| Custom-agent contract assumptions | Malformed requests and accidental duplicate submissions. | Separate adapters, tri-state capabilities, and connector-specific contract suites. |
| Unrun physical platform tests | Unsupported release confidence. | Keep every platform result `NOT RUN` until G2 evidence exists. |

## 18. Architecture exit assessment

- The six requested contracts are defined.
- Polling, streaming, cancellation, query results, deep links, recovery, idempotency, and context limits have explicit capability behavior.
- Conversation affinity and explicit target switching are defined.
- Request identity, delivery phases, timeout, cancellation, retry, and uncertain delivery are defined.
- MOG and capture states are separate.
- Multi-MOG arbitration, offline behavior, crash recovery, and measurable NFRs are defined.
- Alternate adapters have no hidden fallback.
- Every D-01 through D-14 decision maps to requirements and tests.
- All four existing SVG files are reviewed; diagram edits remain with the specialist.
- Physical macOS and Windows verification remains an open release gate.
