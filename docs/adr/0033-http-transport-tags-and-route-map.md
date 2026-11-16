# ADR 0033: R7 HttpTransport takes the request tags and has a fixed route map

- Status: Accepted
- Date: 2026-11-16
- Issue: #342 (for #197)
- Owner: contract-lead

## Context

ADR 0023 built the request identity headers (SEC-06a, SEC-06b) but could not wire them into R7. `HttpTransport::request` took no `Tags` and no call kind. Issue #342 asks for a trait change, a fixed map from route ID to `CallKind`, and an owner for the real `reqwest` client. The IPC `tags` argument already exists on `dbx_http`, `model_invoke`, `mcp_call` and `stt_transcribe`, so no IPC command, TS type or fixture changes.

## Decisions

1. **The trait takes the tags.** `HttpTransport::request(profile, route_id, body, tags: &Tags)`. Every caller of R7 has the tags of its IPC call. This replaces the first part of ADR 0023, decision 1.
2. **One fixed route map.** `http_transport::ROUTES` lists every route ID that R7 sends with its `CallKind`. The 19 `ipc::RouteId` wire names of 4.6 are `Workspace`. Two Unity Gateway model routes are `UnityGateway`: `mlflow.chat_completions` and `mlflow.embeddings` (`POST /ai-gateway/mlflow/v1/...`, NFR-21a, D57). `model_invoke` uses them. The cloud ASR route joins the map when its issue picks the endpoint. A test reads the `RouteId` JSON Schema, so a new IPC route without a map row fails.
3. **Fail closed on an unknown route.** `route_call_kind` matches exactly. An unknown ID, a URL or a path gives `None`. `outbound_headers` then returns `CoreError::Unavailable { component: "R7", reason: "unknown_route" }`, and the transport sends nothing (ARC-6, 4.6 rule 1).
4. **One header helper.** `http_transport::outbound_headers(decorator, route_id, tags)` is the call that every R7 implementation makes per request. It is the R7 twin of `mcp_transport::outbound_identity_headers`. A tags error keeps its content-free `tags_*` code as the `Unavailable` reason.
5. **Owner of the real client.** The `reqwest` + `rustls` client is a feature, not a contract. It needs the host policy of #207 (PR #256), R6 tokens and the rate bucket. contract-lead does not own it. It is split into #364 (MVP-D, area:security). `TODO(#364)` marks the gap. #197 stays open until #364 sends the headers.

## Consequences

- No caller of `HttpTransport::request` existed outside the stub, so the signature change breaks nothing.
- SEC-06a and SEC-06b hold for the helper in tests. They hold on the wire only after #364.
