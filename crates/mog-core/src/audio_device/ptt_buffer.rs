//! The RAM-only buffer of PTT and hands-free audio (CAP-12c, CAP-13a, CAP-13b, CAP-34, #21,
//! ADR 0078).
//!
//! The mic frames reach this buffer through the [`TeeSink`](super::level::TeeSink) of the host,
//! after the level sink. The buffer keeps the audio of one PTT or hands-free lease in RAM and
//! gives it to an [`UtteranceSink`] when the lease stops. The consumer is the speech engine
//! R4 SttEngine (`crate::stt_engine`, #46, ADR 0082), which decodes the same audio from the
//! mic tee during speech and drops the utterance at once.
//!
//! - **RAM only (CAP-12c).** The audio is in heap chunks of [`CHUNK_SAMPLES`] samples. There is
//!   no file, temp file, memory map, log or telemetry. A full chunk is never moved, so a growing
//!   take leaves no copy in freed memory. TODO(#500): the pages are not locked, so the OS can
//!   page them to swap or the Windows page file.
//! - **No pre-roll (CAP-13a).** A take starts only when `capture.state` shows its lease
//!   recording. A frame before that is dropped, and the sink keeps no ring buffer. So the first
//!   data callback after the mic opens (about 10 ms) can be lost. The mic is
//!   closed before a lease starts ([`CaptureCore`](crate::capture::CaptureCore), MOG-09).
//! - **Pause (CAP-13b).** A paused state takes no frame. R2 closes the mic stream on pause.
//! - **Caps.** A PTT take holds at most [`HOLD_MAX_MS`] of audio (CAP-04a) and a hands-free take
//!   at most [`HANDS_FREE_MAX_MS`] (CAP-04b). Later frames are dropped.
//! - **Stop.** A stop reason that drops audio ([`StopReason::discards_audio`]: a PTT tap or cancel,
//!   Esc or zero buffers) wipes the take. A tap that stops hands-free keeps the audio, and so
//!   does the panic stop (ADR 0078, decision 7). [`SinkEvent::Discard`] (CAP-17b) wipes every take at once. Any
//!   other stop makes one contiguous [`Utterance`], sets its peak to -3 dBFS with
//!   [`normalize_peak`] (CAP-20, ADR 0067), wipes the chunks and gives the utterance to the
//!   [`UtteranceSink`].
//! - **Zeroed after STT (CAP-34, section 9.2).** An [`Utterance`] has no `Clone`, and its `Drop`
//!   overwrites every sample with 0 before the memory is freed. Its `Debug` shows no sample.
//!   So the app keeps no copy of audio after the consumer drops the utterance.
//! - **Sessions** are not buffered here. The session journal owns their audio (TODO(#65),
//!   TODO(#116)). A directed PTT in a session uses the session stream, so it is also not
//!   buffered here (TODO(#116)).
//!
//! A frame at another sample rate than the first frame of the take (after a device change,
//! CAP-18) is resampled to the take's rate by linear interpolation. R4 resamples its own tee
//! input to 16 kHz (#46).
//!
//! The audio thread takes the buffer lock for each frame. The other side holds the lock only for
//! short steps: it moves a take out, and then copies, scales and wipes it after it lets go.
//!
//! Handy keeps its recording in a plain `Vec<f32>` with a VAD pre-fill and no zeroing
//! (`src-tauri/src/managers/audio.rs` @ aca8985), so no Handy code is used here.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{compiler_fence, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use super::normalize::normalize_peak;
use super::sink::{FrameSink, MicFrame, SinkEvent};
use crate::capture::{
    parse_lease, CaptureEvents, Lease, LeaseNo, StopReason, Stopped, HANDS_FREE_MAX_MS, HOLD_MAX_MS,
};
use crate::types::{CaptureMode, CaptureState, MogId};

/// The samples in one heap chunk of a take.
pub const CHUNK_SAMPLES: usize = 4_096;

/// The most takes the buffer holds at once: a hands-free take and a PTT take over it (ARC-F9).
pub const MAX_TAKES: usize = 2;

/// How many stopped leases the buffer remembers, so a late `capture.state` cannot open a new
/// take for a lease that already stopped.
const ENDED_MEMORY: usize = 16;

/// The audio cap of a take in ms: 120 s for PTT (CAP-04a), 5 min for hands-free (CAP-04b).
/// `None` for a session, which this buffer does not hold.
pub fn cap_ms(mode: CaptureMode) -> Option<u64> {
    match mode {
        CaptureMode::Ptt => Some(HOLD_MAX_MS),
        CaptureMode::HandsFree => Some(HANDS_FREE_MAX_MS),
        CaptureMode::Session => None,
    }
}

/// The audio cap of a take in samples at `sample_rate`. `None` for a session.
pub fn cap_samples(mode: CaptureMode, sample_rate: u32) -> Option<usize> {
    cap_ms(mode)
        .map(|ms| usize::try_from(u64::from(sample_rate) * ms / 1_000).unwrap_or(usize::MAX))
}

/// Overwrite `samples` with 0 in a way that the optimiser keeps.
pub fn wipe(samples: &mut [f32]) {
    for s in samples.iter_mut() {
        // SAFETY: `s` is a valid, aligned `&mut f32`.
        unsafe { std::ptr::write_volatile(s, 0.0) };
    }
    compiler_fence(Ordering::SeqCst);
}

/// One heap chunk of audio. It is wiped before it is freed. TODO(#500): lock its pages.
struct Chunk(Box<[f32]>);

impl Chunk {
    fn new() -> Self {
        Self(vec![0.0; CHUNK_SAMPLES].into_boxed_slice())
    }
}

impl Drop for Chunk {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// The audio of one PTT or hands-free lease.
struct Take {
    lease: LeaseNo,
    mog_id: MogId,
    mode: CaptureMode,
    /// The rate of the first frame. 0 before it.
    sample_rate: u32,
    /// The cap in samples at `sample_rate`. 0 before the first frame.
    cap: usize,
    chunks: Vec<Chunk>,
    len: usize,
    /// The audio was dropped (CAP-17b). The take takes no more frames and makes no utterance.
    discarded: bool,
}

impl Take {
    fn new(lease: LeaseNo, mog_id: MogId, mode: CaptureMode) -> Self {
        Self {
            lease,
            mog_id,
            mode,
            sample_rate: 0,
            cap: 0,
            chunks: Vec::new(),
            len: 0,
            discarded: false,
        }
    }

    fn full(&self) -> bool {
        self.sample_rate != 0 && self.len >= self.cap
    }

    /// Add one sample. A non-finite sample becomes 0.
    fn push_sample(&mut self, s: f32) {
        let at = self.len % CHUNK_SAMPLES;
        if at == 0 {
            self.chunks.push(Chunk::new());
        }
        if let Some(chunk) = self.chunks.last_mut() {
            if let Some(slot) = chunk.0.get_mut(at) {
                *slot = if s.is_finite() { s } else { 0.0 };
                self.len += 1;
            }
        }
    }

    fn push(&mut self, frame: MicFrame<'_>) {
        if self.discarded || frame.samples.is_empty() || frame.sample_rate == 0 {
            return;
        }
        if self.sample_rate == 0 {
            self.sample_rate = frame.sample_rate;
            self.cap = cap_samples(self.mode, frame.sample_rate).unwrap_or(0);
        }
        if frame.sample_rate == self.sample_rate {
            for s in frame.samples {
                if self.full() {
                    return;
                }
                self.push_sample(*s);
            }
            return;
        }
        // CAP-18: a new device can have another rate. Linear interpolation to the take's rate.
        let n = frame.samples.len();
        let ratio = f64::from(frame.sample_rate) / f64::from(self.sample_rate);
        let out_len = ((n as f64) / ratio).round() as usize;
        for i in 0..out_len {
            if self.full() {
                return;
            }
            let pos = i as f64 * ratio;
            let idx = (pos.floor() as usize).min(n - 1);
            let frac = (pos - idx as f64) as f32;
            let a = frame.samples[idx];
            let b = frame.samples[(idx + 1).min(n - 1)];
            self.push_sample(a + (b - a) * frac);
        }
    }

    /// Drop the audio now (CAP-17b): the chunks are wiped and freed.
    fn discard(&mut self) {
        self.discarded = true;
        self.chunks.clear();
        self.len = 0;
    }

    /// One contiguous utterance. The chunks are wiped when the take drops.
    fn into_utterance(self, reason: StopReason) -> Utterance {
        let mut samples = Vec::with_capacity(self.len);
        let mut left = self.len;
        for chunk in &self.chunks {
            let n = left.min(CHUNK_SAMPLES);
            samples.extend_from_slice(&chunk.0[..n]);
            left -= n;
        }
        LIVE_UTTERANCES.fetch_add(1, Ordering::SeqCst);
        Utterance {
            lease_id: self.lease,
            mog_id: self.mog_id.clone(),
            mode: self.mode,
            reason,
            sample_rate: self.sample_rate,
            samples,
        }
    }
}

/// The [`Utterance`] values that exist now in this process. A content-free count, so a test can
/// prove that the consumer keeps no copy of the audio (CAP-34, #46).
static LIVE_UTTERANCES: AtomicUsize = AtomicUsize::new(0);

/// The number of [`Utterance`] values that exist now. No audio is read.
pub fn live_utterances() -> usize {
    LIVE_UTTERANCES.load(Ordering::SeqCst)
}

/// The audio of one stopped PTT or hands-free lease, in RAM, ready for STT. Its peak is
/// -3 dBFS (CAP-20). It cannot be cloned, its `Debug` shows no sample, and its `Drop` wipes
/// every sample (CAP-34).
pub struct Utterance {
    lease_id: LeaseNo,
    mog_id: MogId,
    mode: CaptureMode,
    reason: StopReason,
    sample_rate: u32,
    samples: Vec<f32>,
}

impl Utterance {
    /// The mono samples in -1.0..=1.0.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }

    /// The sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The lease of the capture.
    pub fn lease_id(&self) -> LeaseNo {
        self.lease_id
    }

    /// The MOG that held the lease.
    pub fn mog_id(&self) -> &str {
        &self.mog_id
    }

    /// The capture mode: PTT or hands-free.
    pub fn mode(&self) -> CaptureMode {
        self.mode
    }

    /// Why the lease stopped.
    pub fn reason(&self) -> StopReason {
        self.reason
    }

    /// The length of the audio in ms.
    pub fn duration_ms(&self) -> u64 {
        if self.sample_rate == 0 {
            return 0;
        }
        self.samples.len() as u64 * 1_000 / u64::from(self.sample_rate)
    }

    /// Overwrite every sample with 0 now. `Drop` does this too.
    pub fn zeroize(&mut self) {
        wipe(&mut self.samples);
    }
}

impl Drop for Utterance {
    fn drop(&mut self) {
        self.zeroize();
        LIVE_UTTERANCES.fetch_sub(1, Ordering::SeqCst);
    }
}

impl fmt::Debug for Utterance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // No sample in a log (CAP-01b).
        f.debug_struct("Utterance")
            .field("lease_id", &self.lease_id)
            .field("mode", &self.mode)
            .field("reason", &self.reason)
            .field("sample_rate", &self.sample_rate)
            .field("len", &self.samples.len())
            .finish_non_exhaustive()
    }
}

/// The consumer of finished utterances: R4 SttEngine (#46). It runs on the thread that
/// stopped the lease (an IPC worker or the cap timer), so it must hand the utterance on fast.
/// When it drops the utterance, the samples are wiped.
pub trait UtteranceSink: Send + Sync + 'static {
    /// One finished utterance.
    fn utterance(&self, utterance: Utterance);
}

/// An [`UtteranceSink`] that drops each utterance, so its samples are wiped at once. The host
/// used it before SttEngine existed (#46); tests and `with_levels` still use it.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoUtteranceSink;

impl UtteranceSink for NoUtteranceSink {
    fn utterance(&self, _utterance: Utterance) {}
}

#[derive(Default)]
struct State {
    /// The lease that takes frames now.
    current: Option<LeaseNo>,
    takes: Vec<Take>,
    ended: VecDeque<LeaseNo>,
    /// The PTT overlay lease on a session lease (ADR 0118). `capture.state` shows the session
    /// lease while it lives, so the internal `overlay_granted` hook sets this.
    overlay: Option<LeaseNo>,
}

struct Shared {
    state: Mutex<State>,
    out: Arc<dyn UtteranceSink>,
}

/// The RAM-only PTT and hands-free buffer. Clones share one buffer. The mic gets its
/// [`PttBuffer::sink`], and R2 gets a [`PttCaptureEvents`] over it.
#[derive(Clone)]
pub struct PttBuffer {
    shared: Arc<Shared>,
}

impl PttBuffer {
    /// An empty buffer that gives each finished utterance to `out`.
    pub fn new(out: Arc<dyn UtteranceSink>) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::default(),
                out,
            }),
        }
    }

    /// The [`FrameSink`] for the mic.
    pub fn sink(&self) -> PttBufferSink {
        PttBufferSink {
            shared: Arc::clone(&self.shared),
        }
    }

    /// The samples that the buffer holds now, over all takes.
    pub fn held_samples(&self) -> usize {
        self.lock().takes.iter().map(|t| t.len).sum()
    }

    /// The number of takes that the buffer holds now.
    pub fn takes(&self) -> usize {
        self.lock().takes.len()
    }

    /// A lease takes frames now.
    pub fn is_taking(&self) -> bool {
        self.lock().current.is_some()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        lock(&self.shared)
    }

    /// Follow one `capture.state`: a PTT or hands-free lease that records and is not paused
    /// takes the frames. Any other state takes none.
    pub fn on_state(&self, state: &CaptureState) {
        {
            // A directed PTT in a session (ADR 0118): the state shows the session lease, and
            // the overlay lease keeps taking frames.
            let mut st = self.lock();
            if let Some(o) = st.overlay {
                if state.mode == Some(CaptureMode::Session)
                    && state.directed_to.is_some()
                    && state.recording
                    && !st.ended.contains(&o)
                {
                    st.current = Some(o);
                    return;
                }
                st.overlay = None;
            }
        }
        let lease = state.lease_id.as_deref().and_then(|w| parse_lease(w).ok());
        let mode = state
            .mode
            .filter(|m| matches!(m, CaptureMode::Ptt | CaptureMode::HandsFree));
        let mut evicted = None;
        {
            let mut st = self.lock();
            match (lease, mode) {
                (Some(lease), Some(mode))
                    if state.recording && !state.paused && !st.ended.contains(&lease) =>
                {
                    if !st.takes.iter().any(|t| t.lease == lease) {
                        if st.takes.len() >= MAX_TAKES {
                            evicted = Some(st.takes.remove(0));
                        }
                        let owner = state.owner.clone().unwrap_or_default();
                        st.takes.push(Take::new(lease, owner, mode));
                    }
                    st.current = Some(lease);
                }
                _ => st.current = None,
            }
        }
        // The wipe of a take runs after the lock is released.
        drop(evicted);
    }

    /// Follow an overlay PTT granted on a session lease: it takes a take of its own, and its
    /// segments carry its own lease ID (ADR 0118).
    pub fn on_overlay(&self, lease: &Lease) {
        let mut evicted = None;
        {
            let mut st = self.lock();
            if st.ended.contains(&lease.id) {
                return;
            }
            if !st.takes.iter().any(|t| t.lease == lease.id) {
                if st.takes.len() >= MAX_TAKES {
                    evicted = Some(st.takes.remove(0));
                }
                st.takes
                    .push(Take::new(lease.id, lease.mog_id.clone(), CaptureMode::Ptt));
            }
            st.overlay = Some(lease.id);
            st.current = Some(lease.id);
        }
        drop(evicted);
    }

    /// Follow one stop: drop the audio, or hand the utterance to the sink.
    pub fn on_stopped(&self, stopped: &Stopped) {
        let take = {
            let mut st = self.lock();
            if st.current == Some(stopped.lease_id) {
                st.current = None;
            }
            if st.overlay == Some(stopped.lease_id) {
                st.overlay = None;
            }
            if !st.ended.contains(&stopped.lease_id) {
                if st.ended.len() >= ENDED_MEMORY {
                    st.ended.pop_front();
                }
                st.ended.push_back(stopped.lease_id);
            }
            let at = st.takes.iter().position(|t| t.lease == stopped.lease_id);
            at.map(|at| st.takes.remove(at))
        };
        let Some(take) = take else {
            return;
        };
        if stopped.reason.discards_audio() || take.discarded || take.len == 0 {
            // CAP-44b, CAP-14b, CAP-17b: the take drops here and its chunks are wiped.
            return;
        }
        let mut utterance = take.into_utterance(stopped.reason);
        normalize_peak(&mut utterance.samples);
        self.shared.out.utterance(utterance);
    }
}

impl fmt::Debug for PttBuffer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PttBuffer")
            .field("takes", &self.takes())
            .finish_non_exhaustive()
    }
}

fn lock(shared: &Shared) -> MutexGuard<'_, State> {
    shared.state.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The mic side of a [`PttBuffer`]. It runs on the audio thread and never panics.
pub struct PttBufferSink {
    shared: Arc<Shared>,
}

impl FrameSink for PttBufferSink {
    fn frame(&mut self, frame: MicFrame<'_>) {
        let mut st = lock(&self.shared);
        let Some(current) = st.current else {
            // No capture records: the frame is dropped (CAP-13a).
            return;
        };
        if let Some(take) = st.takes.iter_mut().find(|t| t.lease == current) {
            take.push(frame);
        }
    }

    fn event(&mut self, event: SinkEvent) {
        if let SinkEvent::Discard = event {
            // CAP-17b: zero buffers. Every mic lease stops, and no transcript is made.
            for take in lock(&self.shared).takes.iter_mut() {
                take.discard();
            }
        }
    }
}

impl fmt::Debug for PttBufferSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PttBufferSink").finish_non_exhaustive()
    }
}

/// Wraps the [`CaptureEvents`] of R2 and drives a [`PttBuffer`]: each `capture.state` and each
/// stop goes to the buffer first, then to the inner sink.
#[derive(Debug)]
pub struct PttCaptureEvents<E> {
    inner: E,
    buffer: PttBuffer,
}

impl<E: CaptureEvents> PttCaptureEvents<E> {
    /// Wrap `inner`.
    pub fn new(inner: E, buffer: PttBuffer) -> Self {
        Self { inner, buffer }
    }

    /// The wrapped sink.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: CaptureEvents> CaptureEvents for PttCaptureEvents<E> {
    fn state(&self, state: &CaptureState) {
        self.buffer.on_state(state);
        self.inner.state(state);
    }

    fn stopped(&self, stopped: &Stopped) {
        self.buffer.on_stopped(stopped);
        self.inner.stopped(stopped);
    }

    fn overlay_granted(&self, lease: &Lease) {
        self.buffer.on_overlay(lease);
        self.inner.overlay_granted(lease);
    }
}
