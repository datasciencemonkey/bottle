//! The sample model (#173). Every value is a number: there is no free-text field (SEC-16b).

/// One decimal megabyte. The limits use decimal units, the same as the SP-04 bench.
pub const MB: u64 = 1_000_000;
/// One decimal gigabyte.
pub const GB: u64 = 1_000 * MB;
/// The most MOGs in a run (the cap of 8 active MOGs).
pub const MAX_MOGS: u8 = 8;

/// The resident memory of shell + brain + `mogs` MOGs, without the STT model and the agent
/// harness (NFR-04a, NFR-04b). One value per sample, in bytes, summed over the Bottle processes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RssRun {
    /// The number of open MOGs, 0..=8.
    pub mogs: u8,
    /// The samples, bytes.
    pub bytes: Vec<u64>,
}

impl RssRun {
    /// The largest sample.
    pub fn peak(&self) -> Option<u64> {
        self.bytes.iter().copied().max()
    }
}

/// The Parakeet peak resident memory over a clip (NFR-05).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParakeetRun {
    /// The audio length of the clip, seconds.
    pub clip_secs: f64,
    /// The peak RSS, bytes.
    pub peak_bytes: u64,
}

/// The idle state that a CPU run measures. No run has a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CpuScenario {
    /// Shell and brain, no MOG panel, hit-poll off: the reference for NFR-09.
    Baseline,
    /// Hidden MOG panels with the 30 Hz hit-poll on (NFR-09).
    HiddenPanels,
    /// MOGs visible, Rive paused (NFR-06a).
    RivePaused,
    /// MOGs visible, Rive animating (NFR-06b).
    RiveAnimating,
}

impl CpuScenario {
    /// Every scenario.
    pub const ALL: [CpuScenario; 4] = [
        CpuScenario::Baseline,
        CpuScenario::HiddenPanels,
        CpuScenario::RivePaused,
        CpuScenario::RiveAnimating,
    ];

    /// The name in the sample file.
    pub fn name(self) -> &'static str {
        match self {
            CpuScenario::Baseline => "baseline",
            CpuScenario::HiddenPanels => "hidden_panels",
            CpuScenario::RivePaused => "rive_paused",
            CpuScenario::RiveAnimating => "rive_animating",
        }
    }

    /// The scenario with this name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// An idle CPU run: the CPU use of the Bottle processes in percent of one core, one value per
/// sample (normally 1 s).
#[derive(Debug, Clone, PartialEq)]
pub struct CpuRun {
    /// The idle state.
    pub scenario: CpuScenario,
    /// The number of MOGs.
    pub mogs: u8,
    /// The samples, percent of one core.
    pub percent: Vec<f64>,
    /// The frames that the Bottle processes presented per second (PresentMon), if recorded.
    pub presents_per_s: Option<f64>,
}

impl CpuRun {
    /// The mean of the samples.
    pub fn mean(&self) -> Option<f64> {
        (!self.percent.is_empty())
            .then(|| self.percent.iter().sum::<f64>() / self.percent.len() as f64)
    }

    /// The nearest-rank 95th percentile of the samples.
    pub fn p95(&self) -> Option<f64> {
        if self.percent.is_empty() {
            return None;
        }
        let mut v = self.percent.clone();
        v.sort_by(f64::total_cmp);
        let rank = (95 * v.len()).div_ceil(100).max(1);
        Some(v[rank - 1])
    }
}

/// All the samples of one run of the harness.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Samples {
    /// The RSS runs, one per MOG count.
    pub rss: Vec<RssRun>,
    /// The Parakeet run.
    pub parakeet: Option<ParakeetRun>,
    /// The idle CPU runs.
    pub cpu: Vec<CpuRun>,
    /// The focus steals counted while a person types in a front app (SP-02).
    pub focus_steals: Option<u32>,
}

impl Samples {
    /// The RSS run with this MOG count.
    pub fn rss_run(&self, mogs: u8) -> Option<&RssRun> {
        self.rss.iter().find(|r| r.mogs == mogs)
    }

    /// The CPU run of this scenario with the most MOGs.
    pub fn cpu_run(&self, scenario: CpuScenario) -> Option<&CpuRun> {
        self.cpu
            .iter()
            .filter(|c| c.scenario == scenario)
            .max_by_key(|c| c.mogs)
    }
}
