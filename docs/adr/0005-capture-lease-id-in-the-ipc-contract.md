# ADR 0005: The capture lease ID in the IPC contract

- Status: Accepted
- Date: 2026-11-16
- Issue: #275 (contract change, found by PR #273)
- Owner: foundation-lead

## Context

CAP-02a (MUST, MVP-D) says: "WebView windows must start capture only through `capture_*` commands with a lease." Its check is "Each capture in the trace has a lease ID" (AT-17). The sketches in 4.5 (IPC table) and 4.7 (`CaptureState`) have no lease field. `capture_stop` and `capture_pause` take only `{mogId}`. The requirement row wins over the sketch.

## Decision

1. `CaptureState` gets `leaseId?: LeaseId`. It holds the lease that owns the mic now, and it is absent when no capture is live. When a PTT lease runs over a suspended hands-free lease (ARC-F9), `leaseId` is the PTT lease; the suspended lease shows again in `capture.state` when it resumes. Every `SegmentRef` (the `capture.segment` payload) gets a required `leaseId`, so each capture item in the trace names its lease (AT-17). In TS the field is added by declaration merging after the 4.7 block in `src/shared/interfaces.ts`, so the 4.7 block stays a verbatim copy. In Rust it is `CaptureState.lease_id` in `mog_core::types`.
2. `capture_stop` and `capture_pause` take `{mogId, leaseId}`. The gateway rejects a missing or malformed lease ID (`invalid_args`). R2 rejects an unknown or released lease with `unknown_lease` (`IpcError::UnknownLease`, the same code as R2 `CaptureError::UnknownLease`). R2 `stop` and `pause` take only the lease ID, so the host that wires R2 (#4) compares the lease owner with `mogId` and returns the same `unknown_lease` on a mismatch. One code for both cases stops a caller from probing the leases of other MOGs.
4. The other R2 errors get IPC codes now: `mic_busy` and `consent_required` (named in 4.5 for `capture_start`) and `invalid_mode`. A device fault maps to `host_failed`.
5. On the wire a lease ID is an opaque string of 1-64 characters from `[A-Za-z0-9-]`. The JSON Schema states the pattern `^[A-Za-z0-9-]{1,64}$`. R2 may keep a `u64` inside and send it as decimal text. A string avoids the 2^53 limit of JS numbers. Rust names: `types::LeaseId` (wire string, outgoing), `ipc::LeaseRef` (checked, incoming); R2 should name its integer differently (for example `LeaseNo`) to stop a wrong import.

## Consequences

- The capture commands still return `not_implemented` until #4 wires R2 behind them. A host timer must call `CaptureCore::tick()` at the deadline that R2 returns (#273).
- R2 has a `resume()`, but 4.5 has no `capture_resume` command. Follow-up: #279.
- The requirements need a fix: 4.5 should list `{mogId, leaseId}` for `capture_stop` and `capture_pause`, 4.7 `CaptureState` should have `leaseId?: string`, and 4.7 `SegmentRef` should have `leaseId: string`. This is posted on #275.
