//! R4 SttEngine: model residency, preload on key-down and the decode of closed VAD segments
//! during speech (#46, CAP-51a, CAP-51b, CAP-51c, CAP-52, CAP-72, ADR 0082).
//!
//! [`LocalSttEngine`] runs one worker thread that owns the recognizer (CAP-48: one worker).
//! Every call is a message to that thread, so no caller waits for a model load or a decode.
//!
//! - **Residency.** On AC power the model loads at start and stays loaded (CAP-51a). On battery
//!   it unloads after 30 min with no use (CAP-51b). The worker reads the power source on each
//!   tick ([`EngineConfig::tick`]). Back on AC, a model that was loaded once loads again.
//! - **Preload.** [`LocalSttEngine::preload`] on the talk-key down, and [`LocalSttEngine::begin`]
//!   on a granted lease, load the model if it is not loaded (CAP-51c). So the key-up path does
//!   not load it.
//! - **Decode during speech.** The mic frames of the live PTT or hands-free lease come in at the
//!   device rate. The worker resamples them to 16 kHz, runs the VAD on 32 ms windows and closes
//!   a segment by the [`SegmentRule`]. It decodes each closed segment at once, while the key is
//!   still down (CAP-52). [`LocalSttEngine::commit`] after key-up decodes only the open tail.
//!   Then it gives one [`SttDone`] with one `Transcript` to the [`TranscriptSink`].
//! - **CAP-20.** Each segment goes through `normalize_peak` (-3 dBFS) just before the
//!   recognizer reads it (the #23 hand-off, ADR 0067).
//! - **CAP-72.** Each transcript has the provider, the model and the runtime of the loader.
//! - **Supervision.** A panic in the recognizer or the loader is caught. The model is dropped,
//!   the lease gets the content-free code `stt_engine_failed`, and the next use loads a new
//!   model. (Handy's engine supervisor restarts a worker process; here the recognizer is in
//!   process, so a caught panic is the restart point.)
//! - **Privacy (CAP-01b, CAP-34, section 9.2).** Audio stays in RAM. A segment buffer has a
//!   fixed capacity, so it never moves, and it is zeroed after each decode and before it is
//!   freed. A dropped capture (a tap, Esc, zero buffers, the panic stop) makes no transcript
//!   and its audio is zeroed. Nothing here writes a log or telemetry: no audio, no text.
//!
//! The adapters connect R4 to R2: [`SttFrameSink`] is the mic tee half next to the PTT buffer
//! (#21), and [`SttCaptureEvents`] wraps the R2 events. R4 is also the PTT buffer's
//! [`UtteranceSink`]: it has decoded the same audio from the tee, so it drops (and so zeroes)
//! the utterance at once (CAP-34).

pub mod power;
mod resample;
pub mod residency;
mod store;
pub mod vad;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{compiler_fence, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

pub use power::{PowerProbe, PowerSource, SystemPowerProbe};
pub use resample::Resampler;
pub use store::{LocalStt, TranscriptStore};
pub use vad::{
    EnergyVad, SegmentCutter, SegmentRule, Vad, STT_SAMPLE_RATE, VAD_WINDOW, VAD_WINDOW_MS,
};

use crate::audio_device::normalize::normalize_peak;
use crate::audio_device::ptt_buffer::{Utterance, UtteranceSink};
use crate::audio_device::sink::{FrameSink, MicFrame, SinkEvent};
use crate::capture::{parse_lease, CaptureEvents, LeaseNo, StopReason, Stopped};
use crate::clock::{MonotonicClock, SystemClock};
use crate::stt::{LOCAL_STT_MODEL, LOCAL_STT_RUNTIME};
use crate::types::{CaptureMode, CaptureState, MogId, Transcript, TranscriptProvider};

/// The most takes the engine holds at once: a hands-free take and a PTT take over it (ARC-F9).
pub const MAX_TAKES: usize = 2;

/// A segment under this peak is not decoded when the VAD found no speech in it: it is
/// silence, and a decode of silence can only make up words.
pub const SILENCE_PEAK_DBFS: f32 = -60.0;

/// The model, the runtime and the provider that a loader's recognizer reports (CAP-72).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SttIdentity {
    /// `Transcript.provider`.
    pub provider: TranscriptProvider,
    /// `Transcript.model`.
    pub model: String,
    /// `Transcript.runtime`.
    pub runtime: String,
}

impl SttIdentity {
    /// Local Parakeet TDT 0.6B v3 int8 on sherpa-onnx (CAP-47a, CAP-48).
    pub fn local_parakeet() -> Self {
        Self {
            provider: TranscriptProvider::LocalParakeet,
            model: LOCAL_STT_MODEL.to_owned(),
            runtime: LOCAL_STT_RUNTIME.to_owned(),
        }
    }
}

/// A loaded speech recognizer. It runs on the engine worker only.
pub trait Recognizer: Send {
    /// Decode one segment of 16 kHz mono samples and return the text.
    fn transcribe(&mut self, samples: &[f32]) -> String;
}

/// Loads the recognizer. `load` runs on the engine worker; it may take seconds.
pub trait RecognizerLoader: Send + Sync + 'static {
    /// Load the model into memory. Errors are content-free codes (`stt_model_missing`, ...).
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str>;
    /// The identity that each transcript records (CAP-72).
    fn identity(&self) -> SttIdentity;
}

/// The loader when no model can load: a build without `local-stt`, or no installed model.
/// TODO(#413): the app uses `stt_local::ParakeetLoader` once the wizard installs the model.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoModelLoader;

impl RecognizerLoader for NoModelLoader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        Err("stt_model_missing")
    }

    fn identity(&self) -> SttIdentity {
        SttIdentity::local_parakeet()
    }
}

/// The result of one kept capture. Its `Debug` shows no transcript text (CAP-01b).
#[derive(Clone, PartialEq, Eq)]
pub struct SttDone {
    /// The lease of the capture.
    pub lease_id: LeaseNo,
    /// The MOG that held the lease.
    pub mog_id: MogId,
    /// PTT or hands-free.
    pub mode: CaptureMode,
    /// The length of the capture audio in ms (at 16 kHz).
    pub duration_ms: u64,
    /// The transcript, or a content-free error code.
    pub result: Result<Transcript, &'static str>,
}

impl std::fmt::Debug for SttDone {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No text, no raw text and no MOG ID in a log: only content-free fields.
        let result: Result<String, &str> = match &self.result {
            Ok(t) => Ok(format!(
                "Transcript {{ segment_id: {:?}, provider: {:?}, model: {:?}, runtime: {:?}, latency_ms: {}, .. }}",
                t.segment_id, t.provider, t.model, t.runtime, t.latency_ms
            )),
            Err(code) => Err(code),
        };
        f.debug_struct("SttDone")
            .field("lease_id", &self.lease_id)
            .field("mode", &self.mode)
            .field("duration_ms", &self.duration_ms)
            .field("result", &result)
            .finish_non_exhaustive()
    }
}

/// The consumer of finished transcripts. It runs on the engine worker, so it must be fast.
pub trait TranscriptSink: Send + Sync + 'static {
    /// One finished capture.
    fn done(&self, done: SttDone);
}

/// A [`TranscriptSink`] that drops each result.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoTranscriptSink;

impl TranscriptSink for NoTranscriptSink {
    fn done(&self, _done: SttDone) {}
}

/// The engine settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngineConfig {
    /// The segment cut rule (CAP-52).
    pub rule: SegmentRule,
    /// The idle time on battery before the model unloads (CAP-51b).
    pub idle_unload: Duration,
    /// How often the worker reads the power source.
    pub tick: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            rule: SegmentRule::default(),
            idle_unload: residency::BATTERY_IDLE_UNLOAD,
            tick: Duration::from_secs(60),
        }
    }
}

/// Makes one VAD for each take.
pub type VadFactory = Arc<dyn Fn() -> Box<dyn Vad> + Send + Sync>;

/// What the engine is built from. [`EngineParts::new`] takes the OS power probe, the system
/// clock, the [`EnergyVad`] and the default config; a test replaces any of them.
pub struct EngineParts {
    /// Loads the recognizer.
    pub loader: Arc<dyn RecognizerLoader>,
    /// Gets each result.
    pub out: Arc<dyn TranscriptSink>,
    /// The power source.
    pub power: Arc<dyn PowerProbe>,
    /// The idle and latency clock.
    pub clock: Arc<dyn MonotonicClock>,
    /// The VAD of each take.
    pub vad: VadFactory,
    /// The settings.
    pub config: EngineConfig,
}

impl EngineParts {
    /// The production parts around `loader` and `out`.
    pub fn new(loader: Arc<dyn RecognizerLoader>, out: Arc<dyn TranscriptSink>) -> Self {
        Self {
            loader,
            out,
            power: Arc::new(SystemPowerProbe),
            clock: Arc::new(SystemClock::new()),
            vad: Arc::new(|| Box::new(EnergyVad::default()) as Box<dyn Vad>),
            config: EngineConfig::default(),
        }
    }
}

/// A snapshot of the engine state, for the tests and the resource budget.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EngineStatus {
    /// The model is in memory.
    pub loaded: bool,
    /// Model loads since start.
    pub loads: u32,
    /// Residency unloads since start (CAP-51b).
    pub unloads: u32,
    /// Takes that wait for their commit.
    pub open_takes: usize,
    /// 16 kHz samples that the engine holds now.
    pub held_samples: usize,
    /// The code of the last failed load, if the model is not loaded.
    pub last_error: Option<&'static str>,
    /// Frame messages that reached the worker since start (content-free; CAP-13a tests).
    pub frame_messages: u64,
}

enum Cmd {
    Preload,
    Begin {
        lease: LeaseNo,
        mog_id: MogId,
        mode: CaptureMode,
    },
    Idle,
    Frames {
        lease: LeaseNo,
        samples: Vec<f32>,
        rate: u32,
    },
    Commit {
        lease: LeaseNo,
        reason: StopReason,
        at: Duration,
    },
    Discard {
        lease: LeaseNo,
    },
    DiscardAll,
    Tick,
    Sync(Sender<()>),
}

struct Inner {
    tx: Sender<Cmd>,
    /// The lease that takes frames now, 0 for none. The audio thread reads it, so it copies
    /// no frame when no capture is live.
    current: AtomicU64,
    status: Arc<Mutex<EngineStatus>>,
    clock: Arc<dyn MonotonicClock>,
}

/// R4: the handle of the engine worker. Clones share one worker. The worker stops when the
/// last handle drops.
#[derive(Clone)]
pub struct LocalSttEngine {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for LocalSttEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSttEngine")
            .field("status", &self.status())
            .finish_non_exhaustive()
    }
}

impl LocalSttEngine {
    /// Start the worker. On AC power it loads the model at once (CAP-51a).
    pub fn spawn(parts: EngineParts) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(EngineStatus::default()));
        let clock = Arc::clone(&parts.clock);
        let worker = Worker::new(parts, Arc::clone(&status));
        // The thread name is fixed text; nothing about audio or text is in it.
        let spawned = std::thread::Builder::new()
            .name("bottle-r4-stt".into())
            .spawn(move || worker.run(rx));
        if spawned.is_err() {
            lock(&status).last_error = Some("stt_engine_failed");
        }
        Self {
            inner: Arc::new(Inner {
                tx,
                current: AtomicU64::new(0),
                status,
                clock,
            }),
        }
    }

    fn send(&self, cmd: Cmd) {
        // A send fails only when the worker is gone; there is nothing to do then.
        let _ = self.inner.tx.send(cmd);
    }

    /// The talk key went down: load the model if it is not loaded (CAP-51c).
    pub fn preload(&self) {
        self.send(Cmd::Preload);
    }

    /// A PTT or hands-free lease records now: its frames go to its take. A new lease also
    /// preloads the model (CAP-51c).
    pub fn begin(&self, lease: LeaseNo, mog_id: &str, mode: CaptureMode) {
        // The `Begin` goes into the channel before the audio thread can see the new lease, and
        // each frame message names its lease. So the first frames of a PTT over hands-free
        // (ARC-F9) never reach the hands-free take.
        self.send(Cmd::Begin {
            lease,
            mog_id: mog_id.to_owned(),
            mode,
        });
        self.inner.current.store(lease, Ordering::SeqCst);
    }

    /// No lease records now (idle, paused or a session): frames are dropped.
    pub fn idle(&self) {
        self.inner.current.store(0, Ordering::SeqCst);
        self.send(Cmd::Idle);
    }

    /// One block of mono mic audio at `rate`, for the live lease. With no live lease the block
    /// is dropped at once (no pre-roll, CAP-13a).
    pub fn frames(&self, samples: &[f32], rate: u32) {
        let lease = self.inner.current.load(Ordering::SeqCst);
        if samples.is_empty() || lease == 0 {
            return;
        }
        self.send(Cmd::Frames {
            lease,
            samples: samples.to_vec(),
            rate,
        });
    }

    /// The lease stopped. A stop that keeps the audio decodes the tail and gives the result.
    /// A stop that drops it (`StopReason::discards_audio`) and the panic stop (MB-05) drop the
    /// take with no transcript.
    pub fn commit(&self, lease: LeaseNo, reason: StopReason) {
        let _ = self
            .inner
            .current
            .compare_exchange(lease, 0, Ordering::SeqCst, Ordering::SeqCst);
        self.send(Cmd::Commit {
            lease,
            reason,
            at: self.inner.clock.now(),
        });
    }

    /// Drop the take of `lease` with no transcript.
    pub fn discard(&self, lease: LeaseNo) {
        let _ = self
            .inner
            .current
            .compare_exchange(lease, 0, Ordering::SeqCst, Ordering::SeqCst);
        self.send(Cmd::Discard { lease });
    }

    /// Drop every take with no transcript (CAP-17b zero buffers).
    pub fn discard_all(&self) {
        self.inner.current.store(0, Ordering::SeqCst);
        self.send(Cmd::DiscardAll);
    }

    /// Read the power source and apply the residency policy now. The worker also does this
    /// on each [`EngineConfig::tick`].
    pub fn tick(&self) {
        self.send(Cmd::Tick);
    }

    /// Wait until the worker has done every earlier call. For tests and for a blocking
    /// [`crate::stt::SttEngine::load_model`].
    pub fn sync(&self) {
        let (tx, rx) = mpsc::channel();
        self.send(Cmd::Sync(tx));
        let _ = rx.recv();
    }

    /// The engine state now.
    pub fn status(&self) -> EngineStatus {
        *lock(&self.inner.status)
    }

    /// The mic tee half: a [`FrameSink`] that feeds this engine.
    pub fn frame_sink(&self) -> SttFrameSink {
        SttFrameSink {
            engine: self.clone(),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Overwrite every sample of `v`, also the spare capacity, with 0 in a way that the
/// optimiser keeps. The same method as the PTT buffer (#21) and `local_store/key_vault.rs`.
fn wipe_vec(v: &mut Vec<f32>) {
    for s in v.iter_mut() {
        // SAFETY: `s` is a valid, aligned `&mut f32`.
        unsafe { std::ptr::write_volatile(s, 0.0) };
    }
    for s in v.spare_capacity_mut() {
        // SAFETY: `s` is a valid, aligned slot of the Vec's own allocation.
        unsafe { std::ptr::write_volatile(s.as_mut_ptr(), 0.0) };
    }
    compiler_fence(Ordering::SeqCst);
    v.clear();
}

/// A buffer of 16 kHz samples with a fixed capacity: it never reallocates, so no unwiped copy
/// is left in freed memory. It is zeroed when it is cleared and when it drops.
struct Wiped(Vec<f32>);

impl Wiped {
    fn with_capacity(n: usize) -> Self {
        Self(Vec::with_capacity(n))
    }

    fn clear(&mut self) {
        wipe_vec(&mut self.0);
    }
}

impl Drop for Wiped {
    fn drop(&mut self) {
        wipe_vec(&mut self.0);
    }
}

/// The audio and the texts of one lease.
struct Take {
    lease: LeaseNo,
    mog_id: MogId,
    mode: CaptureMode,
    resampler: Option<Resampler>,
    vad: Box<dyn Vad>,
    cutter: SegmentCutter,
    /// The open segment at 16 kHz.
    segment: Wiped,
    segment_speech: bool,
    /// Resampled samples that do not fill a VAD window yet.
    window: Wiped,
    /// The resampler output of one block.
    scratch: Wiped,
    texts: Vec<String>,
    samples: u64,
    failed: Option<&'static str>,
}

impl Take {
    fn new(lease: LeaseNo, mog_id: MogId, mode: CaptureMode, parts: &Shared) -> Self {
        let max =
            usize::try_from(parts.config.rule.max_segment_ms * u64::from(STT_SAMPLE_RATE) / 1_000)
                .unwrap_or(usize::MAX / 2);
        Self {
            lease,
            mog_id,
            mode,
            resampler: None,
            vad: (parts.vad)(),
            cutter: SegmentCutter::new(parts.config.rule),
            segment: Wiped::with_capacity(max + VAD_WINDOW),
            segment_speech: false,
            window: Wiped::with_capacity(VAD_WINDOW),
            // 1 s at 48 kHz in one block is far more than a mic callback gives.
            scratch: Wiped::with_capacity(STT_SAMPLE_RATE as usize),
            texts: Vec::new(),
            samples: 0,
            failed: None,
        }
    }

    fn held(&self) -> usize {
        self.segment.0.len() + self.window.0.len()
    }
}

/// The parts that the worker reads and does not change.
struct Shared {
    loader: Arc<dyn RecognizerLoader>,
    out: Arc<dyn TranscriptSink>,
    power: Arc<dyn PowerProbe>,
    clock: Arc<dyn MonotonicClock>,
    vad: VadFactory,
    config: EngineConfig,
    identity: SttIdentity,
}

/// The model slot and its residency state.
struct Model {
    slot: Option<Box<dyn Recognizer>>,
    ever_loaded: bool,
    last_used: Duration,
    loads: u32,
    unloads: u32,
    last_error: Option<&'static str>,
}

impl Model {
    fn ensure_loaded(&mut self, shared: &Shared) -> Result<(), &'static str> {
        self.last_used = shared.clock.now();
        if self.slot.is_some() {
            return Ok(());
        }
        let loader = &shared.loader;
        match catch_unwind(AssertUnwindSafe(|| loader.load())) {
            Ok(Ok(model)) => {
                self.slot = Some(model);
                self.ever_loaded = true;
                self.loads += 1;
                self.last_error = None;
                Ok(())
            }
            Ok(Err(code)) => {
                self.last_error = Some(code);
                Err(code)
            }
            Err(_) => {
                self.last_error = Some("stt_engine_failed");
                Err("stt_engine_failed")
            }
        }
    }

    /// Normalise `segment` to -3 dBFS (CAP-20) and decode it.
    fn decode(&mut self, shared: &Shared, segment: &mut [f32]) -> Result<String, &'static str> {
        self.ensure_loaded(shared)?;
        normalize_peak(segment);
        let Some(model) = self.slot.as_mut() else {
            return Err("stt_engine_failed");
        };
        match catch_unwind(AssertUnwindSafe(|| model.transcribe(segment))) {
            Ok(text) => {
                self.last_used = shared.clock.now();
                Ok(text)
            }
            Err(_) => {
                // The worker survives; the faulted model is dropped and loads again on the
                // next use.
                self.slot = None;
                Err("stt_engine_failed")
            }
        }
    }
}

struct Worker {
    shared: Shared,
    model: Model,
    takes: Vec<Take>,
    current: Option<LeaseNo>,
    frame_messages: u64,
    status: Arc<Mutex<EngineStatus>>,
}

impl Worker {
    fn new(parts: EngineParts, status: Arc<Mutex<EngineStatus>>) -> Self {
        let identity = parts.loader.identity();
        Self {
            shared: Shared {
                loader: parts.loader,
                out: parts.out,
                power: parts.power,
                clock: parts.clock,
                vad: parts.vad,
                config: parts.config,
                identity,
            },
            model: Model {
                slot: None,
                ever_loaded: false,
                last_used: Duration::ZERO,
                loads: 0,
                unloads: 0,
                last_error: None,
            },
            takes: Vec::new(),
            current: None,
            frame_messages: 0,
            status,
        }
    }

    fn run(mut self, rx: Receiver<Cmd>) {
        self.residency(true);
        self.publish();
        loop {
            match rx.recv_timeout(self.shared.config.tick) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => self.residency(false),
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.publish();
        }
    }

    fn publish(&self) {
        *lock(&self.status) = EngineStatus {
            loaded: self.model.slot.is_some(),
            loads: self.model.loads,
            unloads: self.model.unloads,
            open_takes: self.takes.len(),
            held_samples: self.takes.iter().map(Take::held).sum(),
            last_error: if self.model.slot.is_some() {
                None
            } else {
                self.model.last_error
            },
            frame_messages: self.frame_messages,
        };
    }

    fn handle(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Preload => {
                let _ = self.model.ensure_loaded(&self.shared);
            }
            Cmd::Begin {
                lease,
                mog_id,
                mode,
            } => {
                if !self.takes.iter().any(|t| t.lease == lease) {
                    if self.takes.len() >= MAX_TAKES {
                        // The oldest take drops; its audio is zeroed.
                        self.takes.remove(0);
                    }
                    self.takes
                        .push(Take::new(lease, mog_id, mode, &self.shared));
                }
                self.current = Some(lease);
                let _ = self.model.ensure_loaded(&self.shared);
            }
            Cmd::Idle => self.current = None,
            Cmd::Frames {
                lease,
                mut samples,
                rate,
            } => {
                self.frame_messages += 1;
                self.frames(lease, &samples, rate);
                wipe_vec(&mut samples);
            }
            Cmd::Commit { lease, reason, at } => self.commit(lease, reason, at),
            Cmd::Discard { lease } => {
                if self.current == Some(lease) {
                    self.current = None;
                }
                self.takes.retain(|t| t.lease != lease);
            }
            Cmd::DiscardAll => {
                self.current = None;
                self.takes.clear();
            }
            Cmd::Tick => self.residency(false),
            Cmd::Sync(done) => {
                self.publish();
                let _ = done.send(());
            }
        }
    }

    /// CAP-51a / CAP-51b: unload on battery after the idle time; load on AC.
    fn residency(&mut self, startup: bool) {
        let power = self.shared.power.source();
        if self.model.slot.is_some() {
            let idle = self.shared.clock.now().saturating_sub(self.model.last_used);
            if self.takes.is_empty()
                && residency::should_unload_after(power, idle, self.shared.config.idle_unload)
            {
                self.model.slot = None;
                self.model.unloads += 1;
            }
        } else if residency::keeps_resident(power) && (startup || self.model.ever_loaded) {
            let _ = self.model.ensure_loaded(&self.shared);
        }
    }

    fn frames(&mut self, lease: LeaseNo, samples: &[f32], rate: u32) {
        // A frame of a lease that is not live on the worker (stopped, idle or not begun) drops.
        if self.current != Some(lease) {
            return;
        }
        let Some(at) = self.takes.iter().position(|t| t.lease == lease) else {
            return;
        };
        let take = &mut self.takes[at];
        if take.resampler.as_ref().map(Resampler::from_rate) != Some(rate) {
            // A new device (CAP-18) can have another rate; the 16 kHz stream goes on.
            take.resampler = Some(Resampler::new(rate));
        }
        let mut out = std::mem::replace(&mut take.scratch, Wiped(Vec::new()));
        if let Some(rs) = take.resampler.as_mut() {
            rs.process(samples, &mut out.0);
        }
        for s in out.0.iter().copied() {
            let take = &mut self.takes[at];
            take.window.0.push(s);
            take.samples += 1;
            if take.window.0.len() < VAD_WINDOW {
                continue;
            }
            let window = &take.window.0;
            let vad = &mut take.vad;
            let speech = match catch_unwind(AssertUnwindSafe(|| vad.is_speech(window))) {
                Ok(speech) => speech,
                Err(_) => {
                    // A faulted VAD is replaced by the energy VAD for the rest of the take; the
                    // window counts as speech, so no audio is dropped as silence.
                    take.vad = Box::new(EnergyVad::default());
                    true
                }
            };
            take.segment.0.extend_from_slice(&take.window.0);
            take.window.clear();
            take.segment_speech |= speech;
            if take.cutter.push(speech) {
                // CAP-52: a closed segment decodes now, while the key is still down.
                self.close_segment(at);
            }
        }
        out.clear();
        self.takes[at].scratch = out;
    }

    /// Decode the open segment of take `at` (if it holds sound) and zero it.
    fn close_segment(&mut self, at: usize) {
        let take = &mut self.takes[at];
        let sound = take.segment_speech
            || crate::audio_device::normalize::peak_dbfs(&take.segment.0) > SILENCE_PEAK_DBFS;
        if sound && !take.segment.0.is_empty() && take.failed.is_none() {
            match self.model.decode(&self.shared, &mut take.segment.0) {
                Ok(text) => take.texts.push(text),
                Err(code) => take.failed = Some(code),
            }
        }
        take.segment.clear();
        take.segment_speech = false;
    }

    fn commit(&mut self, lease: LeaseNo, reason: StopReason, at: Duration) {
        if self.current == Some(lease) {
            self.current = None;
        }
        let Some(pos) = self.takes.iter().position(|t| t.lease == lease) else {
            return;
        };
        if reason.discards_audio() || reason == StopReason::Panic || self.takes[pos].samples == 0 {
            // CAP-44b, CAP-14b, CAP-17b and the panic stop (MB-05): no transcript.
            self.takes.remove(pos);
            return;
        }
        // The tail: the open segment plus the part window.
        {
            let take = &mut self.takes[pos];
            let part = std::mem::replace(&mut take.window, Wiped(Vec::new()));
            take.segment.0.extend_from_slice(&part.0);
            drop(part);
        }
        self.close_segment(pos);
        let take = self.takes.remove(pos);
        let duration_ms = take.samples * 1_000 / u64::from(STT_SAMPLE_RATE);
        let result = match take.failed {
            Some(code) => Err(code),
            None => {
                let text = take
                    .texts
                    .iter()
                    .map(|t| t.trim())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                let latency = self.shared.clock.now().saturating_sub(at);
                Ok(Transcript {
                    segment_id: segment_id(take.lease),
                    text: text.clone(),
                    raw: text,
                    lang: None,
                    provider: self.shared.identity.provider,
                    model: self.shared.identity.model.clone(),
                    runtime: self.shared.identity.runtime.clone(),
                    latency_ms: u64::try_from(latency.as_millis()).unwrap_or(u64::MAX),
                })
            }
        };
        let done = SttDone {
            lease_id: take.lease,
            mog_id: take.mog_id.clone(),
            mode: take.mode,
            duration_ms,
            result,
        };
        let out = &self.shared.out;
        // A panic in the sink loses this one result; the worker lives on.
        let _ = catch_unwind(AssertUnwindSafe(|| out.done(done)));
    }
}

/// The `segmentId` of the transcript of a PTT or hands-free lease: one segment per lease.
pub fn segment_id(lease: LeaseNo) -> String {
    format!("ptt-{lease}")
}

/// The mic tee half of R4. It runs on the audio thread: with no live lease it returns at once;
/// with one it copies the block into a message for the worker. It never blocks on a decode.
#[derive(Debug)]
pub struct SttFrameSink {
    engine: LocalSttEngine,
}

impl FrameSink for SttFrameSink {
    fn frame(&mut self, frame: MicFrame<'_>) {
        self.engine.frames(frame.samples, frame.sample_rate);
    }

    fn event(&mut self, event: SinkEvent) {
        if let SinkEvent::Discard = event {
            // CAP-17b: zero buffers. No transcript is made.
            self.engine.discard_all();
        }
    }
}

/// Wraps the R2 [`CaptureEvents`] and drives R4: a granted PTT or hands-free state begins (and
/// preloads, CAP-51c) its take, any other state stops the frames, and a stop commits or drops
/// the take. Then the event goes to the inner sink.
#[derive(Debug)]
pub struct SttCaptureEvents<E> {
    inner: E,
    engine: LocalSttEngine,
}

impl<E: CaptureEvents> SttCaptureEvents<E> {
    /// Wrap `inner`.
    pub fn new(inner: E, engine: LocalSttEngine) -> Self {
        Self { inner, engine }
    }

    /// The wrapped sink.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: CaptureEvents> CaptureEvents for SttCaptureEvents<E> {
    fn state(&self, state: &CaptureState) {
        let lease = state.lease_id.as_deref().and_then(|w| parse_lease(w).ok());
        let mode = state
            .mode
            .filter(|m| matches!(m, CaptureMode::Ptt | CaptureMode::HandsFree));
        match (lease, mode) {
            (Some(lease), Some(mode)) if state.recording && !state.paused => {
                let owner = state.owner.as_deref().unwrap_or_default();
                self.engine.begin(lease, owner, mode);
            }
            _ => self.engine.idle(),
        }
        self.inner.state(state);
    }

    fn stopped(&self, stopped: &Stopped) {
        self.engine.commit(stopped.lease_id, stopped.reason);
        self.inner.stopped(stopped);
    }
}

/// R4 is the consumer of the PTT buffer (#21, ADR 0078 decision 9). R4 has read the same
/// audio from the mic tee and decoded it during speech (CAP-52), and the stop reaches R4
/// through [`SttCaptureEvents`]. So the utterance is not decoded a second time: it drops here,
/// which zeroes its samples (CAP-34). The app keeps no second copy of the audio.
impl UtteranceSink for LocalSttEngine {
    fn utterance(&self, utterance: Utterance) {
        drop(utterance);
    }
}
