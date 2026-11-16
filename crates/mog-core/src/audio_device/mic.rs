//! [`MicDevice`]: the audio thread, its messages and the failure notifier (#388).
//!
//! Locks: `control` (the open state) is taken by `open`, `close` and the audio thread. `sink`
//! is taken by the data callback and by the audio thread. The audio thread takes `sink` only
//! while it holds `control`, never the other way, and never drops a stream while it holds
//! `sink`: a host may wait for a data callback in progress when a stream drops.

use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};

use crate::capture::{AudioDevice, AudioSource, CaptureCore, DeviceError};

use super::{codes, mix_to_mono, DataFn, FaultFn, FrameSink, MicFrame, MicHost, StreamFault};

type Hook = Arc<dyn Fn(AudioSource) + Send + Sync>;

/// The link from the backend to [`CaptureCore::device_failed`]. The core owns the device, so
/// the link connects after the core is built:
///
/// ```ignore
/// let (device, link) = MicDevice::spawn(CpalMicHost::new, Box::new(sink))?;
/// let core = Arc::new(CaptureCore::new(Box::new(device), clock, events));
/// link.connect(&core);
/// ```
#[derive(Clone, Default)]
pub struct FailureLink {
    hook: Arc<Mutex<Option<Hook>>>,
}

impl FailureLink {
    /// Report failed streams to `core`. The link holds a weak reference, so it never keeps the
    /// core alive.
    pub fn connect(&self, core: &Arc<CaptureCore>) {
        let weak: Weak<CaptureCore> = Arc::downgrade(core);
        self.connect_fn(move |source| {
            if let Some(core) = weak.upgrade() {
                core.device_failed(source);
            }
        });
    }

    /// Report failed streams to `f`, in place of an earlier target. `f` runs on the notifier
    /// thread, never on the audio thread, so it may call [`AudioDevice::close`].
    pub fn connect_fn(&self, f: impl Fn(AudioSource) + Send + Sync + 'static) {
        *lock(&self.hook) = Some(Arc::new(f));
    }

    fn report(&self, source: AudioSource) {
        let hook = lock(&self.hook).clone();
        if let Some(hook) = hook {
            hook(source);
        }
    }
}

/// The open state of the mic, as `open` and `close` see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Closed,
    /// An open runs on the audio thread. `cancel` is set by a `close` meanwhile.
    Opening {
        generation: u64,
        cancel: bool,
    },
    Open {
        generation: u64,
    },
}

struct Control {
    slot: Slot,
    /// The generation of the last open. Each open gets a new one, so a late close or a late
    /// fault of an old stream never touches a newer stream.
    last_generation: u64,
}

/// The sink and the generation whose frames it takes now (`None`: no stream is open).
struct SinkSlot {
    sink: Box<dyn FrameSink>,
    live: Option<u64>,
}

enum Msg {
    Open {
        generation: u64,
        reply: SyncSender<Result<(), DeviceError>>,
    },
    Close {
        generation: u64,
        reply: SyncSender<()>,
    },
    Fault {
        generation: u64,
    },
    Shutdown,
}

fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The mic backend. See the [module docs](super).
pub struct MicDevice {
    tx: Sender<Msg>,
    control: Arc<Mutex<Control>>,
    audio: Option<JoinHandle<()>>,
    notifier: Option<JoinHandle<()>>,
}

impl MicDevice {
    /// Start the audio thread and the failure notifier thread. `make_host` runs on the audio
    /// thread, so the host need not be `Send`. No stream opens here (MOG-09).
    pub fn spawn<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
    ) -> std::io::Result<(Self, FailureLink)>
    where
        H: MicHost,
        F: FnOnce() -> H + Send + 'static,
    {
        let link = FailureLink::default();
        let (notify_tx, notify_rx) = mpsc::channel::<AudioSource>();
        let notifier = {
            let link = link.clone();
            thread::Builder::new()
                .name("bottle-mic-notify".into())
                .spawn(move || {
                    // Ends when the audio thread ends and drops `notify_tx`.
                    for source in notify_rx {
                        link.report(source);
                    }
                })?
        };
        let control = Arc::new(Mutex::new(Control {
            slot: Slot::Closed,
            last_generation: 0,
        }));
        let (tx, rx) = mpsc::channel::<Msg>();
        let audio = {
            let thread = AudioThread {
                control: Arc::clone(&control),
                sink: Arc::new(Mutex::new(SinkSlot { sink, live: None })),
                tx: tx.clone(),
                notify: notify_tx,
            };
            thread::Builder::new()
                .name("bottle-mic-audio".into())
                .spawn(move || thread.run(make_host(), rx))?
        };
        Ok((
            Self {
                tx,
                control,
                audio: Some(audio),
                notifier: Some(notifier),
            },
            link,
        ))
    }

    /// Set the slot to closed if it still holds `generation` (the audio thread is gone).
    fn forget(&self, generation: u64) {
        let mut c = lock(&self.control);
        match c.slot {
            Slot::Opening { generation: g, .. } | Slot::Open { generation: g }
                if g == generation =>
            {
                c.slot = Slot::Closed;
            }
            _ => {}
        }
    }
}

impl AudioDevice for MicDevice {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        if source == AudioSource::SystemAudio {
            return Err(codes::SYSTEM_AUDIO_UNSUPPORTED);
        }
        let generation = {
            let mut c = lock(&self.control);
            match c.slot {
                Slot::Open { .. } => return Ok(()),
                // R2 never opens a source twice at once.
                Slot::Opening { .. } => return Err(codes::DEVICE_BUSY),
                Slot::Closed => {}
            }
            c.last_generation += 1;
            let generation = c.last_generation;
            c.slot = Slot::Opening {
                generation,
                cancel: false,
            };
            generation
        };
        let (reply, answer) = mpsc::sync_channel(1);
        let sent = self.tx.send(Msg::Open { generation, reply });
        match sent.ok().and_then(|()| answer.recv().ok()) {
            Some(result) => result,
            None => {
                self.forget(generation);
                Err(codes::AUDIO_THREAD_GONE)
            }
        }
    }

    fn close(&self, source: AudioSource) {
        if source == AudioSource::SystemAudio {
            return;
        }
        let generation = {
            let mut c = lock(&self.control);
            match &mut c.slot {
                Slot::Closed => return,
                // R2 never gets here (one device call at a time). For any other caller: do
                // not wait for a slow open (MB-05); the open drops its stream before it returns.
                // So this close returns before the stream stops: during `play()` the OS mic
                // indicator stays on until that open returns (ADR 0038 decision 5).
                Slot::Opening { cancel, .. } => {
                    *cancel = true;
                    return;
                }
                Slot::Open { generation } => *generation,
            }
        };
        let (reply, done) = mpsc::sync_channel(1);
        let sent = self.tx.send(Msg::Close { generation, reply });
        if sent.ok().and_then(|()| done.recv().ok()).is_none() {
            // The audio thread is gone; its stream dropped as the thread unwound.
            self.forget(generation);
        }
    }
}

impl Drop for MicDevice {
    /// Stop the audio thread. An open stream drops with it.
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Shutdown);
        let me = thread::current().id();
        for handle in [self.audio.take(), self.notifier.take()]
            .into_iter()
            .flatten()
        {
            // The notifier may drop the last core (and so this device) in `device_failed`.
            // It cannot join itself; it ends on its own once the audio thread has ended.
            if handle.thread().id() != me {
                let _ = handle.join();
            }
        }
    }
}

/// The state of the audio thread.
struct AudioThread {
    control: Arc<Mutex<Control>>,
    sink: Arc<Mutex<SinkSlot>>,
    /// For the fault callbacks of the streams.
    tx: Sender<Msg>,
    notify: Sender<AudioSource>,
}

impl AudioThread {
    fn run<H: MicHost>(self, mut host: H, rx: Receiver<Msg>) {
        let mut current: Option<(u64, H::Stream)> = None;
        for msg in rx.iter() {
            match msg {
                Msg::Open { generation, reply } => {
                    let result = self.open(&mut host, generation, &mut current);
                    let _ = reply.send(result);
                }
                Msg::Close { generation, reply } => {
                    if current.as_ref().is_some_and(|(g, _)| *g == generation) {
                        self.stop(&mut current);
                    }
                    let _ = reply.send(());
                }
                Msg::Fault { generation } => {
                    if current.as_ref().is_some_and(|(g, _)| *g == generation) {
                        self.stop(&mut current);
                        let _ = self.notify.send(AudioSource::Mic);
                    }
                }
                Msg::Shutdown => break,
            }
        }
        self.stop(&mut current);
    }

    /// Build and start a stream. On success it becomes `current`.
    fn open<H: MicHost>(
        &self,
        host: &mut H,
        generation: u64,
        current: &mut Option<(u64, H::Stream)>,
    ) -> Result<(), DeviceError> {
        // `close` turns an open slot into a Close message, so no stream is current here.
        debug_assert!(current.is_none());
        let stream = match host.build(self.on_data(generation), self.on_fault(generation)) {
            Ok(stream) => stream,
            Err(e) => {
                self.set_closed(generation);
                return Err(e);
            }
        };
        if self.cancelled(generation) {
            self.discard(generation, stream);
            return Err(codes::OPEN_CANCELLED);
        }
        if let Err(e) = host.play(&stream) {
            self.discard(generation, stream);
            return Err(e);
        }
        // Check and publish under one lock, so a close either cancels this open or sees it
        // open and sends a Close message.
        let mut c = lock(&self.control);
        if matches!(c.slot, Slot::Opening { generation: g, cancel: true } if g == generation) {
            drop(stream);
            c.slot = Slot::Closed;
            return Err(codes::OPEN_CANCELLED);
        }
        c.slot = Slot::Open { generation };
        // Frames flow from here on. The few ms between play() and this point are dropped, so
        // a cancelled open never hands audio to the sink.
        lock(&self.sink).live = Some(generation);
        *current = Some((generation, stream));
        Ok(())
    }

    fn cancelled(&self, generation: u64) -> bool {
        matches!(lock(&self.control).slot, Slot::Opening { generation: g, cancel: true } if g == generation)
    }

    /// Drop the stream of an open that does not go on, under the control lock, so no
    /// `close` returns before the drop.
    fn discard<S>(&self, generation: u64, stream: S) {
        let mut c = lock(&self.control);
        drop(stream);
        if matches!(c.slot, Slot::Opening { generation: g, .. } if g == generation) {
            c.slot = Slot::Closed;
        }
    }

    fn set_closed(&self, generation: u64) {
        let mut c = lock(&self.control);
        if matches!(c.slot, Slot::Opening { generation: g, .. } if g == generation) {
            c.slot = Slot::Closed;
        }
    }

    /// Stop the current stream: no more frames, then drop it (the OS indicator goes off),
    /// then mark the slot closed. A `close` that waits on the control lock returns only
    /// after the drop.
    fn stop<S>(&self, current: &mut Option<(u64, S)>) {
        let Some((_, stream)) = current.take() else {
            return;
        };
        let mut c = lock(&self.control);
        lock(&self.sink).live = None;
        drop(stream);
        c.slot = Slot::Closed;
    }

    fn on_data(&self, generation: u64) -> DataFn {
        let sink = Arc::clone(&self.sink);
        let mut mono: Vec<f32> = Vec::new();
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

    fn on_fault(&self, generation: u64) -> FaultFn {
        let tx = self.tx.clone();
        let mut sent = false;
        Box::new(move |fault| {
            if fault == StreamFault::Fatal && !sent {
                sent = true;
                let _ = tx.send(Msg::Fault { generation });
            }
        })
    }
}
