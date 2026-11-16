# ADR 0023: Request identity headers (User-Agent and AI Gateway request tags)

- Status: Accepted (decision 1 amended by ADR 0041)
- Date: 2026-11-16
- Issue: #197
- Owner: security-lead (implemented by impl-197)

## Context

SEC-06a asks HttpTransport to add `Databricks-Ai-Gateway-Request-Tags` (`app`, `mog`, `op`) to each Unity Gateway call. SEC-06b asks for `User-Agent: bottle/<ver> (<os>; mog)` on each call. B0 fixes the product token `bottle` and the tag `app=bottle`. D57 says the tags header is a JSON string map. The real R7 transport belongs to #207 (PR #256, not merged when this ADR is written). SP-18 (#136) is open: it is not known if `MCP_SERVICE` usage rows keep the tags. The IPC tags argument (`Tags`) already exists on `stt_transcribe`, `model_invoke`, `mcp_call` and `dbx_http`, so no contract changes.

## Decisions

1. **One pure module.** `mog_core::request_identity` builds the header values and has the `RequestDecorator` trait. `BottleIdentity` is the implementation. R7 and R8 call it for every request. The `HttpTransport` trait does not change in this ADR. `HttpTransport::request` takes no `Tags` and no call kind, so the real transport needs a trait change (through the contract owner) and a map from route ID to `CallKind`. No issue owns the reqwest wiring yet: #207 (PR #256) owns only the host policy and lists SEC-06a/b out of scope. So the wiring gap is `TODO(#197)`, and #197 stays open.
2. **User-Agent.** The value is `bottle/<version> (<os>; mog)`. The version is the `mog-core` crate version, which is the workspace and app version (a test checks it against `src-tauri/tauri.conf.json`). The OS token is `macos`, `windows` or `linux`, else `other`. No free text goes in the header.
3. **Tags value.** The value is the JSON map `{"app":"bottle","mog":"<id>","op":"<op>"}`, in this key order, with only these 3 keys. `op` uses the IPC wire names of `TagOp`.
4. **No PII, transcript, token or URL in tags.** The MOG ID must have 1 to 64 ASCII letters, digits, `_` or `-`. So a URL, an e-mail address, a space, a quote or a line break cannot pass. The full value must not match a rule-7 secret pattern (`token_guard::contains_secret`). The slug rule and the 3 fixed keys keep the value at 128 bytes or less (a test checks the longest valid value). A bad value fails closed: the request is not sent. Each error has a content-free code (`tags_mog_empty`, `tags_mog_too_long`, `tags_mog_chars`, `tags_secret`, `tags_missing`).
5. **Which calls get tags.** `CallKind::UnityGateway` (model and ASR calls on `/ai-gateway/…`) must have tags; no tags is the error `tags_missing`. `CallKind::Mcp` also gets tags, for SP-18 (`TODO(#136)`). If SP-18 finds that MCP rows drop the tags, attribution uses the User-Agent (the SP-18 fallback). `CallKind::Workspace` (Genie REST, SCIM, Knowledge, Apps) gets the User-Agent only. The MOG ID goes only where usage rows record it.
6. **No spoofing.** `RequestDecorator::decorate` removes every caller-set `User-Agent` and tags header (any ASCII case) before it adds the Bottle values. On error the header list does not change.
7. **Loopback.** `LoopbackEndpoint::call_kind` maps the model proxy to `UnityGateway`, the Genie One MCP proxy to `Mcp` and `bottle-mcp` to none. `LoopbackRequest` holds no harness header, so the handlers build each upstream request with the decorator only (`TODO(#113)`, `TODO(#139)`).
8. **MCP.** `mcp_transport::outbound_identity_headers` gives the headers that every R8 implementation must send.

## Not in scope

- NFR-24 (`stage=<x>` budget tag) is not in the requirement list of #197. It needs a new tag key and a `Tags` contract change.
- The live check of `system.ai_gateway.usage` (AT-36, AT-39) needs a real workspace (`needs:human`, `needs:admin`). The steps are on #197 and in `docs/admin/usage-attribution.md`.
