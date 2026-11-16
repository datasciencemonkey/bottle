# ADR 0005: The capture lease ID in the IPC contract

- Status: Accepted
- Date: 2026-11-16
- Issue: #275 (contract change, found by PR #273)
- Owner: foundation-lead

## Context

CAP-02a (MUST, MVP-D) says: "WebView windows must start capture only through `capture_*` commands with a lease." Its check is "Each capture in the trace has a lease ID" (AT-17). The sketches in 4.5 (IPC table) and 4.7 (`CaptureState`) have no lease field. `capture_stop` and `capture_pause` take only `{mogId}`. The requirement row wins over the sketch.

## Decision

1. `CaptureState` gets `leaseId?: LeaseId`. It is set while a capture is live and absent otherwise. In TS the field is added by declaration merging after the 4.7 block in `src/shared/interfaces.ts`, so the 4.7 block stays a verbatim copy. In Rust it is `CaptureState.lease_id` in `mog_core::types`.
2. `capture_stop` and `capture_pause` take `{mogId, leaseId}`. The gateway rejects a missing or malformed lease ID (`invalid_args`). R2 rejects an unknown, released or foreign lease with the typed error `stale_lease` (`IpcError::StaleLease`).
3. On the wire a lease ID is an opaque string of 1-64 characters from `[A-Za-z0-9-]`. R2 may keep a `u64` inside and send it as decimal text. A string avoids the 2^53 limit of JS numbers.

## Consequences

- The capture commands still return `not_implemented` until #4 wires R2 behind them. A host timer must call `CaptureCore::tick()` at the deadline that R2 returns (#273).
- The requirements need a fix: 4.5 should list `{mogId, leaseId}` for `capture_stop` and `capture_pause`, and 4.7 `CaptureState` should have `leaseId?: string`. This is posted on #275.
