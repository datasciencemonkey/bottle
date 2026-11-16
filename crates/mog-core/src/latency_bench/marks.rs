//! The mark API (#171): one trial per PTT press, one time per pipeline step.

use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::clock::MonotonicClock;

/// A pipeline step that the benchmark times. It has no payload (SEC-16b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mark {
    /// The talk key went down (HotkeyService).
    KeyDown,
    /// The mic delivers frames (CaptureService, NFR-02).
    MicLive,
    /// The talk key went up (HotkeyService).
    KeyUp,
    /// Mood `processing` and the pill "Transcribing" show (NFR-01c).
    LocalAck,
    /// The preview chip shows (NFR-01a, NFR-01b).
    PreviewChip,
    /// The dictated text is in the active app (NFR-01d).
    TextInserted,
}

impl Mark {
    /// Every mark, in pipeline order.
    pub const ALL: [Mark; 6] = [
        Mark::KeyDown,
        Mark::MicLive,
        Mark::KeyUp,
        Mark::LocalAck,
        Mark::PreviewChip,
        Mark::TextInserted,
    ];

    /// The event name in the sample file and the report.
    pub fn name(self) -> &'static str {
        match self {
            Mark::KeyDown => "key_down",
            Mark::MicLive => "mic_live",
            Mark::KeyUp => "key_up",
            Mark::LocalAck => "local_ack",
            Mark::PreviewChip => "preview_chip",
            Mark::TextInserted => "text_inserted",
        }
    }

    /// The mark with this event name.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The mark that must come first in the same trial.
    fn needs(self) -> Option<Mark> {
        match self {
            Mark::KeyDown => None,
            Mark::MicLive | Mark::KeyUp => Some(Mark::KeyDown),
            Mark::LocalAck | Mark::PreviewChip | Mark::TextInserted => Some(Mark::KeyUp),
        }
    }
}

/// The STT path of a trial (section 8.1 columns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SttPath {
    /// Local Parakeet (the default, U5).
    Local,
    /// Databricks ASR (opt-in after the probe).
    Cloud,
}

impl SttPath {
    /// `local` or `cloud`.
    pub fn name(self) -> &'static str {
        match self {
            SttPath::Local => "local",
            SttPath::Cloud => "cloud",
        }
    }

    /// The path with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [SttPath::Local, SttPath::Cloud]
            .into_iter()
            .find(|p| p.name() == name)
    }
}

/// The route of a trial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Route {
    /// UC1 dictation into the active app.
    Dictation,
    /// UC2 question with the preview chip.
    Question,
}

impl Route {
    /// `dictation` or `question`.
    pub fn name(self) -> &'static str {
        match self {
            Route::Dictation => "dictation",
            Route::Question => "question",
        }
    }

    /// The route with this name.
    pub fn parse(name: &str) -> Option<Self> {
        [Route::Dictation, Route::Question]
            .into_iter()
            .find(|r| r.name() == name)
    }
}

/// The id of a trial in one [`LatencyRecorder`]. Ids start at 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TrialId(u64);

impl TrialId {
    /// The number of the trial.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A mark that the recorder cannot accept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarkError {
    /// The recorder has no trial with this id.
    UnknownTrial(TrialId),
    /// The trial already has this mark.
    Duplicate(Mark),
    /// The trial does not have the mark that must come first.
    MissingPrior {
        /// The mark that was given.
        mark: Mark,
        /// The mark that must come first.
        needs: Mark,
    },
    /// The time is before the time of the mark that must come first.
    OutOfOrder {
        /// The mark that was given.
        mark: Mark,
        /// The mark that must come first.
        after: Mark,
    },
}

impl fmt::Display for MarkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MarkError::UnknownTrial(id) => write!(f, "no trial {}", id.get()),
            MarkError::Duplicate(m) => write!(f, "the trial already has {}", m.name()),
            MarkError::MissingPrior { mark, needs } => {
                write!(f, "{} needs {} first", mark.name(), needs.name())
            }
            MarkError::OutOfOrder { mark, after } => {
                write!(f, "{} is before {}", mark.name(), after.name())
            }
        }
    }
}

impl std::error::Error for MarkError {}

/// One PTT press: its STT path, its route and the time of each mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trial {
    id: TrialId,
    path: SttPath,
    route: Route,
    marks: Vec<(Mark, Duration)>,
}

impl Trial {
    /// A trial with no marks (for importers).
    pub fn new(id: u64, path: SttPath, route: Route) -> Self {
        Self {
            id: TrialId(id),
            path,
            route,
            marks: Vec::new(),
        }
    }

    /// The trial id.
    pub fn id(&self) -> TrialId {
        self.id
    }

    /// The STT path.
    pub fn path(&self) -> SttPath {
        self.path
    }

    /// The route.
    pub fn route(&self) -> Route {
        self.route
    }

    /// Every mark and its time, in the order recorded.
    pub fn marks(&self) -> &[(Mark, Duration)] {
        &self.marks
    }

    /// The time of `mark`, if recorded.
    pub fn at(&self, mark: Mark) -> Option<Duration> {
        self.marks.iter().find(|(m, _)| *m == mark).map(|(_, t)| *t)
    }

    /// The time from `from` to `to`, if both are recorded and `to` is not before `from`.
    pub fn span(&self, from: Mark, to: Mark) -> Option<Duration> {
        self.at(to)?.checked_sub(self.at(from)?)
    }

    /// Record `mark` at `at`. It checks duplicates and the order of the pipeline.
    pub fn record(&mut self, mark: Mark, at: Duration) -> Result<(), MarkError> {
        if self.at(mark).is_some() {
            return Err(MarkError::Duplicate(mark));
        }
        if let Some(prior) = mark.needs() {
            match self.at(prior) {
                None => return Err(MarkError::MissingPrior { mark, needs: prior }),
                Some(t) if at < t => return Err(MarkError::OutOfOrder { mark, after: prior }),
                Some(_) => {}
            }
        }
        self.marks.push((mark, at));
        Ok(())
    }
}

/// Records the marks of many trials on one monotonic clock. It is safe to share between threads.
#[derive(Debug)]
pub struct LatencyRecorder {
    clock: Arc<dyn MonotonicClock>,
    trials: Mutex<Vec<Trial>>,
}

impl LatencyRecorder {
    /// A recorder on `clock` (`SystemClock` in the app, `SimClock` in tests).
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            clock,
            trials: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Trial>> {
        self.trials.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Start a trial for one PTT press.
    pub fn begin(&self, path: SttPath, route: Route) -> TrialId {
        let mut trials = self.lock();
        let id = trials.len() as u64 + 1;
        trials.push(Trial::new(id, path, route));
        TrialId(id)
    }

    /// Record `mark` now, on the recorder clock.
    pub fn mark(&self, trial: TrialId, mark: Mark) -> Result<(), MarkError> {
        let now = self.clock.now();
        self.mark_at(trial, mark, now)
    }

    /// Record `mark` at `at` (a time on the recorder clock, for example an OS event time).
    pub fn mark_at(&self, trial: TrialId, mark: Mark, at: Duration) -> Result<(), MarkError> {
        let mut trials = self.lock();
        let index = usize::try_from(trial.0)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .filter(|i| *i < trials.len())
            .ok_or(MarkError::UnknownTrial(trial))?;
        trials[index].record(mark, at)
    }

    /// A copy of every trial.
    pub fn trials(&self) -> Vec<Trial> {
        self.lock().clone()
    }

    /// The trials as a sample file (`import::parse_csv` reads it).
    pub fn to_csv(&self) -> String {
        super::import::to_csv(&self.trials())
    }
}

/// Talk-key samples from the HID rig (NFR-08a, NFR-08b).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HotkeySamples {
    /// Key-down presses.
    pub cycles: u32,
    /// Presses whose key-up HotkeyService saw.
    pub key_ups_seen: u32,
    /// The time from the OS event to HotkeyService, for each key event.
    pub event_latencies: Vec<Duration>,
}

impl HotkeySamples {
    /// Add one press cycle and the delivery time of its events.
    pub fn record_cycle(&mut self, key_up_seen: bool, latencies: &[Duration]) {
        self.cycles = self.cycles.saturating_add(1);
        if key_up_seen {
            self.key_ups_seen = self.key_ups_seen.saturating_add(1);
        }
        self.event_latencies.extend_from_slice(latencies);
    }
}
