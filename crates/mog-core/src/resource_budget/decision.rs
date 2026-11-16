//! The SP-02 criteria and the D73 window plan (#173, #79).
//!
//! SP-02 passes when shell + brain + 1 MOG <= 250 MB, each extra MOG <= 40 MB, 8 MOGs <= 550 MB
//! without STT, idle CPU <= 3 % animating and <= 1 % paused, and 0 focus steals. When it fails,
//! the fallback is "default cap 4, then one overlay window per display with all mascots" (D73).
//!
//! The rule:
//! 1. SP-02 has not run (a criterion has no data): [`WindowPlan::Undecided`], TODO(#79).
//! 2. SP-02 passes: [`WindowPlan::Cap8`], one panel per MOG, up to 8 MOGs.
//! 3. SP-02 fails, and 4 MOGs fit (N=1 <= 250 MB, (N=4 - N=1) / 3 <= 40 MB), no CPU row is over
//!    and there are 0 focus steals: [`WindowPlan::Cap4ThenOverlay`]. The default cap is 4 MOG
//!    panels; more MOGs go into the D73 overlay.
//! 4. All other fails: [`WindowPlan::OverlayPerDisplay`]. A cap does not stop focus steals.

use super::budgets::{evaluate, Outcome};
use super::samples::{Samples, MB};

/// The SP-02 result from the samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sp02Result {
    /// Every criterion holds.
    Pass,
    /// One or more criteria do not hold (the reasons).
    Fail(Vec<String>),
    /// No criterion fails, but one or more have no usable data (the reasons).
    NotRun(Vec<String>),
}

impl Sp02Result {
    /// `pass`, `fail` or `not run`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Sp02Result::Pass => "pass",
            Sp02Result::Fail(_) => "fail",
            Sp02Result::NotRun(_) => "not run",
        }
    }
}

/// The window model that the SP-02 result gives (D73).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowPlan {
    /// One panel per MOG, up to 8 active MOGs.
    Cap8,
    /// A default cap of 4 MOG panels; more MOGs go into one overlay window per display.
    Cap4ThenOverlay,
    /// One overlay window per display hosts all mascots.
    OverlayPerDisplay,
    /// SP-02 has not run.
    Undecided,
}

impl WindowPlan {
    /// The plan as report text.
    pub fn text(self) -> &'static str {
        match self {
            WindowPlan::Cap8 => "cap 8, one panel per MOG",
            WindowPlan::Cap4ThenOverlay => {
                "default cap 4 MOG panels, then one overlay window per display (D73)"
            }
            WindowPlan::OverlayPerDisplay => {
                "one overlay window per display hosts all mascots (D73)"
            }
            WindowPlan::Undecided => "undecided",
        }
    }
}

const SP02_ROWS: [&str; 4] = ["NFR-04a", "NFR-04b", "NFR-06a", "NFR-06b"];
const CPU_ROWS: [&str; 2] = ["NFR-06a", "NFR-06b"];

fn over(s: &Samples, id: &str) -> bool {
    evaluate(id, s).is_some_and(|r| r.outcome == Outcome::Over)
}

/// The SP-02 result from the samples.
pub fn sp02(s: &Samples) -> Sp02Result {
    let mut fails = Vec::new();
    let mut gaps = Vec::new();
    for id in SP02_ROWS {
        let r = evaluate(id, s).expect("known row");
        match r.outcome {
            Outcome::Within => {}
            Outcome::Over => fails.push(format!(
                "{id} over ({}, {})",
                r.measured.unwrap_or_default(),
                r.limit
            )),
            o => gaps.push(format!(
                "{id}: {}{}",
                o.text(),
                r.note.map(|n| format!(" ({n})")).unwrap_or_default()
            )),
        }
    }
    match s.rss_run(8).and_then(|r| r.peak()) {
        Some(peak) if peak > 550 * MB => fails.push(format!(
            "8 MOGs use {:.1} MB; SP-02 needs 550 MB or less without STT",
            peak as f64 / MB as f64
        )),
        Some(_) => {}
        None => gaps.push("no RSS run with 8 MOGs (550 MB criterion)".into()),
    }
    match s.focus_steals {
        Some(0) => {}
        Some(n) => fails.push(format!("{n} focus steals; SP-02 needs 0")),
        None => gaps.push("the focus steals are not recorded".into()),
    }
    if !fails.is_empty() {
        Sp02Result::Fail(fails)
    } else if !gaps.is_empty() {
        Sp02Result::NotRun(gaps)
    } else {
        Sp02Result::Pass
    }
}

fn four_fit(s: &Samples) -> bool {
    let peak = |n: u8| {
        s.rss_run(n)
            .filter(|r| r.bytes.len() >= super::budgets::MIN_RSS_SAMPLES)
            .and_then(|r| r.peak())
    };
    match (peak(1), peak(4)) {
        (Some(one), Some(four)) => one <= 250 * MB && four.saturating_sub(one) <= 3 * 40 * MB,
        _ => false,
    }
}

/// The window plan and the reason for it.
pub fn window_plan(s: &Samples) -> (WindowPlan, String) {
    match sp02(s) {
        Sp02Result::Pass => (WindowPlan::Cap8, "SP-02 passes".into()),
        Sp02Result::NotRun(gaps) => (
            WindowPlan::Undecided,
            format!(
                "SP-02 has not run, TODO(#79): {}; until then the MOG panels stay the default",
                gaps.join("; ")
            ),
        ),
        Sp02Result::Fail(fails) => {
            let why = fails.join("; ");
            if s.focus_steals.is_some_and(|n| n > 0) {
                (
                    WindowPlan::OverlayPerDisplay,
                    format!("SP-02 fails ({why}); a cap does not stop focus steals"),
                )
            } else if four_fit(s) && !CPU_ROWS.iter().any(|id| over(s, id)) {
                (
                    WindowPlan::Cap4ThenOverlay,
                    format!("SP-02 fails ({why}); 4 MOGs fit the memory budget"),
                )
            } else {
                (
                    WindowPlan::OverlayPerDisplay,
                    format!("SP-02 fails ({why}); 4 MOGs do not fit, or have no N=4 run"),
                )
            }
        }
    }
}
