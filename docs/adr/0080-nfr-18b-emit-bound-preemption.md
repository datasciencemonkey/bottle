# ADR 0080: The NFR-18b emit bound leaves out preempted emits

- Status: Superseded (#508). The wall-clock tests and the preemption filter of this ADR were removed. They kept failing on shared runners ("266 of 1000 emits were preempted"). The `mog-core-ffi` C ABI is not in the MVP app path. The non-blocking checks stay in `crates/mog-core-ffi/tests/events.rs`. A device run of NFR-18b stays the release evidence (AT-17).
- Date: 2026-11-18
- Issue: #499
- Owner: security-lead

## Context

NFR-18b says that the `mog-core` event callback returns in 1 ms or less. The test `nfr_18b_event_reaches_the_callback_in_1_ms_or_less` in `crates/mog-core-ffi/tests/events.rs` times 1,000 calls of `emit_event` with the wall clock. It then checks that the p99 is 1 ms or less.

On the shared Linux CI runner, this check failed for 2 PRs that do not touch `mog-core-ffi`. The p99 values were 1.028 ms and 1.16 ms. A rerun passed.

`emit_event` does a secret scan, makes one JSON string and calls `try_send` on a bounded channel. These steps take microseconds. `try_send` never waits for the host. It wakes the delivery thread, which is parked.

On a runner with few CPUs, the scheduler can give the CPU to another thread in the middle of an emit. Often that thread is the delivery thread that the emit has just woken. The wall clock then counts time in which the core does no work and does not wait. A 1% tail of such preemptions is enough to move the p99 past 1 ms.

A wider bound would hide a real regression. The issue does not permit a skip or an ignore.

## Decision

1. **The test reads per-thread context switches.** On Linux, it reads them from `/proc/thread-self/status` immediately before and after each emit. These reads are outside the timed window.
2. **Only emits that were preempted and did not block are left out.**
   - An emit with only involuntary switches lost the CPU to the scheduler. It is left out of the p99.
   - An emit with a voluntary switch blocked: it waited on a lock, on the host or on I/O. It always stays in the bound, so a wait still fails the test.
3. **The bound stays at 1 ms.** At least 90% of the emits must stay in the bound. A runner that is too busy fails with "too busy to measure", and does not pass.
4. **Without the counters, the bound applies to every emit.** This is the case on macOS (the local `pre-push.sh` run) and on Windows. The check there is the same as before.
5. **Unit tests use synthetic samples to fix these rules on every OS:**
   - preempted emits are left out;
   - emits that blocked are kept;
   - the 90% floor holds;
   - without the counters, every emit counts.

## Consequences

- The Linux CI check measures core time, not scheduler noise, and keeps the 1 ms device target.
- The delivery latency checks (p50 1 ms or less, p99 20 ms or less) do not change.
- A device run of NFR-18b on the target hardware stays the release evidence (AT-17).
