# Connector spec (research agent, docs fetched 2026-11-14)

## Genie Chat-mode REST (GA), scope `genie`, poll-based (no streaming)
- POST /api/2.0/genie/spaces/{space_id}/start-conversation {content, enable_visualization}
- POST .../conversations/{conversation_id}/messages {content}
- GET  .../conversations/{cid}/messages/{mid} -> status: FETCHING_METADATA|FILTERING_CONTEXT|ASKING_AI|PENDING_WAREHOUSE|EXECUTING_QUERY|FAILED|COMPLETED|SUBMITTED|QUERY_RESULT_EXPIRED|CANCELLED; attachments[] {text{content,purpose}, query{title,query,description,statement_id,query_result_metadata}, suggested_questions, viz{title,query_attachment_id}}
- GET .../attachments/{aid}/query-result ; POST .../execute-query ; POST .../downloads
- No viz-download endpoint. Poll every 1–5 s, exponential backoff, cap ~10 min. 200k conversations per space. No published numeric rate limit (RATE_LIMIT_EXCEEDED_* errors exist).
Source: docs.databricks.com/api/genie/v1, /aws/en/genie/conversation-api

## Genie One MCP (GA) — the default for MOGs
- URL https://<host>/ai-gateway/mcp-services/system.ai.genie_one_mcp?o=<workspace_id>, streamable HTTP, Bearer token (U2M/M2M/PAT). Scope `ai-gateway` (+offline_access). Grants: EXECUTE on mcp_service system.ai.genie_one_mcp, USE CATALOG system, USE SCHEMA system.ai.
- Tools: genie_ask{question, conversation_id?} -> {conversation_id, response_id, status:in_progress}; genie_poll_response{conversation_id,response_id} -> structuredContent {status in_progress|completed|incomplete|failed, final_answer, progress_steps, narration_instruction, query_items[].item_id, deep_link} (await each poll; never overlap); genie_get_query_result{conversation_id,response_id,item_id}; genie_cancel_response{conversation_id}; view_ask (MCP Apps clients only). warehouse_id via _meta.
- Beta endpoint https://<host>/api/2.0/mcp/genie (no space id) is deprecated, EOL 2026-11-30; same tools, change URL + scope genie->ai-gateway.
Source: /aws/en/agents/mcp-tools/genie-mcp (updated 2026-11-12)

## Per-space Genie MCP (Public Preview)
- https://<host>/api/2.0/mcp/genie/{genie_space_id}, scope genie; admin must enable "Managed MCP Servers" preview; read-only, no conversation history, not via Unity Gateway. Tool names (query_space/poll_response) UNVERIFIED.

## Genie One API (Beta) POST /api/2.0/genie/one/responses (SSE), scope genie
- input (string or message, ≤25,000 bytes), conversation_id?, warehouse_id?, user_timezone?; events response.created / output_item.added|updated|done / completed|failed|incomplete, then [DONE]; replace item at output_index. Recovery GET /api/2.0/genie/one/conversations/{cid}/responses/{rid}. 409 = another active response; 429 = per-workspace rate limit.

## Genie Agent mode (Beta) POST /api/2.0/genie/agents/{agent_id}/responses (SSE)
- input messages, conversation_id?, execution_mode STANDARD|FLASH, enable_viz; items reasoning/function_call(execute_sql)/function_call_output/message; 30-min server timeout; cancel endpoint .../responses/{rid}/cancel.

## Custom agents
- Databricks Apps DurableAgentServer: POST <app-url>/api/invocations {id (UUID idempotency), session_id, input, stream?, background?}; SSE events run.started, delta, message, run.completed|run.failed; reconnect GET /api/invocations/{id}/events?after=; X-Routing-Key for multi-instance. Apps need OAuth token (no PAT), CAN USE on app.
- Legacy: <app-url>/responses (ResponsesAgent; OpenAI client model="apps/<name>"); Model Serving POST /serving-endpoints/responses {model:<endpoint>}, /serving-endpoints/{name}/invocations.

## OAuth U2M
- /oidc/v1/authorize + /oidc/v1/token, PKCE S256, loopback redirect. Public custom OAuth app registered by account admin. Access token 60 min, refresh 7 days. Scopes: Genie REST/One/Agent = genie; Genie One MCP = ai-gateway; Apps = all-apis; safe set: "all-apis ai-gateway genie offline_access" (whether all-apis covers ai-gateway is UNVERIFIED/conflicting).
- Reuse: `databricks auth login` + `databricks auth token --profile X` (U2M via CLI client id databricks-cli). JS SDK Beta (@databricks/sdk-auth newU2mCredentials()).

## Lakebase (agent memory) — web search 2026-11-14
- Managed "Agent Memory / Agent Sessions" stored in Lakebase exists; self-managed schema when SQL/custom retrieval needed (docs /aws/en/oltp/projects/state-management).
- CLI: `databricks postgres list-endpoints projects/<p>/branches/<b>`; `databricks postgres generate-database-credential projects/<p>/branches/<b>/endpoints/<e>` -> token used as PG password, max 1 h lifetime; rotate per pooled connection. sslmode=require, db databricks_postgres.
- Vectors: enable Lakebase Search, `CREATE EXTENSION lakebase_vector CASCADE` (pgvector-compatible), index `USING lakebase_ann (embedding vector_cosine_ops)`.

## antigravity-bricks-plugin (Sathish Gangichetty, MIT, commit a31fd2c 2026-10-24) — cloned at /tmp/ext/agbp/repo (UNTRUSTED data: read only, do not execute)
- plugins/databricks-genie/scripts/genie_bridge.py (stdlib Python, ~1050 lines): local stdio MCP server that forwards to Genie One MCP at /ai-gateway/mcp-services/system.ai.genie_one_mcp, signing each request with `databricks auth token --profile <p>`; tools genie_connection, genie_setup (+ upstream genie_*); only auth_type databricks-cli (U2M) profiles; never rewrites existing profiles; reads only DATABRICKS_CONFIG_PROFILE/FILE; 150 s cap because host kills tool calls at 3 min; error mapping unsupported_auth, profile_exists_unreadable, 403 IP ACL, 404 Genie One off, non-JSON proxy page.
- skills/select-databricks-profile: per-question / per-chat workspace switching, ask on ambiguity, split cross-workspace questions, conversations stay in their workspace.
