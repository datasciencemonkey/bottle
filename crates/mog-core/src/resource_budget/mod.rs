//! Memory and idle CPU budget harness (#173): the shell, the brain, the MOGs and Parakeet.
//!
//! The design follows the #171 latency bench (`docs/adr/0016-latency-benchmark-harness.md`):
//! - [`samples`]: the sample model. RSS runs of shell + brain + N MOGs (N = 0..8, without the STT
//!   model and the agent harness), the Parakeet peak RSS over a clip, and idle CPU runs
//!   (`baseline`, `hidden_panels`, `rive_paused`, `rive_animating`) with an optional PresentMon
//!   presents/s value. Every value is a number. No transcript, audio or token can go in
//!   (SEC-16b, SEC-16c).
//! - [`budgets`]: one budget for each row: NFR-04a, NFR-04b, NFR-05, NFR-06a, NFR-06b, NFR-09.
//! - [`decision`]: the SP-02 criteria and the D73 window plan (cap 8, cap 4 or one overlay
//!   window per display).
//! - [`import`]: the sample CSV and the tool outputs: `ps`, `tasklist`, `powermetrics`,
//!   `typeperf`, PresentMon, the SP-04 bench and `/usr/bin/time -l`.
//! - [`report`]: the AT-21 evidence file. It never writes `pass` for samples from test doubles,
//!   for missing samples, for hardware that is not a reference device, or without the run record.
//! - [`sampler`]: an RSS sampler on any [`RssProbe`]; [`doubles`]: test doubles.
//!
//! Gaps that need other work:
//! - the device runs need a person: TODO(#173) (`docs/tests/RESOURCE-BENCH.md`);
//! - SP-02 gives the N = 0, 1, 2, 4, 8 runs and the focus steals: TODO(#79);
//! - the 30 Hz hit-poll that the `hidden_panels` run must have on: TODO(#81).

pub mod budgets;
pub mod decision;
pub mod doubles;
pub mod import;
pub mod report;
pub mod sampler;
pub mod samples;

pub use budgets::{
    evaluate, evaluate_all, BudgetResult, Outcome, IDS, MIN_CLIP_SECS, MIN_CPU_SAMPLES,
    MIN_RSS_SAMPLES,
};
pub use decision::{sp02, window_plan, Sp02Result, WindowPlan};
pub use report::{Report, RunInfo, Source};
pub use sampler::{collect_rss, RssProbe};
pub use samples::{CpuRun, CpuScenario, ParakeetRun, RssRun, Samples, GB, MB};
