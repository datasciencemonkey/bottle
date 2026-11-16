# ADR 0006: The C ABI caller and the IPC gateway

- Status: Proposed
- Date: 2026-11-16
- Issue: #176
- Owner: mog-b2

## Context

ARC-1 says that `mog-core-ffi` exposes the same command and event envelope as IPC. ADR 0003 left one decision to #176: the window class of a C ABI caller. A later shell (for example Flutter) links the C ABI. That shell takes the place of the Tauri host and the brain window.

## Decisions

1. **The C ABI caller has the brain window class.** `mog_call` calls `mog_core::ipc::handle` with the label `brain`. So the C ABI can call the brain commands, and it cannot call view-only commands such as `view_emit`.
2. **The call envelope.** The request is `{"command": "<name>", "args": {...}}`. Both keys are necessary, and any other key fails with `invalid_json`. `args` is the `args` object of `invoke(name, { args })`.
3. **The reply envelope.** The C ABI has no resolve and reject, so the reply is `{"ok": <result>}` or an error object. The error object is the serialized `IpcError`, for example `{"error":"not_implemented","command":"cli_status"}`. An unknown command name is not echoed: the reply is `{"error":"unknown_command"}`. Other codes are `invalid_argument` (null or not UTF-8), `invalid_json`, `not_initialized`, `not_supported`, `secret_blocked` and `internal`.
4. **Events use the callback, not `brain_subscribe`.** The core sends each core-to-brain item as `{"event": "<name>", "payload": {...}}`, the message shape of the brain Channel. One delivery thread calls the one host callback, so the core side never waits for the host (NFR-18b). Over the C ABI, `brain_subscribe` returns `not_supported`.
5. **No secret crosses (ARC-6).** `mog_init` accepts only `{}` or null, so no token, host or header enters through the config. Each reply and each event passes the patterns of section 4.6 rule 7. A reply that matches becomes `{"error":"secret_blocked"}`. An event that matches is dropped.

## Notes

- The C header `crates/mog-core-ffi/include/mog_core.h` comes from cbindgen 0.29.4 (pinned). A test regenerates it and compares.
- The pattern check in `mog-core-ffi` is temporary. It moves to the shared R12 scrubber (#36).
