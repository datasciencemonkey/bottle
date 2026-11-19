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
//! - **Timeout.** `open` waits at most `open_timeout` for the build. A late build is dropped
//!   by the thread when it returns, so no tap stays on.
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

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::capture::{AudioDevice, AudioSource, DeviceError};

use super::{codes, mix_to_mono, DataFn, FaultFn, FrameSink, MicFrame, MicHost, StreamFault};

/// How long `open` waits for the system audio stream by default.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(8);
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

/// The shared, read-only view of the system audio state. Clones share one value.
#[derive(Debug, Clone, Default)]
pub struct SystemAudioStatus(Arc<AtomicU8>);

impl SystemAudioStatus {
    /// The state now.
    pub fn get(&self) -> SystemAudioAvailability {
        SystemAudioAvailability::from_u8(self.0.load(Ordering::SeqCst))
    }

    fn set(&self, v: SystemAudioAvailability) {
        self.0.store(v.to_u8(), Ordering::SeqCst);
    }
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Msg {
    Open {
        generation: u64,
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
    /// The generation that `open` wants now; `None` after a close or a timeout.
    wanted: Arc<Mutex<Option<u64>>>,
    next: Mutex<u64>,
    status: SystemAudioStatus,
    open_timeout: Duration,
    thread: Option<JoinHandle<()>>,
}

impl SystemAudioDevice {
    /// Start the `bottle-sysaudio` thread over the host that `make_host` builds there. No
    /// stream opens here.
    pub fn spawn<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
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
        let (reply, answer) = mpsc::sync_channel(1);
        let result = tx
            .send(Msg::Open { generation, reply })
            .ok()
            .and_then(|()| answer.recv_timeout(self.open_timeout).ok());
        match result {
            Some(Ok(())) => self.status.set(SystemAudioAvailability::Live),
            // Refused, failed or timed out: the recording goes on mic-only.
            _ => {
                let mut wanted = lock(&self.wanted);
                if *wanted == Some(generation) {
                    *wanted = None;
                }
                self.status.set(SystemAudioAvailability::Unavailable);
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
                Msg::Open { generation, reply } => {
                    self.drop_stream(&mut current);
                    let result = self.build(&mut host, generation);
                    let answer = match result {
                        Ok(stream) => {
                            current = Some((generation, stream));
                            Ok(())
                        }
                        Err(e) => Err(e),
                    };
                    // The opener may have timed out: then nobody reads the reply.
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

    /// Build and play a stream for `generation`, unless a close or a timeout cancelled it
    /// meanwhile; then the late stream drops here.
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
