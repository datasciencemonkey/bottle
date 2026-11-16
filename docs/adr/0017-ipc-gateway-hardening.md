# ADR 0017: The hardened IPC gateway

- Status: Accepted
- Date: 2026-11-16
- Issue: #36
- Owner: security-lead (implemented by impl-36)

## Context

Issue #36 asks R12 IpcGateway to permit each command only for its window labels (SEC-02a), to check each payload against a `deny_unknown_fields` schema (SEC-02b), and to scrub secrets from payloads, logs and error text (SEC-03). SEC-01 and AT-17 ask for IPC and FFI fuzz tests that find no secret. Issue #223 put the command table, the window check and the strict argument types in `mog_core::ipc::handle`. Issue #224 added the rule-7 scrubber (`token_guard`) and `ScrubbingGateway`, but no gateway used it. `ipc.rs`, `src-tauri/src/ipc.rs` and `types.rs` are contract files. The issue left the points below open.

## Decisions

1. **One hardened gateway in `ipc_gateway.rs`.** `CheckedGateway` calls `ipc::handle`. It changes no contract file. `hardened_gateway(log)` returns `ScrubbingGateway<CheckedGateway>`, so every reply gets the rule-7 scrub.
2. **Errors are fixed codes.** A refusal crosses only as `CoreError::IpcRefused { code }`. The code is the serde tag of the `IpcError` variant (`ERROR_CODES`). `ipc_error_code` has no wildcard arm, so a new variant must get a code. The gateway never sends the command name, the window label or a value. This closes the #224 finding that `IpcError::UnknownCommand` echoes the received name.
3. **One content-free log line per call.** The format is `ipc command=<name> window=<class> outcome=<code>`. `<name>` is a name from `COMMANDS`, or `unknown`. `<class>` is `brain`, `hub`, `mog`, `card` or `unknown`, never a MOG ID. The line goes through `scrub` as a second guard. The sink is the `GatewayLog` trait. The log backend is not chosen yet; `NoLog` drops the lines.
4. **No direct print in production code.** A test fails on `println!`, `eprintln!`, `print!`, `eprint!` or `dbg!` in `crates/mog-core/src`, `crates/mog-core-ffi/src` and `src-tauri/src` outside test modules. `testkit.rs` is exempt: it is test-fixture code that prints only a skip reason.
5. **Inbound arguments are checked, not scrubbed.** The gateway rejects a bad payload, but it does not change a valid one. A component that must refuse a secret (MB-02 `config_write`, #25) or remove one before it leaves the device (#90, rule 7 in R7 and R8) still sees the value and can return its own code. A scrub here would hide a token in a config instead of rejecting it.
6. **Host actions return `host_failed`.** `CheckedGateway` has no brain Channel. So `view_emit` and `brain_subscribe` return `host_failed`. The Tauri host does these actions itself.
7. **Labels outside ARC-3 get nothing.** The C ABI label `ffi` is not a window class, so every command returns `window_not_allowed` until #176 decides the FFI window class (PR #277, its ADR).
8. **Fuzz tests use a hand-written seeded PRNG.** The tests use splitmix64 with fixed seeds, so CI runs the same cases each time. They add no crate. A control test proves that the harness finds a leak in a gateway that echoes its arguments, and finds none after `ScrubbingGateway`.

## Gaps

- The Tauri host (`src-tauri/src/ipc.rs`, contract) calls `ipc::handle` directly. Its errors hold only a command name from `COMMANDS` and a label that the app set, and its replies are `{}` today. A later contract change can route its replies, errors and brain Channel items through the scrub. This is reported to security-lead.
- `ipc::IpcError::UnknownCommand` and `WindowNotAllowed` still hold the raw name and label (contract file). The gateway does not send them, and the Tauri host cannot reach them with a name that is not in `COMMANDS`.
- `TODO(#176)`: `mog_call` dispatches through `hardened_gateway`. The FFI fuzz test runs on the `mog_call` that is on main, and covers the new path with no change.
- `TODO(#25)`: `config_write` calls `token_guard::check_config_json` (MB-02).
