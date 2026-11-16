//! Percentiles and the latency budget of each NFR row (#171, section 8.1).

use std::time::Duration;

use super::marks::{HotkeySamples, Mark, Route, SttPath, Trial};

/// A time row needs this many samples before it can be within budget. Below 20 samples the
/// nearest-rank p95 is the maximum, so the p95 has no meaning.
pub const MIN_TIME_SAMPLES: usize = 20;
/// NFR-08a: the HID rig runs 1,000 press cycles (AT-30, SP-03).
pub const MIN_HID_CYCLES: u32 = 1_000;

/// The nearest-rank percentile `p` (0..=100) of `samples`. `None` if `samples` is empty or `p`
/// is over 100.
pub fn percentile(samples: &[Duration], p: u32) -> Option<Duration> {
    if samples.is_empty() || p > 100 {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let rank = (p as usize * sorted.len()).div_ceil(100).max(1);
    Some(sorted[rank - 1])
}

/// The sample count, p50, p95 and maximum of a sample set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Summary {
    /// The sample count.
    pub n: usize,
    /// The nearest-rank median.
    pub p50: Duration,
    /// The nearest-rank 95th percentile.
    pub p95: Duration,
    /// The largest sample.
    pub max: Duration,
}

impl Summary {
    /// The summary of `samples`, or `None` if there are none.
    pub fn of(samples: &[Duration]) -> Option<Self> {
        Some(Self {
            n: samples.len(),
            p50: percentile(samples, 50)?,
            p95: percentile(samples, 95)?,
            max: percentile(samples, 100)?,
        })
    }
}

/// The statistic that a time budget limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stat {
    /// The 95th percentile (rows that say "at p95").
    P95,
    /// Every sample (rows with no percentile, for example NFR-01c).
    Max,
}

/// What a budget measures and its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// The span from one mark to another, for trials with this path and route (`None` = all).
    Span {
        /// The start mark.
        from: Mark,
        /// The end mark.
        to: Mark,
        /// Only trials with this STT path.
        path: Option<SttPath>,
        /// Only trials with this route.
        route: Option<Route>,
        /// The limited statistic.
        stat: Stat,
        /// The limit.
        max: Duration,
    },
    /// The OS event to HotkeyService delivery time.
    KeyEventDelivery {
        /// The limited statistic.
        stat: Stat,
        /// The limit.
        max: Duration,
    },
    /// Every key-down has its key-up, over at least `min_cycles` cycles.
    AllKeyUps {
        /// The fewest cycles that count as a run.
        min_cycles: u32,
    },
}

/// One latency row of the requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    /// The requirement ID, for example `NFR-01a`.
    pub id: &'static str,
    /// What the row measures, in short words.
    pub what: &'static str,
    /// The measure and the limit.
    pub check: Check,
}

impl Budget {
    /// The limit as text, for example `p95 <= 1500 ms`.
    pub fn limit_text(&self) -> String {
        let stat = |s: Stat| match s {
            Stat::P95 => "p95",
            Stat::Max => "every sample",
        };
        match self.check {
            Check::Span { stat: s, max, .. } | Check::KeyEventDelivery { stat: s, max } => {
                format!("{} <= {} ms", stat(s), max.as_millis())
            }
            Check::AllKeyUps { min_cycles } => format!("100 % key-ups in >= {min_cycles} cycles"),
        }
    }
}

/// Settings that change a budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BudgetConfig {
    /// SP-08 passed, so NFR-02 is 60 ms, not 150 ms.
    pub sp08_passed: bool,
}

/// The latency rows of #171, in requirement order.
pub fn budgets(cfg: BudgetConfig) -> Vec<Budget> {
    let ms = Duration::from_millis;
    let span = |from, to, path, route, stat, max| Check::Span {
        from,
        to,
        path,
        route,
        stat,
        max,
    };
    vec![
        Budget {
            id: "NFR-01a",
            what: "key-up to preview chip, local STT",
            check: span(
                Mark::KeyUp,
                Mark::PreviewChip,
                Some(SttPath::Local),
                Some(Route::Question),
                Stat::P95,
                ms(1_500),
            ),
        },
        Budget {
            id: "NFR-01b",
            what: "key-up to preview chip, cloud STT",
            check: span(
                Mark::KeyUp,
                Mark::PreviewChip,
                Some(SttPath::Cloud),
                Some(Route::Question),
                Stat::P95,
                ms(2_200),
            ),
        },
        Budget {
            id: "NFR-01c",
            what: "key-up to local acknowledgement",
            check: span(Mark::KeyUp, Mark::LocalAck, None, None, Stat::Max, ms(100)),
        },
        Budget {
            id: "NFR-01d",
            what: "key-up to inserted text, local STT (UC1)",
            check: span(
                Mark::KeyUp,
                Mark::TextInserted,
                Some(SttPath::Local),
                Some(Route::Dictation),
                Stat::P95,
                ms(1_500),
            ),
        },
        Budget {
            id: "NFR-02",
            what: "key-down to mic live",
            check: span(
                Mark::KeyDown,
                Mark::MicLive,
                None,
                None,
                Stat::P95,
                ms(if cfg.sp08_passed { 60 } else { 150 }),
            ),
        },
        Budget {
            id: "NFR-08a",
            what: "key-ups detected (HID rig)",
            check: Check::AllKeyUps {
                min_cycles: MIN_HID_CYCLES,
            },
        },
        Budget {
            id: "NFR-08b",
            what: "key event delivery (HID rig)",
            check: Check::KeyEventDelivery {
                stat: Stat::P95,
                max: ms(15),
            },
        },
    ]
}

/// The budget of one requirement ID.
pub fn budget(id: &str, cfg: BudgetConfig) -> Option<Budget> {
    budgets(cfg).into_iter().find(|b| b.id == id)
}

/// How a sample set compares with a budget. Only [`Outcome::Within`] counts as a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Enough samples, and the statistic is at or under the limit.
    Within,
    /// Enough samples, and the statistic is over the limit.
    Over,
    /// No samples.
    NoData,
    /// Some samples, but fewer than the row needs.
    TooFewSamples {
        /// Samples or cycles in the set.
        have: usize,
        /// Samples or cycles the row needs.
        need: usize,
    },
}

impl Outcome {
    /// The outcome as report text.
    pub fn text(self) -> String {
        match self {
            Outcome::Within => "within".into(),
            Outcome::Over => "over".into(),
            Outcome::NoData => "no data".into(),
            Outcome::TooFewSamples { have, need } => format!("too few samples ({have} of {need})"),
        }
    }
}

/// One budget, its samples and the outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetResult {
    /// The budget.
    pub budget: Budget,
    /// The summary of the time samples (none for NFR-08a or with no samples).
    pub summary: Option<Summary>,
    /// Key-ups seen and cycles, for NFR-08a.
    pub key_ups: Option<(u32, u32)>,
    /// Span rows: trials of the row with the start mark but no end mark (the chip, the ack, the
    /// mic or the text never came). Each one counts as over the limit.
    pub incomplete: usize,
    /// The outcome.
    pub outcome: Outcome,
}

/// The span samples of `trials` for a span check.
pub fn span_samples(
    trials: &[Trial],
    from: Mark,
    to: Mark,
    path: Option<SttPath>,
    route: Option<Route>,
) -> Vec<Duration> {
    row_trials(trials, path, route)
        .filter_map(|t| t.span(from, to))
        .collect()
}

fn row_trials(
    trials: &[Trial],
    path: Option<SttPath>,
    route: Option<Route>,
) -> impl Iterator<Item = &Trial> {
    trials
        .iter()
        .filter(move |t| path.is_none_or(|p| t.path() == p))
        .filter(move |t| route.is_none_or(|r| t.route() == r))
}

/// The trials of a span row that have `from` but no `to`.
pub fn incomplete_trials(
    trials: &[Trial],
    from: Mark,
    to: Mark,
    path: Option<SttPath>,
    route: Option<Route>,
) -> usize {
    row_trials(trials, path, route)
        .filter(|t| t.at(from).is_some() && t.at(to).is_none())
        .count()
}

fn time_outcome(samples: &[Duration], stat: Stat, max: Duration) -> (Option<Summary>, Outcome) {
    let Some(summary) = Summary::of(samples) else {
        return (None, Outcome::NoData);
    };
    if summary.n < MIN_TIME_SAMPLES {
        let outcome = Outcome::TooFewSamples {
            have: summary.n,
            need: MIN_TIME_SAMPLES,
        };
        return (Some(summary), outcome);
    }
    let value = match stat {
        Stat::P95 => summary.p95,
        Stat::Max => summary.max,
    };
    let outcome = if value <= max {
        Outcome::Within
    } else {
        Outcome::Over
    };
    (Some(summary), outcome)
}

/// Compare the samples with one budget.
pub fn evaluate(budget: &Budget, trials: &[Trial], hotkey: &HotkeySamples) -> BudgetResult {
    let mut incomplete = 0;
    let (summary, key_ups, outcome) = match budget.check {
        Check::Span {
            from,
            to,
            path,
            route,
            stat,
            max,
        } => {
            incomplete = incomplete_trials(trials, from, to, path, route);
            let (s, o) = time_outcome(&span_samples(trials, from, to, path, route), stat, max);
            // A press whose end step never came is over the limit, whatever the other samples say.
            let o = if incomplete > 0 { Outcome::Over } else { o };
            (s, None, o)
        }
        Check::KeyEventDelivery { stat, max } => {
            let (s, o) = time_outcome(&hotkey.event_latencies, stat, max);
            (s, None, o)
        }
        Check::AllKeyUps { min_cycles } => {
            let seen = hotkey.key_ups_seen.min(hotkey.cycles);
            let outcome = if hotkey.cycles == 0 {
                Outcome::NoData
            } else if hotkey.cycles < min_cycles {
                Outcome::TooFewSamples {
                    have: hotkey.cycles as usize,
                    need: min_cycles as usize,
                }
            } else if seen == hotkey.cycles {
                Outcome::Within
            } else {
                Outcome::Over
            };
            (None, Some((seen, hotkey.cycles)), outcome)
        }
    };
    BudgetResult {
        budget: *budget,
        summary,
        key_ups,
        incomplete,
        outcome,
    }
}
