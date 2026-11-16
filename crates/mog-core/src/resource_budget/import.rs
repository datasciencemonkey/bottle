//! Sample files and tool outputs (#173).
//!
//! - The sample file: four fixed columns `kind,scenario,mogs,value`. The value is a number.
//!   There is no free-text column, so the file cannot hold a transcript, audio or token (SEC-16b).
//! - RSS: a `ps -ax -o pid=,rss=,comm=` snapshot (macOS, KiB) or a `tasklist /fo csv /nh`
//!   snapshot (Windows working set, KiB).
//! - CPU: `powermetrics --samplers tasks` text (macOS, CPU ms/s) or a `typeperf` CSV of
//!   `\Process(*)\% Processor Time` (Windows). Both give percent of one core.
//! - PresentMon CSV: the presents per second of the Bottle processes (Rive paused or animating).
//! - Parakeet: the SP-04 bench output (`long_secs=`, `peak_rss_mb=`) and `/usr/bin/time -l`.
//!   The SP-04 import reads only these numeric keys; it never reads the transcript table.
//!
//! The tool imports sum every process that the [`ProcessFilter`] names. The sums count shared
//! pages more than once, so they are on the safe side of a limit.

use std::fmt;

use super::samples::{CpuRun, CpuScenario, ParakeetRun, RssRun, Samples, MAX_MOGS, MB};

/// The first line of a sample file.
pub const CSV_MAGIC: &str = "# bottle-resource-samples v1";
/// The column line of a sample file.
pub const CSV_HEADER: &str = "kind,scenario,mogs,value";

/// A sample file line that cannot be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportError {
    /// The line number, from 1.
    pub line: usize,
    /// What is wrong.
    pub reason: String,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ImportError {}

/// One sample file row: a kind, a scenario, a MOG count and a value.
pub fn row(kind: &str, scenario: &str, mogs: u8, value: impl fmt::Display) -> String {
    format!("{kind},{scenario},{mogs},{value}\n")
}

/// Write `s` as a sample file.
pub fn to_csv(s: &Samples) -> String {
    let mut out = format!("{CSV_MAGIC}\n{CSV_HEADER}\n");
    for r in &s.rss {
        for b in &r.bytes {
            out.push_str(&row("rss", "shell", r.mogs, b));
        }
    }
    if let Some(p) = s.parakeet {
        out.push_str(&row("parakeet_rss", "parakeet", 0, p.peak_bytes));
        out.push_str(&row("parakeet_clip_s", "parakeet", 0, p.clip_secs));
    }
    for c in &s.cpu {
        for v in &c.percent {
            out.push_str(&row("cpu", c.scenario.name(), c.mogs, v));
        }
        if let Some(fps) = c.presents_per_s {
            out.push_str(&row("presents", c.scenario.name(), c.mogs, fps));
        }
    }
    if let Some(n) = s.focus_steals {
        out.push_str(&row("focus_steals", "shell", 0, n));
    }
    out
}

fn cpu_entry(s: &mut Samples, scenario: CpuScenario, mogs: u8) -> &mut CpuRun {
    let i = match s
        .cpu
        .iter()
        .position(|c| c.scenario == scenario && c.mogs == mogs)
    {
        Some(i) => i,
        None => {
            s.cpu.push(CpuRun {
                scenario,
                mogs,
                percent: Vec::new(),
                presents_per_s: None,
            });
            s.cpu.len() - 1
        }
    };
    &mut s.cpu[i]
}

/// Read a sample file. Every row must have the four columns with known values.
pub fn parse_csv(text: &str) -> Result<Samples, ImportError> {
    let err = |line: usize, reason: String| ImportError { line, reason };
    let mut lines = text
        .lines()
        .enumerate()
        .map(|(i, l)| (i + 1, l.trim()))
        .filter(|(_, l)| !l.is_empty());
    match lines.next() {
        Some((_, l)) if l == CSV_MAGIC => {}
        Some((n, _)) => return Err(err(n, format!("the first line must be `{CSV_MAGIC}`"))),
        None => return Err(err(1, "the file is empty".into())),
    }
    match lines.next() {
        Some((_, l)) if l == CSV_HEADER => {}
        Some((n, _)) => return Err(err(n, format!("the column line must be `{CSV_HEADER}`"))),
        None => return Err(err(2, "the column line is missing".into())),
    }
    let mut s = Samples::default();
    // (line, value) of the Parakeet rows.
    let mut peak: Option<(usize, u64)> = None;
    let mut clip: Option<(usize, f64)> = None;
    for (n, l) in lines {
        let cols: Vec<&str> = l.split(',').map(str::trim).collect();
        let [kind, scenario, mogs, value] = cols[..] else {
            return Err(err(n, format!("{} columns; a row has 4", cols.len())));
        };
        let mogs: u8 = mogs
            .parse()
            .ok()
            .filter(|m| *m <= MAX_MOGS)
            .ok_or_else(|| err(n, format!("mogs must be 0..={MAX_MOGS}")))?;
        let int = || {
            value
                .parse::<u64>()
                .map_err(|_| err(n, "the value must be a whole number".into()))
        };
        let num = || {
            value
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .ok_or_else(|| err(n, "the value must be a number >= 0".into()))
        };
        let want = |name: &str| {
            if scenario == name {
                Ok(())
            } else {
                Err(err(n, format!("kind {kind} needs the scenario `{name}`")))
            }
        };
        match kind {
            "rss" => {
                want("shell")?;
                let v = int()?;
                match s.rss.iter_mut().find(|r| r.mogs == mogs) {
                    Some(r) => r.bytes.push(v),
                    None => s.rss.push(RssRun {
                        mogs,
                        bytes: vec![v],
                    }),
                }
            }
            "parakeet_rss" => {
                want("parakeet")?;
                let v = int()?;
                peak = Some((n, peak.map_or(v, |(_, p)| p.max(v))));
            }
            "parakeet_clip_s" => {
                want("parakeet")?;
                clip = Some((n, num()?));
            }
            "cpu" | "presents" => {
                let sc = CpuScenario::parse(scenario)
                    .ok_or_else(|| err(n, format!("unknown CPU scenario `{scenario}`")))?;
                let v = num()?;
                let run = cpu_entry(&mut s, sc, mogs);
                if kind == "cpu" {
                    run.percent.push(v);
                } else {
                    run.presents_per_s = Some(v);
                }
            }
            "focus_steals" => {
                want("shell")?;
                let v = int()?;
                s.focus_steals = Some(u32::try_from(v).map_err(|_| err(n, "too large".into()))?);
            }
            _ => return Err(err(n, format!("unknown kind `{kind}`"))),
        }
    }
    match (peak, clip) {
        (Some((_, peak_bytes)), Some((_, clip_secs))) => {
            s.parakeet = Some(ParakeetRun {
                clip_secs,
                peak_bytes,
            })
        }
        (None, None) => {}
        (Some((n, _)), None) | (None, Some((n, _))) => {
            return Err(err(
                n,
                "a Parakeet run needs one parakeet_rss and one parakeet_clip_s row".into(),
            ))
        }
    }
    Ok(s)
}

/// The processes that count as Bottle in a tool output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessFilter {
    names: Vec<String>,
}

fn normalize(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim()
        .to_ascii_lowercase();
    // typeperf names a second instance `name#1`.
    let base = match base.rsplit_once('#') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
            head.to_string()
        }
        _ => base,
    };
    base.strip_suffix(".exe")
        .map(str::to_string)
        .unwrap_or(base)
}

impl ProcessFilter {
    /// The processes with these names. A name matches the file name of the process with or
    /// without `.exe`, with any case, and a typeperf instance suffix `#n`.
    pub fn names(names: &[&str]) -> Self {
        Self {
            names: names.iter().map(|n| normalize(n)).collect(),
        }
    }

    /// True when `process` (a name or a path) is one of the names.
    pub fn matches(&self, process: &str) -> bool {
        let p = normalize(process);
        self.names.contains(&p)
    }
}

fn no_match(tool: &str) -> String {
    format!("no process in the {tool} output matches the process names")
}

/// The RSS sum, bytes, of the matching processes in a `ps -ax -o pid=,rss=,comm=` snapshot.
pub fn parse_ps(text: &str, filter: &ProcessFilter) -> Result<u64, String> {
    let mut sum = 0u64;
    let mut found = false;
    for line in text.lines() {
        let rest = line.trim_start();
        let Some((_pid, rest)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some((rss, comm)) = rest.split_once(char::is_whitespace) else {
            continue;
        };
        let Ok(kib) = rss.parse::<u64>() else {
            continue;
        };
        if filter.matches(comm.trim()) {
            sum += kib * 1024;
            found = true;
        }
    }
    found.then_some(sum).ok_or_else(|| no_match("ps"))
}

fn csv_cells(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => cells.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    cells.push(cur);
    cells
}

/// The working-set sum, bytes, of the matching processes in a `tasklist /fo csv /nh` snapshot.
pub fn parse_tasklist_csv(text: &str, filter: &ProcessFilter) -> Result<u64, String> {
    let mut sum = 0u64;
    let mut found = false;
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let cells = csv_cells(line.trim());
        let (Some(name), Some(mem)) = (cells.first(), cells.get(4)) else {
            continue;
        };
        let digits: String = mem.chars().filter(char::is_ascii_digit).collect();
        let Ok(kib) = digits.parse::<u64>() else {
            continue;
        };
        if filter.matches(name) {
            sum += kib * 1024;
            found = true;
        }
    }
    found.then_some(sum).ok_or_else(|| no_match("tasklist"))
}

/// The CPU use of the matching processes in each `powermetrics --samplers tasks` sample, in
/// percent of one core (CPU ms/s divided by 10).
pub fn parse_powermetrics(text: &str, filter: &ProcessFilter) -> Result<Vec<f64>, String> {
    let mut out = Vec::new();
    let mut found = false;
    let mut in_table = false;
    let mut cur: Option<f64> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("*** Sampled system activity") {
            out.extend(cur.take());
            in_table = false;
            continue;
        }
        if t.starts_with("*** Running tasks") {
            cur = Some(0.0);
            continue;
        }
        if cur.is_some() && t.starts_with("Name ") {
            in_table = true;
            continue;
        }
        if !in_table {
            continue;
        }
        if t.is_empty() || t.starts_with("***") {
            in_table = false;
            continue;
        }
        let tokens: Vec<&str> = t.split_whitespace().collect();
        let Some(i) = (1..tokens.len().saturating_sub(1))
            .find(|&i| tokens[i].parse::<i64>().is_ok() && tokens[i + 1].parse::<f64>().is_ok())
        else {
            continue;
        };
        if filter.matches(&tokens[..i].join(" ")) {
            let ms_per_s: f64 = tokens[i + 1].parse().unwrap_or(0.0);
            if let Some(c) = cur.as_mut() {
                *c += ms_per_s / 10.0;
            }
            found = true;
        }
    }
    out.extend(cur);
    if found {
        Ok(out)
    } else {
        Err(no_match("powermetrics"))
    }
}

/// The CPU use of the matching processes in each row of a `typeperf` CSV of
/// `\Process(*)\% Processor Time` counters, in percent of one core. A row with no value for a
/// matching counter is dropped, not counted as 0.
pub fn parse_typeperf_csv(text: &str, filter: &ProcessFilter) -> Result<Vec<f64>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = csv_cells(lines.next().ok_or("the typeperf file is empty")?.trim());
    let cols: Vec<usize> = header
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, h)| h.contains("% Processor Time"))
        .filter_map(|(i, h)| {
            let start = h.find("Process(")? + "Process(".len();
            let end = start + h[start..].find(')')?;
            filter.matches(&h[start..end]).then_some(i)
        })
        .collect();
    if cols.is_empty() {
        return Err(no_match("typeperf"));
    }
    let mut out = Vec::new();
    for line in lines {
        let cells = csv_cells(line.trim());
        let values: Option<Vec<f64>> = cols
            .iter()
            .map(|&i| cells.get(i).and_then(|c| c.trim().parse::<f64>().ok()))
            .collect();
        if let Some(v) = values {
            out.push(v.iter().sum());
        }
    }
    if out.is_empty() {
        return Err("the typeperf file has no complete row".into());
    }
    Ok(out)
}

/// The presents per second of the matching processes in a PresentMon CSV that covers `seconds`.
pub fn parse_presentmon_csv(
    text: &str,
    filter: &ProcessFilter,
    seconds: f64,
) -> Result<f64, String> {
    if !(seconds.is_finite() && seconds > 0.0) {
        return Err("the PresentMon capture time must be more than 0 s".into());
    }
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = csv_cells(lines.next().ok_or("the PresentMon file is empty")?.trim());
    let app = header
        .iter()
        .position(|h| h.trim() == "Application")
        .ok_or("the PresentMon file has no Application column")?;
    let n = lines
        .filter(|l| {
            csv_cells(l.trim())
                .get(app)
                .is_some_and(|a| filter.matches(a))
        })
        .count();
    if n == 0 {
        return Err(no_match("PresentMon"));
    }
    Ok(n as f64 / seconds)
}

/// The value of `key=` in the last line that starts with `key=`. Only such lines are read, so
/// the transcript table of the bench is never read.
fn key_value(text: &str, key: &str) -> Option<f64> {
    text.lines()
        .filter_map(|l| l.trim_start().strip_prefix(key)?.strip_prefix('='))
        .filter_map(|v| v.split_whitespace().next()?.parse::<f64>().ok())
        .next_back()
}

/// The peak RSS, bytes, in `/usr/bin/time -l` output (macOS: `<bytes> maximum resident set size`).
pub fn parse_time_l(text: &str) -> Option<u64> {
    text.lines()
        .filter(|l| l.contains("maximum resident set size"))
        .filter_map(|l| l.split_whitespace().next()?.parse::<u64>().ok())
        .max()
}

/// The Parakeet run in SP-04 bench output (`spikes/sp-04`, `bench-p.txt`): `peak_rss_mb=` (the
/// last one) and `long_secs=` (0 when the long stream did not run). A `/usr/bin/time -l` line in
/// the same text counts too; the larger peak wins. `None` without a peak.
pub fn parse_sp04_bench(text: &str) -> Option<ParakeetRun> {
    let in_process = key_value(text, "peak_rss_mb")
        .filter(|v| v.is_finite() && *v >= 0.0)
        .map(|mb| (mb * MB as f64).round() as u64);
    let peak_bytes = match (in_process, parse_time_l(text)) {
        (Some(a), Some(b)) => a.max(b),
        (a, b) => a.or(b)?,
    };
    Some(ParakeetRun {
        clip_secs: key_value(text, "long_secs").unwrap_or(0.0),
        peak_bytes,
    })
}
