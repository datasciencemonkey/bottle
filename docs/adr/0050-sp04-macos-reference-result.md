# ADR 0050: SP-04 macOS reference device and the local STT fallback decision

- Status: Accepted
- Date: 2026-11-16
- Issue: #44 (SP-04); related #282 (NFR-05), #283 (PTT VAD segment length), #45, #46
- Owner: mog-b2

## Context

SP-04 asks for Parakeet TDT 0.6B v3 int8 latency on M1 and M2 Macs and on an i5-1235U or
Ryzen 5 7530U laptop, and for the fallback choice. The limits are: 10 s p95 600 ms or less (Mac),
900 ms or less (Windows); tail p95 400 ms or less; RSS 1.0 GB or less; load 3 s or less. The
fallback chain is Databricks ASR (opt-in, after its probe) → FluidAudio ANE (v1) → NeMo GGUF →
Moonshine v2 (English). No M1 or M2 Mac is available. On 2026-11-16 the user said that the
development Mac (Apple M3 Max, 12 P-cores, 64 GB, macOS 26.7.1) is a valid macOS reference
device. `docs/spikes/SP-04.md` has five twelve-thread runs on it (runs 1-5).

## Decisions

1. **The Apple M3 Max is the macOS reference device for SP-04** (user decision). The Mac limit
   (600 ms) applies to it. M1 and M2 runs are optional extra data.
2. **macOS result:**
   - Load: PASS with a warm cache (0.82-1.56 s). The cold cache was not measured (#432).
   - Peak RSS: FAIL in every run (1.77-1.98 GB).
   - 10 s p95: PASS in 1 of 4 runs (run 1, 593.1 ms at load average 23-24). FAIL in the others
     (1.9-3.2 s at load averages from 13 at the start up to 163).
   - Tail p95 (CONDITIONAL on #283): PASS in run 1 only (232.6 ms).
3. **No step down the fallback chain now.** Local Parakeet on sherpa-onnx stays the STT default
   (B-rules, D78). Reasons:
   - The latency failures match CPU contention from other sessions on the same Mac. The p50
     real-time factor is 0.04 at load 23 and 0.08-0.15 at load 40-122, with the same transcripts.
     No quiet-host run has tested this yet: #432 does. If the 10 s p95 fails on a quiet host,
     this decision must be made again.
   - The next step, Databricks ASR, is opt-in only. It does not help `SttPolicy=local_only` users.
   - The memory of the other engines in the chain was not measured. The RSS question goes to #282
     first.
4. **NFR-05 stays open in #282.** The first steps there are the Handy pattern of a separate STT
   worker process that exits on unload (so all onnxruntime memory goes back to the OS), smaller
   decode segments, and the onnxruntime arena options. If none gets to 1.0 GB, #282 proposes a
   changed NFR-05 value or the FluidAudio ANE step on macOS.
5. **The tail criterion stays CONDITIONAL on #283** (PTT VAD segment length). #46 must decode
   closed VAD segments during the hold (CAP-52) and keep the STT worker at normal priority.
6. **Windows stays open.** `spikes/sp-04/run.ps1` must run on the Windows laptop
   (`needs:human`, `needs:windows`). The 900 ms limit applies there. Until then, SP-04 is not
   closed.

## Consequences

- #45 packages sherpa-onnx 2.0.1 and the model (ADR 0049 in PR #416). #46 builds the engine
  lifetime.
- The STT latency results for AT-21 must state the device and the host load. A requirements
  change for this, and for the SP-04 row (it still names M1/M2), is noted on #44 for the
  requirements owner.
