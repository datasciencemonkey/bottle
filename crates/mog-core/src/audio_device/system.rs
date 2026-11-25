//! The system audio [`AudioDevice`] of a call recording (ADR 0103, #107, #108).
//!
//! [`SystemAudioDevice`] opens the "other side of the call" through a [`MicHost`]: on macOS
//! the Core Audio process tap ([`super::system_macos::TapHost`]), on Windows WASAPI loopback
//! ([`super::system_windows::LoopbackHost`]). Linux has no backend
//! ([`SystemAudioDevice::unavailable`]).
//!
//! - **Its own thread.** The host lives on the `bottle-sysaudio` thread, never on the main
//!   thread. `open` and `close` are messages to it.
//! - **Tolerant open.** A recording must go on mic-only when system audio cannot open (an
//!   older OS, no backend, a refused tap). So `open` always returns `Ok`; the outcome is in
//!   the shared [`SystemAudioStatus`] (`Live` or `Unavailable`), which the recording session
//!   shows as `system_audio`.
//! - **Timeout (CAP-10c).** `open` waits at most its timeout for the build; past that it
//!   answers mic-only (`Unavailable`) so the recording can go on, but it does not give up on
//!   the build. The thread's own `Msg::Open` handler is what finally sets `status`, whether
//!   the build resolves in time or late: a build that succeeds late is adopted (`status` goes
//!   to `Live`, corrected from the opener's own `Unavailable` guess) as long as nothing
//!   cancelled it since (a `close` or a newer `open`); otherwise it drops here, never a live
//!   stream that `status` does not know about. The very first `open` of a device's life waits
//!   longer ([`OPEN_TIMEOUT_FIRST`]): its process tap build can take longer than a later one.
//! - **Close** drops the stream on the thread before it returns (or, while a slow build still
//!   runs, as soon as that build returns), like the mic (MOG-09, CAP-13b).
//! - **Faults.** A stream that the OS invalidates (a default output change) is built again
//!   once; a fatal fault drops the stream and sets `Unavailable`. The device never calls
//!   `CaptureCore::device_failed`: losing the other side does not stop the recording.
//! - Frames are mixed to mono and go to the [`FrameSink`] (the recording tap). Nothing is
//!   logged or written here.
//!
//! [`RecordingDevice`] puts the mic device and this device behind one [`AudioDevice`], so
//! R2 CaptureService stays the only owner of both streams (CAP-01a) and the panic stop closes
//! both (MB-05).

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{AudioDevice, AudioSource, DeviceError};

use super::{codes, mix_to_mono, DataFn, FaultFn, FrameSink, MicFrame, MicHost, StreamFault};

/// How long `open` waits for the system audio stream after the first one (CAP-10c).
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(8);
/// How long the very first `open` waits. The first process tap build can take longer than a
/// later one (observed about 8.6 s end to end on a real machine, review of #593/#107 follow-up);
/// a longer first wait makes the caller's own timeout (which only means "go on mic-only for
/// now") fire less often, while a late build is still adopted instead of lost either way.
pub const OPEN_TIMEOUT_FIRST: Duration = Duration::from_secs(15);
/// How long `close` waits for the thread (it never waits for a slow build beyond this).
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// What the system audio source is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAudioAvailability {
    /// Not open.
    Off,
    /// Open, and the stream runs.
    Live,
    /// Asked to open, but there is no stream (no backend, refused, timed out or lost).
    Unavailable,
}

impl SystemAudioAvailability {
    fn to_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Live => 1,
            Self::Unavailable => 2,
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Live,
            2 => Self::Unavailable,
            _ => Self::Off,
        }
    }
}

/// The diagnostic counters of [`SystemAudioStatus`] (CAP-10c): no audio, no path, no name,
/// counts and timings only. `last-capture.json` reads them through
/// `RecordingSession`/`FinishedRecording` to show why a system audio tap stayed `unavailable`.
#[derive(Debug, Default)]
struct OpenDiag {
    availability: AtomicU8,
    /// Ms the last `open` took to resolve: either the caller's own answer (live or refused
    /// within the timeout), or a later worker adoption of a build that finished past it.
    open_ms: AtomicU64,
    /// The last `open`'s resolution (whichever of the two above it was) took longer than the
    /// timeout that was in force for it.
    timed_out: AtomicBool,
}

/// The shared, read-only view of the system audio state. Clones share one value.
#[derive(Debug, Clone, Default)]
pub struct SystemAudioStatus(Arc<OpenDiag>);

impl SystemAudioStatus {
    /// The state now.
    pub fn get(&self) -> SystemAudioAvailability {
        SystemAudioAvailability::from_u8(self.0.availability.load(Ordering::SeqCst))
    }

    fn set(&self, v: SystemAudioAvailability) {
        self.0.availability.store(v.to_u8(), Ordering::SeqCst);
    }

    /// Ms the last `open` took to resolve (CAP-10c diagnostic); 0 before the first `open`.
    pub fn open_ms(&self) -> u64 {
        self.0.open_ms.load(Ordering::SeqCst)
    }

    /// The last `open`'s resolution took longer than the timeout in force for it (CAP-10c
    /// diagnostic): either the caller gave up with no stream yet, or the worker's late build
    /// finished after that same wait.
    pub fn timed_out(&self) -> bool {
        self.0.timed_out.load(Ordering::SeqCst)
    }

    fn record_open(&self, elapsed_ms: u64, timed_out: bool) {
        self.0.open_ms.store(elapsed_ms, Ordering::SeqCst);
        self.0.timed_out.store(timed_out, Ordering::SeqCst);
    }
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Ms since `started` (CAP-10c diagnostic only: never audio, never a path, never a name).
fn elapsed_ms(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

enum Msg {
    Open {
        generation: u64,
        /// When `open` sent this message (CAP-10c diagnostic): the worker times the build and
        /// play from here, not from when it gets to handle the message.
        started: Instant,
        /// The timeout `open` is waiting against, so the worker can tell a late (CAP-10c)
        /// resolution from an on-time one even after `open` itself has given up.
        timeout: Duration,
        reply: SyncSender<Result<(), DeviceError>>,
    },
    Close {
        reply: SyncSender<()>,
    },
    Fault {
        generation: u64,
        fault: StreamFault,
    },
    Shutdown,
}

struct SinkSlot {
    sink: Box<dyn FrameSink>,
    /// The generation whose frames the sink takes now.
    live: Option<u64>,
}

/// The system audio device. See the [module docs](self).
pub struct SystemAudioDevice {
    tx: Option<Sender<Msg>>,
    /// The generation that `open` wants now; cleared only by `close` (CAP-10c: not by a
    /// timeout, so a build that finishes late can still be adopted instead of leaking a live,
    /// untracked stream).
    wanted: Arc<Mutex<Option<u64>>>,
    next: Mutex<u64>,
    status: SystemAudioStatus,
    open_timeout_first: Duration,
    open_timeout: Duration,
    thread: Option<JoinHandle<()>>,
}

impl SystemAudioDevice {
    /// Start the `bottle-sysaudio` thread over the host that `make_host` builds there. No
    /// stream opens here. The very first `open` of this device's life waits
    /// `open_timeout_first`; every later one waits `open_timeout` (CAP-10c: the first process
    /// tap build can take longer than a later one).
    pub fn spawn<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
        open_timeout_first: Duration,
        open_timeout: Duration,
    ) -> std::io::Result<(Self, SystemAudioStatus)>
    where
        H: MicHost,
        F: FnOnce() -> H + Send + 'static,
    {
        let status = SystemAudioStatus::default();
        let wanted = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::channel::<Msg>();
        let worker = Worker {
            wanted: Arc::clone(&wanted),
            status: status.clone(),
            sink: Arc::new(Mutex::new(SinkSlot { sink, live: None })),
            tx: tx.clone(),
        };
        let thread = thread::Builder::new()
            .name("bottle-sysaudio".into())
            .spawn(move || worker.run(make_host(), rx))?;
        Ok((
            Self {
                tx: Some(tx),
                wanted,
                next: Mutex::new(0),
                status: status.clone(),
                open_timeout_first,
                open_timeout,
                thread: Some(thread),
            },
            status,
        ))
    }

    /// A device for a platform with no system audio backend: every open is `Unavailable`.
    pub fn unavailable() -> (Self, SystemAudioStatus) {
        let status = SystemAudioStatus::default();
        (
            Self {
                tx: None,
                wanted: Arc::new(Mutex::new(None)),
                next: Mutex::new(0),
                status: status.clone(),
                open_timeout_first: Duration::ZERO,
                open_timeout: Duration::ZERO,
                thread: None,
            },
            status,
        )
    }

    /// The shared state of this device.
    pub fn status(&self) -> SystemAudioStatus {
        self.status.clone()
    }
}

impl AudioDevice for SystemAudioDevice {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        if source != AudioSource::SystemAudio {
            return Err(codes::DEVICE_FAILED);
        }
        let Some(tx) = &self.tx else {
            self.status.set(SystemAudioAvailability::Unavailable);
            return Ok(());
        };
        let generation = {
            let mut next = lock(&self.next);
            *next += 1;
            *next
        };
        *lock(&self.wanted) = Some(generation);
        let timeout = if generation == 1 {
            self.open_timeout_first
        } else {
            self.open_timeout
        };
        let (reply, answer) = mpsc::sync_channel(1);
        let started = Instant::now();
        let result = tx
            .send(Msg::Open {
                generation,
                started,
                timeout,
                reply,
            })
            .ok()
            .and_then(|()| answer.recv_timeout(timeout).ok());
        match result {
            // The worker already set `status` and the CAP-10c diagnostics before it answered,
            // on both branches: refused or failed is `Unavailable`, built and played is `Live`.
            Some(Ok(())) | Some(Err(_)) => {}
            // A timeout, not a refusal: the recording goes on mic-only for now. `wanted` stays
            // set (only `close` clears it), so a build that finishes late is still adopted by
            // the worker's own check in `build`, instead of a live stream nobody tracks.
            None => {
                self.status.set(SystemAudioAvailability::Unavailable);
                self.status.record_open(elapsed_ms(started), true);
            }
        }
        Ok(())
    }

    fn close(&self, source: AudioSource) {
        if source != AudioSource::SystemAudio {
            return;
        }
        *lock(&self.wanted) = None;
        if let Some(tx) = &self.tx {
            let (reply, done) = mpsc::sync_channel(1);
            if tx.send(Msg::Close { reply }).is_ok() {
                let _ = done.recv_timeout(CLOSE_WAIT);
            }
        }
        self.status.set(SystemAudioAvailability::Off);
    }
}

impl Drop for SystemAudioDevice {
    fn drop(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.send(Msg::Shutdown);
        }
        if let Some(t) = self.thread.take() {
            if t.thread().id() != thread::current().id() {
                let _ = t.join();
            }
        }
    }
}

struct Worker {
    wanted: Arc<Mutex<Option<u64>>>,
    status: SystemAudioStatus,
    sink: Arc<Mutex<SinkSlot>>,
    tx: Sender<Msg>,
}

impl Worker {
    fn run<H: MicHost>(self, mut host: H, rx: Receiver<Msg>) {
        let mut current: Option<(u64, H::Stream)> = None;
        for msg in rx {
            match msg {
                Msg::Open {
                    generation,
                    started,
                    timeout,
                    reply,
                } => {
                    self.drop_stream(&mut current);
                    let result = self.build(&mut host, generation);
                    // CAP-10c: this is the one place that sets `status` for an open, whether
                    // it answers in time or late (the `open` call may already have given up
                    // and gone on mic-only; this corrects it either way, and nothing here
                    // leaves a live stream that `status` does not know about).
                    let timed_out = started.elapsed() > timeout;
                    let answer = match result {
                        Ok(stream) => {
                            current = Some((generation, stream));
                            self.status.set(SystemAudioAvailability::Live);
                            self.status.record_open(elapsed_ms(started), timed_out);
                            Ok(())
                        }
                        Err(e) => {
                            // `open_cancelled`: a `close` (or a newer `open`) already ran and
                            // owns the status now (`Off`, or that generation's own outcome);
                            // do not undo it. Any other error is a genuine refusal.
                            if e != codes::OPEN_CANCELLED {
                                self.status.set(SystemAudioAvailability::Unavailable);
                                self.status.record_open(elapsed_ms(started), timed_out);
                            }
                            Err(e)
                        }
                    };
                    // The opener may have timed out already: then nobody reads the reply.
                    let _ = reply.try_send(answer);
                }
                Msg::Close { reply } => {
                    self.drop_stream(&mut current);
                    let _ = reply.try_send(());
                }
                Msg::Fault { generation, fault } => {
                    if current.as_ref().map(|c| c.0) != Some(generation) {
                        continue;
                    }
                    match fault {
                        StreamFault::Transient | StreamFault::Rerouted => {}
                        StreamFault::Invalidated => {
                            // One new build on the new default output, under the same open.
                            self.drop_stream(&mut current);
                            match self.build(&mut host, generation) {
                                Ok(stream) => current = Some((generation, stream)),
                                Err(_) => self.status.set(SystemAudioAvailability::Unavailable),
                            }
                        }
                        StreamFault::Fatal => {
                            self.drop_stream(&mut current);
                            self.status.set(SystemAudioAvailability::Unavailable);
                        }
                    }
                }
                Msg::Shutdown => break,
            }
        }
        self.drop_stream(&mut current);
    }

    /// Build and play a stream for `generation`, unless a close or a newer open cancelled it
    /// meanwhile; then the stream drops here, never live and untracked (CAP-10c). A build that
    /// outlives the opener's own timeout is still adopted here, as long as nothing has
    /// cancelled it since: [`Worker::run`]'s `Msg::Open` handler is the one that then sets
    /// `status` to `Live`, whether this returns in time or late.
    fn build<H: MicHost>(&self, host: &mut H, generation: u64) -> Result<H::Stream, DeviceError> {
        let data = self.data_fn(generation);
        let tx = self.tx.clone();
        let fault: FaultFn = Box::new(move |fault| {
            let _ = tx.send(Msg::Fault { generation, fault });
        });
        let stream = host.build(data, fault)?;
        if *lock(&self.wanted) != Some(generation) {
            drop(stream);
            return Err(codes::OPEN_CANCELLED);
        }
        lock(&self.sink).live = Some(generation);
        if let Err(e) = host.play(&stream) {
            lock(&self.sink).live = None;
            drop(stream);
            return Err(e);
        }
        // `play` can itself run long (CAP-10c: an `AudioDeviceStart` that takes longer than the
        // opener's timeout). Check `wanted` again: a `close` or a newer `open` may have
        // superseded this generation while `play` ran. If so, this build must not be adopted.
        if *lock(&self.wanted) != Some(generation) {
            lock(&self.sink).live = None;
            drop(stream);
            return Err(codes::OPEN_CANCELLED);
        }
        Ok(stream)
    }

    fn data_fn(&self, generation: u64) -> DataFn {
        let sink = Arc::clone(&self.sink);
        let mut mono = Vec::new();
        Box::new(move |block| {
            let mut slot = lock(&sink);
            if slot.live != Some(generation) {
                return;
            }
            mix_to_mono(block.samples, block.channels, &mut mono);
            slot.sink.frame(MicFrame {
                samples: &mono,
                sample_rate: block.sample_rate,
            });
        })
    }

    fn drop_stream<S>(&self, current: &mut Option<(u64, S)>) {
        // Stop the frames first, then drop the stream outside the sink lock (a host may wait
        // for a running data callback when its stream drops).
        lock(&self.sink).live = None;
        current.take();
    }
}

/// The mic and the system audio device behind one [`AudioDevice`] for R2 CaptureService.
pub struct RecordingDevice {
    mic: Box<dyn AudioDevice>,
    system: Box<dyn AudioDevice>,
}

impl RecordingDevice {
    /// Route [`AudioSource::Mic`] to `mic` and [`AudioSource::SystemAudio`] to `system`.
    pub fn new(mic: Box<dyn AudioDevice>, system: Box<dyn AudioDevice>) -> Self {
        Self { mic, system }
    }
}

impl AudioDevice for RecordingDevice {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        match source {
            AudioSource::Mic => self.mic.open(source),
            AudioSource::SystemAudio => self.system.open(source),
        }
    }

    fn close(&self, source: AudioSource) {
        match source {
            AudioSource::Mic => self.mic.close(source),
            AudioSource::SystemAudio => self.system.close(source),
        }
    }
}
