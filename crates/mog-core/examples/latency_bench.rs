//! Latency benchmark runner (#171). See `docs/tests/LATENCY-BENCH.md`.
//!
//! ```text
//! cargo run -p mog-core --example latency_bench -- demo [--gate AT-21|AT-30|AT-43]
//! cargo run -p mog-core --example latency_bench -- report --gate AT-30 --device mac-m1-8gb-14.4 \
//!     --date 2026-11-20 --run-by <name> --build <sha> --hardware "Apple M1 8 GB" --os "macOS 14.4" \
//!     [--model <name>]... [--rss-mb N] [--cpu-pct N] [--sp08-passed] \
//!     [--samples <file.csv>] [--sp03-log <file.log>] [--out docs/evidence/AT-30-latency.md]
//! ```
//!
//! `demo` runs the test doubles and prints a report whose Result is always `not run`. `report`
//! reads samples that a person recorded on a reference device. The runner never measures by
//! itself and never writes `pass` for missing or doubled samples.

use std::process::ExitCode;

use mog_core::latency_bench::doubles::{demo_hotkey, demo_pipeline};
use mog_core::latency_bench::import::{parse_csv, parse_sp03_log};
use mog_core::latency_bench::{
    BudgetConfig, Gate, HotkeySamples, Report, ResourceUse, RunInfo, SampleSource, Trial,
};
use mog_core::testkit::{self, Fixture};

const USAGE: &str = "\
latency_bench: Bottle latency benchmark (#171, docs/tests/LATENCY-BENCH.md)

USAGE:
  latency_bench demo [--gate AT-21|AT-30|AT-43]
  latency_bench report --gate <AT-21|AT-30|AT-43> --device <id> --date <YYYY-MM-DD>
      --run-by <name> --build <sha> --hardware <text> --os <text>
      [--model <text>]... [--rss-mb <n>] [--cpu-pct <n>] [--sp08-passed]
      [--samples <file.csv>] [--sp03-log <file.log>] [--out <file.md>]

  demo     test doubles on the simulated clock; the Result is always `not run`
  report   device samples: the recorder CSV (AT-21, AT-43) and/or the SP-03 probe log (AT-30)
  <id>     mac-m1-8gb-14.4, mac-m1-current or win11-i5-16gb
";

#[derive(Default)]
struct Args {
    gate: Option<String>,
    device: Option<String>,
    date: Option<String>,
    run_by: Option<String>,
    build: Option<String>,
    hardware: Option<String>,
    os: Option<String>,
    models: Vec<String>,
    rss_mb: Option<u64>,
    cpu_pct: Option<u32>,
    sp08_passed: bool,
    samples: Option<String>,
    sp03_log: Option<String>,
    out: Option<String>,
}

fn num<T: std::str::FromStr>(flag: &str, v: &str) -> Result<T, String> {
    v.parse().map_err(|_| format!("{flag} needs a number"))
}

fn parse(mut it: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args::default();
    while let Some(flag) = it.next() {
        if flag == "--sp08-passed" {
            a.sp08_passed = true;
            continue;
        }
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--gate" => a.gate = Some(value),
            "--device" => a.device = Some(value),
            "--date" => a.date = Some(value),
            "--run-by" => a.run_by = Some(value),
            "--build" => a.build = Some(value),
            "--hardware" => a.hardware = Some(value),
            "--os" => a.os = Some(value),
            "--model" => a.models.push(value),
            "--rss-mb" => a.rss_mb = Some(num(&flag, &value)?),
            "--cpu-pct" => a.cpu_pct = Some(num(&flag, &value)?),
            "--samples" => a.samples = Some(value),
            "--sp03-log" => a.sp03_log = Some(value),
            "--out" => a.out = Some(value),
            _ => return Err(format!("unknown flag {flag}")),
        }
    }
    Ok(a)
}

fn gate(a: &Args) -> Result<Gate, String> {
    let id = a.gate.as_deref().unwrap_or("AT-21");
    Gate::parse(id).ok_or_else(|| format!("unknown gate {id}; use AT-21, AT-30 or AT-43"))
}

fn info(a: &Args) -> RunInfo {
    RunInfo {
        build: a.build.clone().unwrap_or_default(),
        hardware: a.hardware.clone().unwrap_or_default(),
        os: a.os.clone().unwrap_or_default(),
        models: a.models.clone(),
        resources: ResourceUse {
            peak_rss_mb: a.rss_mb,
            cpu_percent: a.cpu_pct,
        },
        config: BudgetConfig {
            sp08_passed: a.sp08_passed,
        },
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
}

fn run(cmd: &str, a: Args) -> Result<Report, String> {
    let gate = gate(&a)?;
    match cmd {
        "demo" => {
            let trials = demo_pipeline(40).recorder().trials();
            let mut i = info(&a);
            if i.hardware.is_empty() {
                i.hardware = "simulated".into();
            }
            if i.os.is_empty() {
                i.os = "simulated".into();
            }
            Ok(Report::build(
                gate,
                SampleSource::Doubles,
                i,
                &trials,
                &demo_hotkey(1_000),
            ))
        }
        "report" => {
            let raw = a.device.clone().ok_or("--device is needed")?;
            let device =
                match testkit::device_from(|k| (k == testkit::ENV_DEVICE).then(|| raw.clone()))
                    .map_err(|e| e.to_string())?
                {
                    Fixture::Ready(d) => d,
                    Fixture::Skip(r) => return Err(r),
                };
            let trials: Vec<Trial> = match &a.samples {
                Some(p) => parse_csv(&read(p)?).map_err(|e| format!("{p}: {e}"))?,
                None => Vec::new(),
            };
            let hotkey = match &a.sp03_log {
                Some(p) => parse_sp03_log(&read(p)?),
                None => HotkeySamples::default(),
            };
            let source = SampleSource::Device {
                device,
                date_run: a.date.clone().unwrap_or_default(),
                run_by: a.run_by.clone().unwrap_or_default(),
            };
            Ok(Report::build(gate, source, info(&a), &trials, &hotkey))
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next().filter(|c| c != "--help" && c != "-h") else {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    let parsed = match parse(args) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let out = parsed.out.clone();
    let report = match run(&cmd, parsed) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let md = report.to_markdown();
    match out {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &md) {
                eprintln!("error: cannot write {path}: {e}");
                return ExitCode::from(2);
            }
        }
        None => print!("{md}"),
    }
    eprintln!(
        "RESULT {} {} ({})",
        report.gate.id(),
        report.result().as_str(),
        report.gate.budget_ids().join(", ")
    );
    for m in report.missing() {
        eprintln!("  not a pass because {m}");
    }
    ExitCode::SUCCESS
}
