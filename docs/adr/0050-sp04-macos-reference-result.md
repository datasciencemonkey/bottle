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
device. `docs/spikes/SP-04.md` has three runs on it.

## Decisions

1. **The Apple M3 Max is the macOS reference device for SP-04** (user decision). The Mac limit
   (600 ms) applies to it. M1 and M2 runs are optional extra data.
2. **macOS result:** load PASS (0.8-1.3 s); peak RSS FAIL (1.75-1.96 GB in every run); 10 s p95
   and tail p95 PASS on a lightly loaded host (593.1 ms and 232.6 ms at load average 23) and FAIL
   under heavy shared load (2.3-3.2 s and 1.4-2.3 s at load average 40-122).
3. **No step down the fallback chain now.** Local Parakeet on sherpa-onnx stays the STT default
   (B-rules, D78). Reasons: the latency failures come from CPU contention (the p50 real-time factor
   is 0.04 on a quiet host and the transcripts are the same), not from the engine; the next step,
   Databricks ASR, is opt-in only and does not help `SttPolicy=local_only` users; and the RSS
   failure is a memory question that the other engines in the chain do not solve on Windows.
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

- #45 packages sherpa-onnx 2.0.1 and the model (ADR 0049). #46 builds the engine lifetime.
- AT-21 must name the device and the host load with each STT latency result, because the same
  Mac gives a PASS or a FAIL by load alone.
