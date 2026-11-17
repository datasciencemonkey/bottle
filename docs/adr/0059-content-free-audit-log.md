# ADR 0059: The content-free audit log

- Status: Accepted
- Date: 2026-11-17
- Issue: #140 (SEC-23, RTR-30, PG-15, AT-08; 9.2 "Main agent action log")
- Owner: impl-140 for security-lead
- Contract change: #442

## Context

SEC-23 says the app must write a content-free audit entry for each ask, context send, Page call, agent action and sync. Entries hold IDs, sizes and hashes only. RTR-30 adds the tool name for an agent action. PG-15 asks for an entry for each Pages API call. AT-08 says "the audit hash matches": the hash in the entry equals the hash of the text that went to Genie (G-35a).

9.2 keeps the main agent action log for 30 d, with an admin range of 7-90 d, and the deletion is "Rotation". U9 says Bottle deletes no user content automatically. Only content-free security data (the audio journal, logs, the action log and consent records) keeps an automatic expiry.

On main:

- R10 holds the audit log in the one SQLCipher file (4.3), but migration 0001 has no audit table.
- The 4.5 `audit_append` row has `{kind, mogId, opId, sizes}` and no hashes.
- `audit_append` is not routed: the gateway returns `not_implemented`.
- The call sites are stubs: ask (#33), context send (#152), Page call (#203), agent tool (#141, #112) and sync (v1).

Contract files belong to contract-lead.

## Decisions

1. **Two layers.** `mog_core::audit` (Rust, R10) validates and stores entries, and runs the rotation. `src/brain/audit-log.ts` is the brain client that the call sites use. The client checks the same closed sets and sends `audit_append`. Rust checks again, because the brain is not trusted with the store.
2. **Entry shape.** `{ atMs, kind, mogId, opId, sizes, hashes }`. The host stamps `atMs` with the wall clock. The brain does not send a time.
3. **Content-free by construction.** Each string in a stored entry is one of these:
   - a name from a closed set: the kind, the tool, a size key or a hash key;
   - a lowercase UUID (`mogId`);
   - an ID-shaped `opId`: 1-64 chars of `[A-Za-z0-9_-]` that start with a letter or digit and hold no 4.6 rule-7 secret;
   - a SHA-256 value in 64 lowercase hex chars.

   Sizes are whole counts. Anything else is refused, and a refused call writes nothing. The client also refuses any extra field, so no `question` or `text` field can ride along. Errors carry a fixed code only and never echo the refused value.
4. **Kind wire format.** `kind` stays a string, so this needs no contract change: `ask`, `context_send`, `page_call`, `sync`, or `agent_action:<tool>`. `<tool>` is one of the 7 `bottle-mcp` tools (4.7 `BottleTool`) or `genie_one` (the agent's Genie One MCP calls through the proxy, AT-42). A bare `agent_action` is refused, because RTR-30 needs the tool name.
5. **Closed key sets.**
   - Sizes: `chars`, `bytes`, `items`, `question_chars`, `preamble_chars`, `preamble_items`, `answer_chars`, `args_bytes`, `result_bytes`, `synonyms` and `rows`.
   - Hashes: `sent` (the exact text sent to Genie, AT-08), `payload`, `args` and `result`.

   A new key is a code change in both layers.
6. **Hashes.** The client computes SHA-256 with WebCrypto (`sha256Hex`), and `textSizes` gives code points and UTF-8 bytes. A call site passes its text only to these two helpers, never to `record`. The `audit_append` arguments have no `hashes` field yet, so the client sends kind, mogId, opId and sizes, and returns the full record. The Rust `AuditLog::append` already takes the hashes. #442 adds the optional field (`TODO(#442)`).
7. **Store.** `AuditStore` is a trait. `MemoryAuditStore` is the test double, and it is also the store until #442 adds the `audit_log` table (next free migration) and routes `audit_append`. #442 has the exact DDL. It has no foreign key to `mog`, so a MOG delete does not fail on audit rows. Its index starts with `mog_id` (MEM-01).
8. **Rotation.** `AuditRetention` is 30 d by default and accepts only 7-90 d. `AuditLog::rotate(now_ms)` deletes entries older than the retention for all MOGs. An entry that is exactly 30 d old stays. `append` never deletes. There is no trigger, timer or background job. WipeService `run_security_expiries` (#124) is the only caller, which is the same pattern as the consent records (#63, PR #422). The rotation touches the audit store only, never user content (U9).
9. **Admin key.** Appendix C has no MDM key for the action-log retention. The host passes the admin value to `AuditRetention::new`. The key name is a requirements gap, noted on #140.
10. **Call sites.** This PR does not edit the call-site stubs. Each call-site issue calls `createAuditLog().record(...)`: #33 and #152 (ask and context send, with `hashes.sent`), #203 (Page call, with `hashes.payload`), #141 and #112 (agent tool, `agent_action:<tool>`), and the v1 sync.

## Consequences

- The entry shape and the validation are fixed now, so the call sites can build on them.
- Until #442 lands, no entry is stored outside memory, and the hash does not cross IPC.
- The AT-08 `[SEC-23]` test runs against the brain client. The live AT-08 run stays a person step.
