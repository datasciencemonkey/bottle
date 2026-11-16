//! Test doubles for the resource harness (#173). They measure nothing: a report on their
//! samples is always `not run`.

use std::collections::VecDeque;

use super::budgets::{MIN_CPU_SAMPLES, MIN_RSS_SAMPLES};
use super::sampler::RssProbe;
use super::samples::{CpuRun, CpuScenario, ParakeetRun, RssRun, Samples, MB};

/// A probe that returns scripted values in order, then an error.
#[derive(Debug, Clone, Default)]
pub struct ScriptedProbe {
    values: VecDeque<Result<u64, String>>,
}

impl ScriptedProbe {
    /// A probe with these values.
    pub fn new(values: Vec<Result<u64, String>>) -> Self {
        Self {
            values: values.into(),
        }
    }
}

impl RssProbe for ScriptedProbe {
    fn sample(&mut self) -> Result<u64, String> {
        self.values
            .pop_front()
            .unwrap_or_else(|| Err("the scripted probe has no more values".into()))
    }
}

fn rss_run(mogs: u8, peak_mb: u64) -> RssRun {
    let peak = peak_mb * MB;
    let mut bytes = vec![peak - 2 * MB; MIN_RSS_SAMPLES - 1];
    bytes.push(peak);
    RssRun { mogs, bytes }
}

fn cpu_run(scenario: CpuScenario, mogs: u8, mean: f64, fps: Option<f64>) -> CpuRun {
    CpuRun {
        scenario,
        mogs,
        percent: vec![mean; MIN_CPU_SAMPLES],
        presents_per_s: fps,
    }
}

/// A full sample set that is within every budget and passes SP-02: N = 0, 1, 2, 4, 8 at
/// 150 MB + 30 MB per MOG, Parakeet 900 MB over 240 s, and idle CPU under each limit.
pub fn demo_samples() -> Samples {
    Samples {
        rss: [0u8, 1, 2, 4, 8]
            .into_iter()
            .map(|n| rss_run(n, 150 + 30 * u64::from(n)))
            .collect(),
        parakeet: Some(ParakeetRun {
            clip_secs: 240.0,
            peak_bytes: 900 * MB,
        }),
        cpu: vec![
            cpu_run(CpuScenario::Baseline, 0, 0.25, None),
            cpu_run(CpuScenario::HiddenPanels, 8, 0.375, None),
            cpu_run(CpuScenario::RivePaused, 8, 0.5, Some(0.0)),
            cpu_run(CpuScenario::RiveAnimating, 8, 2.5, Some(60.0)),
        ],
        focus_steals: Some(0),
    }
}
