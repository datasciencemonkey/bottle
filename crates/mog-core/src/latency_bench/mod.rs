//! Latency benchmark harness (#171): talk key, PTT pipeline and dictation.
//!
//! - [`marks`]: the mark API. The real pipeline calls [`LatencyRecorder::mark`] at each step:
//!   key-down, mic live, key-up, local acknowledgement, preview chip and inserted text. It takes
//!   the #229 [`crate::clock::MonotonicClock`], so the tests drive it with `SimClock`.
//! - [`stats`]: nearest-rank p50/p95 and one budget for each latency row (NFR-01a..d, NFR-02,
//!   NFR-08a, NFR-08b; section 8.1).
//! - [`report`]: the evidence file for AT-21, AT-30 and AT-43 in the format of
//!   `docs/tests/EVIDENCE-TEMPLATE.md`. It never writes `pass` for samples from test doubles, for
//!   a row with no samples or too few, or without the run date, the runner, models and resource use.
//! - [`import`]: the sample CSV (the recorder export) and the `EV` lines of the SP-03 probe (#5).
//! - [`doubles`]: a scripted pipeline on the simulated clock.
//!
//! Privacy (SEC-16b, SEC-16c): the harness records only event names, the STT path, the route
//! and times. [`Mark`] is a closed enum with no payload, so no transcript, audio or token can go in.
//!
//! The real call sites are not wired yet (TODO(#319) wires them):
//! - key-down and key-up: HotkeyService, TODO(#319); its HID rig log for AT-30, TODO(#323).
//! - mic live: CaptureService (the #14 lease on key-down), TODO(#425) for the host path, with the SP-08 result TODO(#15).
//! - local acknowledgement and preview chip: the brain and the view, TODO(#142). The chip renders
//!   in the view, so its time needs an IPC event: TODO(#318).
//! - inserted text: `insert_text`, TODO(#58).
//! - the STT path: local Parakeet TODO(#46) (SP-04 TODO(#44)), cloud TODO(#52).
//!
//! Device runs need a person: `docs/tests/LATENCY-BENCH.md`.

pub mod doubles;
pub mod import;
pub mod marks;
pub mod report;
pub mod stats;

pub use marks::{
    HotkeySamples, KeySampleTool, LatencyRecorder, Mark, MarkError, Route, SttPath, Trial, TrialId,
};
pub use report::{Gate, GateResult, Report, ResourceUse, RunInfo, SampleSource};
pub use stats::{
    budget, budgets, evaluate, percentile, Budget, BudgetConfig, BudgetResult, Check, Outcome,
    Stat, Summary, MIN_HID_CYCLES, MIN_TIME_SAMPLES,
};
