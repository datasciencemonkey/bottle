//! The benchmark report (#171) in the `docs/tests/EVIDENCE-TEMPLATE.md` format.
//!
//! One file per gate: AT-21 (NFR-01a, NFR-01b, NFR-01c, NFR-02), AT-30 (NFR-08a, NFR-08b) and
//! AT-43 (NFR-01d). The file covers only these latency rows; the other parts of each AT need
//! their own evidence. The report writes `pass` only when every rule below holds:
//! - the samples come from a reference device, not from test doubles;
//! - every row has enough samples and is within its budget;
//! - the run has a date (YYYY-MM-DD), a runner, the build, the hardware and the OS;
//! - for AT-21: the models and the resource use (peak RSS and CPU) are recorded.

use std::time::Duration;

use super::marks::{HotkeySamples, SttPath, Trial};
use super::stats::{
    budget, evaluate, span_samples, BudgetConfig, BudgetResult, Check, Outcome, Summary,
};
use crate::testkit::ReferenceDevice;

/// Where the samples come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleSource {
    /// Test doubles on the simulated clock. Never a measurement.
    Doubles,
    /// A run on a reference device (section 15).
    Device {
        /// The device.
        device: ReferenceDevice,
        /// The run date, YYYY-MM-DD.
        date_run: String,
        /// The person or role that ran it.
        run_by: String,
    },
}

/// A gate that the benchmark gives evidence for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// AT-21 latency and capacity.
    At21,
    /// AT-30 global-key defaults and Secure Input (the HID rig rows).
    At30,
    /// AT-43 UC1 dictation into the active app (the latency row).
    At43,
}

impl Gate {
    /// The AT ID.
    pub fn id(self) -> &'static str {
        match self {
            Gate::At21 => "AT-21",
            Gate::At30 => "AT-30",
            Gate::At43 => "AT-43",
        }
    }

    /// The gate with this AT ID.
    pub fn parse(id: &str) -> Option<Self> {
        [Gate::At21, Gate::At30, Gate::At43]
            .into_iter()
            .find(|g| g.id() == id)
    }

    /// The report title.
    pub fn title(self) -> &'static str {
        match self {
            Gate::At21 => "latency benchmark (talk key, mic, acknowledgement, preview chip)",
            Gate::At30 => "latency benchmark (HID rig key-ups and key event delivery)",
            Gate::At43 => "latency benchmark (dictation key-up to inserted text)",
        }
    }

    /// The requirement IDs that this gate file covers.
    pub fn budget_ids(self) -> &'static [&'static str] {
        match self {
            Gate::At21 => &["NFR-01a", "NFR-01b", "NFR-01c", "NFR-02"],
            Gate::At30 => &["NFR-08a", "NFR-08b"],
            Gate::At43 => &["NFR-01d"],
        }
    }

    /// The evidence file name, in `docs/evidence/`.
    pub fn file_name(self) -> String {
        format!("{}-latency.md", self.id())
    }

    fn other_parts(self) -> &'static str {
        match self {
            Gate::At21 => {
                "STT, cleanup, recall and blurb rows NFR-03..NFR-12, 8 MOGs and the 8 h soak"
            }
            Gate::At30 => {
                "Secure Keyboard Entry, PTT end <= 250 ms, the Carbon chord, AltGr and Copilot \
                 layouts, panic <= 200 ms"
            }
            Gate::At43 => {
                "the target apps, the password field, no Accessibility, Shift to ask, clipboard \
                 restore, no agent turn and no Genie call"
            }
        }
    }
}

/// The resource use during the run (AT-21).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResourceUse {
    /// The peak resident memory of Bottle, MB.
    pub peak_rss_mb: Option<u64>,
    /// The mean CPU use of Bottle during the run, percent of one core.
    pub cpu_percent: Option<u32>,
}

/// What the run used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunInfo {
    /// The commit SHA or installer name and SHA-256.
    pub build: String,
    /// The hardware, for example `Apple M1 8 GB`.
    pub hardware: String,
    /// The OS and version, for example `macOS 14.4`.
    pub os: String,
    /// The STT and cleanup models with their versions.
    pub models: Vec<String>,
    /// The resource use.
    pub resources: ResourceUse,
    /// The budget settings (SP-08 result).
    pub config: BudgetConfig,
}

/// The gate result, with the words of the evidence template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateResult {
    /// Every rule holds.
    Pass,
    /// A device sample set is over a budget.
    Fail,
    /// Some rows are within budget; others have no data or the run record is not complete.
    Partial,
    /// The samples come from doubles, or no row has enough samples.
    NotRun,
}

impl GateResult {
    /// `pass`, `fail`, `partial` or `not run`.
    pub fn as_str(self) -> &'static str {
        match self {
            GateResult::Pass => "pass",
            GateResult::Fail => "fail",
            GateResult::Partial => "partial",
            GateResult::NotRun => "not run",
        }
    }
}

/// One span for one STT path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathRow {
    /// The span, for example `key_up -> preview_chip`.
    pub span: String,
    /// The STT path.
    pub path: SttPath,
    /// The summary, if there are samples.
    pub summary: Option<Summary>,
}

/// The benchmark report for one gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// The gate.
    pub gate: Gate,
    /// Where the samples come from.
    pub source: SampleSource,
    /// What the run used.
    pub info: RunInfo,
    /// One result per budget row of the gate.
    pub results: Vec<BudgetResult>,
    /// The p50/p95 of each span row for the local and the cloud STT path.
    pub paths: Vec<PathRow>,
}

fn fmt_ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

fn valid_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

fn cell(s: &str) -> String {
    let s = s.trim().replace('|', "/").replace('\n', " ");
    if s.is_empty() {
        "-".into()
    } else {
        s
    }
}

impl Report {
    /// Evaluate the gate rows on the samples.
    pub fn build(
        gate: Gate,
        source: SampleSource,
        info: RunInfo,
        trials: &[Trial],
        hotkey: &HotkeySamples,
    ) -> Self {
        let budgets: Vec<_> = gate
            .budget_ids()
            .iter()
            .filter_map(|id| budget(id, info.config))
            .collect();
        let results = budgets
            .iter()
            .map(|b| evaluate(b, trials, hotkey))
            .collect();
        let mut paths = Vec::new();
        for b in &budgets {
            if let Check::Span {
                from, to, route, ..
            } = b.check
            {
                let span = format!("{} -> {}", from.name(), to.name());
                if paths.iter().any(|p: &PathRow| p.span == span) {
                    continue;
                }
                for path in [SttPath::Local, SttPath::Cloud] {
                    let samples = span_samples(trials, from, to, Some(path), route);
                    paths.push(PathRow {
                        span: span.clone(),
                        path,
                        summary: Summary::of(&samples),
                    });
                }
            }
        }
        Self {
            gate,
            source,
            info,
            results,
            paths,
        }
    }

    /// Why the report is not a pass (empty when every rule holds).
    pub fn missing(&self) -> Vec<String> {
        let mut out = Vec::new();
        match &self.source {
            SampleSource::Doubles => {
                out.push("the samples come from test doubles, not from a reference device".into())
            }
            SampleSource::Device {
                date_run, run_by, ..
            } => {
                if !valid_date(date_run.trim()) {
                    out.push("the run date is not YYYY-MM-DD".into());
                }
                if run_by.trim().is_empty() {
                    out.push("the run by value is empty".into());
                }
            }
        }
        for r in &self.results {
            if r.outcome != Outcome::Within && r.outcome != Outcome::Over {
                out.push(format!("{}: {}", r.budget.id, r.outcome.text()));
            }
        }
        for (name, value) in [
            ("build", &self.info.build),
            ("hardware", &self.info.hardware),
            ("OS", &self.info.os),
        ] {
            if value.trim().is_empty() {
                out.push(format!("the {name} is not recorded"));
            }
        }
        if self.gate == Gate::At21 {
            if self.info.models.iter().all(|m| m.trim().is_empty()) {
                out.push("the models are not recorded".into());
            }
            if self.info.resources.peak_rss_mb.is_none()
                || self.info.resources.cpu_percent.is_none()
            {
                out.push("the resource use (peak RSS and CPU) is not recorded".into());
            }
        }
        out
    }

    /// The gate result. Doubles give `not run`, whatever the numbers.
    pub fn result(&self) -> GateResult {
        if self.source == SampleSource::Doubles {
            return GateResult::NotRun;
        }
        if self.results.iter().any(|r| r.outcome == Outcome::Over) {
            return GateResult::Fail;
        }
        if self.missing().is_empty() {
            return GateResult::Pass;
        }
        if self.results.iter().any(|r| r.outcome == Outcome::Within) {
            GateResult::Partial
        } else {
            GateResult::NotRun
        }
    }

    /// The evidence file text.
    pub fn to_markdown(&self) -> String {
        let result = self.result();
        let (date, runner, hardware) = match &self.source {
            SampleSource::Doubles => (
                "-".to_string(),
                "-".to_string(),
                format!("{} (test doubles)", cell(&self.info.hardware)),
            ),
            SampleSource::Device {
                device,
                date_run,
                run_by,
            } => (
                cell(date_run),
                cell(run_by),
                format!("{} ({})", cell(&self.info.hardware), device.id()),
            ),
        };
        let mut md = String::new();
        let mut line = |s: String| {
            md.push_str(&s);
            md.push('\n');
        };
        line(format!(
            "# Evidence: {} {}",
            self.gate.id(),
            self.gate.title()
        ));
        line(String::new());
        line("| Field | Value |".into());
        line("|---|---|".into());
        line(format!("| Gate | {} |", self.gate.id()));
        line("| Gate type | device |".into());
        line(format!("| Result | {} |", result.as_str()));
        line(format!("| Date run | {date} |"));
        line(format!("| Run by | {runner} |"));
        line(format!("| Build | {} |", cell(&self.info.build)));
        line(format!("| Hardware | {hardware} |"));
        line("| Profile | none |".into());
        line(format!(
            "| Requirements | {} |",
            self.gate.budget_ids().join(", ")
        ));
        line(String::new());

        line("## Method".into());
        line(String::new());
        line(
            "Bottle latency benchmark (#171, `docs/tests/LATENCY-BENCH.md`, \
             `cargo run -p mog-core --example latency_bench`)."
                .into(),
        );
        let source = match &self.source {
            SampleSource::Doubles => {
                "test doubles on the simulated clock (`latency_bench::doubles`)".to_string()
            }
            SampleSource::Device { device, .. } => format!("reference device `{}`", device.id()),
        };
        line(format!("- Samples: {source}."));
        line(format!("- OS: {}.", cell(&self.info.os)));
        let models = if self.info.models.is_empty() {
            "not recorded".to_string()
        } else {
            self.info.models.join("; ")
        };
        line(format!("- Models: {models}."));
        let rss = self
            .info
            .resources
            .peak_rss_mb
            .map_or("not recorded".to_string(), |v| format!("{v} MB"));
        let cpu = self
            .info
            .resources
            .cpu_percent
            .map_or("not recorded".to_string(), |v| format!("{v} %"));
        line(format!("- Resource use: peak RSS {rss}; mean CPU {cpu}."));
        line(format!(
            "- SP-08 passed: {} (NFR-02 limit {}).",
            if self.info.config.sp08_passed {
                "yes"
            } else {
                "no"
            },
            if self.info.config.sp08_passed {
                "60 ms"
            } else {
                "150 ms"
            }
        ));
        line(format!(
            "- Statistics: nearest-rank p50 and p95. A time row needs at least {} samples; NFR-08a \
             needs at least {} cycles.",
            super::stats::MIN_TIME_SAMPLES,
            super::stats::MIN_HID_CYCLES
        ));
        line(String::new());

        line("## Raw evidence".into());
        line(String::new());
        if self.source == SampleSource::Doubles {
            line(
                "These samples come from test doubles on the simulated clock. They are not a \
                 measurement."
                    .into(),
            );
            line(String::new());
        }
        line("| Requirement | What | Samples | p50 | p95 | Max | Limit | Outcome |".into());
        line("|---|---|---|---|---|---|---|---|".into());
        for r in &self.results {
            let (n, p50, p95, max) = match (r.summary, r.key_ups) {
                (_, Some((seen, cycles))) => (
                    format!("{seen} of {cycles} key-ups"),
                    "-".into(),
                    "-".into(),
                    "-".into(),
                ),
                (Some(s), None) => (s.n.to_string(), fmt_ms(s.p50), fmt_ms(s.p95), fmt_ms(s.max)),
                (None, None) => ("0".into(), "-".into(), "-".into(), "-".into()),
            };
            line(format!(
                "| {} | {} | {n} | {p50} | {p95} | {max} | {} | {} |",
                r.budget.id,
                r.budget.what,
                r.budget.limit_text(),
                r.outcome.text()
            ));
        }
        if !self.paths.is_empty() {
            line(String::new());
            line("Local and cloud STT paths:".into());
            line(String::new());
            line("| Path | Span | Samples | p50 | p95 |".into());
            line("|---|---|---|---|---|".into());
            for p in &self.paths {
                let (n, p50, p95) = p.summary.map_or(("0".into(), "-".into(), "-".into()), |s| {
                    (s.n.to_string(), fmt_ms(s.p50), fmt_ms(s.p95))
                });
                line(format!(
                    "| {} | {} | {n} | {p50} | {p95} |",
                    p.path.name(),
                    p.span
                ));
            }
        }
        line(String::new());

        line("## Result against the criterion".into());
        line(String::new());
        for r in &self.results {
            line(format!(
                "- {} ({}): {}.",
                r.budget.id,
                r.budget.limit_text(),
                r.outcome.text()
            ));
        }
        line(format!("- Result: `{}`.", result.as_str()));
        for m in self.missing() {
            line(format!("  - Not a pass because {m}."));
        }
        line(format!(
            "- This file covers only the rows above. The other parts of {} ({}) need their own \
             evidence.",
            self.gate.id(),
            self.gate.other_parts()
        ));
        line(String::new());

        line("## Follow-ups".into());
        line(String::new());
        line(
            "- Pipeline marks not wired yet (#319): #7 and #5 (key events), #14 and #15 (mic live), #142 \
             and #318 (acknowledgement and chip), #58 (insertion), #46, #44 and #52 (STT path)."
                .into(),
        );
        md
    }
}
