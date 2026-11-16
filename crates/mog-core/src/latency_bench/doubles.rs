//! Test doubles for the benchmark (#171): a scripted PTT pipeline on the #229 simulated clock.
//!
//! The double does not measure anything. A report on its samples is always `not run`.

use std::sync::Arc;
use std::time::Duration;

use super::marks::{HotkeySamples, LatencyRecorder, Mark, MarkError, Route, SttPath, TrialId};
use crate::clock::{MonotonicClock, SimClock};

/// The timing of one scripted press. `mic_live` and `hold` count from key-down; `ack`, `chip`
/// and `insert` count from key-up. `None` means the step does not occur.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptedTiming {
    /// Key-down to mic live.
    pub mic_live: Duration,
    /// Key-down to key-up.
    pub hold: Duration,
    /// Key-up to local acknowledgement.
    pub ack: Duration,
    /// Key-up to preview chip.
    pub chip: Option<Duration>,
    /// Key-up to inserted text.
    pub insert: Option<Duration>,
}

/// A pipeline double: each press moves the simulated clock and records the marks.
#[derive(Debug)]
pub struct ScriptedPipeline {
    clock: SimClock,
    recorder: LatencyRecorder,
}

impl Default for ScriptedPipeline {
    fn default() -> Self {
        Self::new()
    }
}

impl ScriptedPipeline {
    /// A pipeline double on a new simulated clock at zero.
    pub fn new() -> Self {
        let clock = SimClock::new();
        let recorder = LatencyRecorder::new(Arc::new(clock.clone()));
        Self { clock, recorder }
    }

    /// The recorder with the scripted trials.
    pub fn recorder(&self) -> &LatencyRecorder {
        &self.recorder
    }

    /// The simulated clock.
    pub fn clock(&self) -> &SimClock {
        &self.clock
    }

    /// Run one press now and record its marks in time order.
    pub fn press(
        &self,
        path: SttPath,
        route: Route,
        t: ScriptedTiming,
    ) -> Result<TrialId, MarkError> {
        let id = self.recorder.begin(path, route);
        let start = self.clock.now();
        let mut steps = vec![
            (Duration::ZERO, Mark::KeyDown),
            (t.mic_live, Mark::MicLive),
            (t.hold, Mark::KeyUp),
            (t.hold + t.ack, Mark::LocalAck),
        ];
        if let Some(c) = t.chip {
            steps.push((t.hold + c, Mark::PreviewChip));
        }
        if let Some(i) = t.insert {
            steps.push((t.hold + i, Mark::TextInserted));
        }
        steps.sort();
        for (offset, mark) in steps {
            let now = self.clock.now();
            self.clock.advance((start + offset).saturating_sub(now));
            self.recorder.mark(id, mark)?;
        }
        Ok(id)
    }
}

/// A small deterministic spread around `base` (no randomness, so a run is repeatable).
fn spread(base: u64, i: u64, width: u64) -> Duration {
    Duration::from_millis(base + (i * 37 + 11) % width)
}

/// A demo data set from the double: `n` presses on each of local question, cloud question and
/// local dictation, inside the section 8.1 design budget. For `latency_bench demo` and tests.
pub fn demo_pipeline(n: u64) -> ScriptedPipeline {
    let p = ScriptedPipeline::new();
    for i in 0..n {
        let base = ScriptedTiming {
            mic_live: spread(30, i, 60),
            hold: spread(800, i, 1_500),
            ack: spread(20, i, 50),
            chip: Some(spread(600, i, 700)),
            insert: None,
        };
        let cloud = ScriptedTiming {
            chip: Some(spread(1_100, i, 800)),
            ..base
        };
        let dictation = ScriptedTiming {
            chip: None,
            insert: Some(spread(650, i, 700)),
            ..base
        };
        // The double never breaks the mark order, so these cannot fail.
        for (path, route, t) in [
            (SttPath::Local, Route::Question, base),
            (SttPath::Cloud, Route::Question, cloud),
            (SttPath::Local, Route::Dictation, dictation),
        ] {
            if p.press(path, route, t).is_ok() {
                p.clock.advance(Duration::from_millis(500));
            }
        }
    }
    p
}

/// A demo HID rig run from the double: `cycles` presses, every key-up seen, 1-10 ms delivery.
pub fn demo_hotkey(cycles: u32) -> HotkeySamples {
    let mut hk = HotkeySamples::default();
    for i in 0..u64::from(cycles) {
        hk.record_cycle(true, &[spread(1, i, 9), spread(1, i + 3, 9)]);
    }
    hk
}
