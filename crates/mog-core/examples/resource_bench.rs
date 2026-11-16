//! Memory and idle CPU benchmark runner (#173). See `docs/tests/RESOURCE-BENCH.md`.
//!
//! The import and sample commands print sample-file rows; append them to the file that `init`
//! starts. `report` reads the file and writes the AT-21 evidence. The runner never writes `pass`
//! for samples from doubles, for missing samples or for hardware that is not a reference device.
//! Exit codes: 0 for pass, partial and not run; 1 for fail; 2 for a usage or file error.

use std::process::{Command, ExitCode};
use std::time::Duration;

use mog_core::latency_bench::GateResult;
use mog_core::resource_budget::doubles::demo_samples;
use mog_core::resource_budget::import::{
    parse_csv, parse_powermetrics, parse_presentmon_csv, parse_ps, parse_sp04_bench,
    parse_tasklist_csv, parse_time_l, parse_typeperf_csv, row, ProcessFilter, CSV_HEADER,
    CSV_MAGIC,
};
use mog_core::resource_budget::{collect_rss, CpuScenario, Report, RssProbe, RunInfo, Source};
use mog_core::testkit::{self, Fixture};

const USAGE: &str = "\
resource_bench: Bottle memory and idle CPU benchmark (#173, docs/tests/RESOURCE-BENCH.md)

USAGE:
  resource_bench demo
  resource_bench init                                  (prints the sample-file header)
  resource_bench sample-rss --mogs <0-8> --name <process>... [--count 10] [--interval-s 2]
  resource_bench import-powermetrics --scenario <s> --mogs <n> --name <process>... <file>
  resource_bench import-typeperf     --scenario <s> --mogs <n> --name <process>... <file>
  resource_bench import-presentmon   --scenario <s> --mogs <n> --seconds <t> --name <process>... <file>
  resource_bench import-sp04 <bench-p.txt> [--time-l <time.txt>]
  resource_bench focus-steals <n>
  resource_bench report (--device <id> | --other-hardware) --date <YYYY-MM-DD> --run-by <name>
      --build <sha> --hardware <text> --os <text> [--stt-model <text>]
      --samples <file.csv> [--sp04-bench <bench-p.txt>] [--out <file.md>]

  <s>   baseline, hidden_panels, rive_paused or rive_animating
  <id>  mac-m1-8gb-14.4, mac-m1-current or win11-i5-16gb
";

#[derive(Default)]
struct Args {
    names: Vec<String>,
    values: std::collections::HashMap<String, String>,
    flags: Vec<String>,
    positional: Vec<String>,
}

impl Args {
    fn parse(it: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut a = Args::default();
        let mut it = it.peekable();
        while let Some(arg) = it.next() {
            if arg == "--other-hardware" {
                a.flags.push(arg);
            } else if let Some(flag) = arg.strip_prefix("--") {
                let v = it.next().ok_or_else(|| format!("--{flag} needs a value"))?;
                if flag == "name" {
                    a.names.push(v);
                } else {
                    a.values.insert(flag.to_string(), v);
                }
            } else {
                a.positional.push(arg);
            }
        }
        Ok(a)
    }

    fn get(&self, k: &str) -> Option<&str> {
        self.values.get(k).map(String::as_str)
    }

    fn need(&self, k: &str) -> Result<&str, String> {
        self.get(k).ok_or_else(|| format!("--{k} is needed"))
    }

    fn num<T: std::str::FromStr>(&self, k: &str) -> Result<T, String> {
        self.need(k)?
            .parse()
            .map_err(|_| format!("--{k} needs a number"))
    }

    fn filter(&self) -> Result<ProcessFilter, String> {
        if self.names.is_empty() {
            return Err("give one or more --name <process>".into());
        }
        let names: Vec<&str> = self.names.iter().map(String::as_str).collect();
        Ok(ProcessFilter::names(&names))
    }

    fn file(&self) -> Result<String, String> {
        let p = self.positional.first().ok_or("the input file is needed")?;
        read(p)
    }

    fn scenario(&self) -> Result<CpuScenario, String> {
        let s = self.need("scenario")?;
        CpuScenario::parse(s).ok_or_else(|| format!("unknown scenario {s}"))
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))
}

/// Reads `ps` (macOS, Linux) or `tasklist` (Windows) once per sample.
struct SystemProbe {
    filter: ProcessFilter,
}

impl RssProbe for SystemProbe {
    fn sample(&mut self) -> Result<u64, String> {
        let run = |cmd: &str, args: &[&str]| {
            let out = Command::new(cmd)
                .args(args)
                .output()
                .map_err(|e| format!("cannot run {cmd}: {e}"))?;
            Ok::<_, String>(String::from_utf8_lossy(&out.stdout).into_owned())
        };
        if cfg!(windows) {
            parse_tasklist_csv(&run("tasklist", &["/fo", "csv", "/nh"])?, &self.filter)
        } else {
            parse_ps(&run("ps", &["-ax", "-o", "pid=,rss=,comm="])?, &self.filter)
        }
    }
}

fn cpu_rows(a: &Args, values: &[f64]) -> Result<String, String> {
    let (s, mogs) = (a.scenario()?, a.num::<u8>("mogs")?);
    Ok(values
        .iter()
        .map(|v| row("cpu", s.name(), mogs, format!("{v:.4}")))
        .collect())
}

fn report(a: &Args) -> Result<Report, String> {
    let date_run = a.get("date").unwrap_or_default().to_string();
    let run_by = a.get("run-by").unwrap_or_default().to_string();
    let source = if a.flags.iter().any(|f| f == "--other-hardware") {
        Source::OtherHardware { date_run, run_by }
    } else {
        let raw = a.need("device")?.to_string();
        let device = match testkit::device_from(|k| (k == testkit::ENV_DEVICE).then(|| raw.clone()))
            .map_err(|e| e.to_string())?
        {
            Fixture::Ready(d) => d,
            Fixture::Skip(r) => return Err(r),
        };
        Source::Device {
            device,
            date_run,
            run_by,
        }
    };
    let path = a.need("samples")?;
    let mut samples = parse_csv(&read(path)?).map_err(|e| format!("{path}: {e}"))?;
    if let Some(p) = a.get("sp04-bench") {
        samples.parakeet =
            Some(parse_sp04_bench(&read(p)?).ok_or(format!("{p}: no peak_rss_mb line"))?);
    }
    let info = RunInfo {
        build: a.get("build").unwrap_or_default().into(),
        hardware: a.get("hardware").unwrap_or_default().into(),
        os: a.get("os").unwrap_or_default().into(),
        stt_model: a.get("stt-model").unwrap_or_default().into(),
    };
    Ok(Report::build(source, info, &samples))
}

fn print_report(r: &Report, out: Option<&str>) -> Result<ExitCode, String> {
    let md = r.to_markdown();
    match out {
        Some(path) => std::fs::write(path, &md).map_err(|e| format!("cannot write {path}: {e}"))?,
        None => print!("{md}"),
    }
    eprintln!("RESULT AT-21 {} (resource rows)", r.result().as_str());
    for m in r.missing() {
        eprintln!("  not a pass because {m}");
    }
    Ok(if r.result() == GateResult::Fail {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn run(cmd: &str, a: &Args) -> Result<ExitCode, String> {
    let out = match cmd {
        "demo" => {
            let info = RunInfo {
                build: "-".into(),
                hardware: "simulated".into(),
                os: "simulated".into(),
                stt_model: "simulated".into(),
            };
            return print_report(&Report::build(Source::Doubles, info, &demo_samples()), None);
        }
        "report" => return print_report(&report(a)?, a.get("out")),
        "init" => format!("{CSV_MAGIC}\n{CSV_HEADER}\n"),
        "sample-rss" => {
            let mogs = a.num::<u8>("mogs")?;
            let count = a.get("count").map_or(Ok(10), |_| a.num("count"))?;
            let secs = a
                .get("interval-s")
                .map_or(Ok(2), |_| a.num::<u64>("interval-s"))?;
            let mut probe = SystemProbe {
                filter: a.filter()?,
            };
            let run = collect_rss(
                &mut probe,
                mogs,
                count,
                Duration::from_secs(secs),
                &mut std::thread::sleep,
            )?;
            run.bytes
                .iter()
                .map(|b| row("rss", "shell", mogs, b))
                .collect()
        }
        "import-powermetrics" => cpu_rows(a, &parse_powermetrics(&a.file()?, &a.filter()?)?)?,
        "import-typeperf" => cpu_rows(a, &parse_typeperf_csv(&a.file()?, &a.filter()?)?)?,
        "import-presentmon" => {
            let fps = parse_presentmon_csv(&a.file()?, &a.filter()?, a.num("seconds")?)?;
            row("presents", a.scenario()?.name(), a.num("mogs")?, fps)
        }
        "import-sp04" => {
            let mut run = parse_sp04_bench(&a.file()?).ok_or("no peak_rss_mb line")?;
            if let Some(p) = a.get("time-l") {
                if let Some(b) = parse_time_l(&read(p)?) {
                    run.peak_bytes = run.peak_bytes.max(b);
                }
            }
            row("parakeet_rss", "parakeet", 0, run.peak_bytes)
                + &row("parakeet_clip_s", "parakeet", 0, run.clip_secs)
        }
        "focus-steals" => {
            let n: u32 = a
                .positional
                .first()
                .and_then(|v| v.parse().ok())
                .ok_or("focus-steals needs a count")?;
            row("focus_steals", "shell", 0, n)
        }
        _ => return Err(format!("unknown command {cmd}")),
    };
    print!("{out}");
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let Some(cmd) = args.next().filter(|c| c != "--help" && c != "-h") else {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    match Args::parse(args).and_then(|a| run(&cmd, &a)) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
