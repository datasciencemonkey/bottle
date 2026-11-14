# Team M: Motherbricks memory requirements

Date: 2026-11-14  
Owner: Team M (Memory)  
Status: Round 1 specialist deliverable  
Scope: D-03, D-06 and D-14  
Primary deliverable: local-first memory with optional Lakebase synchronization

## 1. Executive decision

Motherbricks shall use an encrypted local store for interactive memory.
Lakebase shall provide durable storage when the user configures synchronization.
Local recall shall not wait for a network operation.

The MVP shall use a Motherbricks-owned Lakebase schema.
The schema shall support stable identifiers, provenance, retention, tombstones and idempotent synchronization.
The current managed Agent Memory service shall not be the MVP system of record.
It is Beta.
Its store-level access model and non-atomic `put()` behavior do not match the complete Motherbricks data model.
A later adapter can support it after an interface spike.

Lakebase Search can provide remote vector search.
It is not required for local dictation, local exact recall or basic Lakebase synchronization.
Local semantic recall shall use a local embedding index.
Remote `lakebase_vector` shall support rebuild, administrator-approved remote search and optional synchronization of derived embeddings.

The name `approved_context` means that the user saved an item for later recall.
It does not grant permission to transmit that item.
Each outbound context payload requires a separate approval under D-03.

No existential user question blocks this design.

## 2. Primary-source findings

Access date for every source in this section: 2026-11-14.

### 2.1 Lakebase credentials and connections

| Finding | Status | Design effect |
|---|---|---|
| Lakebase OAuth database credentials expire one hour after generation. | Verified | Generate a credential before a new physical connection. Do not store it as MOG configuration. |
| Expiration is enforced at login. An open connection can remain active after the credential expires. | Verified | Rotation must not discard a healthy transaction. Reconnect when the server closes a connection. |
| `databricks postgres generate-database-credential <endpoint-resource> --output json` returns `token` and `expire_time`. | Verified | The native core can invoke the CLI without a shell command string. Use the endpoint resource for the bound workspace. |
| OAuth database credentials are workspace-scoped. Cross-workspace token authentication is not supported. | Verified | Bind each credential request and database connection to the same canonical workspace. |
| The built-in PgBouncer pooler does not support OAuth authentication. | Verified | Use the direct Lakebase endpoint for the MVP OAuth adapter. Implement pooling inside Motherbricks with a fresh credential for each new physical connection. |
| Lakebase connections have a 24-hour idle timeout and a three-day maximum connection life. | Verified | Treat connection loss as normal. Retry only safe synchronization operations. |
| Lakebase requires TLS. `sslmode=require` encrypts but does not verify the server. `sslmode=verify-full` with `sslrootcert=system` verifies the publicly trusted server certificate. | Verified | Require `verify-full` and the system root store. Do not fall back to `require`. |

The token-rotation example in the authentication guide refreshes two minutes before expiry.
Motherbricks shall use that two-minute safety window at minimum.
The application shall generate a fresh credential for each new physical connection.
It shall not close an active transaction only because the login credential reached its expiry time.

### 2.2 Lakebase Search and `lakebase_vector`

| Finding | Status | Design effect |
|---|---|---|
| Lakebase Search is GA on AWS as of 2026-09-18. | Verified for AWS | Verify support in each target cloud before enablement. Do not infer the same status in every cloud. |
| Lakebase Search requires Postgres 16 or later. | Verified | Check the server version during capability discovery. |
| Enablement restarts all computes and drops active connections. | Verified | Require an administrator action outside interactive recall. Warn about the restart. |
| Enablement cannot be reversed. | Verified | Motherbricks shall not enable Lakebase Search automatically. |
| `CREATE EXTENSION IF NOT EXISTS lakebase_vector CASCADE` installs `lakebase_vector` and its pgvector dependency. | Verified | Run migrations through an authorized administrator or a migration role. |
| `lakebase_vector` supplies the `lakebase_ann` index and supports pgvector vector types, distance operators and query syntax. | Verified | Keep the embedding provider separate from the storage provider. |
| Embeddings are generated externally. The vector column and index must match the model output dimension. | Verified | Store model identity and source content hash with each derived embedding. Do not fix a dimension in this requirements document. |
| `lakebase_ann.prefilter` can apply non-vector filters before full-precision reranking. | Verified | Use exact scope predicates in every query. Treat prefiltering as an optimization, not an authorization control. |
| Some tuning functions do not support a role subject to RLS. `lakebase_ann_latency` applies RLS. | Verified | Tune on an approved test corpus. Measure authorized query latency with the production security posture. |

Basic durable synchronization shall work without Lakebase Search.
A deployment without Search shall retain exact and structured remote queries.
It shall rebuild local embeddings from memory text when an approved embedding model is available.

### 2.3 Managed Agent Memory and Agent Sessions

| Finding | Status | Design effect |
|---|---|---|
| Agent Memory and Agent Sessions are Beta. An administrator must enable Agent Memory Sessions in Previews. | Verified | Do not make either service an MVP dependency. |
| Agent Memory automatically provisions Lakebase storage. It provides long-term memory across conversations. | Verified | Evaluate it as a later managed adapter. |
| An entry is identified by `actor_id`, optional `session_id` and `path`. It contains free-form `content` and a retrieval `description`. | Verified | A managed adapter would need a lossless mapping for Motherbricks identifiers and provenance. |
| Search combines BM25 and semantic matching. It returns at most 100 entries and has no pagination. | Verified | Do not use managed search for a complete export or an unbounded rebuild. |
| Access control is store-level. `actor_id`, `session_id`, `path` and `metadata` are not access controls. | Verified | Do not place mutually isolated users in one managed store. Use a separate store for each security boundary if this adapter is later approved. |
| Any workspace user can list and get memory stores, including IDs and descriptions. | Verified | Keep sensitive content out of store IDs and descriptions. |
| `put()` is not atomic. Concurrent writes to the same path can overwrite each other. | Verified | Do not use managed `put()` as the authoritative conflict-control mechanism. |
| A memory pipeline can rewrite, merge, split or delete entries written directly by an application. | Verified | Do not attach a pipeline to a Motherbricks authoritative store without a revision and provenance contract. |
| Agent Sessions store short-term interaction state. Agent Memory stores long-term memory. The stores are independent. | Verified | Do not equate a pinned Genie conversation with permanent MOG memory. |
| The earlier Unity Catalog managed memory API is legacy and is marked for turndown. | Verified | Do not use `/api/2.1/unity-catalog/memory-stores` for Motherbricks. |

The self-managed Agent Memory guide is labeled `Legacy`.
The same guide still directs users to direct SQL when they need a custom schema.
Motherbricks needs that custom schema.
This design therefore uses the current Lakebase project interfaces, not the legacy Unity Catalog memory API or legacy agent-server templates.
The director should preserve this distinction in the response document.

## 3. Architecture and trust boundaries

### 3.1 Store roles

| Store | Role | Network on recall path | Contains authoritative content |
|---|---|---|---|
| Encrypted local memory store | Interactive reads, local writes, active working set and outbound queue | No | Yes for local-only and unsynchronized records |
| Lakebase Motherbricks schema | Durable records, tombstones, change sequence and optional cross-device recovery | Only during explicit sync, rebuild or remote search | Yes for synchronized records |
| Managed Agent Memory | Optional later adapter for durable long-term entries | Yes | No in the MVP |
| Genie conversation | Backend-owned conversation context | Yes | No for MOG memory |
| Local derived search index | Exact and semantic candidate generation after local decryption | No | No. It is rebuildable |

The local store and Lakebase can both contain authoritative records.
The synchronization protocol defines which revision is current.
A derived embedding, a cached response blurb and a search ranking are never authoritative sources.

### 3.2 Local storage decision

The MVP should use SQLite with SQLCipher in the native core.
The Security team must approve the final cryptographic library and key-handling design.
Each MOG shall have a unique random data-encryption key.
An installation key-encryption key in the operating-system keychain shall wrap each per-MOG key.
The Security team defines the equivalent wrapped-key approach in SEC-029 and SEC-030.
The database, the local index files and the outbound queue shall remain encrypted at rest.
An unwrapped key shall not be stored next to the database.
The webview shall not receive the key or a database credential.

The local store shall use transactions.
A memory mutation and its outbound queue entry shall commit in one transaction.
A search-index update can follow the memory commit because the index is rebuildable.

### 3.3 Scope model

Every memory belongs to this exact scope tuple:

`(canonical_workspace_id, verified_owner_actor_id, mog_id)`

The canonical workspace ID is the scope key.
A CLI profile alias, workspace display name and hostname are not scope keys.
The source workspace of a backend response is stored separately as provenance.
This distinction lets a MOG record which workspace produced an answer without widening recall.

Local-only records that have no authenticated workspace shall use a local installation scope.
The user shall explicitly assign or copy those records before workspace synchronization.

A one-off workspace selection shall not change the scope of existing memories.
Cross-MOG recall shall require an explicit copy or an approved shared-memory feature.
Cross-workspace recall shall require an explicit copy.
A copy into a new scope shall receive a new `memory_id`.
Its provenance shall identify the source memory and the user approval.

## 4. Memory types and write policy

| Type | Purpose | Default sensitivity | Default retention class | Default synchronization | Default outbound policy |
|---|---|---|---|---|---|
| `user_preference` | A preference that the user explicitly states or confirms | `sensitive` | `pinned` | Allowed when sync is enabled | Ask for each payload |
| `meeting_summary` | A user-reviewed summary of a meeting | Highest sensitivity of its sources | `days_90` | Allowed when sync is enabled | Ask for each payload |
| `approved_context` | A user-saved context item for later recall | `sensitive` | `days_30` | Allowed when sync is enabled | Ask for each payload |
| `question` | A cleaned question and its routing metadata | `sensitive` | `days_30` | Allowed when sync is enabled | Ask for each payload |
| `response_reference` | A response blurb, identifiers and a validated Databricks link | `sensitive` | `days_30` | Allowed when sync is enabled | Ask for each payload |
| `mog_configuration` | Non-secret MOG settings | `standard` unless a value is sensitive | `mog_lifetime` | Allowed when sync is enabled | Never send as context by default |

The following artifacts use separate artifact records:

- Raw audio.
- Raw transcript segments.
- Cleaned transcript segments.
- Full backend responses when the user retains them.

Raw audio is not durable memory by default.
A meeting summary does not automatically approve the retention of its source audio.
A meeting recording does not automatically approve a durable summary.

### 4.1 Memory write approval

An explicit `Save memory` command is a write approval.
An explicit preference correction is a write approval.
A model, transcript, Genie response or memory pipeline cannot grant write approval to itself.
Model-derived long-term memories shall start as local drafts.
The user shall confirm a draft before it becomes an active durable memory.

Operational transcript artifacts can follow the meeting retention policy without becoming agentic memories.
Motherbricks shall show the type, scope and retention class before a durable memory save when the save was model-proposed.

### 4.2 Sensitivity and sharing labels

| Label | Meaning | Required behavior |
|---|---|---|
| `standard` | Ordinary nonsecret configuration or internal metadata | Encrypt locally. Apply normal sync and outbound approval rules. This label does not mean public. |
| `sensitive` | Personal, meeting or business-confidential content | Encrypt locally. Show its label in context preview. |
| `restricted` | Content that policy prevents from leaving the local device | Disable Lakebase sync. Disable outbound context transmission. |

`restricted` content shall not enter a remote embedding request.
Access tokens, database credentials, OAuth refresh tokens, keychain keys and client secrets shall never be memory content.
Motherbricks shall reject a configuration memory that contains a reserved credential field.
Automatic classifiers can propose a sensitivity label.
They shall not lower a user-selected label.

## 5. Practical logical schema and data dictionary

This is a logical schema.
SQL types can differ between SQLite and Lakebase.
`UUID`, `JSON`, `TIMESTAMPTZ`, `BIGINT` and `BLOB` describe the intended data.
The embedding column shall use the vector type supported by the selected local runtime and Lakebase capability.
No embedding dimension is prescribed.

### 5.1 `mog_scope` — local and Lakebase

| Field | Type | Rules |
|---|---|---|
| `mog_id` | UUID | Stable MOG identifier. Part of the scope key. |
| `workspace_id` | Text | Canonical workspace identifier or local installation scope. Part of the scope key. |
| `owner_actor_id` | Text | Verified Databricks identity or verified local owner. Part of the scope key. |
| `home_profile_alias` | Text | CLI routing hint only. Never an authorization boundary. |
| `lakebase_binding_id` | UUID nullable | References non-secret Lakebase connection metadata. |
| `configuration_json` | JSON | Non-secret MOG configuration. Reject reserved secret fields. |
| `scope_revision` | BIGINT | Starts at 1. Increases for each scope configuration mutation. |
| `created_at` | Timestamp | UTC creation time. |
| `deleted_at` | Timestamp nullable | Set for MOG or scope deletion. |
| `deletion_mutation_id` | UUID nullable | Identifies the scope tombstone. |

Primary key: `(workspace_id, owner_actor_id, mog_id)`.

### 5.2 `memory` — local and Lakebase

| Field | Type | Rules |
|---|---|---|
| `memory_id` | UUID | Stable identifier created once. Never reused. |
| `workspace_id` | Text | Scope value. Required on every query. |
| `owner_actor_id` | Text | Verified owner. Required on every query. |
| `mog_id` | UUID | Owning MOG. Required on every query. |
| `kind` | Enum | One of the six memory types. |
| `schema_version` | Integer | Version of the canonical payload. |
| `payload_json` | JSON | Canonical typed content. Contains no credentials. |
| `search_text` | Text | Derived searchable text. Excluded for restricted remote records. |
| `title` | Text nullable | Short user-visible label. |
| `review_state` | Enum | `draft`, `confirmed` or `rejected`. Only `confirmed` enters durable recall. |
| `sensitivity` | Enum | `standard`, `sensitive` or `restricted`. |
| `sync_policy` | Enum | `allowed` or `local_only`. |
| `outbound_policy` | Enum | `ask_each_time` or `never`. |
| `retention_class` | Enum | Explicit retention class. |
| `expires_at` | Timestamp nullable | Required for a custom expiry. |
| `created_at` | Timestamp | UTC creation time supplied by the first writer. |
| `updated_at` | Timestamp | UTC mutation time for display. Not the conflict authority. |
| `revision` | BIGINT | Starts at 1. Increases by one for each accepted mutation. |
| `last_mutation_id` | UUID | Identifier of the mutation that produced this row. |
| `last_writer_id` | Text | Installation or replica identifier. |
| `content_sha256` | Binary | Hash of canonical payload and scope metadata. |
| `provenance_json` | JSON | Source fields defined below. |
| `deleted_at` | Timestamp nullable | Set when a tombstone is current. |

Primary key: `(workspace_id, owner_actor_id, mog_id, memory_id)`.  
Alternate key for lookup: `memory_id` must still be checked against the full scope.

### 5.3 Provenance fields in `provenance_json`

| Field | Requirement |
|---|---|
| `source_type` | `user_entry`, `push_to_talk`, `meeting`, `genie_one_mcp`, `genie_space_rest`, `custom_agent`, `import` or `system`. |
| `source_id` | Stable source artifact or external object identifier when available. |
| `source_workspace_id` | Workspace that produced the source. Required for a backend response. |
| `capture_session_id` | Local capture session when applicable. |
| `raw_artifact_id` | Raw transcript or audio artifact when retained and permitted. |
| `cleaned_artifact_id` | Cleaned transcript artifact when applicable. |
| `backend_type` and `backend_id` | Selected backend adapter and configured backend identity. |
| `conversation_id`, `response_id`, `query_item_id` | Backend identifiers when supplied. |
| `source_uri` | Validated HTTPS Databricks link when supplied. Never an automatic fetch instruction. |
| `source_created_at` | Original source timestamp. |
| `created_by` | Verified user or trusted local service identity. Never model-selected. |
| `derivation` | `user_authored`, `deterministic`, `stt`, `cleanup_model`, `summary_model` or `imported`. |
| `model_provenance` | Configured STT, cleanup, summary or embedding model identifiers when applicable. |
| `copied_from_memory_id` | Source memory for an explicit cross-scope copy. |
| `copy_approval_id` | Local approval record for an explicit cross-scope copy. |
| `supersedes_memory_id` | Optional user-approved replacement relationship. |

### 5.4 `memory_mutation` — local and Lakebase

| Field | Type | Rules |
|---|---|---|
| `mutation_id` | UUID | Stable idempotency key for one state transition. |
| Full scope and `memory_id` | Scope fields | Identify the target record. |
| `base_revision` | BIGINT | Revision observed by the writer. Use 0 for creation. |
| `new_revision` | BIGINT | `base_revision + 1`. |
| `mutation_kind` | Enum | `create`, `update`, `delete`, `scope_delete` or `retention_delete`. |
| `payload_json` | JSON nullable | New content for create or update. Null for deletion. |
| `writer_id` | Text | Replica that created the mutation. |
| `client_created_at` | Timestamp | Display and diagnostic value only. |
| `server_received_at` | Timestamp nullable | Assigned by Lakebase. |
| `payload_sha256` | Binary | Detects duplicate delivery and malformed replays. |

Unique key: `(workspace_id, owner_actor_id, mog_id, mutation_id)`.  
Unique key: `(workspace_id, owner_actor_id, mog_id, memory_id, new_revision)`.

### 5.5 `memory_change` — Lakebase only

| Field | Type | Rules |
|---|---|---|
| `change_seq` | BIGSERIAL | Lakebase-assigned pull order. |
| `mutation_id` | UUID | References the accepted mutation. |
| Full scope and `memory_id` | Scope fields | Allow filtered incremental pull. |
| `new_revision` | BIGINT | Current revision after this change. |
| `change_kind` | Enum | Content or tombstone change. |
| `server_received_at` | Timestamp | Lakebase receipt time. |

Unique key: `(workspace_id, owner_actor_id, mog_id, mutation_id)`.  
Index: `(workspace_id, owner_actor_id, mog_id, change_seq)`.

Lakebase shall apply the memory update, mutation insert and change insert in one database transaction.
The change sequence is the synchronization authority.
Client clocks are not the synchronization authority.

### 5.6 `sync_outbox` — local only

| Field | Type | Rules |
|---|---|---|
| `mutation_id` | UUID | Stable retry key. |
| Full scope and `memory_id` | Scope fields | Identify the mutation target. |
| `state` | Enum | `pending`, `leased`, `retry_wait`, `acked`, `conflict` or `blocked`. |
| `attempt_count` | Integer | Increases for each transport attempt. |
| `lease_owner` | Text nullable | Sync worker identifier. |
| `lease_expires_at` | Timestamp nullable | Expired leases return to `pending`. |
| `next_attempt_at` | Timestamp nullable | Backoff schedule. |
| `last_error_category` | Text nullable | Sanitized category only. |
| `created_at` and `acked_at` | Timestamps | Queue age and completion evidence. |

The mutation payload comes from the same local transaction.
A retry shall reuse the original `mutation_id` and payload hash.

### 5.7 `sync_cursor` and `sync_conflict` — local plus Lakebase metadata as needed

`sync_cursor` fields:

- `replica_id`.
- Full synchronization scope.
- `last_change_seq`.
- `snapshot_id`.
- `minimum_valid_change_seq`.
- `updated_at`.

`sync_conflict` fields:

- `conflict_id`.
- Full scope and `memory_id`.
- `local_mutation_id`.
- `remote_revision`.
- `local_revision`.
- `remote_content_sha256`.
- `local_content_sha256`.
- `state`: `open`, `resolved` or `abandoned`.
- `resolution_mutation_id` nullable.
- `detected_at` and `resolved_at`.

A conflict preserves both payloads in encrypted storage.
It does not silently select the newest wall-clock timestamp.

### 5.8 `memory_embedding` — derived local and optional Lakebase data

| Field | Type | Rules |
|---|---|---|
| Full scope and `memory_id` | Scope fields | Prevent cross-scope vector recall. |
| `memory_revision` | BIGINT | Source memory revision. |
| `source_content_sha256` | Binary | Invalidates stale embeddings. |
| `embedding_provider` | Text | `local` or an approved remote provider. |
| `embedding_model` | Text | Configured model identifier. |
| `embedding_model_version` | Text nullable | Provider version when available. |
| `embedding` | Vector or encrypted BLOB | Dimension is selected by the model-specific migration. |
| `created_at` | Timestamp | Derivation time. |

Unique key: full scope, `memory_id`, `embedding_model` and `source_content_sha256`.

A model change creates a new derived index generation.
It does not mutate memory content.
Queries shall use one compatible index generation.
An embedding inherits the sensitivity and retention class of its source content.
Deletion of source content shall remove its embedding and every local index entry.

### 5.9 `artifact` and `memory_link` — local and Lakebase

`artifact` contains:

- `artifact_id` and full scope.
- `artifact_kind`: `raw_audio`, `raw_transcript`, `clean_transcript` or `full_response`.
- Local encrypted object reference or Lakebase object reference.
- `content_sha256` and byte count.
- `created_at`, `retention_class` and `expires_at`.
- `deleted_at` and deletion mutation identifier.
- Provenance for the capture or backend response.

`memory_link` contains:

- Full scope.
- `from_memory_id` and `to_memory_id` or `to_artifact_id`.
- `relation`: `derived_from`, `answers`, `supersedes`, `copied_from` or `summarizes`.

Raw audio bytes should remain in crash-safe encrypted segment files.
The `artifact` row stores their references and lifecycle state.

### 5.10 `context_approval` — local only

| Field | Type | Rules |
|---|---|---|
| `approval_id` | UUID | One approval decision. |
| `payload_sha256` | Binary | Hash of the exact serialized outbound body and canonical target. Architecture calls this `payload_hash`. |
| `question_text` | Text | Cleaned question shown in preview. |
| `selection_json` | JSON | Memory IDs, revisions and artifact IDs included in the payload. |
| `target_json` | JSON | Canonical workspace, profile alias, backend type, backend ID and pinned conversation ID. |
| `truncation_json` | JSON | Per-item and total truncation disclosures. |
| `decision` | Enum | `approved` or `denied`. |
| `created_at` and `expires_at` | Timestamps | Approval is short-lived. Proposed lifetime: five minutes. |
| `consumed_at` | Timestamp nullable | Prevents reuse for another question. |
| `result` | Enum nullable | `submitted`, `failed`, `cancelled` or `unknown`. |

The exact payload can be regenerated from the immutable selection and canonical envelope builder.
If any selected revision, target or truncation value changes, the payload hash shall change.
The approval then becomes invalid.
Context audit records omit payload content by default after the approval expires.

### 5.11 `deletion_journal` — Motherbricks application scope

| Field | Type | Rules |
|---|---|---|
| `deletion_mutation_id` | UUID | Stable deletion retry key. |
| `workspace_id`, `owner_actor_id`, `mog_id` | Scope fields | Identify the remote records to delete. |
| `deleted_through_revision` | BIGINT nullable | Optional high-water mark for scope deletion. |
| `requested_at` | Timestamp | Local deletion time. |
| `remote_ack_at` | Timestamp nullable | Null means remote deletion is not verified. |
| `state` | Enum | `pending`, `acked` or `blocked`. |
| `purge_after` | Timestamp | Earliest removal of this metadata-only journal entry. |

The journal shall be encrypted.
It contains no memory payload, transcript, audio, response content or pending outbound context.
It survives deletion of the MOG content database so an offline deletion can propagate later.
Security SEC-036 independently defines an encrypted minimal cloud-deletion receipt.
Architecture also defines a minimum deletion outbox in the `DELETING` state.
The director shall integrate this metadata-only exception to the phrase “delete all local MOG data.”

## 6. Retention and deletion matrix

| Object or class | Default rule | Tombstone rule | Cache rule |
|---|---|---|---|
| `session` artifact other than recovery audio | Expire 24 hours after the owning interaction ends | Create a deletion mutation if synchronized | Evict after successful transcription and artifact policy permit |
| `days_30` | Delete 30 days after creation or user-selected refresh | Propagate deletion | Evict only after durable sync |
| `days_90` | Delete 90 days after creation or user-selected refresh | Propagate deletion | Evict only after durable sync |
| `pinned` | Keep until the user deletes or replaces it | Propagate deletion | Retain locally while active |
| `mog_lifetime` | Delete with the MOG | Scope tombstone | Remove with the MOG database |
| `custom` | Require an explicit `expires_at` | Propagate deletion | Respect expiry before eviction |
| Raw audio | Delete after successful transcription | Delete any synchronized artifact reference | Preserve crash-safe segments until transcription succeeds |
| Crash-safe recovery audio after incomplete transcription | Delete after successful transcription or at seven days, whichever comes first | Delete any synchronized artifact reference | Warn before the seven-day deadline |
| Retained meeting audio | Require a meeting-specific retention class before recording starts. Default to 30 days when enabled. | Propagate deletion | Apply an earlier user-selected expiry when present |
| Content tombstone | Keep metadata without content until all known replicas acknowledge it, then keep a 30-day grace period | Do not resurrect the same `memory_id` | Remove content immediately |
| Scope tombstone | Keep until all known replicas acknowledge it, then keep a 90-day grace period | Reject late writes from stale replicas | Keep only the metadata-only deletion journal |
| Sync conflict | Retain both versions until user resolution or the parent retention policy expires | Resolution creates a new mutation | Do not evict an unresolved local conflict |

The user can select a longer retention class unless organization policy forbids it.
The user can always request earlier local deletion.
A legal-hold extension point can prevent expiry purge when organization policy requires it.
The hold must not be inferred from a transcript or model response.

When tombstone history is purged, Lakebase shall advance `minimum_valid_change_seq`.
A replica with an older cursor shall complete a full snapshot rebase before it can push.
This rule prevents a stale replica from resurrecting purged deletions.

Deleting a response reference shall not claim deletion of the Genie conversation.
Deleting a meeting summary shall not claim deletion of source data in Databricks.
Remote deletion is complete only after the selected Lakebase acknowledges the deletion mutation.

## 7. Recall design and budgets

### 7.1 Exact and structured recall

Exact recall supports:

- Memory ID lookup.
- Source identifier lookup.
- Kind, sensitivity and review-state filters.
- Creation, source and expiry time ranges.
- Exact term and prefix search over approved search text.
- Provenance filters for workspace, backend, conversation and capture session.

Every query shall apply the full scope tuple before results are returned.
Deleted, expired, rejected and superseded records are excluded by default.
A user can explicitly inspect tombstones and conflict versions in a management view.

The preliminary director budget remains mandatory:

- **Local structured recall:** p95 at or below 50 ms.
- **Test set:** 10,000 active local records on named reference hardware.
- **Measurement:** time from a ready local query call to a fully scoped result list.
- **Exclusion:** display rendering time is measured separately.
- **Network requirement:** zero network calls.

### 7.2 Semantic recall

Semantic recall shall use a local compatible embedding generation.
It shall apply scope, expiry and policy filters before candidates are returned.
It shall rank candidates by model-compatible distance, exact-match boosts and a deterministic memory-ID tie-break.
Recency can break a similarity tie.
Recency shall not overpower a higher similarity score without a documented ranking rule.

Proposed MVP budgets for Architecture to integrate:

| Stage | Proposed p95 target | Measurement boundary |
|---|---:|---|
| Local semantic retrieval with a ready query embedding | 150 ms | Scoped candidate query and top-20 ranking over 10,000 active records |
| End-to-end warm local semantic recall | 750 ms | Query text through embedding generation, scoped retrieval and result hydration |
| Optional Lakebase semantic search | No fixed interactive guarantee | Display pending state and measure separately by workspace and network |

The 150 ms and 750 ms values are proposed acceptance targets, not benchmark results.
Team A shall name reference hardware.
Team M shall run at least 1,000 representative queries after a documented warm-up.
The release candidate should reach Recall@10 of at least 0.90 on an approved labeled MOG corpus.
The release candidate shall return zero out-of-scope records in the isolation suite.

**Integration collision:** Team A NFR-A-005 proposes 250 ms p95 for 10,000 indexed records with a local query embedding.
Its measurement section includes local query embedding and vector search.
That is not equivalent to the two Team M budgets above.
Team M recommends separate retrieval-only and end-to-end budgets because embedding generation can dominate latency.
The director shall select one consistent set after the `REF-MAC` and `REF-WIN` benchmark environments are defined.
The final document shall not publish both end-to-end targets.

If the local embedding model is unavailable, semantic recall shall report that it is unavailable.
Exact recall shall continue.
Remote embedding generation shall require sync eligibility and network approval.
It shall not occur for `restricted` or `local_only` content.

### 7.3 Outbound context recall

Local recall and outbound sharing are separate decisions.
Enabling context sharing allows Motherbricks to propose relevant local items.
It does not approve transmission.

The context builder shall create one canonical payload containing:

1. The cleaned question.
2. Each selected item as untrusted data.
3. The source and timestamp of each item.
4. The memory ID and revision in local audit metadata.
5. Every truncation applied to an item or the full envelope.
6. A statement that recalled text is data and not an instruction.

The preview shall show the exact text that will leave Motherbricks.
It shall show the target workspace and backend.
The user shall approve the payload hash for one submission.
An ambient meeting item shall not be selected automatically.
An item with `outbound_policy = never` shall not enter the payload.

## 8. Synchronization protocol

### 8.1 Write and push sequence

1. Validate the write approval and scope.
2. Create or reuse a stable `mutation_id` for a retry of the same user operation.
3. Commit the memory mutation and outbox record in one local transaction.
4. Update the derived local index after the commit.
5. Return local success without waiting for Lakebase.
6. Generate a fresh Lakebase credential for a new physical connection when sync is enabled.
7. Connect to the bound direct Lakebase host with `sslmode=verify-full` and `sslrootcert=system`.
8. Push each mutation with its original `base_revision`, `new_revision` and payload hash.
9. Commit the remote memory, mutation and change records atomically.
10. Mark the outbox item `acked` only after the remote commit or a verified duplicate response.

A lost response creates uncertain delivery.
The worker shall retry the same mutation.
It shall not create a new memory or mutation ID.

### 8.2 Pull sequence

1. Read the local `last_change_seq` for the exact scope and replica.
2. Request remote changes after that sequence in ascending order.
3. Apply each change once by `mutation_id` and revision.
4. Advance the cursor in the same local transaction as the applied changes.
5. Rebuild affected local index entries after the transaction.
6. Require a snapshot rebase if the cursor is older than `minimum_valid_change_seq`.

A pull shall not widen the requested scope.
A remote record that fails scope validation shall be quarantined and reported.
It shall not enter the local search index.

### 8.3 Conflict handling

Creation and update use optimistic concurrency through `base_revision`.

- A repeated mutation ID with the same payload hash returns the prior result.
- A repeated mutation ID with a different payload hash is rejected as a protocol error.
- A mutation with the current `base_revision` is accepted once.
- A mutation behind the current revision is compared with existing history.
- Divergent content creates a conflict record.
- Motherbricks preserves both versions.
- User resolution creates a new revision based on the current remote revision.
- Wall-clock time does not silently resolve divergent content.

Deletion is terminal for a `memory_id`.
A tombstone rejects every older or concurrent content update.
Restoration creates a new `memory_id` with `supersedes_memory_id` provenance.
This rule prevents delayed packets from resurrecting deleted content.

### 8.4 Sync states

| State | Entry condition | Exit condition | Failure behavior |
|---|---|---|---|
| `pending` | Local transaction commits | Worker acquires a lease | Remains durable across restart |
| `leased` | Worker starts transport | Ack, retry schedule, conflict or lease expiry | Expiry returns the item to `pending` |
| `retry_wait` | Safe transient failure occurs | `next_attempt_at` arrives | Preserve mutation ID and payload |
| `acked` | Remote commit or verified duplicate is known | Terminal | Keep sanitized audit metadata |
| `conflict` | Remote revision diverges | User resolves or abandons the local edit | Preserve both versions |
| `blocked` | Policy, validation or permanent authorization failure occurs | User changes policy, scope or credentials | Do not retry in a tight loop |

Backoff shall use exponential delay with jitter.
The worker shall honor `Retry-After` when supplied.
One automatic retry burst shall contain at most five attempts, consistent with Architecture.
The item shall then wait for a new connectivity event, application restart or explicit `Sync now` action.
Authorization failure shall not repeatedly invoke the CLI.
Sync cancellation shall stop new transport work.
It shall not roll back an already acknowledged remote mutation.

## 9. Export, import and rebuild

### 9.1 Export

The export shall be user-initiated.
It shall contain a versioned manifest and canonical memory records.
It shall include stable IDs, scopes, types, provenance, retention, sensitivity, revisions and tombstones needed for a faithful restore.
It shall exclude access tokens, database credentials, keychain material and cached CLI output.
Derived embeddings should be excluded because they are rebuildable.
An export containing sensitive content shall be encrypted by default.
A plaintext export shall display a specific plaintext warning.
The destination and scope shall be visible before export.
The UI shall disclose that exported copies are outside Motherbricks deletion control.

### 9.2 Import and explicit copy

Same-scope restore preserves stable IDs and detects duplicates by mutation and content hashes.
Import into a different MOG or workspace is an explicit copy.
It requires a preview and creates new memory IDs in the target scope.
It preserves the original IDs only as provenance.
It does not copy outbound approvals.

### 9.3 Rebuild

A local rebuild shall:

1. Verify the selected owner and workspace.
2. Preserve unsynchronized local mutations before replacing a database.
3. Read an authoritative Lakebase snapshot for the exact scope.
4. Apply changes after the snapshot sequence.
5. Apply current tombstones.
6. Rebuild exact indexes.
7. Rebuild compatible embeddings from eligible memory text when needed.
8. Verify record, revision and tombstone counts.
9. Make the new store active only after validation succeeds.

Lakebase cannot recover a local-only record that never synchronized.
The UI shall disclose this limit before a destructive rebuild.

## 10. Numbered requirements

Release key: `MVP` is the first desktop release. `LATER` requires a separate feature gate.
Every `MUST` requirement links to at least one acceptance test.

### 10.1 MEM requirements

| ID | Priority / release | Requirement | Rationale | Tests / verification |
|---|---|---|---|---|
| MEM-001 | MUST / MVP | Motherbricks shall commit an approved local memory write without waiting for Lakebase. | D-06, D-12 | AT-MEM-001; automated test |
| MEM-002 | MUST / MVP | Motherbricks shall make Lakebase synchronization optional for dictation and local recall. | D-06, UC-03 | AT-MEM-001; demonstration |
| MEM-003 | MUST / MVP | Each memory shall have an immutable `memory_id` and the complete scope tuple. | D-06 | AT-MEM-003; automated test |
| MEM-004 | MUST / MVP | Each memory shall use one of the six defined memory types. | Team M charter | AT-MEM-004; inspection and automated test |
| MEM-005 | MUST / MVP | Each memory shall store source, source timestamp, derivation and creator provenance. | D-03, D-06 | AT-MEM-004; schema and automated test |
| MEM-006 | MUST / MVP | Each memory shall have sensitivity, synchronization, outbound and retention policies. | D-06, D-14 | AT-MEM-004; automated test |
| MEM-007 | MUST / MVP | Motherbricks shall exclude expired and deleted records from normal recall. | D-14 | AT-MEM-015; automated test |
| MEM-008 | MUST / MVP | Motherbricks shall store raw audio as an artifact rather than a durable memory by default. | D-14 | AT-MEM-015; platform test |
| MEM-009 | MUST / MVP | Motherbricks shall require user approval before a model-proposed draft becomes durable memory. | Team M charter | AT-MEM-004; automated test |
| MEM-010 | MUST / MVP | Local exact recall shall apply the full scope before returning any result. | D-06 | AT-MEM-003; automated test |
| MEM-011 | MUST / MVP | Local semantic recall shall apply the same scope and policy filters as exact recall. | D-06 | AT-MEM-002, AT-MEM-003; automated test |
| MEM-012 | MUST / MVP | Local structured recall shall meet p95 at or below 50 ms over the defined 10,000-record test set. | Director budget | AT-MEM-001; benchmark |
| MEM-013 | SHOULD / MVP | Warm local semantic recall should meet the proposed 750 ms end-to-end and 150 ms retrieval-only p95 budgets. | Director request for a separate budget | AT-MEM-002; benchmark |
| MEM-014 | MUST / MVP | Cache eviction shall preserve every local-only, unsynchronized, draft and unresolved-conflict record. | D-06, D-12 | AT-MEM-014; automated test |
| MEM-015 | MUST / MVP | A local memory mutation and its outbox entry shall commit atomically. | D-12 | AT-MEM-012; fault-injection test |
| MEM-016 | MUST / MVP | Lakebase shall store accepted mutations and ordered change records for incremental synchronization. | Team M design | AT-MEM-007; integration test |
| MEM-017 | MUST / MVP | A synchronization retry shall reuse the original mutation ID and payload hash. | D-12 | AT-MEM-007; integration test |
| MEM-018 | MUST / MVP | Motherbricks shall preserve divergent concurrent content as a conflict rather than apply silent last-writer-wins behavior. | Data integrity | AT-MEM-011; integration test |
| MEM-019 | MUST / MVP | A deletion tombstone shall reject older and concurrent updates for the same memory ID. | D-06, D-14 | AT-MEM-010, AT-MEM-011; integration test |
| MEM-020 | MUST / MVP | Offline MOG deletion shall leave a metadata-only deletion journal for later Lakebase propagation. | D-14 and exit-test collision | AT-MEM-010; platform test |
| MEM-021 | MUST / MVP | Motherbricks shall export canonical memories without credentials or derived-only secrets. | Team M charter | AT-MEM-013; automated test and inspection |
| MEM-022 | MUST / MVP | A rebuild shall preserve unsynchronized local mutations before replacing the local store. | Data integrity | AT-MEM-013; fault-injection test |
| MEM-023 | MUST / MVP | A cross-MOG or cross-workspace memory transfer shall create an explicitly approved copy in the target scope. | D-06, D-08 | AT-MEM-003; automated test |
| MEM-024 | MUST / MVP | Lakebase Search enablement shall require an administrator action and capability discovery. | Irreversible enablement restarts computes | AT-MEM-016; primary-source and platform test |
| MEM-025 | MUST / MVP | An embedding migration shall identify its model and source content hash without assuming a fixed dimension. | Model portability | AT-MEM-016; integration test |
| MEM-026 | SHOULD / LATER | A managed Agent Memory adapter should pass lossless identity, retention, deletion, conflict and isolation tests before release. | Current managed service is Beta | AT-MEM-017; spike and inspection |
| MEM-027 | MUST / MVP | The outbound context builder shall include question, item source, item timestamp and truncation disclosures required by D-03. | D-03 | AT-MEM-005; automated test |
| MEM-028 | SHOULD / MVP | Motherbricks should record synchronization and outbound audit metadata without content logging by default. | Governance | AT-MEM-020; log inspection |

### 10.2 SEC requirements owned by Team M

The `SEC-MEM` namespace avoids collisions with the Security team’s general requirements.

| ID | Priority / release | Requirement | Rationale | Tests / verification |
|---|---|---|---|---|
| SEC-MEM-001 | MUST / MVP | Motherbricks shall encrypt the local database, local index files and outbound queue at rest. | D-06, D-14 | AT-MEM-020; Security inspection |
| SEC-MEM-002 | MUST / MVP | Each MOG shall use a unique data-encryption key protected by an installation key-encryption key or wrapped key in the operating-system keychain. | Team S SEC-029 and SEC-030 | AT-MEM-020; platform test |
| SEC-MEM-003 | MUST / MVP | Motherbricks shall reject credentials and key material as memory or MOG configuration content. | D-07, secret control | AT-MEM-020; automated test |
| SEC-MEM-004 | MUST / MVP | Every Lakebase connection shall use `sslmode=verify-full` with the system root store. | Verified TLS guidance | AT-MEM-009; network test |
| SEC-MEM-005 | MUST / MVP | Motherbricks shall not fall back to `sslmode=require` or disable certificate verification. | `require` does not verify the server | AT-MEM-009; network test |
| SEC-MEM-006 | MUST / MVP | A Lakebase credential request shall use the workspace and endpoint resource bound to the target Lakebase connection. | Workspace-scoped credentials | AT-MEM-008, AT-MEM-009; integration test |
| SEC-MEM-007 | MUST / MVP | A Lakebase credential shall remain in native process memory and shall be redacted from logs, crash reports, exports and memory records. | D-07 | AT-MEM-008, AT-MEM-020; inspection |
| SEC-MEM-008 | MUST / MVP | The Lakebase role shall have only the database, schema, table and sequence privileges required by the Motherbricks adapter. | Least privilege | AT-MEM-003, AT-MEM-020; permission test |
| SEC-MEM-009 | MUST / MVP | Lakebase RLS or an equivalent database-enforced policy shall restrict each authenticated user to authorized owner, workspace and MOG rows. | Team S SEC-033 | AT-MEM-003; database policy test |
| SEC-MEM-010 | MUST / MVP | Application-level MOG filters shall remain as defense in depth when server policies also enforce MOG access. | D-06 | AT-MEM-003; automated test |
| SEC-MEM-011 | MUST / MVP | Outbound context approval shall bind the exact payload hash, selected revisions, target and expiry to one submission. | D-03 | AT-MEM-005; automated test |
| SEC-MEM-012 | MUST / MVP | A remembered item shall not be transmitted solely because context sharing is enabled or the item has type `approved_context`. | D-03 | AT-MEM-005; automated test |
| SEC-MEM-013 | MUST / MVP | Recalled text shall be labeled and handled as untrusted data rather than executable instructions. | D-13, prompt-injection boundary | AT-MEM-006; adversarial test |
| SEC-MEM-014 | MUST / MVP | Restricted and `local_only` memories shall not enter remote embedding, Lakebase sync or outbound context requests. | Data policy | AT-MEM-005, AT-MEM-006; automated test |
| SEC-MEM-015 | MUST / MVP | Motherbricks shall report Lakebase deletion as pending until remote acknowledgement is verified. | D-14 | AT-MEM-010, AT-MEM-018; demonstration |
| SEC-MEM-016 | MUST / MVP | Motherbricks shall not claim Genie-side deletion when only local or Lakebase memory was deleted. | D-14 | AT-MEM-018; demonstration |
| SEC-MEM-017 | SHOULD / MVP | Organization policy should be able to require customer-managed keys for a new Lakebase project. | Governance option | AT-MEM-020; administrator inspection |

### 10.3 REL requirements owned by Team M

The `REL-MEM` namespace avoids collisions with general reliability requirements.

| ID | Priority / release | Requirement | Rationale | Tests / verification |
|---|---|---|---|---|
| REL-MEM-001 | MUST / MVP | Lakebase unavailability shall not block local dictation, approved local writes or local recall. | D-06, D-12 | AT-MEM-001; offline test |
| REL-MEM-002 | MUST / MVP | A transient sync failure shall use bounded exponential backoff with jitter and at most five attempts in one automatic burst. | D-12 and Architecture retry contract | AT-MEM-007; integration test |
| REL-MEM-003 | MUST / MVP | A sync worker shall reuse the original mutation after a lost response or process restart. | Idempotent recovery | AT-MEM-007, AT-MEM-012; fault injection |
| REL-MEM-004 | MUST / MVP | Credential expiry shall not discard a queued mutation or an active local memory commit. | One-hour credential lifetime | AT-MEM-008; integration test |
| REL-MEM-005 | MUST / MVP | A new physical Lakebase connection shall use a credential with at least two minutes before its stated expiry. | Documented rotation example | AT-MEM-008; integration test |
| REL-MEM-006 | MUST / MVP | Motherbricks shall reconnect after idle or maximum-lifetime connection closure without duplicating accepted mutations. | Documented connection limits | AT-MEM-019; integration test |
| REL-MEM-007 | MUST / MVP | A crashed or expired outbox lease shall return to `pending` without changing its mutation ID. | Crash recovery | AT-MEM-012; fault injection |
| REL-MEM-008 | MUST / MVP | A stale sync cursor shall trigger a snapshot rebase before that replica can push new mutations. | Resurrection prevention | AT-MEM-013; integration test |
| REL-MEM-009 | MUST / MVP | A rebuild shall activate the replacement store only after scope, revision and tombstone validation succeeds. | Safe recovery | AT-MEM-013; integration test |
| REL-MEM-010 | SHOULD / MVP | Schema migrations should be transactional, versioned and reversible through a tested restore path. | Upgrade safety | AT-MEM-016, AT-MEM-019; migration test |
| REL-MEM-011 | MUST / MVP | Motherbricks shall not automatically submit an unsent Genie question after memory synchronization resumes. | D-12 | AT-MEM-007; integration test |

## 11. Acceptance tests

Unless a test states otherwise, `MVP platform` means macOS and Windows with the native core, an encrypted local store and a test Lakebase project.
Benchmarks shall record hardware, operating system, build, model, data distribution, warm-up and query count.
A screenshot alone is not evidence for deletion, isolation, security or timing.

### AT-MEM-001: Offline structured recall and local writes

- **Requirements:** MEM-001, MEM-002, MEM-012, REL-MEM-001.
- **Release/platform:** MVP platform.
- **Preconditions:** Seed 10,000 confirmed active memories in one scope. Include other-scope decoys. Disable the network. Prepare at least 1,000 exact and structured queries.
- **Actions:** Run the query suite. Save a local memory. Run local dictation with the local STT fixture. Attempt a Lakebase sync.
- **Expected:** Every result is in scope. Structured recall p95 is at most 50 ms. The new memory receives a stable ID and queued mutation. Dictation and recall complete. Sync reports offline without changing local success.
- **Evidence:** Query traces, latency percentiles, network-deny trace, local transaction record and outbox record.
- **Method:** Automated benchmark and offline integration test.

### AT-MEM-002: Semantic recall latency and quality

- **Requirements:** MEM-011, MEM-013.
- **Release/platform:** MVP platform with the selected local embedding runtime.
- **Preconditions:** Seed the same 10,000-record corpus. Add an approved labeled query set. Warm the model and index using a documented procedure.
- **Actions:** Run at least 1,000 semantic queries. Measure retrieval-only time with ready embeddings. Measure end-to-end time from query text. Calculate Recall@10. Repeat with the embedding runtime unavailable.
- **Expected:** Proposed p95 targets are 150 ms retrieval-only and 750 ms end-to-end. Recall@10 is at least 0.90. Every result is in scope. Exact recall continues when semantic recall is unavailable.
- **Evidence:** Per-stage timings, model identity, index generation, quality report and unavailable-mode result.
- **Method:** Automated benchmark. Architecture approves the final budget after reference hardware is named.

### AT-MEM-003: Scope isolation and explicit copy

- **Requirements:** MEM-003, MEM-010, MEM-011, MEM-023, SEC-MEM-008, SEC-MEM-009, SEC-MEM-010.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Create two MOGs and two workspaces for one owner. Add a second owner. Place near-duplicate text and embeddings in every scope.
- **Actions:** Run exact and semantic queries from each scope. Attempt ID lookup with only a foreign memory ID. Attempt SQL without a MOG predicate under a broadly authorized test role. Copy one item with explicit approval.
- **Expected:** No unauthorized record is returned. Server policies enforce owner, workspace and authorized MOG boundaries. Application filters provide a second MOG boundary. The approved copy receives a new ID, target scope and source provenance. No approval is copied.
- **Evidence:** Query plans, database audit output, result manifests and copy record.
- **Method:** Automated application and database-policy tests.

### AT-MEM-004: Write approval, type and provenance validation

- **Requirements:** MEM-004, MEM-005, MEM-006, MEM-009.
- **Release/platform:** MVP platform.
- **Preconditions:** Prepare explicit user saves, a model-proposed preference, a meeting summary draft and records with missing provenance or retention.
- **Actions:** Save each record. Attempt to activate the model draft without approval. Attempt to save a new type and a reserved credential field.
- **Expected:** Explicit saves become active with complete provenance. The model draft remains inactive until approval. Invalid type, missing retention and reserved credential fields are rejected. No network request is required for a local save.
- **Evidence:** Validation results, approved record revisions and inactive draft state.
- **Method:** Automated schema and state tests.

### AT-MEM-005: Exact outbound payload approval

- **Requirements:** MEM-027, SEC-MEM-011, SEC-MEM-012, SEC-MEM-014.
- **Release/platform:** MVP platform.
- **Preconditions:** Create standard, sensitive, restricted and `approved_context` records. Enable context sharing. Select a workspace and backend.
- **Actions:** Build and preview a payload. Change one source revision. Change the target workspace. Reuse an approval for a second submission. Attempt to include restricted content. Attempt to send an `approved_context` item without payload approval.
- **Expected:** The payload contains question, sources, timestamps and truncation disclosures. Every change invalidates the hash. Approval is single-use. Restricted content is excluded. Type or setting alone never authorizes transmission.
- **Evidence:** Canonical payload fixtures, hashes, approval state and network spy results.
- **Method:** Automated contract and transport tests.

### AT-MEM-006: Recalled content is untrusted data

- **Requirements:** SEC-MEM-013, SEC-MEM-014.
- **Release/platform:** MVP platform.
- **Preconditions:** Store fixtures containing instructions to reveal secrets, change workspaces, start recording, call tools and send memory. Include malicious embedding text and a restricted record.
- **Actions:** Recall the items locally. Add an eligible item to an approved context payload. Observe routing, capture, network and tool-call behavior.
- **Expected:** The text appears only as quoted data. It causes no tool call, capture change, profile change or extra transmission. Restricted text never leaves the device.
- **Evidence:** Rendered envelope, tool-call trace, capture state log and network trace.
- **Method:** Adversarial automated test with Security review.

### AT-MEM-007: Idempotent retry and uncertain delivery

- **Requirements:** MEM-016, MEM-017, REL-MEM-002, REL-MEM-003, REL-MEM-011.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Queue a create, an update and a deletion. Add an unsent Genie question as a separate router fixture.
- **Actions:** Drop the response after Lakebase commits. Retry with the same mutation. Inject transient failures and `Retry-After`. Reconnect the network.
- **Expected:** One memory revision and one change exists per accepted mutation. Backoff uses jitter and honors `Retry-After`. An automatic burst stops after five attempts and waits for a new trigger. Local state eventually acknowledges the remote commit. The unsent Genie question remains unsent.
- **Evidence:** Mutation IDs, database counts, retry schedule and router state.
- **Method:** Fault-injection integration test.

### AT-MEM-008: Credential rotation without memory loss

- **Requirements:** SEC-MEM-006, SEC-MEM-007, REL-MEM-004, REL-MEM-005.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Use short test credential lifetimes or a credential stub with controlled `expire_time`. Queue changes before, during and after expiry.
- **Actions:** Open new physical connections with an unexpired credential, a credential inside the two-minute window and an expired credential. Expire a login credential while a transaction remains open. Inspect logs and crash reports.
- **Expected:** A new connection uses a freshly generated credential with at least two minutes remaining. An active transaction follows normal connection behavior. Queued changes are preserved. No token appears in persistent content or diagnostics. Cross-workspace credential use fails closed.
- **Evidence:** Credential-expiry stubs, connection events, queue hashes and redaction report. Do not retain real tokens in evidence.
- **Method:** Automated integration and secret-scanning tests.

### AT-MEM-009: TLS peer verification and workspace binding

- **Requirements:** SEC-MEM-004, SEC-MEM-005, SEC-MEM-006.
- **Release/platform:** MVP platform plus a TLS test proxy.
- **Preconditions:** Present a valid publicly trusted endpoint, an untrusted certificate, a hostname mismatch and a workspace-endpoint mismatch.
- **Actions:** Attempt each connection. Inspect negotiated configuration.
- **Expected:** Only the correctly bound, verified endpoint connects. `verify-full` and the system root store are used. No fallback to `require`, `verify-ca` with the wrong host or disabled verification occurs.
- **Evidence:** Sanitized TLS traces and connection errors.
- **Method:** Automated network test.

### AT-MEM-010: Offline deletion and tombstone propagation

- **Requirements:** MEM-019, MEM-020, SEC-MEM-015.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Synchronize a memory and a second MOG. Disconnect the network. Delete the first memory, then delete its MOG.
- **Actions:** Inspect local storage while offline. Reconnect and complete fresh user authentication. Push the deletion mutations. Attempt a delayed update from a stale replica.
- **Expected:** Local content and the local index are removed immediately. Pending remote deletion is visible. Only the metadata-only deletion journal remains for the deleted MOG. Lakebase accepts the tombstones after reconnection. Stale updates cannot resurrect content. Remote completion appears only after acknowledgement.
- **Evidence:** Before-and-after encrypted database counts, deletion journal, remote change records and stale-write rejection.
- **Method:** Automated offline/recovery test plus file inspection.

### AT-MEM-011: Concurrent update conflict and delete ordering

- **Requirements:** MEM-018, MEM-019.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Two replicas hold revision 1 of one memory.
- **Actions:** Commit divergent updates from both replicas. Resolve the conflict. Concurrently update and delete another record. Replay the delayed update.
- **Expected:** Both divergent payloads are preserved in a conflict. Resolution creates a new revision. The tombstone rejects the delayed update. Restoration, if requested, creates a new memory ID.
- **Evidence:** Mutation history, conflict payloads, resolution revision and tombstone behavior.
- **Method:** Automated concurrency integration test.

### AT-MEM-012: Crash-safe local transaction and lease recovery

- **Requirements:** MEM-015, REL-MEM-003, REL-MEM-007.
- **Release/platform:** MVP platform.
- **Preconditions:** Instrument commits and outbox leases with crash points.
- **Actions:** Crash before commit, after memory commit and before outbox visibility, during transport and after remote commit but before local acknowledgement. Restart after each crash.
- **Expected:** No partial memory/outbox pair is visible. An expired lease returns to `pending`. A retry uses the same mutation ID. The remote database contains no duplicate accepted mutation.
- **Evidence:** Fault-injection log, recovered database state and remote counts.
- **Method:** Automated fault-injection test.

### AT-MEM-013: Export, import and rebuild

- **Requirements:** MEM-021, MEM-022, REL-MEM-008, REL-MEM-009.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Seed active memories, tombstones, conflicts and one unsynchronized local memory. Create an old cursor and a second target scope.
- **Actions:** Export and inspect the archive. Restore into the same scope. Copy into the second scope. Rebuild from a snapshot and incremental changes. Attempt to push from the stale cursor before rebase.
- **Expected:** Export contains required memory metadata and no credentials. Same-scope restore is idempotent. Cross-scope import creates approved new IDs. Rebuild preserves unsynchronized mutations and current tombstones. The stale replica must rebase before push. Validation failure leaves the old store active.
- **Evidence:** Manifest, hash comparisons, record counts, revision counts and recovery log.
- **Method:** Automated integration and archive-inspection tests.

### AT-MEM-014: Cache eviction safety

- **Requirements:** MEM-014.
- **Release/platform:** MVP platform.
- **Preconditions:** Fill the cache with synced records, local-only records, pending mutations, drafts and unresolved conflicts.
- **Actions:** Apply cache pressure. Recall evicted synced records while online and offline. Inspect every protected category.
- **Expected:** Only durable synced working-set entries are evicted. Protected records remain intact. An evicted remote record can be restored by an explicit rebuild when online. Offline recall does not claim availability of an evicted record.
- **Evidence:** Cache inventory, queue hashes and recall status.
- **Method:** Automated storage test.

### AT-MEM-015: Retention and raw-audio lifecycle

- **Requirements:** MEM-007, MEM-008.
- **Release/platform:** MVP platform.
- **Preconditions:** Create records for every retention class. Create raw audio with default policy and with meeting-specific retention.
- **Actions:** Advance the test clock past each expiry. Complete and fail transcription. Delete a retained meeting. Run recall after deletion.
- **Expected:** Default raw audio is deleted only after successful transcription. Failed transcription retains crash-safe segments under policy. Retained audio follows its explicit class. Expired and deleted records do not appear in normal recall. Required tombstones remain.
- **Evidence:** Artifact lifecycle records, file deletion checks and recall results.
- **Method:** Automated test with Voice and Security review.

### AT-MEM-016: Search capability and embedding migration

- **Requirements:** MEM-024, MEM-025, REL-MEM-010.
- **Release/platform:** MVP Lakebase integration. `lakebase_ann` test runs only where Search is supported.
- **Preconditions:** Use a Postgres 16+ test project. Test both Search-disabled and Search-enabled states. Use two embedding models with different output dimensions.
- **Actions:** Discover capabilities. Request administrator enablement. Observe the compute restart. Install the extension. Build model-specific indexes. Migrate from one embedding generation to another. Attempt an incompatible query.
- **Expected:** Basic sync works without Search. Motherbricks does not enable Search itself. The administrator sees restart and irreversibility warnings. Each index uses its model dimension. An incompatible generation is rejected without memory-content changes.
- **Evidence:** Capability report, migration log, index metadata and rollback restore test.
- **Method:** Platform integration and migration test.

### AT-MEM-017: Managed Agent Memory later-adapter spike

- **Requirements:** MEM-026.
- **Release/platform:** LATER. Beta-enabled test workspace.
- **Preconditions:** Enable the preview with administrator approval. Create stores for separate security boundaries.
- **Actions:** Map stable IDs, provenance, retention, tombstones and revisions. Exercise concurrent `put()` operations, 100-result search boundaries, export, deletion and actor isolation.
- **Expected:** The adapter is released only if every required mapping is lossless or an explicit product reduction is approved. Actor filters are never treated as authorization. The legacy Unity Catalog API is unused.
- **Evidence:** Mapping table, API traces, limitations list and Security decision.
- **Method:** Technical spike and primary-source verification. This is not an MVP release gate.

### AT-MEM-018: Deletion claims remain source-specific

- **Requirements:** SEC-MEM-015, SEC-MEM-016.
- **Release/platform:** MVP platform.
- **Preconditions:** Have a synced response reference and a linked Genie conversation. Keep Lakebase offline for the first deletion.
- **Actions:** Delete the reference offline. Reconnect. Delete the MOG after Lakebase acknowledgement.
- **Expected:** The UI distinguishes local deletion, pending Lakebase deletion and acknowledged Lakebase deletion. It never states that the Genie conversation was deleted without a separate verified operation.
- **Evidence:** UI state transitions, remote acknowledgement and backend call trace.
- **Method:** Demonstration backed by automated state assertions.

### AT-MEM-019: Connection closure and schema migration recovery

- **Requirements:** REL-MEM-006, REL-MEM-010.
- **Release/platform:** MVP platform plus Lakebase integration.
- **Preconditions:** Have active pooled connections and queued mutations. Prepare a versioned schema migration.
- **Actions:** Force idle closure and maximum-lifetime closure. Interrupt a migration and its restore procedure. Resume sync.
- **Expected:** Reconnection creates a valid fresh connection. Accepted mutations are not duplicated. A failed migration does not expose a partially active local schema. The tested restore path returns service.
- **Evidence:** Connection lifecycle, mutation counts, migration version and restore verification.
- **Method:** Fault-injection and migration tests.

### AT-MEM-020: Encryption, secret and administrator-policy inspection

- **Requirements:** MEM-028, SEC-MEM-001, SEC-MEM-002, SEC-MEM-003, SEC-MEM-007, SEC-MEM-008, SEC-MEM-017.
- **Release/platform:** MVP platform. CMK check applies only when organization policy requires it.
- **Preconditions:** Populate content with canary secrets and meeting text. Generate logs, crash reports, exports and sync errors.
- **Actions:** Inspect database and index files while locked. Inspect keychain use. Scan all diagnostics and exports. Test least-privilege roles. Inspect Lakebase CMK status when required.
- **Expected:** Persistent content is encrypted. The data key is not colocated with the database. No canary or real credential leaks. Excess database privileges are denied. A required CMK status is `Active`; otherwise Lakebase setup is blocked by policy.
- **Evidence:** File-entropy and parser checks, keychain trace, secret scan, permission tests and sanitized administrator status.
- **Method:** Security inspection and automated secret tests.

## 12. Dependencies and interface consumers

| Consumer or reviewer | Required contract or decision |
|---|---|
| Architecture | Approve `MemoryStore`, mutation, cursor, deletion journal and rebuild contracts. Resolve the semantic-budget collision C-M-12. |
| Security | Approve SQLCipher or the selected equivalent, per-MOG key lifecycle, exact server-enforced scope model and export encryption. Security SEC-036 already supports the encrypted deletion receipt. |
| Platform | Verify CLI U2M invocation for `generate-database-credential`, canonical workspace identifiers, direct Lakebase connectivity and target-cloud Search status. |
| Voice/Desktop | Own raw-audio segment files. Call the artifact retention contract after transcription success, failure and meeting deletion. |
| Backend/Genie | Supply pinned workspace, backend, conversation, response and deep-link provenance. Never treat memory as an independent Genie upload. |
| UX | Show local-only, pending sync, conflict, pending deletion and acknowledged deletion states. Implement exact payload preview and memory correction flows. |
| Director | Reserve the `SEC-MEM` and `REL-MEM` IDs or map them into the final non-colliding sequence. Resolve collisions C-M-03 and C-M-04. |

### 12.1 Proposed `MemoryStore` contract

| Operation | Network behavior | Required result |
|---|---|---|
| `save_local(input, approval)` | None | Stable memory ID, revision and queued mutation when eligible |
| `recall_exact(scope, query, filters, deadline)` | None | Scoped records with provenance |
| `recall_semantic(scope, text, filters, deadline)` | None for local generation and recall | Scoped ranked candidates with model provenance |
| `preview_context(question, selection, target)` | None | Canonical payload, truncation report and payload hash |
| `record_context_decision(payload_hash, decision)` | None | Single-use local approval or denial |
| `delete_memory(scope, memory_id)` | None for local commit | Local tombstone and durable deletion mutation |
| `delete_mog(scope, mog_id)` | None for local commit | Local content removal and metadata-only deletion journal |
| `sync_now(scope)` | Lakebase only | Acknowledged, retry, conflict or blocked status per mutation |
| `export(scope, destination, approval)` | None | Encrypted versioned archive and manifest |
| `rebuild(scope, source)` | Lakebase when selected | Validated replacement store without loss of unsynchronized mutations |

`MemoryStore` does not submit a Genie question.
The router performs that operation only after a valid context approval and the normal backend request rules.

Architecture's `sync_change_id` maps to Team M's `mutation_id`.
Architecture's `payload_hash` maps to Team M's `payload_sha256`, including the canonical target in the hash preimage.

## 13. Collision register

| ID | Collision | Team M resolution | Required action |
|---|---|---|---|
| C-M-01 | Lakebase is described as fast lookup, but remote latency cannot meet the 50 ms local target. | Local encrypted recall is on the interaction path. Lakebase is durable and optional. | Director retains D-06. |
| C-M-02 | Managed Agent Memory is recommended for new agents, but Motherbricks needs custom revisions, provenance, retention and tombstones. | Use a Motherbricks-owned Lakebase schema for MVP. Evaluate managed Agent Memory later. | Director retains this recommendation. |
| C-M-03 | The self-managed Agent Memory guide says `Legacy` but also recommends direct SQL for a custom schema. | Use current Lakebase project APIs and a custom schema. Do not use legacy agent-server templates or the legacy Unity Catalog memory API. | Director should state this distinction to avoid claiming that Lakebase itself is deprecated. |
| C-M-04 | D-14 requires deletion of local MOG data, while the exit test requires offline deletion to propagate later. | Keep an encrypted Motherbricks-level, metadata-only deletion journal after MOG content deletion. Security SEC-036 and Architecture independently agree. | Director shall integrate the exception. No further product decision is needed. |
| C-M-05 | Workspace mobility suggests shared recall, while D-06 and D-08 require isolation and conversation affinity. | Switching workspace selects a different recall scope. Explicit copy creates a new target-scope memory. | UX must show the active scope. |
| C-M-06 | `approved_context` can be misread as permanent network consent. | The type permits local recall only. Every outbound payload needs its own approval. | Director should include this glossary note. |
| C-M-07 | Databricks examples use `sslmode=require`, but that mode does not verify the server. | Require `verify-full` with `sslrootcert=system`. | Platform validates on macOS and Windows. |
| C-M-08 | Lakebase Search is useful but enablement is irreversible and drops connections. | Make enablement an administrator capability step, not an app side effect. Basic sync does not need it. | Platform and administrator documentation. |
| C-M-09 | Wall-clock last-writer-wins would simplify sync but can discard corrections or resurrect deletion. | Use revisions, idempotency keys, conflict records and terminal tombstones. | Architecture integrates the state machine. |
| C-M-10 | Managed memory `actor_id` can look like an authorization boundary. | Treat only the managed store as its security boundary. Use RLS and exact application filters in the custom Lakebase schema. | Security review and cross-scope tests. |
| C-M-11 | Local semantic recall needs model-specific vectors, but a fixed dimension would prevent model changes. | Version embedding generations and rebuild derived indexes from canonical text. | Architecture and model owners provide model metadata. |
| C-M-12 | Team M proposes 150 ms retrieval-only and 750 ms end-to-end semantic budgets. Team A NFR-A-005 proposes 250 ms including local query embedding and vector search. | Measure embedding generation separately from retrieval. Keep the 50 ms structured target unchanged. Select one end-to-end semantic target after reference-environment benchmarks. | Director and Architecture shall resolve before NFR sign-off. |

## 14. Open issues

### Blocking before MVP implementation completes

1. **Security key design:** Select and approve the local encryption library, per-MOG key wrapping, key rotation, lock behavior and export encryption.
2. **Workspace identity integration:** Platform P-C-029 defines workspace-ID resolution from profile data or the same-host response header. Architecture must expose that stable ID to `MemoryStore` instead of a profile alias.
3. **CLI-to-Lakebase path:** Platform must verify that the supported CLI U2M flow can generate a Lakebase credential on each MVP desktop platform.
4. **Reference hardware and semantic budget:** Architecture must define `REF-MAC` and `REF-WIN`. The director must resolve C-M-12 before MEM-013 sign-off.
5. **Remote authorization:** Security must approve the least-privilege role and exact server-enforced owner, workspace and MOG policy. Runtime roles must not bypass that policy.
6. **Per-MOG key lifecycle:** Security must approve key wrapping, key rotation, cryptographic erasure and the encrypted deletion receipt defined by Team S and Team M.

### User confirmation

None.
These issues are technical release gates rather than existential product questions.

### Later release

1. Managed Agent Memory adapter.
2. Agent Sessions adapter for non-Genie local agent interactions.
3. Managed memory pipelines after their writes can preserve Motherbricks provenance and revision rules.
4. Shared cross-MOG memory with explicit membership and authorization.
5. Remote Lakebase semantic search as an interactive option after workspace-specific benchmarks.

## 15. Sources

All sources are Databricks primary documentation.
All were accessed on 2026-11-14.

1. Lakebase authentication, credential lifetime, workspace scope, pooling limits, connection limits and rotation examples:  
   https://docs.databricks.com/aws/en/oltp/projects/authentication
2. External application SDK connection pattern and fresh credentials for new pooled connections:  
   https://docs.databricks.com/aws/en/oltp/projects/external-apps-connect
3. Lakebase connection strings and TLS requirement:  
   https://docs.databricks.com/aws/en/oltp/projects/connection-strings
4. Lakebase TLS peer verification with `sslmode=verify-full` and system roots:  
   https://docs.databricks.com/aws/en/oltp/projects/logical-replication-publication
5. `lakebase_vector` extension, pgvector compatibility, `lakebase_ann`, filtering and tuning behavior:  
   https://docs.databricks.com/aws/en/oltp/projects/lakebase-vector
6. Lakebase Search requirements, irreversible enablement, compute restart and external embeddings:  
   https://docs.databricks.com/aws/en/oltp/projects/lakebase-search
7. Lakebase Search GA release note for AWS:  
   https://docs.databricks.com/aws/en/release-notes/lakebase/
8. Current managed Agent Memory Beta, identity, retrieval, concurrency and access limitations:  
   https://docs.databricks.com/aws/en/agents/agent-memory/managed-memory
9. Agent Sessions Beta and independent session/memory lifecycle:  
   https://docs.databricks.com/aws/en/agents/agent-memory/managed-sessions
10. Store-level permissions and security-boundary behavior:  
    https://docs.databricks.com/aws/en/agents/agent-memory/store-permissions
11. Managed memory-pipeline behavior and its ability to rewrite direct entries:  
    https://docs.databricks.com/aws/en/agents/agent-memory/memory-pipelines
12. Self-managed custom-schema guidance and legacy label:  
    https://docs.databricks.com/aws/en/agents/agent-memory/self-managed-memory
13. Current Agent Memory and Agent Sessions positioning:  
    https://docs.databricks.com/aws/en/agents/custom-agents/stateful-agents
14. Legacy managed memory API warning:  
    https://docs.databricks.com/aws/en/agents/agent-memory/memory-store-api
15. Lakebase Data API, RLS and least-privilege guidance:  
    https://docs.databricks.com/aws/en/oltp/projects/data-api
16. Lakebase database roles and privileges, including bypass considerations:  
    https://docs.databricks.com/aws/en/oltp/projects/manage-roles-permissions
17. Lakebase customer-managed key requirements and project status:  
    https://docs.databricks.com/aws/en/oltp/projects/customer-managed-keys

