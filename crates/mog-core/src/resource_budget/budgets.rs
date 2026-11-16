//! The budget of each memory and CPU row (#173, section 5.11).
//!
//! | Row | Measure | Limit |
//! |---|---|---|
//! | NFR-04a | peak RSS, shell + brain + 1 MOG | <= 250 MB |
//! | NFR-04b | (peak N=8 - peak N=1) / 7 | <= 40 MB |
//! | NFR-05 | Parakeet peak RSS over a clip of 4 min or more | <= 1.0 GB |
//! | NFR-06a | mean idle CPU, Rive paused | <= 1 % |
//! | NFR-06b | mean idle CPU, 8 MOGs animating | <= 3 % |
//! | NFR-09 | mean CPU, hidden panels + hit-poll, minus the baseline | < 0.2 % |
//!
//! CPU is in percent of one core. The memory units are decimal (1 MB = 10^6 bytes).

use super::samples::{CpuRun, CpuScenario, RssRun, Samples, GB, MAX_MOGS, MB};

/// The rows of #173, in requirement order.
pub const IDS: [&str; 6] = [
    "NFR-04a", "NFR-04b", "NFR-05", "NFR-06a", "NFR-06b", "NFR-09",
];
/// An RSS run needs this many samples.
pub const MIN_RSS_SAMPLES: usize = 5;
/// A CPU run needs this many samples (60 s at 1 sample per second).
pub const MIN_CPU_SAMPLES: usize = 60;
/// NFR-05 is measured over a 4 min clip.
pub const MIN_CLIP_SECS: f64 = 240.0;
/// More presents per second than this: Rive is not paused.
pub const PAUSED_MAX_PRESENTS_PER_S: f64 = 1.0;
/// Fewer presents per second than this: Rive is not animating.
pub const ANIMATING_MIN_PRESENTS_PER_S: f64 = 10.0;

const EPS: f64 = 1e-9;

/// The outcome of one row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The samples are within the limit.
    Within,
    /// The samples are over the limit.
    Over,
    /// The row has no samples.
    NoData,
    /// The row has fewer samples than the minimum.
    TooFew,
    /// The samples do not measure what the row asks (see the note).
    Invalid,
}

impl Outcome {
    /// The outcome as report text.
    pub fn text(self) -> &'static str {
        match self {
            Outcome::Within => "within",
            Outcome::Over => "over",
            Outcome::NoData => "no data",
            Outcome::TooFew => "too few samples",
            Outcome::Invalid => "invalid run",
        }
    }
}

/// The result of one row.
#[derive(Debug, Clone, PartialEq)]
pub struct BudgetResult {
    /// The requirement ID.
    pub id: &'static str,
    /// What the row measures, in short words.
    pub what: &'static str,
    /// The measure and the limit.
    pub limit: String,
    /// The sample count that the row used.
    pub samples: usize,
    /// The measured value as text, if there is one.
    pub measured: Option<String>,
    /// The outcome.
    pub outcome: Outcome,
    /// Why the outcome is not within or over.
    pub note: Option<String>,
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / MB as f64)
}

fn pct(v: f64) -> String {
    format!("{v:.2} %")
}

struct Row {
    id: &'static str,
    what: &'static str,
    limit: String,
}

impl Row {
    fn result(
        self,
        samples: usize,
        measured: Option<String>,
        outcome: Outcome,
        note: Option<String>,
    ) -> BudgetResult {
        BudgetResult {
            id: self.id,
            what: self.what,
            limit: self.limit,
            samples,
            measured,
            outcome,
            note,
        }
    }

    fn none(self, outcome: Outcome, note: impl Into<String>) -> BudgetResult {
        self.result(0, None, outcome, Some(note.into()))
    }
}

fn within(ok: bool) -> Outcome {
    if ok {
        Outcome::Within
    } else {
        Outcome::Over
    }
}

/// The RSS run with `mogs` MOGs and its peak, or the result to give when it is not usable.
fn rss_peak(s: &Samples, mogs: u8) -> Result<(&RssRun, u64), (Outcome, String)> {
    let Some(run) = s.rss_run(mogs) else {
        return Err((Outcome::NoData, format!("no RSS run with N={mogs}")));
    };
    if run.bytes.len() < MIN_RSS_SAMPLES {
        return Err((
            Outcome::TooFew,
            format!(
                "the N={mogs} run has {} samples; it needs {MIN_RSS_SAMPLES}",
                run.bytes.len()
            ),
        ));
    }
    let peak = run.peak().unwrap_or(0);
    Ok((run, peak))
}

fn cpu_mean(run: Option<&CpuRun>, name: &str) -> Result<(usize, f64), (Outcome, String)> {
    let Some(run) = run else {
        return Err((Outcome::NoData, format!("no `{name}` CPU run")));
    };
    if run.percent.len() < MIN_CPU_SAMPLES {
        return Err((
            Outcome::TooFew,
            format!(
                "the `{name}` run has {} samples; it needs {MIN_CPU_SAMPLES}",
                run.percent.len()
            ),
        ));
    }
    Ok((run.percent.len(), run.mean().unwrap_or(f64::NAN)))
}

fn nfr_04a(s: &Samples) -> BudgetResult {
    let row = Row {
        id: "NFR-04a",
        what: "shell + brain + 1 MOG, without STT model and agent harness",
        limit: "peak <= 250 MB".into(),
    };
    match rss_peak(s, 1) {
        Err((o, why)) => row.none(o, why),
        Ok((run, peak)) => row.result(
            run.bytes.len(),
            Some(format!("{} peak", mb(peak))),
            within(peak <= 250 * MB),
            None,
        ),
    }
}

fn nfr_04b(s: &Samples) -> BudgetResult {
    let row = Row {
        id: "NFR-04b",
        what: "each extra MOG, measured with 8",
        limit: "(N=8 - N=1) / 7 <= 40 MB".into(),
    };
    let one = match rss_peak(s, 1) {
        Err((o, why)) => return row.none(o, format!("{why}; the row needs N=1 and N=8")),
        Ok(v) => v,
    };
    let eight = match rss_peak(s, MAX_MOGS) {
        Err((o, why)) => return row.none(o, format!("{why}; the row needs N=1 and N=8")),
        Ok(v) => v,
    };
    let extra = eight.1.saturating_sub(one.1);
    let per = extra as f64 / 7.0 / MB as f64;
    row.result(
        one.0.bytes.len() + eight.0.bytes.len(),
        Some(format!("{per:.1} MB per extra MOG")),
        within(extra <= 7 * 40 * MB),
        largest_step(s),
    )
}

/// The largest step per MOG between two measured MOG counts (N >= 1), as a note when it is over
/// 40 MB. The row is the mean from N=1 to N=8 (ADR 0020); a large step is shown, not hidden.
fn largest_step(s: &Samples) -> Option<String> {
    let mut peaks: Vec<(u8, u64)> = s
        .rss
        .iter()
        .filter(|r| r.mogs >= 1 && r.bytes.len() >= MIN_RSS_SAMPLES)
        .filter_map(|r| Some((r.mogs, r.peak()?)))
        .collect();
    peaks.sort_unstable();
    peaks
        .windows(2)
        .map(|w| {
            let per = w[1].1.saturating_sub(w[0].1) as f64 / f64::from(w[1].0 - w[0].0);
            (per, w[0].0, w[1].0)
        })
        .filter(|(per, ..)| *per > 40.0 * MB as f64)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(per, a, b)| {
            format!(
                "the step from N={a} to N={b} adds {:.1} MB per MOG",
                per / MB as f64
            )
        })
}

fn nfr_05(s: &Samples) -> BudgetResult {
    let row = Row {
        id: "NFR-05",
        what: "Parakeet peak resident memory",
        limit: format!("peak <= {} MB over >= {MIN_CLIP_SECS:.0} s", GB / MB),
    };
    let Some(p) = s.parakeet else {
        return row.none(Outcome::NoData, "no Parakeet run");
    };
    let measured = Some(format!(
        "{} peak over {:.0} s",
        mb(p.peak_bytes),
        p.clip_secs
    ));
    if !(p.clip_secs.is_finite() && p.clip_secs >= MIN_CLIP_SECS) {
        return row.result(
            1,
            measured,
            Outcome::Invalid,
            Some(format!(
                "the clip is {:.0} s; the row needs a 4 min clip (>= {MIN_CLIP_SECS:.0} s)",
                p.clip_secs
            )),
        );
    }
    row.result(1, measured, within(p.peak_bytes <= GB), None)
}

fn nfr_06a(s: &Samples) -> BudgetResult {
    let row = Row {
        id: "NFR-06a",
        what: "idle CPU, no capture, Rive paused",
        limit: "mean <= 1 %".into(),
    };
    let run = s.cpu_run(CpuScenario::RivePaused);
    let (n, mean) = match cpu_mean(run, "rive_paused") {
        Err((o, why)) => return row.none(o, why),
        Ok(v) => v,
    };
    let measured = Some(format!("{} mean", pct(mean)));
    let run = run.expect("cpu_mean checked the run");
    if run.mogs == 0 {
        return row.result(
            n,
            measured,
            Outcome::Invalid,
            Some("the rive_paused run has 0 MOGs, so no Rive is paused".into()),
        );
    }
    if let Some(fps) = run.presents_per_s {
        if fps > PAUSED_MAX_PRESENTS_PER_S {
            return row.result(
                n,
                measured,
                Outcome::Invalid,
                Some(format!(
                    "PresentMon shows {fps:.1} presents/s, so Rive is not paused"
                )),
            );
        }
    }
    row.result(n, measured, within(mean <= 1.0 + EPS), None)
}

fn nfr_06b(s: &Samples) -> BudgetResult {
    let row = Row {
        id: "NFR-06b",
        what: "idle CPU, no capture, 8 MOGs animating",
        limit: "mean <= 3 %".into(),
    };
    let run = s.cpu_run(CpuScenario::RiveAnimating);
    let (n, mean) = match cpu_mean(run, "rive_animating") {
        Err((o, why)) => return row.none(o, why),
        Ok(v) => v,
    };
    let measured = Some(format!("{} mean", pct(mean)));
    let run = run.expect("cpu_mean checked the run");
    if run.mogs != MAX_MOGS {
        return row.result(
            n,
            measured,
            Outcome::Invalid,
            Some(format!(
                "the run has {} MOGs; the row needs 8 MOGs",
                run.mogs
            )),
        );
    }
    if let Some(fps) = run.presents_per_s {
        if fps < ANIMATING_MIN_PRESENTS_PER_S {
            return row.result(
                n,
                measured,
                Outcome::Invalid,
                Some(format!(
                    "PresentMon shows {fps:.1} presents/s, so Rive is not animating"
                )),
            );
        }
    }
    row.result(n, measured, within(mean <= 3.0 + EPS), None)
}

fn nfr_09(s: &Samples) -> BudgetResult {
    let row = Row {
        id: "NFR-09",
        what: "hidden MOG panels + 30 Hz hit-poll, idle",
        limit: "hidden_panels - baseline < 0.2 %".into(),
    };
    let hidden = s.cpu_run(CpuScenario::HiddenPanels);
    let (nh, mh) = match cpu_mean(hidden, "hidden_panels") {
        Err((o, why)) => return row.none(o, format!("{why}; the cost needs both runs")),
        Ok(v) => v,
    };
    let baseline = s.cpu_run(CpuScenario::Baseline);
    let (nb, mb_) = match cpu_mean(baseline, "baseline") {
        Err((o, why)) => return row.none(o, format!("{why}; the cost needs both runs")),
        Ok(v) => v,
    };
    let delta = mh - mb_;
    let measured = Some(format!("{} over baseline", pct(delta)));
    if hidden.map_or(0, |h| h.mogs) == 0 {
        return row.result(
            nh + nb,
            measured,
            Outcome::Invalid,
            Some("the hidden_panels run has 0 MOGs, so it has no hidden panel".into()),
        );
    }
    if let Some(b) = baseline.filter(|b| b.mogs != 0) {
        return row.result(
            nh + nb,
            measured,
            Outcome::Invalid,
            Some(format!(
                "the baseline run has {} MOGs; it needs 0 MOGs and the hit-poll off",
                b.mogs
            )),
        );
    }
    row.result(nh + nb, measured, within(delta < 0.2 - EPS), None)
}

/// The result of the row `id`, or `None` for an ID that is not a #173 row.
pub fn evaluate(id: &str, s: &Samples) -> Option<BudgetResult> {
    Some(match id {
        "NFR-04a" => nfr_04a(s),
        "NFR-04b" => nfr_04b(s),
        "NFR-05" => nfr_05(s),
        "NFR-06a" => nfr_06a(s),
        "NFR-06b" => nfr_06b(s),
        "NFR-09" => nfr_09(s),
        _ => return None,
    })
}

/// The result of every row, in [`IDS`] order.
pub fn evaluate_all(s: &Samples) -> Vec<BudgetResult> {
    IDS.iter().filter_map(|id| evaluate(id, s)).collect()
}
