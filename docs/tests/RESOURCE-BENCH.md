# Memory and idle CPU benchmark (#173)

The benchmark checks the memory and idle CPU of Bottle against the budget rows. It writes the
AT-21 evidence for these rows in the format of `docs/tests/EVIDENCE-TEMPLATE.md`.
The design choices are in `docs/adr/0024-resource-budget-harness.md`. It uses the same report
rules as the latency benchmark (`docs/tests/LATENCY-BENCH.md`).

## Rows and budgets

| Row | Run | Measure | Limit |
|---|---|---|---|
| NFR-04a | shell + brain + 1 MOG, without the STT model and the agent harness | peak RSS | ≤ 250 MB |
| NFR-04b | the same with 8 MOGs | (peak N=8 − peak N=1) / 7 | ≤ 40 MB |
| NFR-05 | Parakeet over a clip of 4 min (240 s) or more | peak RSS | ≤ 1.0 GB |
| NFR-06a | `rive_paused`: MOGs visible, Rive paused, no capture | mean CPU | ≤ 1 % |
| NFR-06b | `rive_animating`: 8 MOGs, Rive animating, no capture | mean CPU | ≤ 3 % |
| NFR-09 | `hidden_panels` (hit-poll on at 30 Hz) minus `baseline` | mean CPU | < 0.2 % |

- Memory is the RSS sum of all Bottle processes (on macOS also the WebKit processes; on
  Windows the WebView2 processes). The sum counts shared pages more than once, so it is on the
  safe side. The units are decimal: 1 MB = 10^6 bytes, 1 GB = 10^9 bytes (the same as SP-04).
- CPU is in percent of ONE core (`powermetrics` CPU ms/s ÷ 10, or the Windows counter
  `\Process(*)\% Processor Time`). This is stricter than percent of the whole machine.
- An RSS run needs 5 samples. A CPU run needs 60 samples (60 s at 1 per second).
- PresentMon gives presents per second, not CPU. It checks the run: more than 1 present/s in
  `rive_paused` means Rive is not paused; less than 10 present/s in `rive_animating` means Rive
  does not animate. Such a run is an `invalid run`, not `within`.
- `baseline` is shell + brain with no MOG panel (0 MOGs) and the hit-poll off. `hidden_panels`
  has the MOG panels hidden (1 MOG or more) and the 30 Hz hit-poll on (TODO(#81): the hit-poll
  lands with #81). `rive_paused` needs 1 MOG or more. A run that breaks these rules is an
  `invalid run`.
- NFR-04b is the mean from N=1 to N=8. When the N=2 or N=4 runs show a step of more than 40 MB
  per MOG, the report shows that step as a note.

## SP-02 and the D73 window plan

The report also gives the SP-02 result from the same samples and the window plan:

1. SP-02 passes (NFR-04a, NFR-04b, 8 MOGs ≤ 550 MB, NFR-06a, NFR-06b, 0 focus steals): cap 8,
   one panel per MOG.
2. SP-02 fails with focus steals, or 4 MOGs do not fit (N=1 ≤ 250 MB and (N=4 − N=1) / 3 ≤ 40 MB),
   or a CPU row is over: one overlay window per display hosts all mascots (D73).
3. SP-02 fails, 4 MOGs fit, NFR-06a and NFR-06b are within, and there are 0 focus steals: a
   default cap of 4 MOG panels; more MOGs go into the D73 overlay.
4. SP-02 fails, 4 MOGs fit, but a CPU row or the focus steals have no data: undecided. The plan
   never picks cap 4 from missing data.
5. SP-02 has not run (a criterion has no data and none fails): undecided. SP-02 is open: TODO(#79).

## Rules for a pass

The report writes `pass` only when all these rules hold:

1. The samples come from a reference device (`--device`). Test doubles always give `not run`.
2. Each row has enough samples, measures what the row asks, and is within its limit.
3. The run has a date (YYYY-MM-DD), a runner, the build, the hardware and the OS, and for NFR-05
   the STT model.

A row over its limit gives `fail`. This is also true for a real run on hardware that is not a
reference device (`--other-hardware`): such a run can fail, but it can never pass. When no row
is over and a rule does not hold, the result is `partial` (some rows within) or `not run`.
Each file has `Scope | part: NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b, NFR-09 only`.

**Known fail:** SP-04 (#44, PR #281) measured a Parakeet peak RSS of 1.91 GB on an Apple M3 Max
(`docs/spikes/SP-04.md`). Its bench output gives NFR-05 `over` and the result `fail`:

```
cargo run -p mog-core --example resource_bench -- init --out /tmp/sp04.csv
cargo run -p mog-core --example resource_bench -- report --other-hardware --date 2026-11-16 \
  --run-by impl-44 --build "PR #281" --hardware "Apple M3 Max 64 GB" --os macOS \
  --stt-model "parakeet-tdt-0.6b-v3 int8" --samples /tmp/sp04.csv \
  --sp04-bench crates/mog-core/tests/fixtures/resource_budget/sp04-bench-m3max.txt
```

The runner prints `RESULT AT-21 fail (resource rows)` and exits 1. The test
`nfr_05_sp04_m3_max_run_is_reported_as_a_fail` keeps this. SP-04 expects the same RSS on M1, M2
and Windows, so the fallback chain of section 8.3 (SP-04 row) applies.

## Parts

- `mog_core::resource_budget::samples`: the sample model. Every value is a number, so no
  transcript, audio or token can go in (SEC-16b, SEC-16c).
- `budgets`: the rows. `decision`: SP-02 and the window plan. `report`: the evidence file.
- `import`: the sample CSV and the tool outputs (`ps`, `tasklist`, `powermetrics`, `typeperf`,
  PresentMon, the SP-04 bench, `/usr/bin/time -l`). The SP-04 import reads only the lines that
  start with `long_secs=` or `peak_rss_mb=`, so it never reads the transcript table of the bench.
  Every import gives an error when no process matches the names (not a value of 0).
- `sampler`: `collect_rss` on any `RssProbe`. The tests use `ScriptedProbe` and the #229 `SimClock`.
- `cargo run -p mog-core --example resource_bench -- --help`: the runner.

The sample file:

```
# bottle-resource-samples v1
kind,scenario,mogs,value
rss,shell,1,182000000
cpu,rive_paused,8,0.4
presents,rive_paused,8,0
parakeet_rss,parakeet,0,910000000
parakeet_clip_s,parakeet,0,240
focus_steals,shell,0,0
```

## Tests

```
cargo test -p mog-core --test resource_budget
```

The device test skips and prints the reason when the fixture is not set:

| Variable | Value |
|---|---|
| `BOTTLE_TEST_DEVICE` | The reference device (`docs/tests/README.md`). |
| `BOTTLE_RES_SAMPLES` | The path of the sample file of the device run. |
| `BOTTLE_RES_SP04_BENCH` | Optional: the path of the SP-04 `bench-p.txt` of the same device. |

A skipped test is never a pass. Only a person on a reference device sets these variables.

## Device steps (a person)

Do not run these steps on a machine that is not a reference device. They need no Databricks
profile. Never put a token in a file. Quit every other app first (on macOS, Safari and Mail
too: they share the WebKit process names).

The steps have two parts:

- **Part A: Parakeet (NFR-05). You can run it now.** About 30 min: the SP-04 rerun (under 30 min,
  `docs/spikes/SP-04.md`) plus about 2 min to build the runner. Steps A1 to A3.
- **Part B: memory and CPU (NFR-04a, NFR-04b, NFR-06a, NFR-06b, NFR-09). BLOCKED** until the
  build can open N = 0, 1, 2, 4 and 8 MOGs and pause Rive (SP-02, TODO(#79)), and has the 30 Hz
  hit-poll (TODO(#81)). About 30 min. Steps B1 to B4, then step A3 again with the same file.

The runner writes the sample file itself (`init --out`, then `--append` on each command). Do not
use the shell `>` or `>>`: Windows PowerShell 5.1 writes UTF-16 with them. Write each `--name`
flag in full in each command, as below (a shell variable with all the flags does not work in zsh
or PowerShell).

### macOS

In your Bottle checkout on `main`, in zsh or bash: `cargo build -p mog-core --example resource_bench`,
then `RB=$PWD/target/debug/examples/resource_bench; rb() { "$RB" "$@"; }`.

- **A1.** `rb init --out ~/res.csv`
- **A2.** Run `spikes/sp-04/run.sh` (`docs/spikes/SP-04.md`, "Rerun on reference hardware"). It
  writes `~/sp04-work/out/bench-p.txt`.
- **A3.** Write the report and run the device test:
  ```
  rb report --device mac-m1-8gb-14.4 --date $(date +%F) --run-by "<your name>" \
    --build $(git rev-parse --short HEAD) \
    --hardware "$(sysctl -n machdep.cpu.brand_string), $(( $(sysctl -n hw.memsize) / 1073741824 )) GB" \
    --os "macOS $(sw_vers -productVersion)" --stt-model "parakeet-tdt-0.6b-v3 int8" \
    --samples ~/res.csv --sp04-bench ~/sp04-work/out/bench-p.txt \
    --out docs/evidence/AT-21-resources-mac-m1-8gb-14.4.md
  BOTTLE_TEST_DEVICE=mac-m1-8gb-14.4 BOTTLE_RES_SAMPLES=~/res.csv \
    BOTTLE_RES_SP04_BENCH=~/sp04-work/out/bench-p.txt \
    cargo test -p mog-core --test resource_budget at_21_device
  ```
  Use `mac-m1-current` on the Mac with the current macOS. Expected after Part A only: NFR-05 is
  `over` (SP-04 measured 1.91 GB), the other rows are `no data`, and the result is `fail`
  (exit 1). The device test fails too: it stops on the first row that is not `within` (NFR-04a,
  `no data`, until Part B runs). This is the true result: record it, do not change the limit.
- **B1.** Start Bottle with local STT not loaded and no agent harness running. Wait 30 s.
- **B2.** Memory, for N = 0, 1, 2, 4 and 8 open MOGs (with cards open). Set N MOGs, wait 30 s, then
  (10 samples in 20 s):
  ```
  rb sample-rss --mogs <N> --name Bottle --name com.apple.WebKit.WebContent \
    --name com.apple.WebKit.GPU --name com.apple.WebKit.Networking --append ~/res.csv
  ```
- **B3.** CPU, no capture, 2 min per scenario. Set the scenario, wait 30 s, then:
  ```
  sudo powermetrics -i 1000 -n 120 --samplers tasks -o ~/pm-<scenario>.txt
  rb import-powermetrics --scenario <scenario> --mogs <N> --name Bottle \
    --name com.apple.WebKit.WebContent --name com.apple.WebKit.GPU \
    --name com.apple.WebKit.Networking --append ~/res.csv ~/pm-<scenario>.txt
  ```
  The scenarios: `baseline` with N = 0 (no MOG panel, hit-poll off); `hidden_panels` with N = 8
  (panels hidden, hit-poll on); `rive_paused` with N = 8; `rive_animating` with N = 8.
- **B4.** Focus steals (SP-02): type in TextEdit for 2 min while the 8 MOGs animate. Count each time
  TextEdit loses the keyboard focus, then `rb focus-steals <count> --append ~/res.csv`.

### Windows 11 (PowerShell)

In your Bottle checkout on `main`: `cargo build -p mog-core --example resource_bench`, then
`Set-Alias rb "$PWD\target\debug\examples\resource_bench.exe"`.

- **A1.** `rb init --out $HOME\res.csv`
- **A2.** Run `spikes/sp-04/run.ps1`. It writes `$HOME\sp04-work\out\bench-p.txt`.
- **A3.** Write the report and run the device test (the result is as on macOS):
  ```
  $cpu = (Get-CimInstance Win32_Processor).Name
  $gb = [math]::Round((Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory / 1GB)
  $os = (Get-CimInstance Win32_OperatingSystem)
  rb report --device win11-i5-16gb --date (Get-Date -Format yyyy-MM-dd) --run-by "<your name>" `
    --build (git rev-parse --short HEAD) --hardware "$cpu, $gb GB" `
    --os "$($os.Caption) $($os.Version)" --stt-model "parakeet-tdt-0.6b-v3 int8" `
    --samples $HOME\res.csv --sp04-bench $HOME\sp04-work\out\bench-p.txt `
    --out docs\evidence\AT-21-resources-win11-i5-16gb.md
  $env:BOTTLE_TEST_DEVICE = "win11-i5-16gb"
  $env:BOTTLE_RES_SAMPLES = "$HOME\res.csv"
  $env:BOTTLE_RES_SP04_BENCH = "$HOME\sp04-work\out\bench-p.txt"
  cargo test -p mog-core --test resource_budget at_21_device
  ```
- **B1, B2, B4.** As on macOS, with `--name Bottle --name msedgewebview2` and `$HOME\res.csv`
  (`rb sample-rss` reads `tasklist`). Type in Notepad for B4.
- **B3.** For each scenario (2 min):
  ```
  typeperf "\Process(Bottle*)\% Processor Time" "\Process(msedgewebview2*)\% Processor Time" -si 1 -sc 120 -f CSV -y -o $HOME\cpu-<scenario>.csv
  rb import-typeperf --scenario <scenario> --mogs <N> --name Bottle --name msedgewebview2 --append $HOME\res.csv $HOME\cpu-<scenario>.csv
  ```
  For `rive_paused` and `rive_animating`, also run PresentMon for 60 s at the same time, in a
  second PowerShell window that you open as administrator (PresentMon needs it):
  ```
  PresentMon.exe --output_file $HOME\pm-<scenario>.csv --timed 60 --terminate_after_timed
  rb import-presentmon --scenario <scenario> --mogs 8 --seconds 60 --name Bottle --name msedgewebview2 --append $HOME\res.csv $HOME\pm-<scenario>.csv
  ```
