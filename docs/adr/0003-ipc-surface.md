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
3. **Event names use `:`.** Tauri accepts only alphanumerics, `-`, `/`, `:` and `_` in event names. Each dotted name of 4.5 maps `.` to `:`. For example `hotkey.ptt` is `hotkey:ptt` and `mog.view_state` is `mog:view_state`. `view_emit` reaches the brain as `view:input`.
4. **The call envelope.** The front end calls `invoke(name, { args })`. One invoke handler serves all commands, so a new command needs one table row, one argument type and one capability entry.

## Notes

- `profile_login` takes `workspaceUrl`, as 4.5 says. It is the host for `databricks auth login --host`, not a request target, so 4.6 rule 1 does not apply. A test allows this field on `profile_login` only.
- `dbx_http` takes a `RouteId` from the fixed list of 4.6. A URL fails to parse.
- Error values name the command and the field, never an argument value (R12 secret scrubbing).
- Every command returns `not_implemented` until its feature issue fills in the component (#225 stubs).
- The C ABI (`mog_call`, R15) still uses `StubIpcGateway`. #176 decides which window class an `ffi` caller has, then routes `mog_call` through `mog_core::ipc::handle`.
