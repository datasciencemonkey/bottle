# ADR 0006: The C ABI caller and the IPC gateway

- Status: Accepted
- Date: 2026-11-16
- Issue: #176
- Owner: mog-b2

## Context

ARC-1 says that `mog-core-ffi` exposes the same command and event envelope as IPC. ADR 0003 and ADR 0019 (decision 7) left one decision to #176: the window class of a C ABI caller. A later shell (for example Flutter) links the C ABI. That shell takes the place of the Tauri host and the brain window. ADR 0019 also gave #176 the job of making `mog_call` use the hardened R12 gateway.

## Decisions

1. **The C ABI caller has the brain window class.** `mog_call` calls `hardened_gateway_with(NoLog, FfiComponents)` (ADR 0019) with the label `brain`. So the C ABI can call the brain commands, and it cannot call view-only commands such as `view_emit`. Each call gets the gateway's window check, the `deny_unknown_fields` check, one content-free log line and the reply scrub. The log sink is `NoLog` until the log backend is chosen.
2. **The call envelope.** The request is `{"command": "<name>", "args": {...}}`. Both keys are necessary. Any other key fails with `invalid_json`. `args` is the `args` object of `invoke(name, { args })`.
3. **The reply envelope.** The C ABI has no resolve and reject, so the reply is `{"ok": <result>}` or `{"error": "<code>"}`. The code is `CoreError::code`, for example the `IpcRefused` code `not_implemented`. A reply never holds a command name, a window label or a value (ADR 0019 decision 2). The ABI adds the codes `invalid_argument` (null or not UTF-8), `invalid_json`, `not_initialized`, `not_supported`, `secret_blocked` and `internal`.
4. **Events use the callback, not `brain_subscribe`.** The core sends each core-to-brain item as `{"event": "<name>", "payload": {...}}`, the message shape of the brain Channel. One delivery thread calls the one host callback, so the core side never waits for the host (NFR-18b). The queue holds `MOG_EVENT_QUEUE_CAPACITY` (1024) events. When it is full, a new event is dropped and counted. Over the C ABI, `brain_subscribe` passes the gateway checks, and then `FfiComponents` returns `not_supported`.
5. **No secret crosses (ARC-6).** `mog_init` accepts only `{}` or null, so no token, host or header enters through the config. Each reply and each event passes the shared guard `mog_core::token_guard` (ADR 0013). A reply that matches becomes `{"error":"secret_blocked"}`. An event that matches is dropped.
6. **Host rules for the callback.** The host must clear the callback before it frees `user_data` or exits. The library does not support unloading. `mog_set_event_callback` blocks until a callback in flight returns, so the host must not call it while it holds a lock that the callback takes.

## Notes

- The C header `crates/mog-core-ffi/include/mog_core.h` comes from cbindgen 0.29.4 (pinned). A test regenerates it and compares.
