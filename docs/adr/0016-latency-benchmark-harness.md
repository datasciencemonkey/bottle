# ADR 0016: The latency benchmark is a mog-core module with test doubles

- Status: Accepted
- Date: 2026-11-16
- Issue: #171 (depends on #5, #15 and #44; soft, they are spikes)
- Owner: impl-171 (packaging-lead team)

## Context

Issue #171 asks for a repeatable latency benchmark on the reference devices for NFR-01a..d, NFR-02,
NFR-08a and NFR-08b, with the results in the AT-21 report. The real pipeline is not complete:
the hotkey (#7), the capture lifecycle (#14), the STT engine (#46), the preview chip (#142) and
`insert_text` (#58) are open. The reference devices and the Pi Pico rig need a person (#270).
The issue left these choices open: where the benchmark lives, how it decides a pass, and how the
view reports the chip time.

## Decision

1. The benchmark is the module `mog_core::latency_bench`, not a new crate. The marks happen in
   `mog-core` components (HotkeyService, CaptureService, `insert_text`), so they call it directly.
   It needs no new dependency. The runner is the example `latency_bench`.
2. The mark API uses the #229 `MonotonicClock`. Tests use `SimClock` and a scripted pipeline double.
3. A mark is a closed enum with no payload. The sample file has five fixed columns. So no
   transcript, audio or token can go into the benchmark data (SEC-16b, SEC-16c).
4. Budgets come from the rows: NFR-01a, NFR-01b, NFR-01d and NFR-02 limit the p95; NFR-01c has no
   percentile, so every sample must be within 100 ms. NFR-02 is 60 ms when SP-08 passes.
   NFR-08a needs 1,000 cycles or more with every key-up.
5. The percentile is the nearest rank. A time row needs 20 samples or more; below that the p95 is
   the maximum and has no meaning.
6. The report writes `pass` only for samples from a reference device, with every row within budget,
   a run date, a runner, the build, the hardware and the OS; for AT-21 also the models and the
   resource use. Samples from doubles always give `not run`. Each file covers only its latency rows.
7. The chip and the acknowledgement render in the view. Their times need a view-to-core event,
   which is a contract change (#318). Until it lands, the API stays internal to `mog-core`.
   The pipeline wiring and the sample export are #319.
8. AT-30 on macOS reads the `EV` lines of the SP-03 probe (#5), so the rig run can start now.
   NFR-08a and NFR-08b name HotkeyService, and the probe is not the HotkeyService that ships. So a
   file from probe samples is at most `partial`. The HotkeyService log is #323.
9. A press with the start mark of a row but no end mark (no chip, no acknowledgement, no mic, no
   inserted text) counts as over the limit. A missed step is the failure that the rows guard against.
10. Each benchmark file has `Scope | part: <IDs> only`. `scripts/check_gate_evidence.py` does not
    accept `part` files alone for a ticked checklist row, so the latency rows cannot stand in for
    the whole AT.

## Consequences

- `cargo test -p mog-core --test latency_bench` runs the harness in CI-able tests with no device.
- The device tests skip with the reason until a person sets the fixture variables.
- AT-21 and AT-43 device runs wait for #319. AT-30 on macOS can run now with the Pico rig.
- The runner trusts the person who gives `--device`. The evidence review (AT-22) checks the run.
