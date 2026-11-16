//! The AT-21 resource report (#173) in the `docs/tests/EVIDENCE-TEMPLATE.md` format.
//!
//! The file covers only NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b and NFR-09 (`Scope | part`).
//! It writes `pass` only when every rule holds:
//! - the samples come from a reference device (section 15), not from test doubles;
//! - every row has enough samples, measures what the row asks and is within its limit;
//! - the run has a date (YYYY-MM-DD), a runner, the build, the hardware, the OS and the STT model.
//!
//! A row over its limit gives `fail`, also on hardware that is not a reference device: a real
//! measurement over a limit is not hidden (for example SP-04 on an M3 Max, NFR-05).

use super::budgets::{evaluate_all, BudgetResult, Outcome, IDS, MIN_CPU_SAMPLES, MIN_RSS_SAMPLES};
use super::decision::{sp02, window_plan};
use super::samples::{Samples, MB};
use crate::latency_bench::report::{cell, valid_date};
use crate::latency_bench::GateResult;
use crate::testkit::ReferenceDevice;

/// Where the samples come from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Test doubles. Never a measurement.
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
    /// A real run on hardware that is not a reference device. It can fail, never pass.
    OtherHardware {
        /// The run date, YYYY-MM-DD.
        date_run: String,
        /// The person or role that ran it.
        run_by: String,
    },
}

/// What the run used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunInfo {
    /// The commit SHA or installer name and SHA-256.
    pub build: String,
    /// The hardware, for example `Apple M1 8 GB`.
    pub hardware: String,
    /// The OS and version.
    pub os: String,
    /// The STT model and version of the NFR-05 run.
    pub stt_model: String,
}

/// The resource report.
#[derive(Debug, Clone, PartialEq)]
pub struct Report {
    /// Where the samples come from.
    pub source: Source,
    /// What the run used.
    pub info: RunInfo,
    /// The samples.
    pub samples: Samples,
    /// One result per row, in [`IDS`] order.
    pub results: Vec<BudgetResult>,
}

impl Report {
    /// Evaluate every row on the samples.
    pub fn build(source: Source, info: RunInfo, samples: &Samples) -> Self {
        Self {
            source,
            info,
            samples: samples.clone(),
            results: evaluate_all(samples),
        }
    }

    /// Why the report is not a pass (empty when every rule holds).
    pub fn missing(&self) -> Vec<String> {
        let mut out = Vec::new();
        let (date_run, run_by) = match &self.source {
            Source::Doubles => {
                out.push("the samples come from test doubles, not from a reference device".into());
                (None, None)
            }
            Source::Device {
                date_run, run_by, ..
            } => (Some(date_run), Some(run_by)),
            Source::OtherHardware { date_run, run_by } => {
                out.push(
                    "the hardware is not a reference device (section 15), so the run can fail \
                     but cannot pass"
                        .into(),
                );
                (Some(date_run), Some(run_by))
            }
        };
        if date_run.is_some_and(|d| !valid_date(d.trim())) {
            out.push("the run date is not YYYY-MM-DD".into());
        }
        if run_by.is_some_and(|r| r.trim().is_empty()) {
            out.push("the run by value is empty".into());
        }
        for r in &self.results {
            if r.outcome != Outcome::Within && r.outcome != Outcome::Over {
                let note = r
                    .note
                    .as_deref()
                    .map(|n| format!(" ({n})"))
                    .unwrap_or_default();
                out.push(format!("{}: {}{note}", r.id, r.outcome.text()));
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
        if self.samples.parakeet.is_some() && self.info.stt_model.trim().is_empty() {
            out.push("the STT model of the NFR-05 run is not recorded".into());
        }
        out
    }

    /// The gate result. Doubles give `not run`, whatever the numbers.
    pub fn result(&self) -> GateResult {
        if self.source == Source::Doubles {
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
        let hw = cell(&self.info.hardware);
        let (date, runner, hardware, source) = match &self.source {
            Source::Doubles => (
                "-".to_string(),
                "-".to_string(),
                format!("{hw} (test doubles)"),
                "test doubles (`resource_budget::doubles`)".to_string(),
            ),
            Source::Device {
                device,
                date_run,
                run_by,
            } => (
                cell(date_run),
                cell(run_by),
                format!("{hw} ({})", device.id()),
                format!("reference device `{}`", device.id()),
            ),
            Source::OtherHardware { date_run, run_by } => (
                cell(date_run),
                cell(run_by),
                format!("{hw} (not a reference device)"),
                "a real run on hardware that is not a reference device".to_string(),
            ),
        };
        let ids = IDS.join(", ");
        let mut md = String::new();
        let mut line = |s: String| {
            md.push_str(&s);
            md.push('\n');
        };
        line("# Evidence: AT-21 resource budgets (memory and idle CPU)".into());
        line(String::new());
        line("| Field | Value |".into());
        line("|---|---|".into());
        line("| Gate | AT-21 |".into());
        line("| Gate type | device |".into());
        line(format!("| Result | {} |", result.as_str()));
        line(format!("| Date run | {date} |"));
        line(format!("| Run by | {runner} |"));
        line(format!("| Build | {} |", cell(&self.info.build)));
        line(format!("| Hardware | {hardware} |"));
        line("| Profile | none |".into());
        line(format!("| Scope | part: {ids} only |"));
        line(format!("| Requirements | {ids} |"));
        line(String::new());

        line("## Method".into());
        line(String::new());
        line(
            "Bottle resource benchmark (#173, `docs/tests/RESOURCE-BENCH.md`, \
             `cargo run -p mog-core --example resource_bench`)."
                .into(),
        );
        line(format!("- Samples: {source}."));
        line(format!("- OS: {}.", cell(&self.info.os)));
        let model = if self.info.stt_model.trim().is_empty() {
            "not recorded".to_string()
        } else {
            cell(&self.info.stt_model)
        };
        line(format!("- STT model (NFR-05): {model}."));
        line(format!(
            "- Memory: the RSS sum of the Bottle processes, without the STT model and the agent \
             harness; the peak of each run; decimal MB. A run needs {MIN_RSS_SAMPLES} samples."
        ));
        line(format!(
            "- CPU: percent of one core, the mean of each run, no capture. A run needs \
             {MIN_CPU_SAMPLES} samples. NFR-09 is the hidden_panels run minus the baseline run."
        ));
        line(String::new());

        line("## Raw evidence".into());
        line(String::new());
        if self.source == Source::Doubles {
            line("These samples come from test doubles. They are not a measurement.".into());
            line(String::new());
        }
        line("| Requirement | What | Samples | Measured | Limit | Outcome |".into());
        line("|---|---|---|---|---|---|".into());
        for r in &self.results {
            line(format!(
                "| {} | {} | {} | {} | {} | {} |",
                r.id,
                r.what,
                r.samples,
                r.measured.as_deref().unwrap_or("-"),
                r.limit,
                r.outcome.text()
            ));
        }
        line(String::new());
        if !self.samples.rss.is_empty() {
            line("Memory by MOG count:".into());
            line(String::new());
            line("| MOGs | Samples | Peak |".into());
            line("|---|---|---|".into());
            let mut rss: Vec<_> = self.samples.rss.iter().collect();
            rss.sort_by_key(|r| r.mogs);
            for r in rss {
                let peak = r.peak().unwrap_or(0) as f64 / MB as f64;
                line(format!("| {} | {} | {peak:.1} MB |", r.mogs, r.bytes.len()));
            }
            line(String::new());
        }
        if !self.samples.cpu.is_empty() {
            line("Idle CPU runs:".into());
            line(String::new());
            line("| Scenario | MOGs | Samples | Mean | p95 | Presents/s |".into());
            line("|---|---|---|---|---|---|".into());
            for c in &self.samples.cpu {
                let f = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.2} %"));
                line(format!(
                    "| {} | {} | {} | {} | {} | {} |",
                    c.scenario.name(),
                    c.mogs,
                    c.percent.len(),
                    f(c.mean()),
                    f(c.p95()),
                    c.presents_per_s
                        .map_or("not recorded".into(), |v| format!("{v:.1}"))
                ));
            }
            line(String::new());
        }
        line(format!(
            "Focus steals (SP-02): {}.",
            self.samples
                .focus_steals
                .map_or("not recorded".into(), |n| n.to_string())
        ));
        line(String::new());

        line("## Result against the criterion".into());
        line(String::new());
        for r in &self.results {
            let note = r
                .note
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            line(format!(
                "- {} ({}): {}{note}.",
                r.id,
                r.limit,
                r.outcome.text()
            ));
        }
        let (plan, why) = window_plan(&self.samples);
        line(format!(
            "- SP-02 from these samples: {}.",
            sp02(&self.samples).as_str()
        ));
        line(format!("- Window plan (D73): {}. {why}.", plan.text()));
        line(format!("- Result: `{}`.", result.as_str()));
        for m in self.missing() {
            line(format!("  - Not a pass because {m}."));
        }
        line(
            "- Scope `part`: this file covers only the rows above. The other parts of AT-21 \
             (latency rows, STT, cleanup, recall and blurb, 8 MOGs and the 8 h soak) need their \
             own evidence, and `scripts/check_gate_evidence.py` does not accept a `part` file \
             alone for a ticked row."
                .into(),
        );
        line(String::new());

        line("## Follow-ups".into());
        line(String::new());
        line("- Device runs on the reference devices: #173 (needs:human).".into());
        line("- SP-02 runs (N = 0, 1, 2, 4, 8, focus steals) and the window plan: #79.".into());
        line("- The 30 Hz hit-poll for the hidden_panels run: #81.".into());
        line(
            "- Parakeet over NFR-05: SP-04 #44 and its fallback chain (section 8.3, SP-04 row)."
                .into(),
        );
        md
    }
}
