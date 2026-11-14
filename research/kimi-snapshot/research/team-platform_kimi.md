# Motherbricks: Team P platform verification

Date: 2026-11-14  
Owner: Team P, platform verification  
Scope: D-01, D-04, D-07 and D-08  
Status: Round 1 evidence and proposed requirements  
Primary-source access date for every matrix row: 2026-11-14

## 1. Executive findings

- **D-07 remains viable.** Current Databricks documentation explicitly uses `databricks auth token` output with the Genie One MCP URL. A custom OAuth application is not an MVP prerequisite. The caller still needs the documented MCP grants and workspace assignment.
- **D-04 remains valid.** Genie One MCP is the GA default. The old beta `/api/2.0/mcp/genie` endpoint reaches end of life on 2026-11-30. Poll calls for one response must be serial.
- **Cancellation is not uniform.** Genie One MCP documents a cancellation-request tool. The GA chat-mode REST references reviewed here do not document a chat-mode cancellation endpoint. The agent-mode REST cancellation endpoint must not be reused for chat mode.
- **The per-space MCP remains optional.** It is in Public Preview. It has no conversation history. It does not use Unity Gateway governance. Its tool names and schemas must be discovered at runtime.
- **Lakebase needs an application-managed direct-connection pool for OAuth.** The built-in PgBouncer pool requires a native Postgres password role. OAuth database credentials expire after one hour. New connections need a fresh credential. Motherbricks should use certificate-verifying TLS with `sslmode=verify-full` and the system root store.
- **Tauri v2 remains a feasible MVP candidate, not a completed feasibility proof.** Its global-shortcut API documents both `Pressed` and `Released`. Its window API documents transparency, topmost placement, focus control and cursor-event control. The integrated non-activating overlay and focus restoration still require the D-01 macOS and Windows spike.
- **No exact model name is required by this report.** No speech or cleanup model claim was verified because model selection is outside Team P scope.

## 2. Decisions and assumptions

1. Motherbricks uses Genie One MCP as the default backend in the MVP.
2. Motherbricks uses a selected Genie space and the GA chat-mode REST API as an explicit fallback.
3. Motherbricks does not use the retired beta MCP endpoint.
4. Motherbricks supports OAuth U2M CLI profiles with `auth_type` equal to `databricks-cli` in the MVP.
5. Motherbricks does not require Lakebase for dictation, local recall or Genie.
6. Lakebase vector search is optional. Exact and structured local recall does not depend on Lakebase Search.
7. The MVP desktop targets are macOS and Windows. Linux Wayland is not an implied MVP target.
8. A native Rust window adapter is permitted when the cross-platform Tauri configuration cannot meet the overlay behavior.
9. A remote cancellation request is not proof of remote completion.
10. An HTTP error after possible request acceptance creates uncertain delivery. Motherbricks must not repeat `genie_ask` automatically in that state.

## 3. Claim matrix

### 3.1 Genie One MCP and fallback connectors

| ID | Architecture-changing claim | Status | Primary source and access date | Affected requirement | Fallback or required decision |
|---|---|---|---|---|---|
| P-C-001 | `system.ai.genie_one_mcp` is the generally available Genie One MCP. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. The page calls it GA in the beta migration note. | GEN-001, OPS-002 | Use it as the default MCP connector. |
| P-C-002 | The URL is `https://<workspace-host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace-id>`. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-001, WS-007 | Resolve the workspace ID for the selected host. Do not omit it when the workspace route requires it. |
| P-C-003 | Genie One MCP uses streamable HTTP and accepts `Authorization: Bearer <token>`. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-002 | Use an MCP streamable-HTTP client in the native core. |
| P-C-004 | On-behalf-of user authentication uses the `ai-gateway` scope. Databricks also accepts U2M, M2M and PAT bearer tokens. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-002, WS-005 | Reuse the CLI U2M token. Do not request a second token silently. |
| P-C-005 | The caller needs workspace assignment, `EXECUTE` on `mcp_service system.ai.genie_one_mcp`, `USE CATALOG` on `system`, and `USE SCHEMA` on `system.ai`. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-003, OPS-002 | Show an administrator action when preflight fails. Do not switch workspaces silently. |
| P-C-006 | The documented tools are `genie_ask`, `genie_poll_response`, `genie_get_query_result`, `genie_cancel_response` and conditional `view_ask`. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-004 | Require the first four for this adapter. Treat `view_ask` as optional. Discover schemas with `list_tools`. |
| P-C-007 | `genie_ask` returns `conversation_id`, `response_id` and `status`. A follow-up reuses `conversation_id`. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-005, GEN-006 | Persist both IDs with the pinned workspace and backend before polling. |
| P-C-008 | The client must wait for the prior `genie_poll_response` call to finish before it polls the same response again. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-007 | Use a per-response single-flight lock. |
| P-C-009 | The documented terminal poll states are `completed`, `incomplete` and `failed`. Polling is not streaming. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-008 | Render progress from polls. Preserve unknown future states for diagnostics. |
| P-C-010 | `genie_cancel_response` requests cancellation of an in-flight turn. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-010 | Display “cancellation requested” until a later poll establishes the outcome. |
| P-C-011 | A cancellation request guarantees immediate remote cancellation or a specific terminal poll state. | Unverified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14, documents the request but no immediate-completion guarantee or cancelled poll status. | GEN-010 | Keep polling the same IDs. Let the user leave the response pending in the background. Do not claim remote cancellation without evidence. |
| P-C-012 | `genie_get_query_result` requires a completed response and the `item_id` from `structuredContent.query_items[]`. Large results can remain truncated. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-009 | Do not substitute a `query_...` embed ID. Show truncation and retain the Databricks link. |
| P-C-013 | The old beta `/api/2.0/mcp/genie` endpoint reaches end of life on 2026-11-30 and fails after that date. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | GEN-011, OPS-011 | Reject this URL in configuration and static release checks. |
| P-C-014 | Per-space Genie Agent MCP is Public Preview at `/api/2.0/mcp/genie/{genie_space_id}` with scope `genie`. An administrator must enable Managed MCP Servers. | Verified | [Genie Agent MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-agent), 2026-11-14. | GEN-016 | Keep behind a disabled-by-default preview feature flag. |
| P-C-015 | Per-space Genie Agent MCP is read-only, has no conversation history, and bypasses Unity Gateway tracing, grants, policies and guardrails. | Verified | [Genie Agent MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-agent), 2026-11-14. | GEN-016, GEN-017 | Do not promise conversation continuity or Unity Gateway governance for this adapter. |
| P-C-016 | Per-space Genie Agent MCP tool names and input schemas are fixed by the primary documentation. | Unverified | [Genie Agent MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-agent), 2026-11-14, does not specify a tool contract. | GEN-016, GEN-017 | Discover tools at runtime. Disable the adapter if the required ask and poll capabilities cannot be identified. |
| P-C-017 | Chat-mode start and follow-up operations are GA and use scope `genie`. | Verified | [Start a conversation](https://docs.databricks.com/api/genie/v1/genie-start-conversation.md) and [Send a message](https://docs.databricks.com/api/genie/v1/genie-create-conversation-message.md), 2026-11-14. | GEN-013 | Use the selected `space_id`. Start a new conversation after a backend switch. |
| P-C-018 | The GA chat-mode message GET operation returns message status and attachments. | Verified | [Get a conversation message](https://docs.databricks.com/api/genie/v1/genie-get-conversation-message.md), 2026-11-14. | GEN-013, GEN-014 | Poll the saved `message_id`. Handle `COMPLETED`, `FAILED`, `CANCELLED` and `QUERY_RESULT_EXPIRED`. |
| P-C-019 | Databricks recommends REST polling every 1–5 seconds, exponential backoff up to one minute, and a ten-minute limit for most queries. | Verified | [Use the Genie Agents API](https://docs.databricks.com/aws/en/genie-agents/conversation-api), 2026-11-14. | GEN-014 | Stop active polling at the local deadline without deleting the remote message. Offer manual resume. |
| P-C-020 | The GA attachment query-result operation requires a query attachment and message status `EXECUTING_QUERY` or `COMPLETED`. | Verified | [Get an attachment SQL result](https://docs.databricks.com/api/genie/v1/genie-get-message-attachment-query-result.md), 2026-11-14. | GEN-013 | Preserve statement metadata and truncated flags. Do not use the deprecated top-level `query_result`. |
| P-C-021 | GA chat-mode REST has a documented remote cancellation endpoint equivalent to Genie One MCP. | Unverified | The reviewed [Genie conversation reference](https://docs.databricks.com/api/genie/v1/conversation) and [chat-mode message reference](https://docs.databricks.com/api/genie/v1/genie-get-conversation-message.md), 2026-11-14, do not establish one. The documented [cancel endpoint](https://docs.databricks.com/api/genie/v1/genie-cancel-response.md) is explicitly agent mode. | GEN-015, GEN-017 | Advertise `remote_cancel=false` for chat-mode REST until a chat-mode operation is verified. Local stop must not be labelled remote cancellation. |

### 3.2 Databricks CLI identity and workspace routing

| ID | Architecture-changing claim | Status | Primary source and access date | Affected requirement | Fallback or required decision |
|---|---|---|---|---|---|
| P-C-022 | `databricks auth profiles` discovers profiles without printing access tokens. JSON output is supported. Current source defines `name`, `host`, optional `account_id`, optional `workspace_id`, `cloud`, `auth_type`, `valid` and optional `default`. | Verified | [CLI auth commands](https://docs.databricks.com/aws/en/dev-tools/cli/reference/auth-commands), [CLI profile source](https://github.com/databricks/cli/blob/main/cmd/auth/profiles.go), and [CLI acceptance output](https://github.com/databricks/cli/blob/main/acceptance/cmd/auth/profiles/output.txt), 2026-11-14. | WS-001, WS-002, WS-007 | Pin tested CLI versions. Reject unknown output schemas instead of guessing. |
| P-C-023 | `databricks auth describe --profile <name>` reports the resolved host and `auth_type`. U2M login writes `databricks-cli` as the auth type. | Verified | [CLI auth commands](https://docs.databricks.com/aws/en/dev-tools/cli/reference/auth-commands), 2026-11-14. | WS-003 | Reject PAT and M2M profiles in the MVP. |
| P-C-024 | `databricks auth login` starts browser OAuth U2M and can update an existing profile to `databricks-cli`. | Verified | [CLI auth commands](https://docs.databricks.com/aws/en/dev-tools/cli/reference/auth-commands) and [CLI authentication](https://docs.databricks.com/aws/en/dev-tools/cli/authentication), 2026-11-14. | WS-004 | Show the target host and profile before login. Never invoke login as hidden recovery. |
| P-C-025 | `databricks auth token` supports U2M only, refreshes an expired or near-expiry token, and returns `access_token`, `token_type` and `expiry`. | Verified | [CLI auth commands](https://docs.databricks.com/aws/en/dev-tools/cli/reference/auth-commands), 2026-11-14. | WS-005, WS-006 | Keep the returned token in native memory. Serialize token acquisition per profile. |
| P-C-026 | OAuth U2M access tokens expire after one hour. CLI v1.0.0 and later stores them in OS-native secure storage by default, with an optional plaintext fallback. | Verified | [CLI authentication](https://docs.databricks.com/aws/en/dev-tools/cli/authentication) and [OAuth U2M](https://docs.databricks.com/aws/en/dev-tools/auth/oauth-u2m), 2026-11-14. | WS-006, OPS-001 | Use a tested CLI version. Do not read or copy the CLI token cache into Motherbricks storage. |
| P-C-027 | A CLI-issued token works with Genie One MCP. | Verified | [Genie Code CLI](https://docs.databricks.com/aws/en/genie-code/genie-code-cli) shows `databricks auth token --profile <profile-name>` feeding the Genie One MCP URL. [Connect Codex](https://docs.databricks.com/aws/en/ai-gateway/coding-agent-codex) recommends CLI U2M for Unity Gateway and names `system.ai.genie_one_mcp`. Both accessed 2026-11-14. | WS-005, GEN-002, D-07 | Keep CLI identity as the MVP default. A custom OAuth app is only a contingency for a failed supported-environment test. |
| P-C-028 | The CLI token defaults to `all-apis`. | Verified | [Connect to an API Databricks app](https://docs.databricks.com/aws/en/dev-tools/databricks-apps/connect-local) and [OAuth U2M](https://docs.databricks.com/aws/en/dev-tools/auth/oauth-u2m), 2026-11-14. | WS-005 | Do not claim that Motherbricks mints a least-privilege `ai-gateway` token through the CLI. Record this as a security trade-off. |
| P-C-029 | Genie One MCP needs a workspace ID, which can come from the `x-databricks-org-id` response header when it is not already known. | Verified | [Genie One MCP server](https://docs.databricks.com/aws/en/agents/mcp-tools/genie-mcp), 2026-11-14. | WS-007 | Use `workspace_id` from supported profile JSON when present. Otherwise resolve and cache the header for the same host. |

### 3.3 Lakebase prerequisites

| ID | Architecture-changing claim | Status | Primary source and access date | Affected requirement | Fallback or required decision |
|---|---|---|---|---|---|
| P-C-030 | `databricks postgres generate-database-credential <endpoint-resource> --output json` returns a database token and `expire_time`. The credential expires after one hour. Expiry is enforced at login, not on an already open connection. | Verified | [Lakebase authentication](https://docs.databricks.com/aws/en/oltp/projects/authentication), 2026-11-14. | OPS-003, OPS-004 | Generate a fresh credential for each new physical connection. Keep completed transactions and queued changes during rotation. |
| P-C-031 | OAuth database authentication needs a corresponding Postgres role and a token from the same workspace that owns the project. | Verified | [Lakebase authentication](https://docs.databricks.com/aws/en/oltp/projects/authentication), 2026-11-14. | OPS-003, WS-009 | Pin the Lakebase configuration to its workspace. Reject cross-workspace credential use. |
| P-C-032 | The built-in PgBouncer connection pool does not support OAuth roles. | Verified | [Lakebase authentication](https://docs.databricks.com/aws/en/oltp/projects/authentication) and [Connection pooling](https://docs.databricks.com/aws/en/oltp/projects/connection-pooling), 2026-11-14. | OPS-006 | Use direct endpoints with an application-managed pool. Do not use the `-pooler` host for OAuth. |
| P-C-033 | Lakebase requires TLS. `sslmode=require` encrypts without server verification. `sslmode=verify-full` with `sslrootcert=system` also verifies the publicly trusted server certificate. | Verified | [Connect with psql](https://docs.databricks.com/aws/en/oltp/projects/connect-psql) and [Replicate data from Lakebase](https://docs.databricks.com/aws/en/oltp/projects/logical-replication-publication), 2026-11-14. | OPS-005 | Use `verify-full` and the system root store. Disable Lakebase sync if the driver cannot verify the server. |
| P-C-034 | Lakebase Search requires Postgres 16 or later. Enabling it restarts all project computes and is irreversible. `lakebase_vector` then installs with `CASCADE`, which installs pgvector. | Verified | [Lakebase Search](https://docs.databricks.com/aws/en/oltp/projects/lakebase-search) and [lakebase_vector](https://docs.databricks.com/aws/en/oltp/projects/lakebase-vector), 2026-11-14. | OPS-007 | Require an administrator maintenance action. Keep local recall available when Search is absent. |

### 3.4 Tauri and operating-system windows

| ID | Architecture-changing claim | Status | Primary source and access date | Affected requirement | Fallback or required decision |
|---|---|---|---|---|---|
| P-C-035 | Tauri v2 global shortcut exposes `Pressed` and `Released` states. The underlying crate supports Windows, macOS and Linux X11. | Verified | [Tauri global shortcut](https://v2.tauri.app/plugin/global-shortcut/), [ShortcutEvent](https://v2.tauri.app/reference/javascript/global-shortcut/), and [global-hotkey crate](https://docs.rs/global-hotkey/latest/global_hotkey/), 2026-11-14. | OPS-008, D-01 | Use the plugin for the first chord-shortcut spike. A custom hook is contingency, not a proven prerequisite. |
| P-C-036 | Press/release delivery, modifier release order, cancellation and stuck-key recovery are safe for Motherbricks on every supported machine without a platform test. | Unverified | The API contract exists in [Tauri global shortcut](https://v2.tauri.app/plugin/global-shortcut/), but it is not an application-level timing or recovery guarantee. Accessed 2026-11-14. | OPS-008 | Keep D-01 G2 as a release blocker. Add a watchdog and a native-hook contingency if the spike fails. |
| P-C-037 | The underlying global-hotkey crate documents Linux as X11 only. | Verified | [global-hotkey crate](https://docs.rs/global-hotkey/latest/global_hotkey/), 2026-11-14. | OPS-010 | Do not claim generic Linux Wayland support. Use a compositor-specific portal adapter in a later release. |
| P-C-038 | Tauri v2 documents transparent and undecorated windows, always-on-top placement, initial focus control, focusability and cursor-event ignoring. | Verified | [Tauri window API](https://docs.rs/tauri/latest/tauri/window/struct.Window.html) and [WindowBuilder](https://docs.rs/tauri/latest/tauri/window/struct.WindowBuilder.html), 2026-11-14. | OPS-009 | Use separate mascot and keyboard-capable panels. Verify actual focus and hit testing in the spike. |
| P-C-039 | `setVisibleOnAllWorkspaces` is unsupported on Windows. | Verified | [Tauri window API](https://docs.rs/tauri/latest/tauri/window/struct.Window.html), 2026-11-14. | OPS-009, OPS-010 | Do not require that API on Windows. Test topmost behavior across virtual desktops separately. |
| P-C-040 | macOS provides `NSWindow.StyleMask.nonactivatingPanel` for an `NSPanel` that does not activate its owning app. | Verified | [Apple nonactivatingPanel](https://developer.apple.com/documentation/appkit/nswindow/stylemask-swift.struct/nonactivatingpanel), 2026-11-14. | OPS-009, D-01 | Use a small AppKit adapter if the Tauri-only spike cannot preserve the target app's focus. |
| P-C-041 | Windows provides `WS_EX_NOACTIVATE` and `WS_EX_TOPMOST`. `WS_EX_NOACTIVATE` alone does not handle every mouse-activation path. | Verified | [Microsoft extended window styles](https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles) and [CreateWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw), 2026-11-14. | OPS-009, D-01 | Handle `WM_MOUSEACTIVATE` in a Win32 adapter when required by the spike. |
| P-C-042 | Tauri exposes native `NSWindow` and `HWND` handles for a platform adapter. | Verified | [Tauri macOS Window](https://docs.rs/tauri/latest/x86_64-apple-darwin/tauri/window/struct.Window.html) and [Tauri Windows Window](https://docs.rs/tauri/latest/x86_64-pc-windows-msvc/tauri/window/struct.Window.html), 2026-11-14. | OPS-009 | Isolate native window code behind one overlay interface. |
| P-C-043 | A standard Tauri configuration alone proves a click-through, non-activating mascot and a separate keyboard-capable response card on macOS and Windows. | Unverified | [Tauri window API](https://docs.rs/tauri/latest/tauri/window/struct.Window.html), [Apple nonactivatingPanel](https://developer.apple.com/documentation/appkit/nswindow/stylemask-swift.struct/nonactivatingpanel), and [Microsoft CreateWindowExW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-createwindowexw), 2026-11-14, document primitives but not the integrated Motherbricks behavior. | OPS-009, D-01 | Retain the two-panel native-adapter contingency and the G2 spike. |

## 4. Numbered requirements

Release values: `MVP`, `MVP fallback`, `Preview`, or `Later`.  
Verification values: `Automated`, `Platform`, `Inspection`, `Demonstration`, or `Primary source`.

### 4.1 GEN: Genie and backend adapters

| ID | Priority | Release | Requirement | Rationale | Acceptance | Verification |
|---|---|---|---|---|---|---|
| GEN-001 | MUST | MVP | Motherbricks shall construct the Genie One MCP URL from the selected HTTPS workspace host and its resolved workspace ID. | D-04, P-C-002 | P-AT-001, P-AT-012 | Automated |
| GEN-002 | MUST | MVP | The Genie One adapter shall use MCP streamable HTTP with the bearer token held in the native core. | P-C-003 | P-AT-001, P-AT-012 | Automated |
| GEN-003 | MUST | MVP | Motherbricks shall preflight workspace assignment and the documented Genie One MCP grants before enabling submission. | P-C-005 | P-AT-001 | Demonstration |
| GEN-004 | MUST | MVP | The Genie One adapter shall discover tool schemas and require `genie_ask`, `genie_poll_response`, `genie_get_query_result` and `genie_cancel_response`. | P-C-006 | P-AT-001 | Automated |
| GEN-005 | MUST | MVP | Motherbricks shall persist `conversation_id` and `response_id` with the originating target before the first poll. | P-C-007 | P-AT-002 | Automated |
| GEN-006 | MUST | MVP | A Genie One follow-up shall reuse only the conversation ID pinned to the active target. | P-C-007, D-08 | P-AT-002, P-AT-013 | Automated |
| GEN-007 | MUST | MVP | Motherbricks shall allow only one in-flight `genie_poll_response` call for a response ID. | P-C-008 | P-AT-002 | Automated |
| GEN-008 | MUST | MVP | The adapter shall label poll updates as polling states and preserve completed, incomplete, failed and unrecognized states distinctly. | P-C-009 | P-AT-002 | Automated |
| GEN-009 | MUST | MVP | Motherbricks shall request a full query result only after completion and with a documented query `item_id`. | P-C-012 | P-AT-003 | Automated |
| GEN-010 | MUST | MVP | Motherbricks shall show “cancellation requested” after `genie_cancel_response` returns and continue to track the same response. | P-C-010, P-C-011 | P-AT-004 | Automated |
| GEN-011 | MUST | MVP | Motherbricks shall reject `https://<host>/api/2.0/mcp/genie` as a backend URL. | P-C-013 | P-AT-005 | Automated |
| GEN-012 | MUST | MVP | Motherbricks shall include approved context in the next Genie question string because no independent context-upload tool is documented. | D-03, P-C-006 | P-AT-006 | Inspection |
| GEN-013 | MUST | MVP fallback | The chat-mode REST adapter shall use the explicitly selected `space_id` for start, follow-up, poll and query-result calls. | P-C-017, P-C-018, P-C-020 | P-AT-007 | Automated |
| GEN-014 | MUST | MVP fallback | The chat-mode REST adapter shall use serial polling with exponential backoff and a visible local polling deadline. | P-C-019 | P-AT-007 | Automated |
| GEN-015 | MUST | MVP fallback | The chat-mode REST adapter shall report remote cancellation as unavailable until a chat-mode cancellation operation is verified. | P-C-021 | P-AT-008 | Inspection |
| GEN-016 | SHOULD | Preview | Motherbricks shall discover per-space MCP capabilities at runtime and disable that adapter when required capabilities are absent. | P-C-014, P-C-015, P-C-016 | P-AT-009 | Automated |
| GEN-017 | MUST | MVP | Every backend adapter shall declare conversation history, query-result retrieval, deep-link and remote-cancellation capabilities explicitly. | P-C-015, P-C-016, P-C-021 | P-AT-008, P-AT-009 | Inspection |

### 4.2 WS: CLI profiles and workspace affinity

| ID | Priority | Release | Requirement | Rationale | Acceptance | Verification |
|---|---|---|---|---|---|---|
| WS-001 | MUST | MVP | Motherbricks shall invoke the Databricks CLI by executable path and argument array without a shell command string. | D-07 | P-AT-010 | Inspection |
| WS-002 | MUST | MVP | Motherbricks shall parse `databricks auth profiles --skip-validate --output json` using the versioned compatibility schema. | P-C-022 | P-AT-010 | Automated |
| WS-003 | MUST | MVP | Motherbricks shall reject a selected profile unless resolved `auth_type` is `databricks-cli`. | D-07, P-C-023 | P-AT-010 | Automated |
| WS-004 | MUST | MVP | Motherbricks shall start `databricks auth login` only after the user confirms the profile name and host. | D-07, P-C-024 | P-AT-011 | Demonstration |
| WS-005 | MUST | MVP | Motherbricks shall acquire tokens with `databricks auth token --profile <name> --timeout <bounded-duration>` and parse the returned expiry. | P-C-025, P-C-027 | P-AT-011, P-AT-012 | Automated |
| WS-006 | MUST | MVP | Motherbricks shall serialize token acquisition for each profile and reject an expired returned token. | P-C-025, P-C-026 | P-AT-011 | Automated |
| WS-007 | MUST | MVP | Motherbricks shall resolve the Genie One workspace ID from supported profile data or the same-host `x-databricks-org-id` response header. | P-C-002, P-C-029 | P-AT-013 | Automated |
| WS-008 | MUST | MVP | Motherbricks shall send a profile token only to the canonical HTTPS host bound to that profile. | D-07 | P-AT-012, P-AT-013 | Automated |
| WS-009 | MUST | MVP | Motherbricks shall pin a conversation to its profile, workspace, backend type, backend identifier and conversation identifier. | D-08, P-C-031 | P-AT-013 | Automated |
| WS-010 | MUST | MVP | A one-off workspace selection shall not invoke `databricks auth switch` or change the MOG home profile. | D-07, D-08 | P-AT-013 | Automated |
| WS-011 | MUST | MVP | Motherbricks shall not repeat `genie_ask` automatically after an authentication failure with uncertain request delivery. | D-12, P-C-007 | P-AT-014 | Automated |

### 4.3 OPS: Administration and platform prerequisites

| ID | Priority | Release | Requirement | Rationale | Acceptance | Verification |
|---|---|---|---|---|---|---|
| OPS-001 | MUST | MVP | The installer shall verify the Databricks CLI version against the tested compatibility manifest before enabling remote backends. | P-C-022, P-C-026 | P-AT-018 | Automated |
| OPS-002 | MUST | MVP | Administrator setup shall include the Genie One MCP grants and workspace assignment documented in P-C-005. | P-C-005 | P-AT-001 | Inspection |
| OPS-003 | MUST | Lakebase option | Motherbricks shall generate Lakebase credentials for the configured direct endpoint with a Postgres role in the same workspace. | P-C-030, P-C-031 | P-AT-015 | Platform |
| OPS-004 | MUST | Lakebase option | Motherbricks shall generate a fresh Lakebase credential before each new physical connection when the cached credential is absent or near expiry. | P-C-030 | P-AT-015 | Automated |
| OPS-005 | MUST | Lakebase option | Motherbricks shall connect to Lakebase with `sslmode=verify-full` and `sslrootcert=system`. | P-C-033 | P-AT-015 | Platform |
| OPS-006 | MUST | Lakebase option | Motherbricks shall use an application-managed pool on the direct Lakebase endpoint for OAuth authentication. | P-C-032 | P-AT-015 | Inspection |
| OPS-007 | MUST | Lakebase Search option | An administrator shall approve the irreversible Lakebase Search enablement and compute restart before Motherbricks installs `lakebase_vector`. | P-C-034 | P-AT-016 | Demonstration |
| OPS-008 | MUST | MVP | The release gate shall include passing macOS and Windows tests for global press, release, modifier release order, cancellation and stuck-key recovery. | P-C-035, P-C-036, D-01 | P-AT-017 | Platform |
| OPS-009 | MUST | MVP | Motherbricks shall pass separate non-activating mascot and keyboard-capable response-panel tests before enabling the floating UI. | P-C-038 through P-C-043, D-01 | P-AT-017 | Platform |
| OPS-010 | SHOULD | Later | Motherbricks shall advertise Linux support only for windowing systems named in the compatibility manifest. | P-C-037, P-C-039 | P-AT-018 | Inspection |
| OPS-011 | MUST | MVP | The release source scan shall reject the retired beta Genie MCP path and unreviewed preview dependencies. | P-C-013, P-C-014 | P-AT-005, P-AT-018 | Automated |
| OPS-012 | MUST | MVP | Lakebase unavailability shall not disable dictation, local recall or Genie operation. | D-06, P-C-034 | P-AT-016 | Demonstration |

## 5. Acceptance tests

These are required procedures, not completed test results. Live tests must use dedicated test workspaces and test data. Evidence must redact tokens, user identifiers, workspace IDs and result content that is not approved for the test report.

### P-AT-001: Genie One connection and grants

- **Links:** GEN-001, GEN-002, GEN-003, GEN-004, OPS-002.
- **Release/platform:** MVP; macOS and Windows; live integration plus automated schema test.
- **Preconditions:** One test user has workspace assignment but lacks `EXECUTE`. A second test user has all documented grants.
- **Actions:** Run preflight as the first user. Run preflight as the second user. Call MCP `initialize` and `list_tools`. Compare the discovered schemas with the adapter fixtures.
- **Expected:** The first user receives an administrator action and no submission control. The second user connects through streamable HTTP. All four required tools are present. `view_ask` is not required.
- **Evidence:** Redacted preflight result, effective-grant output, protocol trace without tokens, and discovered schema hash.

### P-AT-002: Ask, poll and follow up

- **Links:** GEN-005, GEN-006, GEN-007, GEN-008.
- **Release/platform:** MVP; platform-neutral adapter test with a controlled MCP fixture and one live confirmation.
- **Preconditions:** The fixture delays completion through five polls and records request timing.
- **Actions:** Submit a question. Schedule polls at a rate that would overlap without a lock. Complete the first response. Submit a follow-up with the saved conversation ID. Inject an unrecognized state in a separate fixture run.
- **Expected:** Conversation and response IDs are stored before polling. No poll overlaps another poll for that response. The follow-up uses the same target and conversation ID. The unknown state remains visible and does not become `completed`.
- **Evidence:** ID binding record, timestamped call trace, rendered state sequence and fixture log.

### P-AT-003: Query result and truncation

- **Links:** GEN-009.
- **Release/platform:** MVP; automated fixture and live integration.
- **Preconditions:** A completed response contains a valid `pub_..._output` item ID and a separate `query_...` embed marker. One live result exceeds the inline result limit.
- **Actions:** Request the result with the valid item ID. Repeat with the embed marker. Inspect the large-result rendering.
- **Expected:** The valid ID returns schema and rows. The embed marker is not substituted. Truncation is visible. The full Databricks link remains available.
- **Evidence:** Redacted tool arguments, row counts, schema summary, truncation flag and link-target validation.

### P-AT-004: Cancellation with uncertain outcome

- **Links:** GEN-010.
- **Release/platform:** MVP; automated fixture and live integration.
- **Preconditions:** One response is in progress. The fixture can return success, transport failure or a late completed result after cancellation.
- **Actions:** Request cancellation. Drop the cancellation response in one run. Return a late completed response in another run.
- **Expected:** The UI first shows “cancellation requested.” It does not claim remote cancellation. A transport failure leaves the original IDs available. A late completed response is shown as completed rather than discarded.
- **Evidence:** Cancellation call trace, poll sequence, state transitions and rendered labels.

### P-AT-005: Retired endpoint rejection

- **Links:** GEN-011, OPS-011.
- **Release/platform:** MVP; automated static and unit test on every build platform.
- **Preconditions:** Test configurations contain the retired beta URL, the per-space preview URL and the GA Genie One URL.
- **Actions:** Validate each configuration. Scan source, default configuration and release documentation for the retired path.
- **Expected:** The beta URL is rejected. The per-space URL requires its preview flag. The GA URL passes. No release default contains the retired path.
- **Evidence:** Validation results and source-scan report. A path string used only as a negative test fixture must be allowlisted by test ID.

### P-AT-006: Context uses the approved question payload

- **Links:** GEN-012.
- **Release/platform:** MVP; automated payload test and inspection.
- **Preconditions:** Context sharing is disabled in the first run and enabled with two approved items in the second run.
- **Actions:** Submit the same question in both runs. Inspect outbound MCP calls.
- **Expected:** The first call contains no context item. The second call contains the bounded context envelope in `genie_ask.question`. No call claims to upload a separate memory store.
- **Evidence:** Redacted payload snapshots and tool-call inventory.

### P-AT-007: Chat-mode REST fallback

- **Links:** GEN-013, GEN-014.
- **Release/platform:** MVP fallback; automated fixture and live integration.
- **Preconditions:** Two Genie spaces contain different test data. The fixture returns intermediate states before `COMPLETED` and later returns `QUERY_RESULT_EXPIRED`.
- **Actions:** Start a conversation in space A. Send a follow-up. Poll through completion. Fetch the attachment result. Attempt to reuse the conversation with space B. Simulate a ten-minute inconclusive poll run.
- **Expected:** Every URL contains space A. Space B requires a new conversation. Query results use the attachment endpoint. The local deadline preserves the remote identifiers and offers manual resume.
- **Evidence:** Redacted URL trace, poll intervals, backoff sequence, attachment ID binding and resume state.

### P-AT-008: Backend cancellation capabilities

- **Links:** GEN-015, GEN-017.
- **Release/platform:** MVP and MVP fallback; inspection plus automated capability test.
- **Preconditions:** One Genie One adapter and one chat-mode REST adapter are configured.
- **Actions:** Query both capability descriptors. Select Stop for a pending response on each adapter.
- **Expected:** Genie One exposes a cancellation request. Chat-mode REST exposes local stop only. The UI does not label local stop as remote cancellation. No agent-mode cancellation URL is generated for chat mode.
- **Evidence:** Capability descriptors, generated URL inventory and rendered labels.

### P-AT-009: Per-space preview discovery

- **Links:** GEN-016, GEN-017.
- **Release/platform:** Preview; automated fixture and optional live preview workspace.
- **Preconditions:** Managed MCP Servers is disabled in one workspace and enabled in another. One fixture omits a usable poll tool.
- **Actions:** Run adapter discovery in each condition. Inspect history and governance capability labels.
- **Expected:** A disabled preview produces a setup message. A missing capability disables the adapter. A valid connection is marked read-only, no-history and outside Unity Gateway governance.
- **Evidence:** Discovered tool list, capability descriptor and redacted error result.

### P-AT-010: CLI discovery and U2M filtering

- **Links:** WS-001, WS-002, WS-003.
- **Release/platform:** MVP; macOS and Windows automated process test.
- **Preconditions:** Test CLI configuration contains U2M, PAT, M2M, malformed and duplicate-host profiles.
- **Actions:** Discover profiles without validation. Inspect the process invocation. Select each profile type. Supply JSON with an unknown schema version or missing host.
- **Expected:** No shell interprets the arguments. Only `databricks-cli` profiles are eligible. Duplicate hosts require an explicit profile choice. Unknown schemas fail closed without a token request.
- **Evidence:** Argument array, redacted JSON fixtures, eligibility results and error states.

### P-AT-011: Expired profile recovery

- **Links:** WS-004, WS-005, WS-006.
- **Release/platform:** MVP; macOS and Windows platform test.
- **Preconditions:** A MOG has saved non-secret configuration. Its CLI refresh credential is expired or revoked. Two token requests arrive concurrently.
- **Actions:** Attempt token acquisition. Cancel the first login prompt. Confirm the second login prompt. Complete browser login. Repeat the concurrent token request with a near-expiry fixture.
- **Expected:** Login never starts without confirmation. Cancellation preserves the MOG configuration. Successful login preserves the selected profile. Token acquisition is single-flight. An expired returned token is rejected.
- **Evidence:** Process trace, prompt sequence, redacted expiry values and configuration hash before and after recovery.

### P-AT-012: CLI token with Genie One MCP

- **Links:** GEN-001, GEN-002, WS-005, WS-008.
- **Release/platform:** MVP; live macOS and Windows integration.
- **Preconditions:** A U2M CLI profile has the required grants. Network instrumentation can observe the destination host without recording the token.
- **Actions:** Acquire a token with the documented CLI command. Initialize Genie One MCP. Run a read-only test question. Force token expiry and acquire another token before a new request.
- **Expected:** The CLI token initializes the MCP and submits the question. The token appears only in the authorization header to the bound HTTPS host. Refresh preserves the profile. No token enters the webview, logs or persistent MOG data.
- **Evidence:** Redacted destination trace, MCP initialization result, secret scan and refreshed expiry metadata.

### P-AT-013: Workspace ID and conversation affinity

- **Links:** GEN-006, WS-007, WS-008, WS-009, WS-010.
- **Release/platform:** MVP; two test workspaces; automated and live integration.
- **Preconditions:** Profile A and profile B use different hosts. One profile supplies `workspace_id`; the other requires header discovery.
- **Actions:** Resolve both IDs. Start a conversation in workspace A. Select workspace B for a one-off question. Attempt to attach the old conversation ID to workspace B.
- **Expected:** Each MCP URL contains the ID for its own host. Workspace B receives only profile B's token. The old conversation is not reused. The one-off selection does not change the MOG home profile or CLI default.
- **Evidence:** Redacted host-to-ID bindings, destination trace, conversation affinity record and default-profile hash.

### P-AT-014: Authentication failure without duplicate submission

- **Links:** WS-011.
- **Release/platform:** MVP; automated transport-failure fixture.
- **Preconditions:** The fixture can return 401 before acceptance and can drop the connection after accepting `genie_ask`.
- **Actions:** Trigger both failures. Refresh the CLI token. Inspect subsequent application actions.
- **Expected:** The application marks authentication stale. It does not repeat an accepted or uncertain `genie_ask`. The user can start a new question explicitly after reviewing the state.
- **Evidence:** Tool-call count, delivery-state record and user-action trace.

### P-AT-015: Lakebase rotation, TLS and pooling

- **Links:** OPS-003, OPS-004, OPS-005, OPS-006.
- **Release/platform:** Lakebase option; macOS and Windows platform test with a test project.
- **Preconditions:** The test identity has a same-workspace OAuth role. A local queue contains one unsent memory mutation. The driver can inspect TLS verification.
- **Actions:** Open a pooled direct connection. Advance the credential beyond its refresh threshold. Open another connection. Attempt the `-pooler` host with OAuth. Attempt a connection with certificate verification disabled. Revoke network access during rotation and restore it.
- **Expected:** New physical connections receive valid credentials. The direct endpoint uses `verify-full` and the system root store. OAuth does not use the built-in pooler. Unverified TLS fails closed. The queued mutation survives and synchronizes once after reconnection.
- **Evidence:** Redacted endpoint type, TLS mode, connection-generation count, queue journal and duplicate check.

### P-AT-016: Lakebase Search administration and degradation

- **Links:** OPS-007, OPS-012.
- **Release/platform:** Lakebase Search option; administrator demonstration and automated degradation test.
- **Preconditions:** A disposable Postgres 16 project has active test connections. Lakebase Search is disabled.
- **Actions:** Start Search enablement without administrator approval. Repeat with approval. Record the compute restart. Install `lakebase_vector CASCADE`. Disable network access and perform dictation, local recall and a Genie question.
- **Expected:** No irreversible action occurs without approval. The restart is disclosed. The extension and pgvector become available only after enablement. Network loss does not disable dictation or local recall. A Genie question shows a connectivity error without changing the Genie adapter configuration.
- **Evidence:** Approval record, maintenance event, extension versions and offline operation results.

### P-AT-017: Tauri hotkey and overlay spike

- **Links:** OPS-008, OPS-009.
- **Release/platform:** MVP; current supported macOS and Windows releases on named hardware.
- **Preconditions:** A Tauri v2 spike has a chord shortcut, transparent topmost mascot, separate keyboard-capable card, focus logger and simulated audio source.
- **Actions:** Run 100 press/release cycles with another application focused. Release modifiers before the primary key. Press Escape. Simulate focus loss and a missing release event. Click transparent and opaque mascot regions. Open the card and type. Restore the target application.
- **Expected:** Each accepted press has one terminal release or watchdog stop. No capture remains hidden. The mascot does not steal target focus. The card accepts keyboard input only when intentionally opened. Focus returns to the prior target. Hotkey visual response has p95 at or below 100 ms. Any failure activates the native-adapter decision.
- **Evidence:** Platform and Tauri versions, event trace, focus trace, timing samples, screen recording and native-adapter decision record.

### P-AT-018: Compatibility and release-source gate

- **Links:** OPS-001, OPS-010, OPS-011.
- **Release/platform:** MVP and Later; automated build test.
- **Preconditions:** The compatibility manifest lists tested CLI versions, desktop operating systems and preview flags.
- **Actions:** Test one listed CLI version and one unlisted version. Scan backend URLs, feature flags and platform claims. Inspect Linux wording.
- **Expected:** An unlisted CLI version blocks remote backends with an actionable message. Preview adapters remain disabled unless explicitly enabled. Linux wording does not imply Wayland support without a listed adapter and test result.
- **Evidence:** Manifest version, compatibility results, source-scan report and release-note excerpt.

## 6. Dependencies and interface consumers

| Interface or decision | Consumer | Team P contract |
|---|---|---|
| `BackendAdapter` capabilities | Team A, Team U | Expose polling, history, query results, deep links and remote cancellation independently. Do not infer one capability from another. |
| Genie poll scheduler | Team A, Team U | Use one single-flight operation per response. Preserve IDs through cancellation and restart. |
| `AuthProvider` CLI implementation | Team A, Team S, Team U | Use explicit profile selection, bounded process execution, returned expiry and host binding. Keep tokens in native memory. |
| Workspace switching | Team A, Team S, Team U | Resolve workspace ID for the selected host. Pin existing conversations. Never call `auth switch` for a one-off selection. |
| Lakebase connection factory | Team M, Team S | Generate credentials for new direct connections. Use `verify-full`. Use application-managed pooling for OAuth. |
| Floating overlay | Team V, Team U | Test Tauri primitives first. Use the native AppKit or Win32 adapter only behind the same overlay interface. |
| Administrator guide | Team S, Team U | Include Genie grants, CLI version requirements, Lakebase role setup and the irreversible Search enablement warning. |

## 7. Collisions requiring director review

### P-X-001: Chat-mode REST fallback cannot inherit Genie One cancellation

- **Decision at risk:** D-04 and the shared adapter contract.
- **Evidence:** P-C-010 and P-C-021.
- **Recommended direction:** Keep Genie One cancellation. Set `remote_cancel=false` for chat-mode REST. Let local stop hide active polling while preserving the remote response ID. Do not use the agent-mode cancel path for chat mode.
- **Severity:** Blocking for the adapter contract. Not existential for the project.

### P-X-002: D-08 release label for agent-mode API needs a fresh decision

- **Decision at risk:** The statement “Genie One API or Genie Agent API: Later beta adapter.”
- **Evidence:** Genie One response fields remain marked Beta in the [Genie One response object](https://docs.databricks.com/api/genie-one/v1/response/object.md), accessed 2026-11-14. The current [agent-mode cancellation reference](https://docs.databricks.com/api/genie/v1/genie-cancel-response.md) is marked GA, while the [agent-mode create-response reference](https://docs.databricks.com/api/genie/v1/agent-mode-create-response) still reports preview-related feature errors and does not expose a clear launch-stage label in the fetched content. Both accessed 2026-11-14.
- **Recommended direction:** Keep both APIs outside the MVP. Do not call the whole agent-mode API Beta or GA from the cancellation page alone. Assign a later connector review that records launch status per operation and workspace prerequisite.
- **Severity:** Later-release documentation collision. It does not change the Genie One MCP default.

### P-X-003: A custom global hook is not proven necessary

- **Decision at risk:** Any interpretation of D-01 that requires a custom native hotkey hook before the spike.
- **Evidence:** P-C-035 verifies `Pressed` and `Released` in the Tauri plugin. P-C-036 leaves application-level reliability unverified.
- **Recommended direction:** Test the Tauri plugin first. Keep a watchdog. Use a native hook only for a measured failure or a binding that the plugin cannot support.
- **Severity:** Implementation simplification. The Tauri decision itself remains unchanged.

### P-X-004: Lakebase OAuth pooling must use direct connections

- **Decision at risk:** Any Lakebase design that assumes the built-in PgBouncer endpoint with CLI U2M-derived OAuth credentials.
- **Evidence:** P-C-032.
- **Recommended direction:** Use direct endpoints and an application-managed pool. Rotate credentials when creating physical connections. Do not introduce native Postgres passwords as a silent fallback.
- **Severity:** Blocking for the Lakebase connection design. Local-first operation remains unaffected.

## 8. Open issues

### Blocking

1. Team V must complete P-AT-017 on macOS and Windows before D-01 is considered proven.
2. Team A must adopt per-adapter remote-cancellation capabilities before the UI contract is frozen.
3. Team M must adopt direct OAuth connections and credential rotation before the Lakebase schema and sync work is frozen.
4. A live CLI-to-Genie-One test must pass in each supported cloud and workspace configuration before OPS-001 is finalized.

### User confirmation

No existential user question is open from Team P. The CLI identity decision can remain unchanged. Lakebase remains optional. Desktop remains the first release.

### Later release

1. Per-space MCP tool discovery and governance labeling.
2. Linux Wayland global shortcuts through a supported portal adapter.
3. Per-operation launch-status verification for Genie One API and agent-mode API adapters.
4. A least-privilege alternative to the CLI's default `all-apis` token if Team S requires it.

## 9. Team P exit assessment

- Every MVP external dependency in the assigned scope has a current primary citation or a release-blocking test.
- Every unverified claim has a feature flag, an explicit fallback or a removal recommendation.
- No requirement uses the retired beta MCP endpoint.
- No exact model name or unverified per-space tool schema is presented as fact.
- D-07 does not need to reopen the custom OAuth application decision on current evidence.
