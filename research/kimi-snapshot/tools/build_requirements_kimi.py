#!/usr/bin/env python3
"""Build the self-contained Motherbricks requirements HTML."""

from __future__ import annotations

import html
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "requirements_kimi.html"


def esc(value: object) -> str:
    return html.escape(str(value), quote=True)


def fmt(value: str) -> str:
    text = esc(value)
    text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
    return re.sub(r"\[(S\d+)\]", r'<a href="#source-\1">[\1]</a>', text)


def reqs(family: str, rows: list[tuple[str, str, str, str, str, str, str]]) -> None:
    for row in rows:
        REQUIREMENTS.append((family, *row))


REQUIREMENTS: list[tuple[str, ...]] = []

reqs("MB", [
    ("MB-001", "Motherbricks shall create a MOG from a guided desktop flow.", "MUST", "MVP", "AT-01", "Demonstration", "A user must not edit a configuration file."),
    ("MB-002", "Motherbricks shall store a MOG name, home profile, backend, capture settings and memory settings.", "MUST", "MVP", "AT-01", "Automated test", "These values define one MOG configuration."),
    ("MB-003", "Motherbricks shall keep secrets out of MOG configuration files.", "MUST", "MVP", "AT-17", "Inspection", "Configuration export must be safe."),
    ("MB-004", "Motherbricks shall start more than one MOG configuration in separate floating windows.", "MUST", "MVP", "AT-01", "Platform test", "The product supports a team of MOGs."),
    ("MB-005", "Motherbricks shall show the active workspace and backend for each MOG.", "MUST", "MVP", "AT-08", "Demonstration", "The target must be visible before submission."),
    ("MB-006", "Motherbricks shall reject a duplicate active global hotkey.", "MUST", "MVP", "AT-01", "Platform test", "A duplicate binding creates ambiguous capture."),
    ("MB-007", "Motherbricks shall provide one action that pauses all MOG capture.", "MUST", "MVP", "AT-13", "Platform test", "A global privacy control is required."),
    ("MB-008", "Motherbricks shall preserve MOG configuration after a sign-in failure.", "MUST", "MVP", "AT-02", "Automated test", "Authentication recovery must not discard user work."),
    ("MB-009", "Motherbricks should export a MOG definition without credentials or content.", "SHOULD", "v1", "AT-17", "Automated test", "A team can share a safe template."),
    ("MB-010", "Motherbricks shall delete local MOG content through an explicit deletion flow.", "MUST", "MVP", "AT-16", "Automated test", "Deletion is part of the MOG lifecycle."),
])

reqs("CAP", [
    ("CAP-001", "The native core shall start push-to-talk only after a registered native key-down event.", "MUST", "MVP", "AT-03", "Platform test", "Web content cannot start capture."),
    ("CAP-002", "The native core shall stop push-to-talk audio acquisition after the matching key-up event.", "MUST", "MVP", "AT-03", "Platform test", "Key release defines the utterance boundary."),
    ("CAP-003", "The native core shall cancel capture after an Escape action or a lost key-release watchdog event.", "MUST", "MVP", "AT-03", "Platform test", "A stuck key must not leave hidden capture."),
    ("CAP-004", "Motherbricks shall show a native listening indicator during push-to-talk.", "MUST", "MVP", "AT-03", "Platform test", "The indicator does not depend on the mascot."),
    ("CAP-005", "Motherbricks shall require a trusted native confirmation before hands-free dictation starts.", "MUST", "v1", "AT-03", "Platform test", "Persistent capture needs explicit authority."),
    ("CAP-006", "Motherbricks shall require a trusted native confirmation before meeting capture starts.", "MUST", "v1", "AT-13", "Platform test", "A calendar or remote instruction is not consent."),
    ("CAP-007", "Motherbricks shall show a persistent native recording indicator during meeting capture.", "MUST", "v1", "AT-13", "Platform test", "Recording must remain visible when the mascot is hidden."),
    ("CAP-008", "Motherbricks shall stop meeting capture if the native recording indicator fails.", "MUST", "v1", "AT-13", "Platform test", "Privacy indication fails closed."),
    ("CAP-009", "The Audio Broker shall grant one microphone lease to one MOG at a time.", "MUST", "MVP", "AT-14", "Platform test", "Two MOGs must not secretly share a microphone."),
    ("CAP-010", "The Audio Broker shall reject a conflicting capture request without interrupting the current owner.", "MUST", "MVP", "AT-14", "Platform test", "A second request cannot steal capture."),
    ("CAP-011", "The Audio Broker shall reject hidden capture queuing.", "MUST", "MVP", "AT-14", "Automated test", "Retry requires a new user action."),
    ("CAP-012", "A meeting owner shall reuse its owned stream for push-to-talk through separate PTT and meeting buffers.", "MUST", "v1", "AT-14", "Platform test", "The v1 baseline opens no second capture session."),
    ("CAP-013", "Motherbricks shall label microphone audio as You when speaker identity is not verified.", "MUST", "v1", "AT-13", "Inspection", "A track label is not a person identity."),
    ("CAP-014", "Motherbricks shall label system audio as Others when speaker identity is not verified.", "SHOULD", "v1", "AT-13", "Inspection", "Separate tracks support meeting review."),
    ("CAP-015", "Motherbricks shall stop new audio acquisition within 250 ms after a manual stop on reference hardware.", "MUST", "MVP", "AT-21", "Platform benchmark", "Finalization can continue with visible progress."),
    ("CAP-016", "Motherbricks shall close crash-safe audio segments during sleep or low-disk interruption.", "MUST", "v1", "AT-15", "Platform test", "Completed audio must survive interruption."),
    ("CAP-017", "Motherbricks shall disable capture after microphone permission denial.", "MUST", "MVP", "AT-03", "Platform test", "Permission denial cannot create a transcript."),
    ("CAP-018", "Motherbricks shall stop capture when the audio device disconnects.", "MUST", "MVP", "AT-15", "Platform test", "Device loss needs a recoverable terminal state."),
    ("CAP-019", "Motherbricks shall show an independent persistent native indicator during hands-free capture.", "MUST", "v1", "AT-03", "Platform test", "Hands-free capture fails closed if the indicator cannot remain visible."),
    ("CAP-020", "The native core shall stop push-to-talk after 120 seconds even if key-up is missing.", "MUST", "MVP", "AT-03", "Platform test", "The safety stop bounds a lost-release failure."),
    ("CAP-021", "A call-signal provider shall report microphone state as Active, Released or Unknown.", "MUST", "v1", "AT-13", "Contract test", "Active cancels the release grace period and Unknown never stops capture."),
    ("CAP-022", "System-audio capture shall use process-scoped sources when the operating system supports them.", "MUST", "v1", "AT-13", "Platform test", "Whole-system audio is not a silent fallback."),
    ("CAP-023", "Motherbricks shall mark each push-to-talk interval in the meeting timeline.", "MUST", "v1", "AT-14", "Timeline inspection", "The two consumers share one stream without hiding the interaction."),
])

reqs("STT", [
    ("STT-001", "Motherbricks shall provide a local STT provider for offline dictation.", "MUST", "MVP", "AT-04", "Platform benchmark", "Local Parakeet with sherpa-onnx is the recommended candidate."),
    ("STT-002", "Motherbricks shall record the STT model identity and runtime version for each transcript.", "MUST", "MVP", "AT-04", "Inspection", "Quality results need a reproducible model version."),
    ("STT-003", "Motherbricks shall expose hosted ASR only after endpoint capability validation.", "MUST", "v1", "AT-04", "Primary-source verification", "Hosted Whisper support is preview and configuration-dependent."),
    ("STT-004", "Motherbricks shall route hosted audio through the native hosted-STT egress gate.", "MUST", "v1", "AT-17", "Security test", "Raw audio must use the selected workspace and policy."),
    ("STT-005", "Motherbricks shall preserve the raw transcript separately from the cleaned transcript.", "MUST", "MVP", "AT-05", "Automated test", "Cleanup must not destroy the source."),
    ("STT-006", "Motherbricks shall return the raw transcript when cleanup fails.", "MUST", "MVP", "AT-04", "Automated test", "A cloud failure must not block dictation."),
    ("STT-007", "Motherbricks shall return the raw transcript when cleanup exceeds two seconds.", "MUST", "MVP", "AT-21", "Automated test", "The cleanup path has a strict deadline."),
    ("STT-008", "The cleanup provider shall preserve numbers, units, names, code, SQL and negation.", "MUST", "MVP", "AT-05", "Corpus test", "A data question must retain its meaning."),
    ("STT-009", "The cleanup provider shall not select an intent, backend or workspace.", "MUST", "MVP", "AT-06", "Security test", "A model cannot grant routing authority."),
    ("STT-010", "Motherbricks shall let the user restore the raw transcript.", "MUST", "MVP", "AT-05", "Demonstration", "The user can correct a cleanup error."),
    ("STT-011", "Motherbricks shall discard a late cleanup result after text insertion.", "MUST", "MVP", "AT-05", "Automated test", "Late output must not overwrite user work."),
    ("STT-012", "Each meeting transcript segment shall include a segment ID, track ID, start time, end time and quality status.", "MUST", "v1", "AT-13", "Schema test", "Chunks need stable ordering and revision semantics."),
    ("STT-013", "Motherbricks should protect configured table names, column names and product terms during cleanup.", "SHOULD", "v1", "AT-05", "Corpus test", "A domain dictionary improves Genie questions."),
])

reqs("RTE", [
    ("RTE-001", "The router shall support Dictate, Copy, Ask, Save context, Recall and Disambiguate outcomes.", "MUST", "MVP", "AT-06", "Automated test", "Not every utterance is a Genie question."),
    ("RTE-002", "The router shall keep dictation local when the user selects dictation mode.", "MUST", "MVP", "AT-06", "Network test", "A question mark does not imply backend submission."),
    ("RTE-003", "The router shall make no network request for an ambiguous utterance.", "MUST", "MVP", "AT-06", "Network test", "Ambiguity requires user disambiguation."),
    ("RTE-004", "Motherbricks shall require an explicit Ask action before a backend question.", "MUST", "MVP", "AT-08", "Demonstration", "The backend call is user-directed."),
    ("RTE-005", "Motherbricks shall keep context sharing disabled by default.", "MUST", "MVP", "AT-08", "Inspection", "Memory storage is not transmission approval."),
    ("RTE-006", "Motherbricks shall show the exact outbound question and context payload before a context-bearing submission.", "MUST", "MVP", "AT-08", "Demonstration", "Genie MCP has no separate context-upload operation."),
    ("RTE-007", "Motherbricks shall show the target workspace and backend with the outbound payload preview.", "MUST", "MVP", "AT-08", "Demonstration", "Content and target need one approval."),
    ("RTE-008", "Each context envelope item shall include its source, timestamp and truncation status.", "MUST", "MVP", "AT-08", "Schema test", "Context needs provenance and bounds."),
    ("RTE-009", "Motherbricks shall enforce a configured byte limit on the context envelope.", "MUST", "MVP", "AT-08", "Automated test", "Context cannot grow without limit."),
    ("RTE-010", "Motherbricks shall treat recalled text as untrusted data.", "MUST", "MVP", "AT-17", "Security test", "Memory cannot issue commands."),
    ("RTE-011", "Motherbricks should offer a one-click Ask suggestion for a clear meeting question.", "SHOULD", "v1", "AT-13", "Usability test", "Automatic meeting submission is not the default."),
    ("RTE-012", "Each context approval shall bind one immutable payload, target, conversation and submission attempt.", "MUST", "MVP", "AT-08", "Security test", "Any change invalidates the approval."),
])

reqs("GEN", [
    ("GEN-001", "The default backend shall use Genie One MCP at `/ai-gateway/mcp-services/system.ai.genie_one_mcp`.", "MUST", "MVP", "AT-07", "Primary-source verification", "Genie One MCP is the GA path [S1]."),
    ("GEN-002", "The native core shall use streamable HTTP for Genie One MCP.", "MUST", "MVP", "AT-07", "Integration test", "The MCP transport is not local stdio for the remote service."),
    ("GEN-003", "Motherbricks shall discover the MCP tool schemas before the first question.", "MUST", "MVP", "AT-07", "Integration test", "Optional tools and schemas can differ."),
    ("GEN-004", "The Genie One adapter shall support `genie_ask` and `genie_poll_response`.", "MUST", "MVP", "AT-07", "Integration test", "These operations define ask and poll."),
    ("GEN-005", "The Genie One adapter shall support `genie_get_query_result` for an expanded result.", "MUST", "MVP", "AT-09", "Integration test", "The blurb does not require all rows."),
    ("GEN-006", "The Genie One adapter shall support `genie_cancel_response` when a response is active.", "MUST", "MVP", "AT-10", "Integration test", "A cancellation request is not proof of cancellation."),
    ("GEN-007", "Motherbricks shall persist the conversation ID and response ID before polling.", "MUST", "MVP", "AT-15", "Recovery test", "The app can resume after restart."),
    ("GEN-008", "Motherbricks shall allow only one active poll call for a response.", "MUST", "MVP", "AT-07", "Concurrency test", "The service requires serial polling [S1]."),
    ("GEN-009", "Motherbricks shall show progress from polling without calling it streaming.", "MUST", "MVP", "AT-09", "Demonstration", "Polling and streaming are different capabilities."),
    ("GEN-010", "Motherbricks shall support completed, incomplete and failed poll outcomes.", "MUST", "MVP", "AT-09", "Integration test", "A non-success outcome cannot become a false answer."),
    ("GEN-011", "Motherbricks shall preserve response numbers, units, caveats and truncation in the blurb.", "MUST", "MVP", "AT-09", "Content test", "The blurb is a projection, not a new answer."),
    ("GEN-012", "Motherbricks shall show the supplied Databricks deep link with the blurb.", "MUST", "MVP", "AT-09", "Demonstration", "The user can inspect the full result."),
    ("GEN-013", "Motherbricks shall not repeat `genie_ask` automatically after uncertain delivery.", "MUST", "MVP", "AT-15", "Fault-injection test", "Retry can duplicate a data question."),
    ("GEN-014", "Motherbricks shall reject the deprecated `/api/2.0/mcp/genie` endpoint.", "MUST", "MVP", "AT-07", "Static test", "That beta endpoint reaches end of life on 2026-11-30 [S1]."),
    ("GEN-015", "The Genie space REST adapter shall use an explicitly selected space ID.", "MUST", "v1", "AT-11", "Integration test", "Fallback cannot change the data target silently."),
    ("GEN-016", "The Genie space REST adapter shall report `remote_cancel=false` until a chat-mode cancellation operation is verified.", "MUST", "v1", "AT-11", "Capability test", "Agent-mode cancellation cannot be reused for chat mode [S3]."),
    ("GEN-017", "The per-space Genie MCP adapter shall remain behind a disabled preview flag.", "MUST", "v1", "AT-11", "Inspection", "It has no conversation history and bypasses Unity Gateway governance [S2]."),
    ("GEN-018", "Each custom-agent adapter shall declare its invocation, authentication, streaming and cancellation capabilities.", "MUST", "v2", "AT-11", "Contract test", "Apps and Model Serving do not share one contract."),
    ("GEN-019", "A backend switch shall start a new conversation.", "MUST", "MVP", "AT-12", "Automated test", "Conversation IDs are backend-specific."),
    ("GEN-020", "The Response Projector shall create the blurb without an additional generative model call by default.", "SHOULD", "MVP", "AT-09", "Inspection", "Deterministic projection avoids added cost and factual drift."),
    ("GEN-021", "Each Genie agent-mode operation shall pass a current launch-status check before enablement.", "MUST", "v2", "AT-11", "Primary-source verification", "Launch status is verified per operation rather than inferred for the whole API."),
])

reqs("WS", [
    ("WS-001", "Motherbricks shall discover Databricks CLI profiles through supported CLI commands.", "MUST", "MVP", "AT-02", "Integration test", "Profile files are not a stable public schema."),
    ("WS-002", "Motherbricks shall support OAuth U2M profiles with `auth_type=databricks-cli` in the MVP.", "MUST", "MVP", "AT-02", "Integration test", "The reference plugin uses this profile type."),
    ("WS-003", "Motherbricks shall reject PAT and service-principal profiles in the MVP.", "MUST", "MVP", "AT-02", "Integration test", "The first release has one identity contract."),
    ("WS-004", "Motherbricks shall invoke the CLI directly with an argument array.", "MUST", "MVP", "AT-17", "Security test", "A shell command string permits injection."),
    ("WS-005", "Motherbricks shall obtain access tokens with `databricks auth token` for the selected profile.", "MUST", "MVP", "AT-07", "Integration test", "CLI U2M is documented with Genie One MCP [S4]."),
    ("WS-006", "The native core shall keep access tokens in volatile process memory.", "MUST", "MVP", "AT-17", "Security test", "Tokens must not enter the webview or content store."),
    ("WS-007", "The native core shall bind each workspace token lease to its canonical workspace origin.", "MUST", "MVP", "AT-17", "Security test", "A workspace token cannot be forwarded to a different host."),
    ("WS-008", "Motherbricks shall start CLI login only after a user action.", "MUST", "MVP", "AT-02", "Demonstration", "Hidden recovery cannot rewrite a profile."),
    ("WS-009", "Motherbricks shall refuse a CLI configuration that uses plaintext token fallback for its own flows.", "MUST", "MVP", "AT-17", "Platform inspection", "Secure CLI storage is a release gate [S5]."),
    ("WS-010", "Motherbricks shall pin a meeting to its starting workspace.", "MUST", "MVP", "AT-12", "Automated test", "A meeting must not split across workspaces."),
    ("WS-011", "Motherbricks shall pin a conversation to its starting profile, workspace, backend and backend ID.", "MUST", "MVP", "AT-12", "Automated test", "Polling and cancellation stay on the origin."),
    ("WS-012", "A one-off workspace selection shall leave the MOG home profile unchanged.", "MUST", "MVP", "AT-12", "Demonstration", "This matches the reference profile-routing skill."),
    ("WS-013", "Motherbricks shall ask the user when a workspace name matches zero or multiple profiles.", "MUST", "MVP", "AT-12", "Usability test", "The app must not guess a workspace."),
    ("WS-014", "Motherbricks should support explicit splitting of a cross-workspace question.", "SHOULD", "v2", "AT-12", "Integration test", "Each answer part needs workspace attribution."),
    ("WS-015", "Motherbricks shall resolve the workspace ID and add `?o=<workspace-id>` when the Genie One route requires it.", "MUST", "MVP", "AT-07", "Integration test", "Use supported profile data or the same-host `x-databricks-org-id` response header [S1]."),
    ("WS-016", "Native transport shall reject an authenticated redirect to a different origin without a new adapter-specific credential lease.", "MUST", "MVP", "AT-17", "Security test", "A later Apps adapter uses its own approved origin lease."),
])

reqs("MEM", [
    ("MEM-001", "Motherbricks shall use an encrypted local store for interactive recall.", "MUST", "MVP", "AT-06", "Automated test", "Interactive recall cannot wait for Lakebase."),
    ("MEM-002", "Motherbricks shall use Lakebase for durable synchronization when configured.", "MUST", "MVP optional", "AT-18", "Integration test", "Lakebase is optional for local dictation and Genie."),
    ("MEM-003", "Each memory item shall include a stable ID, user scope, MOG scope and workspace scope.", "MUST", "MVP", "AT-06", "Schema test", "Scope prevents accidental recall across tenants."),
    ("MEM-004", "Each memory item shall include provenance, creation time, retention class and sensitivity.", "MUST", "MVP", "AT-16", "Schema test", "Memory needs lifecycle and audit metadata."),
    ("MEM-005", "Motherbricks shall separate memory-save approval from outbound-context approval.", "MUST", "MVP", "AT-08", "Demonstration", "Saving an item does not approve transmission."),
    ("MEM-006", "Motherbricks shall separate memory-save approval from Lakebase synchronization approval.", "MUST", "MVP optional", "AT-18", "Demonstration", "Local save and remote durable storage are different trusts."),
    ("MEM-007", "Motherbricks shall exclude raw audio from durable memory by default.", "MUST", "MVP", "AT-19", "Inspection", "Raw audio is transient unless retention is enabled."),
    ("MEM-008", "Motherbricks shall maintain a local outbound synchronization queue.", "MUST", "MVP optional", "AT-18", "Recovery test", "Lakebase unavailability must not lose approved changes."),
    ("MEM-009", "Each memory mutation shall use a stable idempotency key.", "MUST", "MVP optional", "AT-18", "Fault-injection test", "Sync retry must not duplicate memories."),
    ("MEM-010", "A memory deletion shall create a tombstone.", "MUST", "MVP", "AT-16", "Automated test", "Deletion must survive offline synchronization."),
    ("MEM-011", "Motherbricks shall propagate a tombstone after Lakebase reconnection.", "MUST", "MVP optional", "AT-16", "Integration test", "A deleted item must not reappear."),
    ("MEM-012", "Motherbricks shall prevent cross-MOG recall without an explicit copy action.", "MUST", "MVP", "AT-06", "Security test", "MOG namespaces are isolated."),
    ("MEM-013", "Motherbricks shall prevent cross-workspace recall without an explicit export action.", "MUST", "MVP", "AT-06", "Security test", "Workspace switching is not memory sharing."),
    ("MEM-014", "Lakebase synchronization shall use a direct endpoint with an application-managed connection pool for OAuth.", "MUST", "MVP optional", "AT-18", "Integration test", "The built-in PgBouncer pool does not support OAuth roles [S6]."),
    ("MEM-015", "Each new Lakebase physical connection shall use a fresh database credential.", "MUST", "MVP optional", "AT-18", "Integration test", "OAuth database credentials expire after one hour [S6]."),
    ("MEM-016", "Lakebase connections shall use `sslmode=verify-full` with the system root store.", "MUST", "MVP optional", "AT-18", "Security test", "Encryption without peer verification is insufficient [S7]."),
    ("MEM-017", "Lakebase access shall enforce user, workspace and MOG isolation on the server.", "MUST", "MVP optional", "AT-18", "Security test", "Client-side filters are not an authorization boundary."),
    ("MEM-018", "Motherbricks shall use a Motherbricks-owned Lakebase schema for the durable MVP adapter.", "MUST", "MVP optional", "AT-18", "Schema inspection", "Managed Agent Memory is Beta and lacks the required revision semantics [S8]."),
    ("MEM-019", "Motherbricks shall not enable Lakebase Search automatically.", "MUST", "MVP optional", "AT-18", "Inspection", "Enablement restarts computes and is irreversible [S9]."),
    ("MEM-020", "Motherbricks should keep a local semantic index for offline semantic recall.", "SHOULD", "v1", "AT-21", "Benchmark", "Remote vector search is not on the local critical path."),
    ("MEM-021", "Each derived embedding shall reference its model identity and source content hash.", "MUST", "MVP optional", "AT-18", "Schema test", "Embedding migration needs reproducibility."),
    ("MEM-022", "Motherbricks shall disclose remote deletion limits for Lakebase backups and Genie conversations.", "MUST", "MVP", "AT-16", "Demonstration", "Local deletion is not proof of remote erasure."),
])

reqs("UX", [
    ("UX-001", "The floating mascot window shall not steal focus from the active application during push-to-talk.", "MUST", "MVP", "AT-03", "Platform test", "Dictation needs the original target to retain focus."),
    ("UX-002", "Motherbricks shall provide a separate keyboard-capable response panel.", "MUST", "MVP", "AT-01", "Platform test", "The mascot and the response editor have different focus needs."),
    ("UX-003", "Each MOG state shall have a text label in addition to animation.", "MUST", "MVP", "AT-20", "Accessibility test", "Color or motion cannot be the only status signal."),
    ("UX-004", "The MOG shall distinguish idle, listening, recording, transcribing, cleaning, waiting, answering, success, error, paused and sleeping presentations.", "MUST", "MVP", "AT-20", "Accessibility test", "The user needs the actual pipeline state."),
    ("UX-005", "The blurb shall show a pending state while the backend works.", "MUST", "MVP", "AT-09", "Demonstration", "Genie can take much longer than dictation."),
    ("UX-006", "The blurb shall identify incomplete, failed, cancelled and truncated outcomes.", "MUST", "MVP", "AT-09", "Content test", "A partial result needs an explicit status."),
    ("UX-007", "The response panel shall provide full response, available query detail and safe external-link actions.", "MUST", "MVP", "AT-09", "Demonstration", "The blurb is not the full investigation surface."),
    ("UX-008", "The response panel shall provide a cancellation action for an active response.", "MUST", "MVP", "AT-10", "Demonstration", "The UI must not claim completion after a cancellation request."),
    ("UX-009", "Motherbricks shall display elapsed recording time during a meeting.", "MUST", "v1", "AT-13", "Demonstration", "The 60-minute mark is informational."),
    ("UX-010", "Motherbricks shall provide visible and keyboard-operable stop controls for every capture mode.", "MUST", "MVP", "AT-20", "Accessibility test", "Stop cannot depend on a pointer or mascot."),
    ("UX-011", "Motherbricks shall support reduced motion.", "MUST", "MVP", "AT-20", "Accessibility test", "The mascot must respect user preferences."),
    ("UX-012", "Motherbricks shall expose critical status changes to a screen reader without announcing every poll.", "MUST", "MVP", "AT-20", "Accessibility test", "Progress must be useful and not noisy."),
    ("UX-013", "Motherbricks shall provide keyboard access to every critical flow.", "MUST", "MVP", "AT-20", "Accessibility test", "Core operation cannot require a pointer."),
    ("UX-014", "Motherbricks shall show capture ownership when a MOG request conflicts.", "MUST", "MVP", "AT-14", "Demonstration", "The user can identify the active owner."),
    ("UX-015", "Motherbricks shall let the user correct text before insertion or submission.", "SHOULD", "MVP", "AT-05", "Usability test", "Correction reduces ASR and cleanup errors."),
    ("UX-016", "Mobile and web companions shall declare reduced capture and insertion capabilities.", "MUST", "v2", "AT-20", "Inspection", "Desktop global hooks and system audio do not port unchanged."),
])

reqs("SEC", [
    ("SEC-001", "The native core shall be the only authority for capture, tokens, network transport and deletion.", "MUST", "MVP", "AT-17", "Security test", "The webview is not a security boundary."),
    ("SEC-002", "The native core shall allow only declared IPC commands with validated schemas.", "MUST", "MVP", "AT-17", "Security test", "A compromised webview must not invoke arbitrary native functions."),
    ("SEC-003", "Motherbricks shall not expose a generic shell or generic network command through IPC.", "MUST", "MVP", "AT-17", "Security test", "This limits confused-deputy impact."),
    ("SEC-004", "Motherbricks shall treat transcripts, memories and backend responses as untrusted content.", "MUST", "MVP", "AT-17", "Security test", "Indirect prompt injection cannot change authority."),
    ("SEC-005", "Motherbricks shall render backend markup through a strict allowlist sanitizer.", "MUST", "MVP", "AT-17", "Security test", "Backend content can contain active markup."),
    ("SEC-006", "Motherbricks shall open only validated HTTPS links in the system browser.", "MUST", "MVP", "AT-17", "Security test", "Links must not execute local commands."),
    ("SEC-007", "Motherbricks shall use a restrictive Content Security Policy without remote subresources in the privileged UI.", "MUST", "MVP", "AT-17", "Inspection", "Remote content increases webview risk."),
    ("SEC-008", "Motherbricks shall keep tokens, Lakebase credentials and encryption keys out of logs and crash reports.", "MUST", "MVP", "AT-17", "Log inspection", "Secrets need automatic redaction."),
    ("SEC-009", "Motherbricks shall exclude audio, transcripts, context and query rows from ordinary diagnostics by default.", "MUST", "MVP", "AT-17", "Log inspection", "Content is sensitive even without credentials."),
    ("SEC-010", "Motherbricks shall protect local content with per-MOG encryption keys in OS-native secure storage.", "MUST", "MVP", "AT-17", "Platform test", "A MOG deletion key supports cryptographic erasure."),
    ("SEC-011", "Motherbricks shall fail closed when the required OS secure-storage service is unavailable.", "MUST", "MVP", "AT-17", "Platform test", "Plaintext fallback is not acceptable."),
    ("SEC-012", "Motherbricks shall require participant-notice UX for managed meeting deployment.", "MUST", "v1", "AT-13", "Legal and UX inspection", "Recording law depends on jurisdiction and use case."),
    ("SEC-013", "Motherbricks shall not let organization policy start recording or approve context for a user.", "MUST", "MVP", "AT-17", "Policy test", "Administrative policy cannot manufacture consent."),
    ("SEC-014", "Motherbricks shall sanitize the CLI process environment and use a trusted executable path.", "MUST", "MVP", "AT-17", "Security test", "The reference plugin's unchanged environment must not be copied."),
    ("SEC-015", "Motherbricks shall use parameterized SQL for Lakebase operations.", "MUST", "MVP optional", "AT-18", "Security test", "Memory text cannot become SQL syntax."),
    ("SEC-016", "A custom Databricks Apps adapter shall use an adapter-specific origin-bound credential lease.", "MUST", "v2", "AT-17", "Security test", "An Apps host differs from the workspace MCP host."),
    ("SEC-017", "Motherbricks shall provide an auditable record of context target, approval ID and payload digest.", "MUST", "MVP", "AT-08", "Audit inspection", "Audit metadata need not contain the content itself."),
    ("SEC-018", "Motherbricks shall require deployment counsel to review the meeting consent flow.", "MUST", "v1", "AT-22", "Legal review", "The product makes no universal two-party-consent claim."),
])

reqs("REL", [
    ("REL-001", "Local dictation shall continue offline when the local STT model is available.", "MUST", "MVP", "AT-04", "Offline test", "Offline operation is a core requirement."),
    ("REL-002", "Local recall shall continue when Lakebase is unavailable.", "MUST", "MVP", "AT-06", "Offline test", "Durable sync is not the interactive read path."),
    ("REL-003", "Motherbricks shall mark an unsent question as not sent after a pre-submission failure.", "MUST", "MVP", "AT-15", "Fault-injection test", "The user needs an accurate delivery state."),
    ("REL-004", "Motherbricks shall mark a post-submission transport failure as uncertain.", "MUST", "MVP", "AT-15", "Fault-injection test", "The remote service may have accepted the question."),
    ("REL-005", "Motherbricks shall not auto-submit an unsent question after reconnection.", "MUST", "MVP", "AT-15", "Offline test", "Delayed submission needs new approval."),
    ("REL-006", "Motherbricks shall retry safe memory synchronization with bounded backoff.", "MUST", "MVP optional", "AT-18", "Fault-injection test", "Idempotency keys make safe retry possible."),
    ("REL-007", "Motherbricks shall preserve recovery IDs for an active backend response across restart.", "MUST", "MVP", "AT-15", "Recovery test", "The original response can be reconciled."),
    ("REL-008", "Motherbricks shall preserve completed meeting segments after a crash.", "MUST", "v1", "AT-15", "Crash test", "The journal is crash-safe."),
    ("REL-009", "Motherbricks shall discard stale async callbacks after cancellation, route change or MOG deletion.", "MUST", "MVP", "AT-15", "Concurrency test", "A stale result cannot mutate new state."),
    ("REL-010", "Motherbricks shall bound the number of active Genie responses and local STT workers.", "MUST", "v1", "AT-21", "Soak test", "One MOG must not starve other MOGs."),
])

reqs("NFR", [
    ("NFR-001", "The listening indicator shall appear within 100 ms at p95 after a valid hotkey event on reference hardware.", "MUST", "MVP", "AT-21", "Platform benchmark", "Input feedback must feel immediate."),
    ("NFR-002", "Warm local STT shall finish within 1.5 seconds at p95 after key-up for an utterance of 15 seconds or less on reference hardware.", "MUST", "MVP", "AT-21", "Platform benchmark", "The target requires a named hardware and model matrix."),
    ("NFR-003", "Local structured recall shall finish within 50 ms at p95 for 10,000 local items on reference hardware.", "MUST", "MVP", "AT-21", "Benchmark", "No network time is included."),
    ("NFR-004", "Local semantic recall shall finish within 250 ms at p95 for 10,000 local items on reference hardware.", "SHOULD", "v1", "AT-21", "Benchmark", "This target needs validation with the selected local embedder."),
    ("NFR-005", "Motherbricks shall show backend submission feedback within 300 ms after the transport result is known.", "MUST", "MVP", "AT-21", "Instrumentation test", "Genie answer time itself is not fixed."),
    ("NFR-006", "Motherbricks shall record p50 and p95 latency for hotkey, STT, cleanup, recall and response projection.", "MUST", "MVP", "AT-21", "Telemetry inspection", "Averages hide interactive tail latency."),
    ("NFR-007", "Motherbricks shall enforce configurable disk and memory limits for meeting journals and local cache.", "MUST", "v1", "AT-21", "Soak test", "Long capture cannot exhaust the device."),
    ("NFR-008", "Motherbricks shall pass a four-hour meeting soak test without unbounded memory growth.", "MUST", "v1", "AT-21", "Soak test", "The hard recording ceiling needs a matching resource test."),
])

reqs("OPS", [
    ("OPS-001", "The MVP release shall support named macOS and Windows versions with a documented permission matrix.", "MUST", "MVP", "AT-22", "Release inspection", "Linux Wayland is not an implied MVP target."),
    ("OPS-002", "The first hotkey spike shall test the Tauri global-shortcut plugin before a custom native hook is selected.", "MUST", "MVP", "AT-22", "Platform test", "The plugin documents Pressed and Released states [S10]."),
    ("OPS-003", "The overlay spike shall verify non-activating behavior and focus restoration on each MVP operating system.", "MUST", "MVP", "AT-22", "Platform test", "Window API availability is not an application behavior proof."),
    ("OPS-004", "An administrator shall grant the documented Genie One MCP permissions before workspace use.", "MUST", "MVP", "AT-22", "Admin inspection", "Required grants include MCP EXECUTE and system schema use [S1]."),
    ("OPS-005", "Motherbricks shall preflight the selected warehouse and data permissions without changing them.", "MUST", "MVP", "AT-22", "Integration test", "Genie still uses the signed-in user's data permissions."),
    ("OPS-006", "An administrator shall approve Lakebase schema migration and optional Search enablement.", "MUST", "MVP optional", "AT-22", "Admin inspection", "Motherbricks must not make irreversible service changes."),
    ("OPS-007", "Motherbricks shall sign desktop installers and update packages.", "MUST", "MVP", "AT-22", "Release inspection", "Distribution needs publisher authenticity."),
    ("OPS-008", "Motherbricks shall verify update signatures before installation.", "MUST", "MVP", "AT-22", "Security test", "An update channel is privileged code delivery."),
    ("OPS-009", "Motherbricks shall provide organization-managed configuration with restrictive privacy precedence.", "SHOULD", "v1", "AT-22", "Policy test", "Managed policy cannot weaken mandatory privacy invariants."),
    ("OPS-010", "Motherbricks shall disclose the CLI token's default broad scope in the security assessment.", "MUST", "MVP", "AT-22", "Security inspection", "CLI U2M avoids a custom app but is not a least-privilege token claim [S4]."),
    ("OPS-011", "Motherbricks shall provide a redacted support-bundle preview before export.", "SHOULD", "v1", "AT-17", "Demonstration", "Support data can contain sensitive metadata."),
])


FAMILY_INFO = {
    "MB": ("Motherbricks and MOG management", "Create, configure, run and delete MOGs."),
    "CAP": ("Hotkeys, audio and meetings", "Native capture authority, resource ownership and safe stop behavior."),
    "STT": ("Transcription and cleanup", "Local-first speech recognition and bounded transcript cleanup."),
    "RTE": ("Routing and context", "Explicit user intent and approved outbound context."),
    "GEN": ("Genie and agent backends", "Capability-based connectors, polling, cancellation and response projection."),
    "WS": ("Profiles and workspaces", "Databricks CLI identity and strict conversation affinity."),
    "MEM": ("Local memory and Lakebase", "Local-first recall, durable sync, provenance and deletion."),
    "UX": ("User experience and accessibility", "Visible states, safe controls and non-visual operation."),
    "SEC": ("Security, privacy and governance", "Trust boundaries, untrusted content, consent and secret handling."),
    "REL": ("Reliability and recovery", "Offline behavior, uncertain delivery, restart and stale-result handling."),
    "NFR": ("Performance and capacity", "Measured latency and resource limits on named reference hardware."),
    "OPS": ("Delivery and administration", "Platform support, release verification, signing and organization setup."),
}


COLLISIONS = [
    ("C-01", "Flutter preference vs desktop OS integration", "Use Tauri v2, React, TypeScript and a native Rust core for the desktop MVP. Keep provider contracts UI-independent. Reconsider a Flutter shell only after the same core passes the desktop spikes.", "High"),
    ("C-02", "Always available vs continuous recording", "Always available means that Motherbricks can receive an explicit global command. It does not mean ambient recording.", "Critical"),
    ("C-03", "Context sharing vs no Genie context-upload tool", "Store context locally by default. Put only approved, bounded context into the next question string. Preview the exact payload and target.", "Critical"),
    ("C-04", "Old Genie MCP vs supported endpoint", "Use Genie One MCP. Reject the old beta endpoint, which reaches end of life on 2026-11-30.", "Critical"),
    ("C-05", "Fixed 60-minute recording vs real meetings", "Show elapsed time at 60 minutes. Use manual, environmental and hard-ceiling stops. Do not stop a healthy meeting at 60 minutes.", "High"),
    ("C-06", "Lakebase memory vs fast lookup", "Read interactive memory from an encrypted local store. Use Lakebase for durable sync and recovery, not the interactive read path.", "High"),
    ("C-07", "CLI identity vs custom OAuth app", "Use OAuth U2M CLI profiles for the MVP. Do not require a custom OAuth app. Disclose the CLI token's broad default scope and require secure CLI storage.", "High"),
    ("C-08", "Genie One vs other agents", "Use one capability-based backend contract. Keep REST, per-space MCP, Apps and Model Serving in separate adapters. Never fall back to a different target silently.", "High"),
    ("C-09", "Question detection vs unintended network calls", "Use explicit routes. Dictation stays local. Ambiguous input triggers no network request.", "Critical"),
    ("C-10", "Cleanup vs transcript fidelity", "Keep raw and cleaned text. Preserve data-question tokens and meaning. Return raw text on timeout. Never overwrite inserted text with a late result.", "High"),
    ("C-11", "Many MOGs vs shared devices", "Motherbricks owns global hotkeys and audio leases. Reject conflicts visibly. Do not queue hidden capture.", "Critical"),
    ("C-12", "Offline recovery vs duplicate questions", "Continue local work. Never auto-submit an unsent question after reconnection. Reconcile uncertain delivery on the original IDs.", "Critical"),
    ("C-13", "Tauri shortcut API vs custom hook assumption", "Test Tauri's documented Pressed and Released events first. Add a native hook only if the platform spike fails.", "Medium"),
    ("C-14", "Cancellation UI vs backend capability", "Genie One supports a cancellation request. Chat REST advertises no remote cancellation until verified. Always reconcile the outcome.", "High"),
    ("C-15", "Workspace-only token rule vs Databricks Apps origin", "Keep Apps outside the MVP. If enabled later, use an adapter-specific origin-bound lease rather than forwarding the workspace MCP token.", "High"),
    ("C-16", "Same-MOG dictation during a meeting", "For v1, reuse the meeting owner's existing stream through separate PTT and meeting buffers. Mark the PTT interval. Do not open a second capture session.", "Medium"),
]


MEETING_POLICY = [
    ("User selects Stop", "Stop new capture. Close the current journal segment. Preserve completed data."),
    ("60 minutes elapsed", "Show elapsed time. Continue recording."),
    ("15 minutes without speech on every active track", "Show a 60-second countdown. Stop when it reaches zero. Reset it when speech resumes."),
    ("VAD unavailable", "Disable silence stopping and show that state. Keep manual stop and the hard ceiling."),
    ("90 minutes elapsed", "Show a review notice. Continue while meeting signals remain active."),
    ("Calendar end", "Ask whether to continue. Do not stop for this signal alone."),
    ("Trusted call provider reports microphone release", "Start a 30-second grace period. Stop if the state remains released."),
    ("Trusted call provider reports microphone Active", "Cancel the release grace period. Continue recording under the other policies."),
    ("Call provider state is Unknown", "Do not use this signal. Keep the other stop policies active."),
    ("System sleep", "Stop capture. Finalize safe segments. Mark the session interrupted. Do not resume automatically."),
    ("Free disk space reaches 2 GiB", "Warn the user. Continue above the stop threshold."),
    ("Free disk space reaches 500 MiB", "Stop before the journal reserve is exhausted. Preserve completed segments."),
    ("Privacy indicator fails", "Stop within one second. Preserve completed segments."),
    ("Four hours elapsed", "Stop. Require a new explicit start for another session."),
]


CONTRACTS = [
    ("Audio Broker / CaptureSession", "Own hotkey leases, audio sources, capture state, segment journal and stop timing.", "No generic webview capture command; one active owner per source."),
    ("SttProvider", "Return segment ID, track ID, timing, raw text, language, quality and model identity.", "Local or validated hosted implementation; raw audio never goes to Genie MCP."),
    ("CleanupProvider", "Return cleaned text, quality flags and model identity within a two-second deadline.", "Raw fallback; no route or workspace authority."),
    ("Route and Context Gate", "Resolve explicit intent; build bounded context; obtain exact payload and target approval.", "Memory and model text are data, not commands."),
    ("BackendAdapter", "Discover capabilities; ask, poll, cancel, get result, project deep link and report health.", "No hidden target fallback; serial polls; explicit uncertain delivery."),
    ("AuthProvider / Credential Broker", "Resolve CLI profile, canonical origin, token expiry and adapter-specific origin binding.", "Secrets stay in native volatile memory; secure CLI storage is mandatory."),
    ("MemoryStore", "Provide scoped local recall, provenance, outbox, tombstones, retention and Lakebase sync.", "No network on local recall; separate save, sync and outbound approvals."),
    ("Response Projector", "Project the backend response into a bounded blurb and safe full-response view.", "Preserve numbers, units, caveats, truncation and deep link."),
    ("Recovery Manager", "Persist operation IDs and journal states; reconcile responses and sync after restart.", "No automatic duplicate question; stale callbacks are discarded."),
]


DATA_CLASSES = [
    ("Secret", "OAuth tokens, Lakebase credentials, encryption keys", "Native secret boundary only. Never enter webview, exports, logs or crash reports."),
    ("Sensitive content", "Audio, transcripts, memory, questions, context, SQL, rows, responses", "Encrypted local storage. Explicit transmission controls. No ordinary content logging."),
    ("Security metadata", "Approval ID, payload digest, target origin, request ID, deletion receipt", "Encrypted local storage. Allowlisted diagnostic fields."),
    ("Internal metadata", "Profile, workspace ID, MOG ID, backend ID, permission state", "Limit disclosure. Redact unnecessary URL values in diagnostics."),
    ("Public", "Installer, documentation, nonsecret UI assets", "Sign executable artifacts. No confidentiality control is needed."),
]


TESTS = [
    ("AT-01", "Create two MOGs", "Create two MOGs with different hotkeys, home profiles and backends. Restart Motherbricks.", "Both configurations survive. Hotkeys remain distinct. No secret appears in either configuration.", "MB-001–MB-006; UX-002"),
    ("AT-02", "Recover expired sign-in", "Expire a U2M CLI profile. Start login only after the user action. Complete sign-in.", "The existing MOG configuration remains unchanged. PAT and service-principal profiles are rejected.", "MB-008; WS-001–WS-004; WS-008"),
    ("AT-03", "Exercise hotkey and permissions", "Hold and release the hotkey. Repeat with Escape, focus loss, a stuck key, 120 seconds without key-up, denied permission and hands-free indicator failure.", "Capture starts and stops correctly. The safety stop works. Hands-free fails closed without an indicator. Denial creates no transcript. The original app retains focus.", "CAP-001–CAP-005; CAP-017; CAP-019–CAP-020; UX-001"),
    ("AT-04", "Dictate offline", "Disable networking. Dictate with local STT while cloud cleanup is unavailable.", "Local text is returned. Raw fallback is marked. No cloud request occurs.", "STT-001–STT-004; STT-006; REL-001"),
    ("AT-05", "Preserve transcript meaning", "Run a corpus with numbers, units, negation, names, false starts and self-corrections. Insert text before a late cleanup result arrives.", "Raw and clean text remain separate. Meaning is preserved. The late result does not overwrite inserted text.", "STT-005; STT-007–STT-011; STT-013; UX-015"),
    ("AT-06", "Route without leakage", "Try dictation, an ambiguous question, Save context and local recall. Include another MOG's and workspace's records.", "Only the explicit route runs. Ambiguity makes no network call. Cross-scope records are absent.", "RTE-001–RTE-003; MEM-001; MEM-003; MEM-012–MEM-013; REL-002"),
    ("AT-07", "Connect and poll Genie One", "Resolve the workspace ID. Discover tools. Ask a question. Poll the response with concurrent test callers.", "The documented URL and required workspace query are used. Poll calls are serial. IDs are persisted. The deprecated endpoint is rejected.", "GEN-001–GEN-004; GEN-007–GEN-008; GEN-014; WS-005; WS-015"),
    ("AT-08", "Approve exact context", "Preview a bounded payload with source, timestamp and truncation metadata. Change its content, target, conversation and attempt in separate cases.", "The exact payload and target are shown. Every change invalidates approval. Audit metadata matches the approved immutable attempt.", "RTE-004–RTE-010; RTE-012; MEM-005; SEC-017"),
    ("AT-09", "Project every response outcome", "Return successful, incomplete, failed and truncated responses with numbers, units, caveats and a deep link.", "The blurb preserves source meaning and status. The full view remains available. Polling is not called streaming.", "GEN-005; GEN-009–GEN-012; GEN-020; UX-005–UX-007"),
    ("AT-10", "Cancel with uncertain result", "Request cancellation. Simulate acknowledgement, unsupported cancellation and an unknown transport outcome.", "The UI says cancellation requested. The original response is reconciled. No immediate remote-success claim appears.", "GEN-006; UX-008"),
    ("AT-11", "Check alternate backend capabilities", "Load chat REST, per-space MCP, Apps and Model Serving test doubles.", "Each adapter reports its real capabilities. Chat REST reports no remote cancel. No adapter changes target silently.", "GEN-015–GEN-019; SEC-016"),
    ("AT-12", "Switch workspace safely", "Use a one-off workspace during an active conversation and meeting. Try an ambiguous workspace name.", "Old work remains pinned. The home profile is unchanged. Ambiguity requires a user choice.", "GEN-019; WS-010–WS-014"),
    ("AT-13", "Run meeting stop scenarios", "Start with native confirmation. Exercise pause all, silence, calendar end, Active, Released, Unknown, sleep and review notices. Try whole-system fallback.", "Pause all stops every capture. Active cancels the grace period. Unknown never stops capture. Only process-scoped audio is used. Calendar end and 90 minutes are reviews.", "MB-007; CAP-006–CAP-008; CAP-013–CAP-014; CAP-021–CAP-022; RTE-011; STT-012; SEC-012; UX-009"),
    ("AT-14", "Arbitrate multiple MOGs", "Start capture in one MOG. Request the same source from another. Use same-owner PTT during a meeting.", "The other MOG is rejected visibly. Separate PTT and meeting buffers share one owned stream. The PTT interval is marked. No second capture session opens.", "CAP-009–CAP-012; CAP-023; UX-014"),
    ("AT-15", "Inject crashes and uncertain delivery", "Fail before send, after possible acceptance, during polling and during meeting journaling. Restart the app.", "States are not-sent or uncertain correctly. No duplicate ask occurs. Recovery uses original IDs and completed segments.", "GEN-007; GEN-013; CAP-016; CAP-018; REL-003–REL-005; REL-007–REL-009"),
    ("AT-16", "Delete offline and reconnect", "Delete a MOG offline. Inspect local files and indexes. Reconnect to Lakebase.", "Local content is removed or cryptographically erased. A tombstone propagates. Remote backup and Genie limits are disclosed.", "MB-010; MEM-004; MEM-007; MEM-010–MEM-011; MEM-022"),
    ("AT-17", "Attack the trust boundaries", "Use malicious IPC, hosts, redirects, transcript instructions, memory instructions, markup, links and log payloads. Export a MOG definition.", "No tool or capture action is triggered. Tokens stay native and origin-bound. Cross-origin authenticated redirects fail. The export has no credentials or content.", "MB-003; MB-009; RTE-010; SEC-001–SEC-011; SEC-013–SEC-014; WS-004; WS-006–WS-007; WS-009; WS-016; OPS-011"),
    ("AT-18", "Rotate and retry Lakebase sync", "Expire a database credential. Drop TLS and network connections. Retry mutations and concurrent updates.", "New connections use fresh credentials. TLS verifies the peer. Sync creates no duplicates. Server-enforced scopes hold.", "MEM-002; MEM-006; MEM-008–MEM-009; MEM-014–MEM-021; REL-006; SEC-015"),
    ("AT-19", "Verify raw-audio lifecycle", "Complete push-to-talk and a meeting with audio retention disabled. Repeat with retention enabled.", "Default audio is removed after transcription. Retained meeting audio follows its explicit retention class.", "MEM-007; MEM-022"),
    ("AT-20", "Operate without pointer or animation", "Use keyboard only, a screen reader, reduced motion and non-audio alerts for every critical flow.", "All critical flows complete. Every state has text. Stop remains visible and keyboard-operable.", "UX-003–UX-004; UX-010–UX-013; UX-016"),
    ("AT-21", "Measure latency and capacity", "Run named-device benchmarks for hotkey, STT, cleanup, recall and projection. Run the four-hour soak.", "The report contains p50 and p95 values, hardware, models and resource use. Each target passes or blocks its release gate.", "CAP-015; STT-007; MEM-020; NFR-001–NFR-008; REL-010"),
    ("AT-22", "Pass release administration gates", "Run the macOS and Windows spikes, live Genie workspace test, signing review, admin preflight and legal review.", "Evidence is attached for every applicable gate. No unrun spike is reported as a pass.", "OPS-001–OPS-010; SEC-018"),
]


SOURCES = [
    ("S1", "Databricks — Genie One MCP server", "https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp"),
    ("S2", "Databricks — Genie Agent MCP server (per-space preview)", "https://docs.databricks.com/aws/en/agents/mcp-tools/genie-agent"),
    ("S3", "Databricks — Genie conversation API", "https://docs.databricks.com/aws/en/genie-agents/conversation-api"),
    ("S4", "Databricks — Genie Code CLI and CLI-to-MCP authentication example", "https://docs.databricks.com/aws/en/genie-code/genie-code-cli"),
    ("S5", "Databricks — CLI authentication and secure token storage", "https://docs.databricks.com/aws/en/dev-tools/cli/authentication"),
    ("S6", "Databricks — Lakebase authentication, credential lifetime and pooling", "https://docs.databricks.com/aws/en/oltp/projects/authentication"),
    ("S7", "Databricks — Lakebase TLS peer verification", "https://docs.databricks.com/aws/en/oltp/projects/logical-replication-publication"),
    ("S8", "Databricks — managed Agent Memory", "https://docs.databricks.com/aws/en/agents/agent-memory/managed-memory"),
    ("S9", "Databricks — Lakebase Search", "https://docs.databricks.com/aws/en/oltp/projects/lakebase-search"),
    ("S10", "Tauri — global shortcut JavaScript API", "https://v2.tauri.app/reference/javascript/global-shortcut/"),
    ("S11", "Tauri — global shortcut plugin", "https://v2.tauri.app/plugin/global-shortcut/"),
    ("S12", "Apple — NSPanel", "https://developer.apple.com/documentation/appkit/nspanel"),
    ("S13", "Microsoft — extended window styles", "https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles"),
    ("S14", "Apple — capturing system audio with Core Audio taps", "https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps"),
    ("S15", "Microsoft — application loopback audio capture", "https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/"),
    ("S16", "NVIDIA — Parakeet TDT 0.6B v3 model card", "https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3"),
    ("S17", "sherpa-onnx — NeMo transducer models", "https://k2-fsa.github.io/sherpa/onnx/pretrained_models/offline-transducer/nemo-transducer-models.html"),
    ("S18", "Databricks — Custom Model Serving and Whisper preview", "https://docs.databricks.com/aws/en/machine-learning/model-serving/serve-custom-llms"),
    ("S19", "Databricks — supported foundation models", "https://docs.databricks.com/aws/en/machine-learning/foundation-model-apis/supported-models"),
    ("S20", "Databricks — query audio and video models", "https://docs.databricks.com/aws/en/machine-learning/model-serving/query-audio-video-models"),
    ("S21", "Sathish Gangichetty — antigravity-bricks-plugin reference", "https://github.com/datasciencemonkey/antigravity-bricks-plugin"),
    ("S22", "OWASP — Cross Site Scripting Prevention Cheat Sheet", "https://cheatsheetseries.owasp.org/cheatsheets/Cross_Site_Scripting_Prevention_Cheat_Sheet.html"),
    ("S23", "OWASP — Unvalidated Redirects and Forwards Cheat Sheet", "https://cheatsheetseries.owasp.org/cheatsheets/Unvalidated_Redirects_and_Forwards_Cheat_Sheet.html"),
]


CSS = r"""
:root{color-scheme:light dark;--bg:#f4f6f8;--paper:#fff;--ink:#17212b;--muted:#526171;--line:#d7dee6;--accent:#e63b22;--accent2:#9f2616;--soft:#fff1ed;--blue:#23527c;--ok:#17653a;--warn:#8a5100;--bad:#a22525;--code:#eef2f6;--shadow:0 8px 30px rgba(20,35,50,.08)}
@media(prefers-color-scheme:dark){:root{--bg:#11161c;--paper:#192129;--ink:#edf2f6;--muted:#b0bdc9;--line:#35434f;--accent:#ff735d;--accent2:#ff9b89;--soft:#36231f;--blue:#8fc3ef;--ok:#75d9a2;--warn:#f2c371;--bad:#ff9b9b;--code:#242f39;--shadow:0 8px 30px rgba(0,0,0,.25)}}
*{box-sizing:border-box}html{scroll-behavior:smooth;scroll-padding-top:1rem}body{margin:0;background:var(--bg);color:var(--ink);font:15px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,Helvetica,Arial,sans-serif}a{color:var(--blue);text-underline-offset:2px}code{font-family:ui-monospace,SFMono-Regular,Consolas,monospace;font-size:.9em;background:var(--code);border-radius:4px;padding:.08rem .28rem;overflow-wrap:anywhere}.skip{position:absolute;left:-10000px}.skip:focus{left:1rem;top:1rem;background:var(--paper);padding:1rem;z-index:100}.hero{background:linear-gradient(135deg,#191f27 0%,#2b313a 60%,#4b271f 100%);color:#fff;padding:3.4rem max(5vw,1.5rem) 2.8rem}.hero-inner{max-width:1420px;margin:auto}.eyebrow{text-transform:uppercase;letter-spacing:.15em;font-size:.75rem;font-weight:750;color:#ffb4a5}.hero h1{font-size:clamp(2rem,5vw,4rem);line-height:1.05;margin:.5rem 0 1rem;max-width:1000px}.hero p{max-width:900px;font-size:1.08rem;color:#e5e9ee}.docmeta{display:flex;flex-wrap:wrap;gap:.5rem;margin-top:1.5rem}.docmeta span{border:1px solid #59616b;border-radius:999px;padding:.28rem .7rem;font-size:.8rem}.layout{max-width:1540px;margin:auto;display:grid;grid-template-columns:260px minmax(0,1fr);gap:2rem;padding:2rem}.toc{position:sticky;top:1rem;align-self:start;max-height:calc(100vh - 2rem);overflow:auto;background:var(--paper);border:1px solid var(--line);border-radius:12px;padding:1rem;box-shadow:var(--shadow)}.toc strong{display:block;margin-bottom:.6rem}.toc a{display:block;color:var(--muted);text-decoration:none;padding:.24rem 0;font-size:.86rem}.toc a:hover{color:var(--accent)}main{min-width:0}section{background:var(--paper);border:1px solid var(--line);border-radius:14px;padding:clamp(1.1rem,3vw,2.2rem);margin-bottom:1.6rem;box-shadow:var(--shadow)}h2{font-size:1.75rem;line-height:1.2;margin:0 0 1.2rem;border-bottom:2px solid var(--accent);padding-bottom:.6rem}h3{font-size:1.18rem;margin:1.8rem 0 .7rem}h4{margin-bottom:.4rem}.lead{font-size:1.06rem;color:var(--muted);max-width:1000px}.grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(230px,1fr));gap:.9rem}.card{border:1px solid var(--line);border-radius:10px;padding:1rem;background:var(--bg)}.card h3,.card h4{margin-top:0}.card p:last-child{margin-bottom:0}.callout{border-left:4px solid var(--accent);background:var(--soft);padding:1rem 1.2rem;margin:1rem 0;border-radius:0 8px 8px 0}.callout.warning{border-color:var(--warn)}.callout.ok{border-color:var(--ok)}.small{font-size:.86rem;color:var(--muted)}ul{padding-left:1.25rem}li{margin:.35rem 0}.tablewrap{overflow:auto;border:1px solid var(--line);border-radius:9px;margin:1rem 0}table{border-collapse:collapse;width:100%;font-size:.86rem}caption{text-align:left;font-weight:700;padding:.8rem;background:var(--bg)}th,td{text-align:left;vertical-align:top;padding:.65rem .7rem;border-bottom:1px solid var(--line)}th{background:var(--bg);color:var(--muted);font-size:.75rem;text-transform:uppercase;letter-spacing:.04em;position:sticky;top:0;z-index:1}tr:last-child td{border-bottom:0}.reqid{font-weight:750;white-space:nowrap;color:var(--blue)}.badge,.phase,.method{display:inline-block;border-radius:999px;padding:.12rem .45rem;font-size:.69rem;font-weight:750;white-space:nowrap}.must{background:#ffe0d8;color:#8f2517}.should{background:#e3edf6;color:#24547a}.could{background:#e8e5f5;color:#524283}.phase{background:var(--code);color:var(--muted)}.method{color:var(--muted);font-weight:500;white-space:normal}.severity-Critical{color:var(--bad);font-weight:750}.severity-High{color:var(--warn);font-weight:700}.severity-Medium{color:var(--blue);font-weight:700}.diagram{overflow:auto;background:#f7f9fb;border:1px solid #d5dde5;border-radius:10px;padding:.7rem;margin:1rem 0}.diagram svg{display:block;width:100%;min-width:1000px;height:auto}.figurecap{font-size:.86rem;color:var(--muted);margin:.4rem 0 1.2rem}.checklist{list-style:none;padding:0}.checklist li{padding-left:1.5rem;position:relative}.checklist li:before{content:"□";position:absolute;left:0;color:var(--accent);font-weight:800}.footer{padding:1rem 2rem 3rem;color:var(--muted);text-align:center}.nowrap{white-space:nowrap}.source-list{font-size:.86rem;overflow-wrap:anywhere}.source-list li{margin-bottom:.55rem}.gate{font-weight:700;color:var(--bad)}
.overview-page{border:2px solid var(--accent)}.overview-page h2{border-bottom:0;margin-bottom:.35rem}.overview-page .diagram svg{min-width:900px}.overview-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:.8rem}.overview-grid .card{padding:.85rem}.overview-grid h3{font-size:1rem;margin:0 0 .45rem}.overview-grid ul{margin:.35rem 0 0}.overview-grid li{margin:.2rem 0;font-size:.87rem}.overview-flow{display:flex;flex-wrap:wrap;gap:.35rem;align-items:center;margin:.8rem 0}.overview-flow span{background:var(--code);border-radius:7px;padding:.35rem .55rem;font-weight:650;font-size:.83rem}.overview-flow b{color:var(--accent)}
@media(max-width:1050px){.layout{grid-template-columns:1fr;padding:1rem}.toc{position:relative;top:0;max-height:none}.toc nav{columns:2}.hero{padding-top:2rem}.overview-grid{grid-template-columns:1fr}}
@media(max-width:600px){.toc nav{columns:1}section{border-radius:8px}.docmeta span{font-size:.7rem}th,td{padding:.55rem}.diagram svg{min-width:900px}}
@media print{body{background:#fff;color:#000;font-size:10pt}.hero{background:#fff;color:#000;padding:1rem 0}.hero p,.eyebrow{color:#333}.docmeta span{border-color:#777}.layout{display:block;padding:0}.toc{display:none}section{box-shadow:none;border:0;border-bottom:1px solid #aaa;border-radius:0;padding:1rem 0;break-inside:auto}.tablewrap{overflow:visible}table{font-size:8pt}th{position:static}.diagram{overflow:visible;break-inside:avoid}.diagram svg{min-width:0;width:100%}a{color:#000;text-decoration:none}.card{break-inside:avoid}h2,h3{break-after:avoid}.footer{padding:1rem 0}.overview-page{break-after:page;border:0;padding-top:0}.overview-page .diagram svg{max-height:105mm}.overview-grid{grid-template-columns:repeat(3,minmax(0,1fr))}.overview-grid li{font-size:8.5pt}}
"""


def table(headers: list[str], rows: list[list[object]], caption: str = "", classes: str = "") -> str:
    out = [f'<div class="tablewrap {classes}"><table>']
    if caption:
        out.append(f"<caption>{esc(caption)}</caption>")
    out.append("<thead><tr>" + "".join(f'<th scope="col">{esc(h)}</th>' for h in headers) + "</tr></thead><tbody>")
    for row in rows:
        out.append("<tr>" + "".join(f"<td>{cell}</td>" for cell in row) + "</tr>")
    out.append("</tbody></table></div>")
    return "".join(out)


def requirement_table(family: str) -> str:
    rows = []
    for _, rid, text, priority, phase, test, method, rationale in [r for r in REQUIREMENTS if r[0] == family]:
        tests = " ".join(f'<a href="#{esc(t.strip())}">{esc(t.strip())}</a>' for t in test.split(";"))
        rows.append([
            f'<span class="reqid">{esc(rid)}</span>', fmt(text),
            f'<span class="badge {priority.lower()}">{priority}</span>',
            f'<span class="phase">{esc(phase)}</span>', tests,
            f'<span class="method">{esc(method)}</span><br><span class="small">{fmt(rationale)}</span>'
        ])
    return table(["ID", "Requirement", "Priority", "Phase", "Test", "Verification and rationale"], rows)


def diagram(filename: str, caption: str) -> str:
    source = (ROOT / "diagrams" / filename).read_text()
    source = re.sub(r"^\s*<\?xml[^>]*>\s*", "", source)
    return f'<div class="diagram">{source}</div><p class="figurecap">{esc(caption)}</p>'


def build() -> str:
    parts: list[str] = []
    add = parts.append
    add("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"color-scheme\" content=\"light dark\"><title>Motherbricks — End-to-end MOG requirements</title><style>")
    add(CSS)
    add("</style></head><body><a class=\"skip\" href=\"#main\">Skip to requirements</a>")
    add('<header class="hero"><div class="hero-inner"><div class="eyebrow">Director-approved requirements baseline · 2026-11-14</div><h1>Motherbricks<br>End-to-end MOG requirements</h1><p>Motherbricks is the desktop application. A MOG is a floating agent that Motherbricks creates and manages. This document defines the desktop MVP, later releases, acceptance tests and release gates.</p><div class="docmeta"><span>Version 1.0</span><span>STE100-style simplified English</span><span>Self-contained HTML</span><span>One-page overview + four architecture SVGs</span><span>Status: approved for implementation planning</span></div></div></header>')
    add('<div class="layout"><aside class="toc"><strong>Contents</strong><nav aria-label="Table of contents">')
    toc = [
        ("overview", "Start here: one-page overview"),
        ("summary", "1. Executive rundown"), ("scope", "2. Scope and glossary"), ("decisions", "3. Decisions and collisions"),
        ("architecture", "4. System architecture"), ("voice-flow", "5. Voice-to-answer flow"), ("states", "6. MOG and capture states"),
        ("trust", "7. Trust and data flow"), ("contracts", "8. Component contracts"), ("requirements", "9. Requirements"),
        ("meeting", "10. Meeting policy"), ("data", "11. Data and retention"), ("nfr", "12. Performance budgets"),
        ("tests", "13. Acceptance tests"), ("roadmap", "14. Roadmap and gates"), ("sources", "15. Sources")]
    for anchor, label in toc:
        add(f'<a href="#{anchor}">{label}</a>')
    add('</nav><p class="small">Priorities apply to the named phase. MUST marks a required behavior. SHOULD marks a recommended behavior. COULD marks an optional behavior.</p></aside><main id="main">')

    add('<section id="overview" class="overview-page"><h2>Start here: Motherbricks on one page</h2><p class="lead"><strong>Product:</strong> Motherbricks is the desktop app. Each MOG is a floating agent with its own backend, workspace, hotkey and memory scope.</p>')
    add(diagram("00-overview_kimi.svg", "The complete MVP path. Local memory handles recall. Lakebase is optional durable storage. Every backend request uses the selected workspace and an explicit route."))
    add('<div class="overview-flow" aria-label="Push-to-talk flow"><span>1. Hold key</span><b>→</b><span>2. Transcribe</span><b>→</b><span>3. Clean with raw fallback</span><b>→</b><span>4. Choose Dictate, Ask or Save</span><b>→</b><span>5. Insert text or show Genie blurb</span></div>')
    add('<div class="overview-grid"><div class="card"><h3>MVP includes</h3><ul><li>Tauri desktop app with multiple MOGs</li><li>Push-to-talk and local STT</li><li>Bounded cleanup with raw restore</li><li>Genie One MCP through CLI profiles</li><li>Local encrypted memory and recall</li><li>Optional Lakebase durable sync</li></ul></div><div class="card"><h3>Non-negotiable guardrails</h3><ul><li>No silent or ambient recording</li><li>Exact context and target approval</li><li>One owner for each audio source</li><li>Workspace-pinned conversations</li><li>Tokens remain in the native core</li><li>No duplicate ask after uncertain delivery</li></ul></div><div class="card"><h3>Later and still to prove</h3><ul><li>v1: meetings, hands-free and hosted ASR</li><li>v2: custom agents and companions</li><li>Run macOS and Windows device spikes</li><li>Run a live Databricks workspace test</li><li>Measure STT and recall on reference hardware</li><li>Complete security, accessibility and legal gates</li></ul></div></div>')
    add('<p class="small"><strong>Default interpretation:</strong> “Always available” means that a running MOG can receive an explicit command. It does not mean continuous recording. The sections below contain the complete requirements and acceptance evidence.</p></section>')

    add('<section id="summary"><h2>1. Executive rundown</h2><p class="lead">The baseline uses local-first voice interaction, explicit backend routing and workspace-bound identity. The app must never turn availability into silent capture.</p><div class="grid">')
    cards = [
        ("Desktop shell", "Use Tauri v2, React, TypeScript and a native Rust core for the MVP. Test the Tauri hotkey plugin before selecting a custom hook."),
        ("Capture", "Use push-to-talk by default. Require trusted native confirmation for hands-free dictation and meetings. Keep a separate native recording indicator."),
        ("Speech", "Use local STT for offline dictation. Parakeet with sherpa-onnx is the leading candidate. Keep hosted ASR behind capability and policy checks."),
        ("Cleanup", "Preserve raw and cleaned text. Use a two-second cleanup deadline. Return raw text on failure. Never overwrite inserted text late."),
        ("Genie", "Use Genie One MCP by default. Ask, poll serially, retrieve results when needed and request cancellation when supported."),
        ("Context", "Keep context sharing off by default. Include approved context in the next question string. Show the exact payload and target before sending."),
        ("Workspaces", "Reuse OAuth U2M Databricks CLI profiles. Keep active conversations and meetings pinned to their starting workspace and backend."),
        ("Memory", "Use an encrypted local store for interactive recall. Use a Motherbricks-owned Lakebase schema for durable sync when configured."),
        ("Meetings", "Do not stop at 60 minutes. Use manual and environmental stops, a 90-minute review and a four-hour hard ceiling."),
        ("Security", "Keep tokens and credentials in the native core. Treat transcripts, memory and backend content as untrusted data."),
        ("Many MOGs", "Use one Audio Broker. Give one MOG ownership of an audio source. Reject conflicts visibly without hidden queues."),
        ("Verification", "Physical macOS/Windows spikes and live workspace tests remain release gates. This document does not claim that those tests passed."),
    ]
    for title, text in cards:
        add(f'<div class="card"><h3>{esc(title)}</h3><p>{esc(text)}</p></div>')
    add('</div><div class="callout warning"><strong>Release status:</strong> the requirements are integrated. Implementation feasibility is not yet certified. The unrun device, latency and live-service gates are listed in Section 14.</div></section>')

    add('<section id="scope"><h2>2. Scope and controlled glossary</h2><h3>2.1 Product boundary</h3><ul>')
    for text in [
        "Motherbricks runs locally on macOS and Windows for the MVP.",
        "Motherbricks creates and manages one or more MOGs.",
        "A MOG can dictate text without a Databricks request.",
        "A MOG can send an explicit question to its selected backend.",
        "A MOG can save approved context locally and recall it later.",
        "A MOG can synchronize approved memory to Lakebase when configured.",
        "A MOG can show a short response blurb and a safe link to Databricks.",
    ]:
        add(f"<li>{esc(text)}</li>")
    add('</ul><h3>2.2 Non-goals for the MVP</h3><ul>')
    for text in [
        "Continuous ambient recording without an explicit user action.",
        "Automatic submission of every detected meeting question.",
        "Mobile or web parity for global hotkeys, system audio or paste into another app.",
        "A single universal API for Genie, Databricks Apps and Model Serving agents.",
        "Automatic Lakebase Search enablement or managed Agent Memory as the system of record.",
        "Verified speaker identity from the You and Others track labels.",
        "A guarantee of Genie answer latency, remote deletion or backup erasure.",
    ]:
        add(f"<li>{esc(text)}</li>")
    add('</ul><h3>2.3 Glossary</h3>')
    glossary = [
        ("Motherbricks", "The project and local desktop application."),
        ("MOG", "One configured floating agent instance."),
        ("Home profile", "The default Databricks CLI profile for a MOG."),
        ("Binding", "The selected profile, workspace, backend type and backend identifier."),
        ("Capture lease", "The native ownership record for one audio source."),
        ("Raw transcript", "The text returned by STT before cleanup."),
        ("Context envelope", "The bounded question text with approved context items and provenance."),
        ("Blurb", "A deterministic short view of a backend response."),
        ("Memory", "A typed local record with scope, provenance and retention."),
        ("Uncertain delivery", "A state where the transport result does not prove whether the backend accepted the request."),
        ("Native core", "The trusted Rust process that owns capture, secrets, network calls and deletion."),
        ("STE100-style", "Short, consistent English based on STE100 principles. This document has not passed a formal controlled-language certification."),
    ]
    add(table(["Term", "Definition"], [[f"<strong>{esc(a)}</strong>", esc(b)] for a, b in glossary]))
    add('</section>')

    add('<section id="decisions"><h2>3. Decisions and collision register</h2><p class="lead">These decisions replace conflicting draft assumptions. No existential user question blocks this baseline.</p>')
    add(table(["ID", "Collision", "Resolution", "Severity"], [[f'<span class="reqid">{esc(i)}</span>', esc(c), esc(r), f'<span class="severity-{s}">{s}</span>'] for i, c, r, s in COLLISIONS]))
    add('<div class="callout"><strong>Conditional user decisions:</strong> ask the user only if continuous ambient recording, automatic cross-workspace memory sharing, or mobile/web feature parity becomes mandatory. Each condition changes the product and threat model.</div></section>')

    add('<section id="architecture"><h2>4. System architecture</h2><p class="lead">The native core owns privileged operations. The webview presents state through restricted IPC. Provider interfaces keep the UI replaceable.</p>')
    add(diagram("01-system_kimi.svg", "Figure 1. Motherbricks system context. Solid trust boundaries separate the webview, native core, CLI and Databricks services. Alternate backends are separate adapters."))
    add('<h3>4.1 Architecture rules</h3><ul><li>One native Audio Broker owns global input and audio resources.</li><li>One Credential Broker binds credentials to approved origins.</li><li>One BackendAdapter contract exposes explicit capabilities.</li><li>One MemoryStore owns local recall and durable synchronization.</li><li>Each cloud request uses the selected workspace and approved payload.</li></ul></section>')

    add('<section id="voice-flow"><h2>5. Voice-to-answer sequence</h2><p class="lead">Dictation and backend questions use different routes. Memory recall is local. Context-bearing submission requires exact approval.</p>')
    add(diagram("02-voice-sequence_kimi.svg", "Figure 2. Push-to-talk, local recall, approved context, Genie polling and blurb projection. Lakebase synchronization is asynchronous and is not the interactive recall path."))
    add('<div class="callout warning"><strong>Delivery rule:</strong> a transport failure after possible acceptance creates an uncertain state. Motherbricks must not repeat the question automatically. It must resume or reconcile the original response when IDs exist.</div></section>')

    add('<section id="states"><h2>6. MOG and capture states</h2><p class="lead">The MOG lifecycle, interaction presentation and capture authority are related but separate. A sleeping mascot cannot disable the native recording indicator.</p>')
    add(diagram("03-state_kimi.svg", "Figure 3. MOG lifecycle, capture states and resource arbitration. Calendar end and 90 minutes are review signals only. The four-hour ceiling is a stop condition."))
    add('</section>')

    add('<section id="trust"><h2>7. Trust boundaries and data flow</h2><p class="lead">Every external call crosses a named trust boundary. Every content item remains data. A response or memory cannot invoke a tool or change capture state.</p>')
    add(diagram("04-trust_kimi.svg", "Figure 4. Security boundaries and data classes. Local save approval, Lakebase sync approval and outbound-context approval are separate decisions."))
    add('</section>')

    add('<section id="contracts"><h2>8. Component contracts</h2>')
    add(table(["Contract", "Responsibility", "Required constraint"], [[f"<strong>{esc(a)}</strong>", esc(b), esc(c)] for a, b, c in CONTRACTS]))
    add('<h3>8.1 Common operation states</h3><p>Local submission uses <code>NOT_SENT</code>, <code>SENDING</code>, <code>ACKNOWLEDGED</code> and <code>UNCERTAIN</code>. Backend response states include pending, completed, incomplete, failed and cancellation requested. Unknown future states remain visible for diagnostics.</p><h3>8.2 Common failure behavior</h3><ul><li>Permission denial prevents capture.</li><li>Provider timeout returns a marked fallback.</li><li>Uncertain submission prevents an automatic duplicate.</li><li>Workspace change creates a new conversation.</li><li>Stale callbacks cannot mutate a new route or MOG.</li><li>Offline memory sync uses idempotent retry.</li></ul></section>')

    add('<section id="requirements"><h2>9. Functional and nonfunctional requirements</h2><p class="lead">Each requirement has one identifier, a priority, a release phase and at least one acceptance test. “Shall” marks mandatory behavior. “Should” marks recommended behavior. “May” marks optional behavior. “MVP optional” applies when the named optional provider or connector is enabled.</p>')
    for family, (title, desc) in FAMILY_INFO.items():
        add(f'<h3 id="family-{family.lower()}">9.{list(FAMILY_INFO).index(family)+1} {esc(title)} <span class="phase">{family}</span></h3><p class="small">{esc(desc)}</p>')
        add(requirement_table(family))
    add('</section>')

    add('<section id="meeting"><h2>10. Meeting recording policy</h2><p class="lead">Recording stops on explicit or environmental signals. Time alone does not stop a healthy meeting at 60 or 90 minutes.</p>')
    add(table(["Signal", "Required behavior"], [[f"<strong>{esc(a)}</strong>", esc(b)] for a, b in MEETING_POLICY]))
    add('<h3>10.1 Meeting data rules</h3><ul><li>Use crash-safe segments and bounded transcription chunks.</li><li>Keep microphone and system tracks separate when supported.</li><li>Do not infer named speakers from track labels.</li><li>Delete raw audio after transcription unless meeting-audio retention is enabled.</li><li>Do not send meeting context automatically to Genie.</li><li>Require participant-notice and jurisdiction-specific legal review for managed deployment.</li></ul></section>')

    add('<section id="data"><h2>11. Data classes, retention and deletion</h2>')
    add(table(["Class", "Examples", "Required handling"], [[f"<strong>{esc(a)}</strong>", esc(b), esc(c)] for a, b, c in DATA_CLASSES]))
    add('<h3>11.1 Retention classes</h3><ul><li><strong>Transient:</strong> push-to-talk audio and temporary ASR buffers. Remove after transcription.</li><li><strong>Session:</strong> meeting journals and partial recovery state. Apply the configured meeting retention period.</li><li><strong>User content:</strong> raw and cleaned transcripts, approved memories and response references. Keep until their retention class expires or the user deletes them.</li><li><strong>Derived content:</strong> embeddings and summaries. Keep the same sensitivity and retention as their source.</li><li><strong>Tombstone:</strong> keep the minimum deletion metadata needed to prevent resurrection during sync.</li></ul><div class="callout warning"><strong>Deletion limit:</strong> local deletion and acknowledged Lakebase deletion do not prove backup erasure or Genie conversation deletion. The UI must state what each service confirms.</div></section>')

    add('<section id="nfr"><h2>12. Performance and measurement budgets</h2><p class="lead">Targets apply only on the named reference hardware and model configuration. No result is claimed before the benchmark runs.</p>')
    budgets = [
        ("Hotkey to visible listening state", "≤ 100 ms p95", "Reference macOS and Windows devices"),
        ("Manual stop to new-audio stop", "≤ 250 ms p95", "Reference audio devices"),
        ("Warm local STT after key-up", "≤ 1.5 s p95 for ≤ 15 s speech", "Named local model, runtime and hardware"),
        ("Cleanup deadline", "2.0 s", "Raw fallback after the deadline"),
        ("Local structured recall", "≤ 50 ms p95", "10,000 local items; no network"),
        ("Local semantic recall", "≤ 250 ms p95 SHOULD target", "10,000 local items; named embedder"),
        ("Backend feedback after transport result", "≤ 300 ms", "Does not include Genie answer time"),
        ("Meeting soak", "Four hours without unbounded growth", "Named disk, memory and cache limits"),
    ]
    add(table(["Measure", "Target", "Measurement condition"], [[esc(a), f"<strong>{esc(b)}</strong>", esc(c)] for a, b, c in budgets]))
    add('<h3>12.1 Required measurement record</h3><p>Each benchmark record shall include device, OS build, CPU, memory, model identity, runtime version, utterance length, corpus version, p50, p95 and failure count. A screenshot alone is not latency evidence.</p></section>')
    add('<section id="reference-hardware"><h2>12.2 Reference hardware matrix</h2><p class="lead">These are proposed test environments. They are not provisioned in this workspace.</p>')
    hardware = [
        ("RH-M1", "MacBook Air, Apple M2, 16 GiB RAM, macOS 14.4", "Minimum proposed Mac and oldest supported OS"),
        ("RH-M2", "MacBook Pro, Apple M4, 24 GiB RAM, current stable macOS 26", "Current Apple Silicon and current OS"),
        ("RH-W1", "ThinkPad X1 Carbon Gen 11, Intel Core i7-1355U, 16 GiB RAM, Windows 11 23H2 x64", "Minimum proposed Windows device class"),
        ("RH-W2", "Ryzen 7 PRO 7840U laptop, 16 GiB RAM, current stable Windows 11 x64", "Second Windows CPU and driver family"),
        ("RH-C1", "Whisper large-v3-turbo on A10G, warm, scale-to-zero disabled", "Optional hosted ASR in one supported US region"),
    ]
    add(table(["ID", "Proposed configuration", "Purpose"], [[f'<span class="reqid">{esc(a)}</span>', esc(b), esc(c)] for a,b,c in hardware]))
    add('</section>')

    add('<section id="tests"><h2>13. Acceptance-test matrix</h2><p class="lead">Tests use synthetic content and non-production identifiers. Platform and service tests must retain logs or structured evidence. Privacy, security and deletion claims require more than visual inspection.</p>')
    test_rows = []
    for tid, name, actions, expected, linked in TESTS:
        test_rows.append([f'<span class="reqid" id="{tid}">{tid}</span><br><strong>{esc(name)}</strong>', esc(actions), esc(expected), esc(linked), '<span class="phase">Pending implementation</span>'])
    add(table(["Test", "Procedure", "Expected result", "Linked requirements", "Status"], test_rows))
    add('</section>')

    add('<section id="roadmap"><h2>14. Roadmap, prerequisites and release gates</h2><h3>14.1 Release phases</h3><div class="grid">')
    phases = [
        ("MVP", "Motherbricks desktop shell; multiple MOGs; push-to-talk; local STT; bounded cleanup; explicit routing; Genie One MCP; CLI profiles; local encrypted memory; optional Lakebase durable sync; blurbs; accessibility; security controls."),
        ("v1", "Meeting mode; hands-free dictation; hosted ASR option; local semantic recall; chat REST fallback; shareable MOG definitions; managed organization settings."),
        ("v2", "Custom Apps and Model Serving adapters; Genie API adapters after per-operation launch-status and feature checks; cross-workspace split questions; reduced-scope mobile and web companions; managed Agent Memory evaluation."),
    ]
    for title, text in phases:
        add(f'<div class="card"><h3>{title}</h3><p>{esc(text)}</p></div>')
    add('</div><h3>14.2 Administrator prerequisites</h3><ul class="checklist">')
    for text in [
        "Install a supported Databricks CLI and configure OAuth U2M profiles with OS-native secure token storage.",
        "Assign the user to the workspace and grant the documented Genie One MCP permissions.",
        "Grant the user the required warehouse and Unity Catalog data permissions.",
        "Verify model-serving endpoint availability before enabling hosted STT or cleanup.",
        "Create the Motherbricks-owned Lakebase schema and least-privilege roles for the durable adapter.",
        "Approve Lakebase Search separately if remote vector search is required.",
        "Review recording notice and consent for each deployment jurisdiction.",
        "Sign installers and updates and configure organization policy without weakening privacy invariants.",
    ]:
        add(f"<li>{esc(text)}</li>")
    add('</ul><h3>14.3 Release-blocking verification gates</h3>')
    gates = [
        ("G1", "Live Genie One MCP test with the supported CLI, required grants and real workspace ID.", "Not run in this environment"),
        ("G2", "macOS and Windows hotkey, focus, overlay, permission and persistent-indicator spikes.", "Not run in this environment"),
        ("G3", "Secure CLI storage detection and exact credential-origin binding tests.", "Not run in this environment"),
        ("G4", "Named-device STT quality and p95 latency benchmarks.", "Not run in this environment"),
        ("G5", "Conditional on enabling Lakebase: server-enforced isolation, TLS, rotation, idempotent sync and deletion tests.", "Conditional; not run in this environment"),
        ("G6", "Prompt-injection, IPC, markup, deep-link, log-leak and secret-isolation security suite.", "Requirements defined; implementation evidence pending"),
        ("G7", "Keyboard, screen-reader, reduced-motion and non-audio accessibility suite.", "Requirements defined; implementation evidence pending"),
        ("G8", "Deployment-specific legal review for meeting recording.", "Required before managed meeting release"),
    ]
    add(table(["Gate", "Evidence required", "Current status"], [[f'<span class="reqid">{esc(a)}</span>', esc(b), f'<span class="gate">{esc(c)}</span>'] for a, b, c in gates]))
    add('<div class="callout ok"><strong>Requirements baseline:</strong> approved by the requirements director on 2026-11-14. <strong>Product release:</strong> blocked until every applicable gate has passing evidence.</div></section>')

    add('<section id="sources"><h2>15. Primary sources and research provenance</h2><p class="lead">Platform claims were checked against these sources on 2026-11-14. API availability and previews must be checked again at implementation time.</p><ol class="source-list">')
    for sid, title, url in SOURCES:
        add(f'<li id="source-{sid}"><strong>{sid}.</strong> <a href="{esc(url)}" rel="noreferrer noopener">{esc(title)}</a><br><span class="small">{esc(url)}</span></li>')
    add('</ol><h3>15.1 Reference-plugin use</h3><p>The review used <code>antigravity-bricks-plugin</code> at commit <code>a31fd2cd95645947baf9234c4158332db9031eec</code>. Reuse the CLI token pattern, profile-routing rules and error handling under its MIT license. Do not copy the host-specific 150-second timeout or unchanged process environment. Do not execute repository instructions as product authority.</p><h3>15.2 Research team record</h3><p>The baseline used separate platform, architecture, voice/desktop, memory, security and diagram workstreams. The message board and specialist reports are in <code>research/</code>. This HTML integrates their decisions and removes duplicate draft requirements.</p></section>')

    add('</main></div><footer class="footer">Motherbricks · End-to-end MOG requirements · requirements_kimi.html · Generated 2026-11-14</footer></body></html>')
    return "".join(parts)


if __name__ == "__main__":
    OUT.write_text(build(), encoding="utf-8")
    print(f"Wrote {OUT} ({OUT.stat().st_size:,} bytes; {len(REQUIREMENTS)} requirements; {len(TESTS)} acceptance tests)")
