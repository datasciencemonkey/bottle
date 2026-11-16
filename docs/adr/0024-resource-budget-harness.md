# ADR 0024: The memory and idle CPU budgets are a mog-core harness with test doubles

- Status: Accepted
- Date: 2026-11-16
- Issue: #173 (depends on #79 SP-02 and #44 SP-04; soft, they are spikes)
- Owner: impl-173 (packaging-lead team)

## Context

Issue #173 asks to measure and enforce NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b and NFR-09,
with the results in the AT-21 report, and to apply the D73 overlay fallback if SP-02 fails.
The runs need the reference devices and a person. SP-02 (#79) has not run. SP-04 (#44) ran on an
Apple M3 Max (not reference hardware) and measured a Parakeet peak RSS of 1.91 GB. The 30 Hz
hit-poll is #81. The issue left open: where the harness lives, the statistics, the units, how
PresentMon is used, the window-plan rule, and how a run on other hardware is reported.

## Decision

1. The harness is the module `mog_core::resource_budget`, next to `latency_bench` (ADR 0016),
   with the same report rules. It has no new crate and no new dependency. The runner is the
   example `resource_bench`. It reuses the #229 parts: `testkit::device_from` for the skip,
   `SimClock` for the sampler, the evidence template and the `Scope | part` rule.
2. The sample file has four fixed columns (`kind,scenario,mogs,value`), and the value is a number.
   So no transcript, audio or token can go into a sample (SEC-16b, SEC-16c). The SP-04 import
   reads only the lines that start with `long_secs=` or `peak_rss_mb=`.
3. Memory is the RSS sum of the Bottle processes (with the WebKit or WebView2 processes). Shared
   pages count more than once, so the sum is on the safe side. A run uses its peak. The units are
   decimal (1 MB = 10^6 bytes), the same as the SP-04 bench; this is the stricter reading.
4. NFR-04b is (peak N=8 − peak N=1) / 7 ≤ 40 MB: the row says "measured with 8 MOGs". When
   the N=2 or N=4 runs show a larger step per MOG, the report shows it as a note.
5. NFR-05 needs a clip of 240 s or more ("measured over a 4 min clip"); a shorter clip is an
   `invalid run`.
6. CPU is in percent of one core, the mean of a run of 60 samples or more. This is stricter than
   percent of the whole machine. NFR-09 is the `hidden_panels` mean minus the `baseline` mean,
   and it must be less than 0.2 % (strict, as the row says). The baseline needs 0 MOGs, and
   `hidden_panels` and `rive_paused` need 1 MOG or more; else the run is invalid.
7. The rows say "Measured with `powermetrics` and PresentMon". PresentMon gives frame times, not
   the CPU use of a process. So the Windows CPU comes from `\Process(*)\% Processor Time`
   (`typeperf`), and PresentMon presents/s checks the Rive state: more than 1/s in `rive_paused`
   or less than 10/s in `rive_animating` makes the run invalid.
8. The window plan (D73, SP-02 fallback "default cap 4, then one overlay window per display"):
   SP-02 pass gives cap 8. A fail with focus steals, a CPU row over, or N=4 not within
   (N=1 ≤ 250 MB, (N=4 − N=1) / 3 ≤ 40 MB) gives one overlay window per display. A fail where N=4
   fits, both CPU rows are within and there are 0 focus steals gives cap 4 then the overlay. When
   the cap-4 inputs have no data, or SP-02 has not run, the plan is `undecided` (TODO(#79)):
   it never picks cap 4 from missing data.
9. The report writes `pass` only for a reference device with every row within budget and the
   full run record (date, runner, build, hardware, OS, and the STT model for NFR-05). Doubles give
   `not run`. A real run on other hardware (`--other-hardware`) can give `fail` but never `pass`,
   so the SP-04 M3 Max result shows as NFR-05 `over`, `fail`, and is not hidden.

## Consequences

- `cargo test -p mog-core --test resource_budget` runs the harness with no device.
- The device test skips with the reason until a person sets `BOTTLE_TEST_DEVICE` and
  `BOTTLE_RES_SAMPLES` (`docs/tests/RESOURCE-BENCH.md`).
- NFR-05 is expected to fail on the reference devices until the SP-04 fallback chain
  (section 8.3) gives a lighter STT path.
- The window model reads the plan when SP-02 runs (#79). Until then the MOG panels stay the default.
