# Latency benchmark (#171)

The benchmark measures the talk key and the PTT pipeline against the latency rows. It writes the
evidence for AT-21, AT-30 and AT-43 in the format of `docs/tests/EVIDENCE-TEMPLATE.md`.
The design choices are in `docs/adr/0016-latency-benchmark-harness.md`.

## Rows and budgets

| Row | Span | Samples | Limit |
|---|---|---|---|
| NFR-01a | key-up to preview chip | local STT, UC2 question | p95 ≤ 1.5 s |
| NFR-01b | key-up to preview chip | cloud STT, UC2 question | p95 ≤ 2.2 s |
| NFR-01c | key-up to local acknowledgement | all | every sample ≤ 100 ms |
| NFR-01d | key-up to inserted text | local STT, UC1 dictation | p95 ≤ 1.5 s |
| NFR-02 | key-down to mic live | all | p95 ≤ 150 ms (60 ms if SP-08 passes) |
| NFR-08a | key-ups seen (HID rig) | HID cycles | 100 % in ≥ 1,000 cycles |
| NFR-08b | OS event to HotkeyService | HID key events | p95 ≤ 15 ms |

- The percentiles use the nearest rank.
- A time row needs 20 samples or more. With fewer samples, the row is "too few samples".
- The report files: AT-21 has NFR-01a, NFR-01b, NFR-01c and NFR-02. AT-30 has NFR-08a and NFR-08b.
  AT-43 has NFR-01d. Each file covers only these rows. The other parts of each AT need their own evidence.

## Parts

- `mog_core::latency_bench::LatencyRecorder`: the mark API. One trial is one PTT press, with its
  STT path (`local` or `cloud`) and its route (`dictation` or `question`). Marks: `key_down`,
  `mic_live`, `key_up`, `local_ack`, `preview_chip`, `text_inserted`. It uses the #229
  `MonotonicClock`, so the tests use `SimClock`.
- The harness records only event names, the path, the route and times. It never records a
  transcript, audio or a token (SEC-16b, SEC-16c).
- `latency_bench::stats`: the percentiles and the budgets. `latency_bench::report`: the evidence file.
- `latency_bench::import`: the sample CSV and the `EV` lines of the SP-03 probe log (#5).
- `latency_bench::doubles`: a scripted pipeline on the simulated clock.
- `cargo run -p mog-core --example latency_bench -- --help`: the runner.

## Rules for a pass

The report writes `pass` only when all these rules hold:

1. The samples come from a reference device (`--device`). Samples from test doubles always give `not run`.
2. Each row of the gate has enough samples and is within its limit. A row over its limit gives `fail`.
3. The run has a date (YYYY-MM-DD), a runner, the build, the hardware and the OS.
4. AT-21 also needs the models and the resource use (peak RSS and mean CPU).

When a rule does not hold, the result is `partial` (some rows within) or `not run`. The report
lists each reason. `scripts/check_gate_evidence.py` checks the file like any other evidence file.

## Tests

```
cargo test -p mog-core --test latency_bench
```

The device tests skip and print the reason when the fixture is not set:

| Variable | Used by | Value |
|---|---|---|
| `BOTTLE_TEST_DEVICE` | AT-21, AT-43, AT-30 | The reference device (`docs/tests/README.md`). |
| `BOTTLE_TEST_HID_PORT` | AT-30 | The Pi Pico serial port. |
| `BOTTLE_BENCH_SP03_LOG` | AT-30 | The path of the SP-03 probe log of the 1,000-cycle run. |
| `BOTTLE_BENCH_SAMPLES` | AT-21, AT-43 | The path of the sample CSV of the device run. |
| `BOTTLE_BENCH_SP08_PASSED` | AT-21 | `1` if SP-08 passed (NFR-02 is then 60 ms). |

A skipped test is never a pass. Only a person on a reference device sets these variables.

## Device steps (a person, about 30 min per device)

These steps need a person. Do not run them on a machine that is not a reference device.
Never put a token or a profile secret in a file. These steps need no Databricks profile.

### Part A: AT-30 talk key on the HID rig, macOS (about 20 min)

You need: a reference Mac, a Raspberry Pi Pico with CircuitPython 9.x and `adafruit_hid`, and the
SP-03 probe. The probe is in `spikes/sp-03/` (PR #298 for #5). If #298 is not merged, use its branch:

```
cd <your bottle checkout>
git fetch origin pull/298/head:sp03-probe
git worktree add ../bottle-sp03 sp03-probe
cd ../bottle-sp03/spikes/sp-03 && cargo build --release
./target/release/sp03-probe status
```

Expected: `input_monitoring_preflight=1`. If it is 0, give your terminal Input Monitoring in
System Settings > Privacy & Security, then run `status` again.

1. Connect GP14 of the Pico to GND (this stops a run). Copy `spikes/sp-03/pico/code.py` to the
   Pico as `code.py`. Open TextEdit and put it in front.
2. Start the probe for 6 minutes, with the talk key only:
   ```
   ./target/release/sp03-probe listen --seconds 360 --no-carbon --no-sei --no-clamshell > ~/at30-mac.log
   ```
3. Disconnect GP14 from GND, then press the Pico reset (or unplug and plug it in). After 10 s the
   Pico presses Right Option 1,000 times (about 4 to 5 minutes). Do not touch the keyboard.
4. When the Pico prints `DONE` (or the LED stops), wait for the probe to stop. Then check the log:
   ```
   ./target/release/sp03-probe analyze ~/at30-mac.log
   ```
   Expected: `downs=1000 ups=1000 missed_key_ups=0 key_up_percent=100.000` and a p95 ≤ 15 ms.
5. In your Bottle checkout, write the evidence file and run the device test:
   ```
   cargo run -p mog-core --example latency_bench -- report --gate AT-30 \
     --device mac-m1-8gb-14.4 --date $(date +%F) --run-by "<your name>" \
     --build $(git rev-parse --short HEAD) \
     --hardware "$(sysctl -n machdep.cpu.brand_string), $(( $(sysctl -n hw.memsize) / 1073741824 )) GB" \
     --os "macOS $(sw_vers -productVersion)" \
     --sp03-log ~/at30-mac.log --out docs/evidence/AT-30-latency.md
   BOTTLE_TEST_DEVICE=mac-m1-8gb-14.4 BOTTLE_TEST_HID_PORT=/dev/cu.usbmodemXXXX \
     BOTTLE_BENCH_SP03_LOG=~/at30-mac.log cargo test -p mog-core --test latency_bench at_30
   ```
   Use `mac-m1-current` on the Mac with the current macOS. Expected: the runner prints
   `RESULT AT-30 pass` and the test passes. Keep `~/at30-mac.log` with the evidence (link it).
   The probe log has only talk-key events and times, no other keys.
6. Do it again on the second Mac. Write each device to its own file, for example
   `docs/evidence/AT-30-latency-mac-m1-current.md` (copy the output and change `--out`).

Windows 11: not possible yet. The SP-03 probe has no Windows mode (TODO(#5)) and the Windows hook is
#7. When they land, use the same `report --gate AT-30 --device win11-i5-16gb` step with the Windows log.

### Part B: AT-21 and AT-43 PTT pipeline (blocked)

The real pipeline does not call the mark API yet: TODO(#319) (it needs #14, #46, #58, #142 and #318).
When #319 lands, the steps are:

1. Start Bottle with the benchmark switch of #319 on. Use local STT (the default).
2. Do 25 PTT questions (UC2) and 25 dictations (UC1) into TextEdit or Notepad. Hold the talk key
   for 1 to 3 s each time.
3. If the cloud STT probe passed (#51), turn on Databricks ASR and do 25 PTT questions again.
4. While you do steps 2 and 3, record the resource use of Bottle:
   - macOS: `while sleep 5; do ps -o rss=,%cpu= -p $(pgrep -x Bottle); done | tee ~/bottle-res.txt`
   - Windows: `while ($true) { Get-Process Bottle | Select-Object WS,CPU; Start-Sleep 5 }`
   Write down the peak RSS in MB and the mean CPU percent.
5. Write the evidence files (one per gate) and run the device test:
   ```
   cargo run -p mog-core --example latency_bench -- report --gate AT-21 \
     --device <device-id> --date <YYYY-MM-DD> --run-by "<your name>" --build <sha> \
     --hardware "<cpu, memory>" --os "<os and version>" \
     --model "<STT model and version>" --model "<cleanup model>" \
     --rss-mb <peak> --cpu-pct <mean> [--sp08-passed] \
     --samples <sample file from #319> --out docs/evidence/AT-21-latency.md
   BOTTLE_TEST_DEVICE=<device-id> BOTTLE_BENCH_SAMPLES=<sample file> \
     cargo test -p mog-core --test latency_bench at_21_at_43
   ```
   Use `--gate AT-43` with the same sample file for `docs/evidence/AT-43-latency.md`.
