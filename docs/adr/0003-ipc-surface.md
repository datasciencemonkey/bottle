# ADR 0003: The IPC command and event surface

- Status: Accepted
- Date: 2026-11-16
- Issue: #223
- Owner: foundation-lead

## Context

Section 4.5 lists the IPC commands, the windows that may call them, and the events. ARC-4 needs capabilities per window label. ARC-5 needs Channels for streams and events for low-rate notifications. The issue left four details open.

## Decisions

1. **One table in `mog-core`.** The command table, the argument types and the gateway check live in `crates/mog-core/src/ipc.rs` (`mog_core::ipc`). The Tauri host (`src-tauri/src/ipc.rs`) and the C ABI (`mog_call`, R15) use the same `handle` function. `src/shared/ipc.ts` mirrors the table. A test compares it with the generated `src-tauri/schemas/ipc-commands.json`.
2. **Two window checks.** `src-tauri/build.rs` puts every command of the table in the Tauri app manifest. So Tauri rejects a command unless the capability file of the window lists `allow-<command>`. Then `mog_core::ipc::handle` checks the window class again and parses the arguments with `deny_unknown_fields`. A MOG or card view can send `view_emit` only for its own MOG ID.
3. **Brain-only traffic uses a Channel, not events.** In Tauri 2.12.5 a JS listener with the default target `Any` gets every `emit_to` event, and `core:event:allow-listen` has no event-name scope. So a Tauri event is visible to every window that may listen. The brain calls `brain_subscribe { onEvent: Channel }` (a brain-only command that 4.5 does not list) at start. Rust then delivers every core-to-brain item of 4.5 (`hotkey:ptt`, `agent:tool_call`, `view:input` and the others) as `{ event, payload }` on that Channel. The app crate never calls `emit` or `emit_to`; a test checks this. Only the brain-to-view events `mog:view_state` and `card:update` are Tauri events. They carry only what views render. Every view can read them, which we accept because all views show data of the same user. The brain may `emit_to` but not `emit`.
4. **Event names use `:`.** Tauri accepts only alphanumerics, `-`, `/`, `:` and `_` in event names. Each dotted name of 4.5 maps `.` to `:`. For example `hotkey.ptt` is `hotkey:ptt` and `mog.view_state` is `mog:view_state`. `view_emit` reaches the brain as `view:input`.
5. **The call envelope.** The front end calls `invoke(name, { args })`. Any other top-level key fails. One invoke handler serves all commands, so a new command needs one table row, one argument type and one capability entry.

## Notes

- `profile_login` takes `workspaceUrl`, as 4.5 says. It is the host for `databricks auth login --host`, not a request target, so 4.6 rule 1 does not apply. A test allows this field on `profile_login` only.
- `dbx_http` takes a `RouteId` from the fixed list of 4.6. A URL fails to parse.
- Error values name the command and give a fixed reason. They never quote an argument value or key, because serde messages do (R12, 4.6 rule 7). A test sends token-shaped values and checks the errors.
- `ChannelRef` accepts only the Tauri form `__CHANNEL__:<id>`.
- Every command returns `not_implemented` until its feature issue fills in the component (#225 stubs).
- The C ABI (`mog_call`, R15) still uses `StubIpcGateway`. #176 decides which window class an `ffi` caller has, then routes `mog_call` through `mog_core::ipc::handle`.
