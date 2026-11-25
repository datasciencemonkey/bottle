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
//! - **Timeout (CAP-10c).** `open` waits at most [`OPEN_TIMEOUT`] for the build; past that it
//!   answers mic-only (`Unavailable`) so the recording (and `recording_start`, which holds
//!   `capture.rs`'s device mutex for this whole wait) can go on, but it does not give up on
//!   the build. The thread's own `Msg::Open` handler is what finally sets `status`, whether
//!   the build resolves in time or late: a build that succeeds late is adopted (`status` goes
//!   to `Live`, corrected from the opener's own `Unavailable` guess) as long as nothing
//!   cancelled it since (a `close` or a newer `open`); otherwise it drops here, never a live
//!   stream that `status` does not know about. [`RecordingSession::system_state`] has no
//!   latch of its own, so this correction reaches the brain on the very next tick.
//! - **The resolution race (CAP-10c).** The opener's own timeout and the worker's late
//!   resolution can land at nearly the same moment. [`SystemAudioStatus`] settles this with
//!   one lock (`resolution`): the worker's `resolve` always wins for the generation it names;
//!   the opener's `mark_timed_out` only writes when the worker has not resolved that
//!   generation yet. Neither ever leaves a torn `(open_ms, timed_out)` pair.
//! - **Close** drops the stream on the thread before it returns (or, while a slow build still
//!   runs, as soon as that build returns), like the mic (MOG-09, CAP-13b). It waits
//!   [`SystemAudioDevice::spawn`]'s `close_wait` (a test may ask for a short one through
//!   [`SystemAudioDevice::spawn_with_close_wait`] to prove a close that gives up early still
//!   ends with the stream dropped and `status` at `Off`).
//! - **Faults.** A stream that the OS invalidates (a default output change) is built again
//!   once; a fatal fault drops the stream and sets `Unavailable`. A rebuild that the `wanted`
//!   check cancels (`open_cancelled`, because a `close` or a newer `open` ran meanwhile) does
//!   not set `Unavailable` either: whatever cancelled it owns `status` now. The device never
//!   calls `CaptureCore::device_failed`: losing the other side does not stop the recording.
//! - Frames are mixed to mono and go to the [`FrameSink`] (the recording tap). Nothing is
//!   logged or written here.
//!
//! [`RecordingDevice`] puts the mic device and this device behind one [`AudioDevice`], so
//! R2 CaptureService stays the only owner of both streams (CAP-01a) and the panic stop closes
//! both (MB-05).
//!
//! [`RecordingSession::system_state`]: crate::recording_capture::session::RecordingSession::system_state

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{AudioDevice, AudioSource, DeviceError};

use super::{codes, mix_to_mono, DataFn, FaultFn, FrameSink, MicFrame, MicHost, StreamFault};

/// How long `open` waits for the system audio stream (CAP-10c). Short on purpose:
/// `capture.rs`'s `open_needed` holds the device mutex for this whole wait, so a long one
/// would delay `recording_start`, the MB-05 panic stop and PTT. A build that finishes later is
/// still adopted, never lost (see the [module docs](self)), so a short wait here costs nothing
/// but a little extra time on mic-only before CAP-10a catches up.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(3);
/// How long `close` waits for the thread by default (it never waits for a slow build beyond
/// this). [`SystemAudioDevice::spawn_with_close_wait`] lets a test ask for a shorter one.
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

/// One `open`'s diagnostics, as one unit (CAP-10c): a reader never sees a torn
/// `(open_ms, timed_out)` pair, and `generation` plus `done` are what settle the race between
/// the worker's `resolve` and the opener's own `mark_timed_out` for the same `open` (see the
/// [module docs](self)).
#[derive(Debug, Clone, Copy, Default)]
struct Resolution {
    /// The generation this reflects. `0` before the first `open` (generations start at 1).
    generation: u64,
    /// Ms the `open` took to resolve; meaningless while `generation` is `0`.
    open_ms: u64,
    /// That resolution took longer than the timeout in force for it.
    timed_out: bool,
    /// The worker has resolved `generation` with a real outcome (live or a genuine refusal,
    /// never `open_cancelled`). Only sticky stays `false` while a timed-out opener's own note
    /// is just a placeholder a late `resolve` may still correct.
    done: bool,
}

/// The shared, read-only view of the system audio state. Clones share one value.
#[derive(Debug, Clone, Default)]
pub struct SystemAudioStatus(Arc<Shared>);

#[derive(Debug, Default)]
struct Shared {
    availability: AtomicU8,
    resolution: Mutex<Resolution>,
}

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
        lock(&self.0.resolution).open_ms
    }

    /// The last `open`'s resolution took longer than the timeout in force for it (CAP-10c
    /// diagnostic): either the caller gave up with no stream yet, or the worker's late build
    /// finished after that same wait.
    pub fn timed_out(&self) -> bool {
        lock(&self.0.resolution).timed_out
    }

    /// `open` is about to wait for `generation`: clear any diagnostics an older, unresolved
    /// generation left behind (CAP-10c), so a cancelled `open` never leaves stale numbers on
    /// the next one.
    fn begin_open(&self, generation: u64) {
        *lock(&self.0.resolution) = Resolution {
            generation,
            open_ms: 0,
            timed_out: false,
            done: false,
        };
    }

    /// The worker resolves `generation` with a real outcome (CAP-10c): write `status` and the
    /// diagnostics together, under the same lock, and mark the generation done so a
    /// concurrently timed-out `open` does not then clobber this. The `status` write happens
    /// while the lock is still held (not after releasing it): otherwise the opener's own
    /// `mark_timed_out` could run between the two and write `Unavailable` last, leaving `done`
    /// true but `status` wrong. A stale resolve for a generation a newer `open` has already
    /// superseded (`begin_open` moved `resolution.generation` on) is dropped.
    fn resolve(
        &self,
        generation: u64,
        availability: SystemAudioAvailability,
        open_ms: u64,
        timed_out: bool,
    ) {
        let mut res = lock(&self.0.resolution);
        if res.generation != generation {
            return;
        }
        res.open_ms = open_ms;
        res.timed_out = timed_out;
        res.done = true;
        self.0
            .availability
            .store(availability.to_u8(), Ordering::SeqCst);
    }

    /// `open` itself gave up waiting for `generation`. Writes `Unavailable` and the
    /// diagnostics only when the worker has not resolved this exact generation already
    /// (CAP-10c): otherwise this would clobber a `Live` (or a genuine refusal) the worker just
    /// settled. The `status` write happens under the same lock as the `done` check, for the
    /// same reason as `resolve`. Returns whether it wrote.
    fn mark_timed_out(&self, generation: u64, open_ms: u64) -> bool {
        let mut res = lock(&self.0.resolution);
        if res.generation != generation || res.done {
            return false;
        }
        res.open_ms = open_ms;
        res.timed_out = true;
        self.0.availability.store(
            SystemAudioAvailability::Unavailable.to_u8(),
            Ordering::SeqCst,
        );
        true
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
    open_timeout: Duration,
    close_wait: Duration,
    thread: Option<JoinHandle<()>>,
}

impl SystemAudioDevice {
    /// Start the `bottle-sysaudio` thread over the host that `make_host` builds there. No
    /// stream opens here. `close` waits the default [`CLOSE_WAIT`]; a test that needs a
    /// different one calls [`SystemAudioDevice::spawn_with_close_wait`] instead.
    pub fn spawn<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
        open_timeout: Duration,
    ) -> std::io::Result<(Self, SystemAudioStatus)>
    where
        H: MicHost,
        F: FnOnce() -> H + Send + 'static,
    {
        Self::spawn_with_close_wait(make_host, sink, open_timeout, CLOSE_WAIT)
    }

    /// As [`SystemAudioDevice::spawn`], but with `close_wait` instead of the default
    /// [`CLOSE_WAIT`] (CAP-10c test hook): a short `close_wait` proves that a `close` which
    /// gives up early on a long late build still ends with the stream dropped and `status` at
    /// `Off`, never re-adopted.
    pub fn spawn_with_close_wait<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
        open_timeout: Duration,
        close_wait: Duration,
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
                open_timeout,
                close_wait,
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
                open_timeout: Duration::ZERO,
                close_wait: CLOSE_WAIT,
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
        self.status.begin_open(generation);
        let (reply, answer) = mpsc::sync_channel(1);
        let started = Instant::now();
        let result = tx
            .send(Msg::Open {
                generation,
                started,
                timeout: self.open_timeout,
                reply,
            })
            .ok()
            .and_then(|()| answer.recv_timeout(self.open_timeout).ok());
        match result {
            // The worker already set `status` and the CAP-10c diagnostics before it answered,
            // on both branches: refused or failed is `Unavailable`, built and played is `Live`.
            Some(Ok(())) | Some(Err(_)) => {}
            // A timeout, not a refusal: the recording goes on mic-only for now. `wanted` stays
            // set (only `close` clears it), so a build that finishes late is still adopted by
            // the worker's own check in `build`. `mark_timed_out` only writes when the worker
            // has not resolved this generation already (CAP-10c's resolution race).
            None => {
                self.status.mark_timed_out(generation, elapsed_ms(started));
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
                let _ = done.recv_timeout(self.close_wait);
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
                    // and gone on mic-only; this corrects it either way through `resolve`,
                    // which wins any race against the opener's own `mark_timed_out`, and
                    // nothing here leaves a live stream that `status` does not know about).
                    let timed_out = started.elapsed() > timeout;
                    let answer = match result {
                        Ok(stream) => {
                            current = Some((generation, stream));
                            self.status.resolve(
                                generation,
                                SystemAudioAvailability::Live,
                                elapsed_ms(started),
                                timed_out,
                            );
                            Ok(())
                        }
                        Err(e) => {
                            // `open_cancelled`: a `close` (or a newer `open`) already ran and
                            // owns the status now (`Off`, or that generation's own outcome);
                            // do not undo it. Any other error is a genuine refusal.
                            if e != codes::OPEN_CANCELLED {
                                self.status.resolve(
                                    generation,
                                    SystemAudioAvailability::Unavailable,
                                    elapsed_ms(started),
                                    timed_out,
                                );
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
                                // `open_cancelled`: a `close` or a newer `open` ran meanwhile
                                // and owns `status` now; any other error is a genuine loss.
                                Err(e) if e != codes::OPEN_CANCELLED => {
                                    self.status.set(SystemAudioAvailability::Unavailable);
                                }
                                Err(_) => {}
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

#[cfg(test)]
mod tests {
    use super::*;

    /// CAP-10c: the worker's `resolve` lands first, then the opener's own `mark_timed_out`
    /// (late: the worker already answered). `mark_timed_out` must not write, and `status` must
    /// stay `Live` with `done` set.
    #[test]
    fn resolve_then_mark_timed_out_ends_live_with_done_set() {
        let status = SystemAudioStatus::default();
        status.begin_open(1);
        status.resolve(1, SystemAudioAvailability::Live, 100, false);
        let wrote = status.mark_timed_out(1, 200);
        assert!(
            !wrote,
            "a resolved generation must refuse a later mark_timed_out"
        );
        assert_eq!(status.get(), SystemAudioAvailability::Live);
        assert!(lock(&status.0.resolution).done);
    }

    /// CAP-10c: the opener's `mark_timed_out` lands first (the usual, on-time-timeout case),
    /// then the worker's `resolve` corrects it late. `status` must end `Live` with `done` set
    /// (the race this guards: a `resolve` that releases its lock before writing `status` could
    /// let `mark_timed_out` write `Unavailable` after it, leaving `done` true but the status
    /// wrong).
    #[test]
    fn mark_timed_out_then_resolve_ends_live_with_done_set() {
        let status = SystemAudioStatus::default();
        status.begin_open(1);
        let wrote = status.mark_timed_out(1, 50);
        assert!(wrote);
        assert_eq!(status.get(), SystemAudioAvailability::Unavailable);
        assert!(!lock(&status.0.resolution).done);
        status.resolve(1, SystemAudioAvailability::Live, 300, true);
        assert_eq!(status.get(), SystemAudioAvailability::Live);
        assert!(lock(&status.0.resolution).done);
    }

    /// A `resolve` for a generation a newer `open` has already superseded (`begin_open` moved
    /// `resolution.generation` on) must not touch the newer generation's own status or
    /// diagnostics.
    #[test]
    fn a_stale_resolve_for_a_superseded_generation_is_dropped() {
        let status = SystemAudioStatus::default();
        status.begin_open(1);
        status.begin_open(2);
        status.resolve(1, SystemAudioAvailability::Live, 999, true);
        assert_eq!(
            status.get(),
            SystemAudioAvailability::Off,
            "untouched by the stale resolve"
        );
        assert_eq!(lock(&status.0.resolution).generation, 2);
        assert!(!lock(&status.0.resolution).done);
    }
}
