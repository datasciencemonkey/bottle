//! The mic level stream of R2 (CAP-16a, CAP-16b, #23, ADR 0067).
//!
//! [`LevelSink`] is a [`FrameSink`]: it reads the mic frames on the audio thread and sends
//! one RMS level in dBFS per window of [`window_samples`] samples, so at 30 Hz or less
//! (CAP-16a). At most one level goes out per data callback, so a large block never gives a
//! burst. The level goes to a [`LevelOut`], the `onLevel` Channel of `capture_subscribe`.
//! Only a number crosses IPC, never a sample (CAP-01b).
//!
//! [`LevelGate`] opens and closes the stream. [`LevelCaptureEvents`] wraps the
//! [`CaptureEvents`] of R2: it opens the gate while a capture records and closes it when the
//! capture stops or pauses (CAP-16b). A send that started before [`LevelGate::close`] ends
//! before `close` returns, so no level goes out after the stop. The audio thread never waits
//! for the gate: when `close` holds it, that level is dropped.
//!
//! The Channel has one receiver, the brain. The brain sends each level only to the target MOG
//! (`src/brain/level-stream.ts`), so other MOGs stay at level 0.
//!
//! The host (`src-tauri/src/capture_host.rs`, #476) wraps the `capture_subscribe` `onLevel`
//! [`ChannelRef`] in a [`LevelOut`] and puts it in the gate with [`LevelGate::set_out`]. It
//! builds the [`CaptureCore`] with [`LevelCaptureEvents`], and gives the [`MicDevice`] a
//! [`TeeSink`] of the [`LevelSink`] and the PTT buffer (TODO(#21)).
//!
//! TODO(#475): a level frame carries no capture ID. So a level that the brain reads after the
//! next capture's `capture.state` can reach the next target MOG. The contract change binds
//! each frame to its capture.
//!
//! Handy (`src-tauri/src/audio_toolkit/audio/visualizer.rs` @ aca8985) draws an FFT spectrum
//! for its overlay, not an RMS level, so no Handy code is used here.
//!
//! [`ChannelRef`]: crate::ipc::ChannelRef
//! [`CaptureCore`]: crate::capture::CaptureCore
//! [`MicDevice`]: super::MicDevice

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, TryLockError};

use super::sink::{FrameSink, MicFrame, SinkEvent};
use crate::capture::{CaptureEvents, Stopped};
use crate::types::CaptureState;

/// The highest level rate: 30 Hz (CAP-16a, Table UX-LEVEL).
pub const LEVEL_MAX_HZ: u32 = 30;

/// The level of silence. JSON has no `-Infinity`, so silence is this finite floor. The level
/// meter shows 0 for any value under -60 dBFS (Table UX-LEVEL).
pub const LEVEL_FLOOR_DBFS: f32 = -100.0;

/// The RMS level of `samples` in dBFS: `20 * log10(rms)`, with a full-scale square wave at
/// 0 dBFS. Silence, an empty block and a level under the floor give [`LEVEL_FLOOR_DBFS`].
/// Non-finite samples count as 0.
pub fn rms_dbfs(samples: &[f32]) -> f32 {
    let (sum, n) = samples
        .iter()
        .fold((0.0f64, 0usize), |(sum, n), s| (sum + square(*s), n + 1));
    dbfs_of(sum, n)
}

/// The number of samples in one level window at `sample_rate`: the rate over 30, rounded up,
/// so the level rate is 30 Hz or less. A rate of 0 gives 1.
pub fn window_samples(sample_rate: u32) -> usize {
    (sample_rate.div_ceil(LEVEL_MAX_HZ) as usize).max(1)
}

fn square(s: f32) -> f64 {
    if s.is_finite() {
        f64::from(s) * f64::from(s)
    } else {
        0.0
    }
}

fn dbfs_of(sum_sq: f64, n: usize) -> f32 {
    if n == 0 || sum_sq <= 0.0 {
        return LEVEL_FLOOR_DBFS;
    }
    let db = 10.0 * (sum_sq / n as f64).log10();
    (db as f32).max(LEVEL_FLOOR_DBFS)
}

/// The receiver of levels: the `onLevel` Channel of `capture_subscribe`. It runs on the
/// audio thread, so it must be fast, must not block for long and must not panic.
pub trait LevelOut: Send + Sync + 'static {
    /// One RMS level in dBFS, [`LEVEL_FLOOR_DBFS`] or more.
    fn level(&self, dbfs: f32);
}

/// A [`LevelOut`] that drops every level: the receiver before the first `capture_subscribe`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLevelOut;

impl LevelOut for NoLevelOut {
    fn level(&self, _dbfs: f32) {}
}

struct GateShared {
    /// The send lock. `close` takes it, so it waits for a send in flight.
    send: Mutex<Arc<dyn LevelOut>>,
    open: AtomicBool,
    /// Counts each `open`, so the sink starts a fresh window for a new capture.
    epoch: AtomicU64,
}

/// The switch of the level stream (CAP-16b). Clones share one switch.
#[derive(Clone)]
pub struct LevelGate {
    shared: Arc<GateShared>,
}

impl LevelGate {
    /// A closed gate in front of `out`.
    pub fn new(out: Arc<dyn LevelOut>) -> Self {
        Self {
            shared: Arc::new(GateShared {
                send: Mutex::new(out),
                open: AtomicBool::new(false),
                epoch: AtomicU64::new(0),
            }),
        }
    }

    /// Start the stream. Opening an open gate does nothing.
    pub fn open(&self) {
        let _guard = self.lock();
        if !self.shared.open.load(Ordering::SeqCst) {
            self.shared.epoch.fetch_add(1, Ordering::SeqCst);
            self.shared.open.store(true, Ordering::SeqCst);
        }
    }

    /// Stop the stream. It waits for a send in flight, so no level goes out after it returns.
    pub fn close(&self) {
        let _guard = self.lock();
        self.shared.open.store(false, Ordering::SeqCst);
    }

    /// Send the levels to `out` from now on, for example the `onLevel` Channel of a new
    /// `capture_subscribe`. It waits for a send in flight, so no level goes to the old
    /// receiver after it returns. It does not open or close the stream.
    pub fn set_out(&self, out: Arc<dyn LevelOut>) {
        *self.lock() = out;
    }

    /// The stream is open.
    pub fn is_open(&self) -> bool {
        self.shared.open.load(Ordering::SeqCst)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Arc<dyn LevelOut>> {
        self.shared
            .send
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The epoch of the open stream, or `None` when closed. Lock-free, for the audio thread.
    fn open_epoch(&self) -> Option<u64> {
        self.is_open()
            .then(|| self.shared.epoch.load(Ordering::SeqCst))
    }

    /// Send one level if the stream of `epoch` is still open. It never waits: when `open` or
    /// `close` holds the gate, the level is dropped.
    fn try_send(&self, epoch: u64, dbfs: f32) {
        let out = match self.shared.send.try_lock() {
            Ok(out) => out,
            Err(TryLockError::Poisoned(p)) => p.into_inner(),
            Err(TryLockError::WouldBlock) => return,
        };
        if self.shared.open.load(Ordering::SeqCst)
            && self.shared.epoch.load(Ordering::SeqCst) == epoch
        {
            out.level(dbfs);
        }
    }
}

impl std::fmt::Debug for LevelGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LevelGate")
            .field("open", &self.is_open())
            .finish()
    }
}

/// The [`FrameSink`] that makes the level stream (CAP-16a).
#[derive(Debug)]
pub struct LevelSink {
    gate: LevelGate,
    epoch: Option<u64>,
    sample_rate: u32,
    window: usize,
    sum_sq: f64,
    n: usize,
}

impl LevelSink {
    /// A sink that sends through `gate`.
    pub fn new(gate: LevelGate) -> Self {
        Self {
            gate,
            epoch: None,
            sample_rate: 0,
            window: 1,
            sum_sq: 0.0,
            n: 0,
        }
    }

    fn reset(&mut self) {
        self.sum_sq = 0.0;
        self.n = 0;
    }
}

impl FrameSink for LevelSink {
    fn frame(&mut self, frame: MicFrame<'_>) {
        let Some(epoch) = self.gate.open_epoch() else {
            self.epoch = None;
            self.reset();
            return;
        };
        if self.epoch != Some(epoch) || self.sample_rate != frame.sample_rate {
            self.epoch = Some(epoch);
            self.sample_rate = frame.sample_rate;
            self.window = window_samples(frame.sample_rate);
            self.reset();
        }
        let mut last = None;
        for s in frame.samples {
            self.sum_sq += square(*s);
            self.n += 1;
            if self.n >= self.window {
                last = Some(dbfs_of(self.sum_sq, self.n));
                self.reset();
            }
        }
        if let Some(dbfs) = last {
            self.gate.try_send(epoch, dbfs);
        }
    }

    fn event(&mut self, _event: SinkEvent) {
        // A gap or a discard: the next level holds only audio after the marker.
        self.reset();
    }
}

/// Feeds every frame and event to two sinks, `.0` first. For the mic: the level sink and the
/// PTT buffer (TODO(#21)) read one stream.
#[derive(Debug, Default)]
pub struct TeeSink<A, B>(pub A, pub B);

impl<A: FrameSink, B: FrameSink> FrameSink for TeeSink<A, B> {
    fn frame(&mut self, frame: MicFrame<'_>) {
        self.0.frame(frame);
        self.1.frame(frame);
    }

    fn event(&mut self, event: SinkEvent) {
        self.0.event(event);
        self.1.event(event);
    }
}

/// Wraps the [`CaptureEvents`] of R2 and drives the [`LevelGate`] from `capture.state`
/// (CAP-16b): open while a capture records and is not paused, closed otherwise. The gate
/// closes before the inner sink sees the state, so the brain gets no level after the stop
/// state.
#[derive(Debug)]
pub struct LevelCaptureEvents<E> {
    inner: E,
    gate: LevelGate,
}

impl<E: CaptureEvents> LevelCaptureEvents<E> {
    /// Wrap `inner`.
    pub fn new(inner: E, gate: LevelGate) -> Self {
        Self { inner, gate }
    }

    /// The wrapped sink.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

/// A capture that streams the level: it records, is not paused and has a MOG.
pub fn streams_level(state: &CaptureState) -> bool {
    state.recording && !state.paused && (state.owner.is_some() || state.directed_to.is_some())
}

impl<E: CaptureEvents> CaptureEvents for LevelCaptureEvents<E> {
    fn state(&self, state: &CaptureState) {
        if streams_level(state) {
            self.gate.open();
        } else {
            self.gate.close();
        }
        self.inner.state(state);
    }

    fn stopped(&self, stopped: &Stopped) {
        self.inner.stopped(stopped);
    }
}
