//! [`MicDevice`]: the audio thread, its messages and the failure notifier (#388).
//!
//! Locks: `control` (the open state) is taken by `open`, `close` and the audio thread. `sink`
//! is taken by the data callback and by the audio thread. The audio thread takes `sink` only
//! while it holds `control`, never the other way, and never drops a stream while it holds
//! `sink`: a host may wait for a data callback in progress when a stream drops.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::capture::{AudioDevice, AudioSource, CaptureCore, DeviceError};

use super::{
    codes, mix_to_mono, DataFn, DevicePreference, FaultFn, FrameSink, MicFrame, MicHost, SinkEvent,
    StreamFault, ZeroWatch,
};

type Hook = Arc<dyn Fn(AudioSource, MicFault) + Send + Sync>;

/// Why the backend stopped a stream on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicFault {
    /// The stream failed or the device went away, and no new default could open.
    Lost,
    /// The mic gave only zero buffers for the zero window (CAP-17b), for example because the
    /// mic permission is denied. The MOG shows the fix-it card.
    ZeroBuffers,
}

/// The settings of a [`MicDevice`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MicConfig {
    /// The zero-buffer window (CAP-17b): 2 s.
    pub zero_window: Duration,
    /// How often an open stream reads the default input (CAP-18): 200 ms, so a reopen
    /// starts in 200 ms or less after the change.
    pub device_poll: Duration,
    /// How long a lost stream waits for a new device before it stops (section 7.2): 10 s.
    pub lost_grace: Duration,
}

impl Default for MicConfig {
    fn default() -> Self {
        Self {
            zero_window: Duration::from_secs(2),
            device_poll: Duration::from_millis(200),
            lost_grace: Duration::from_secs(10),
        }
    }
}

fn ms(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

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
        self.connect_fault_fn(move |source, fault| {
            if let Some(core) = weak.upgrade() {
                match fault {
                    MicFault::Lost => core.device_failed(source),
                    // TODO(#453): `capture.state` carries the mic fault to the fix-it card.
                    MicFault::ZeroBuffers => core.mic_silent(),
                }
            }
        });
    }

    /// Report failed streams to `f`, in place of an earlier target. `f` runs on the notifier
    /// thread, never on the audio thread, so it may call [`AudioDevice::close`].
    pub fn connect_fn(&self, f: impl Fn(AudioSource) + Send + Sync + 'static) {
        self.connect_fault_fn(move |source, _| f(source));
    }

    /// Like [`FailureLink::connect_fn`], with the reason of the stop.
    pub fn connect_fault_fn(&self, f: impl Fn(AudioSource, MicFault) + Send + Sync + 'static) {
        *lock(&self.hook) = Some(Arc::new(f));
    }

    fn report(&self, source: AudioSource, fault: MicFault) {
        let hook = lock(&self.hook).clone();
        if let Some(hook) = hook {
            hook(source, fault);
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

/// The sink, the stream whose frames it takes now (`None`: no stream is open) and the zero
/// watch of that stream.
struct SinkSlot {
    sink: Box<dyn FrameSink>,
    /// The generation (one per `open`) and the epoch (one per built stream) of the live stream.
    live: Option<(u64, u64)>,
    watch: Option<ZeroWatch>,
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
        epoch: u64,
        fault: StreamFault,
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
    prefer_built_in: Arc<AtomicBool>,
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
        Self::spawn_with(make_host, sink, MicConfig::default())
    }

    /// [`MicDevice::spawn`] with explicit settings (tests use a short zero window).
    pub fn spawn_with<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
        config: MicConfig,
    ) -> std::io::Result<(Self, FailureLink)>
    where
        H: MicHost,
        F: FnOnce() -> H + Send + 'static,
    {
        let link = FailureLink::default();
        let (notify_tx, notify_rx) = mpsc::channel::<(AudioSource, MicFault)>();
        let notifier = {
            let link = link.clone();
            thread::Builder::new()
                .name("bottle-mic-notify".into())
                .spawn(move || {
                    // Ends when the audio thread ends and drops `notify_tx`.
                    for (source, fault) in notify_rx {
                        link.report(source, fault);
                    }
                })?
        };
        let control = Arc::new(Mutex::new(Control {
            slot: Slot::Closed,
            last_generation: 0,
        }));
        let prefer_built_in = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel::<Msg>();
        let audio = {
            let thread = AudioThread {
                control: Arc::clone(&control),
                sink: Arc::new(Mutex::new(SinkSlot {
                    sink,
                    live: None,
                    watch: None,
                })),
                tx: tx.clone(),
                notify: notify_tx,
                prefer_built_in: Arc::clone(&prefer_built_in),
                base: Instant::now(),
                zero_window_ms: ms(config.zero_window),
                device_poll_ms: ms(config.device_poll),
                lost_grace_ms: ms(config.lost_grace),
                epoch: 0,
                default_id: None,
                next_poll_ms: 0,
                retry: None,
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
                prefer_built_in,
            },
            link,
        ))
    }

    /// "Prefer the built-in mic when a Bluetooth headset connects" (CAP-22). It applies at the
    /// next stream build; it asks the OS for nothing (CAP-17a). TODO(#453): the host reads
    /// the app setting and calls this.
    pub fn set_prefer_built_in(&self, prefer: bool) {
        self.prefer_built_in.store(prefer, Ordering::SeqCst);
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
    notify: Sender<(AudioSource, MicFault)>,
    prefer_built_in: Arc<AtomicBool>,
    /// The origin of the zero-watch clock.
    base: Instant,
    zero_window_ms: u64,
    device_poll_ms: u64,
    lost_grace_ms: u64,
    /// The epoch of the last built stream. A reopen (CAP-18) keeps the generation and takes a
    /// new epoch, so a late fault or frame of the old stream never touches the new one.
    epoch: u64,
    /// The opaque ID of the default input at the last build. It is compared, never logged.
    default_id: Option<String>,
    /// When to read the default input again (CAP-18).
    next_poll_ms: u64,
    /// A reopen that waits for a new device: the generation, when the device went, and
    /// when to try again. The slot stays open meanwhile.
    retry: Option<Retry>,
}

#[derive(Debug, Clone, Copy)]
struct Retry {
    generation: u64,
    since_ms: u64,
    next_ms: u64,
}

/// How often a reopen that waits for a device tries again.
const RETRY_EVERY_MS: u64 = 250;

/// The live stream: its generation, its epoch and the stream.
type Current<S> = Option<(u64, u64, S)>;

fn ms_since(base: Instant) -> u64 {
    u64::try_from(base.elapsed().as_millis()).unwrap_or(u64::MAX)
}

impl AudioThread {
    fn run<H: MicHost>(mut self, mut host: H, rx: Receiver<Msg>) {
        let mut current: Current<H::Stream> = None;
        loop {
            let msg = match self.next_wake(current.is_some()) {
                Some(at) => {
                    let wait = at.saturating_sub(self.now_ms()).max(1);
                    match rx.recv_timeout(Duration::from_millis(wait)) {
                        Ok(msg) => msg,
                        Err(RecvTimeoutError::Timeout) => {
                            self.on_timer(&mut host, &mut current);
                            continue;
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                }
                None => match rx.recv() {
                    Ok(msg) => msg,
                    Err(_) => break,
                },
            };
            match msg {
                Msg::Open { generation, reply } => {
                    let result = self.open(&mut host, generation, &mut current);
                    let _ = reply.send(result);
                }
                Msg::Close { generation, reply } => {
                    if current.as_ref().is_some_and(|(g, _, _)| *g == generation) {
                        self.stop(&mut current);
                    }
                    if self.retry.is_some_and(|r| r.generation == generation) {
                        self.give_up(false);
                    }
                    let _ = reply.send(());
                }
                Msg::Fault {
                    generation,
                    epoch,
                    fault,
                } => {
                    if current
                        .as_ref()
                        .is_some_and(|(g, e, _)| *g == generation && *e == epoch)
                    {
                        self.on_stream_fault(&mut host, fault, &mut current);
                    }
                }
                Msg::Shutdown => break,
            }
        }
        self.stop(&mut current);
        self.retry = None;
    }

    fn now_ms(&self) -> u64 {
        ms_since(self.base)
    }

    /// The next timer: the zero-watch deadline and the default-device poll while a stream is
    /// open, or the next retry while a reopen waits. `None`: wait for a message only.
    fn next_wake(&self, open: bool) -> Option<u64> {
        if let Some(r) = self.retry {
            return Some(r.next_ms);
        }
        if !open {
            return None;
        }
        let watch = lock(&self.sink).watch.map_or(u64::MAX, |w| w.deadline_ms());
        let poll = if self.default_id.is_some() {
            self.next_poll_ms
        } else {
            u64::MAX
        };
        Some(watch.min(poll))
    }

    fn on_timer<H: MicHost>(&mut self, host: &mut H, current: &mut Current<H::Stream>) {
        if self.retry.is_some() {
            self.try_again(host, current);
            return;
        }
        if self.check_zero(current) {
            return;
        }
        let now = self.now_ms();
        if self.default_id.is_some() && now >= self.next_poll_ms {
            self.next_poll_ms = now.saturating_add(self.device_poll_ms);
            // CAP-18: an input stream stays on its device (CoreAudio and WASAPI), so a new
            // default input is found here and the stream opens again on it.
            if host.default_input_id() != self.default_id {
                self.reopen(host, current);
            }
        }
    }

    /// CAP-17b: stop a stream that gave no sound for the zero window, tell the sink to drop
    /// its audio (no transcript) and report the fault for the fix-it card. The check and the
    /// stop run under one hold of the control lock.
    fn check_zero<S>(&self, current: &mut Current<S>) -> bool {
        let now = self.now_ms();
        let mut c = lock(&self.control);
        {
            let mut slot = lock(&self.sink);
            if !slot.watch.is_some_and(|w| w.expired(now)) {
                return false;
            }
            slot.live = None;
            slot.watch = None;
        }
        if let Some((_, _, stream)) = current.take() {
            drop(stream);
        }
        c.slot = Slot::Closed;
        drop(c);
        lock(&self.sink).sink.event(SinkEvent::Discard);
        let _ = self.notify.send((AudioSource::Mic, MicFault::ZeroBuffers));
        true
    }

    fn on_stream_fault<H: MicHost>(
        &mut self,
        host: &mut H,
        fault: StreamFault,
        current: &mut Current<H::Stream>,
    ) {
        match fault {
            StreamFault::Transient => {}
            StreamFault::Rerouted => lock(&self.sink).sink.event(SinkEvent::Gap { gap_ms: 0 }),
            // CAP-18 and section 7.2: the stream is gone. Open again on the default device,
            // and wait up to the lost grace (10 s) for one to come. A host that cannot name
            // its default device (no poll) reports the loss at once, as in ADR 0038.
            StreamFault::Invalidated => self.reopen(host, current),
            StreamFault::Fatal if self.default_id.is_some() => self.reopen(host, current),
            StreamFault::Fatal => {
                self.stop(current);
                let _ = self.notify.send((AudioSource::Mic, MicFault::Lost));
            }
        }
    }

    /// CAP-18: drop the old stream and build and play a new one on the default device,
    /// under the same generation (the same lease). The slot stays open, so a `close`
    /// meanwhile queues a Close message that stops the new stream. The sink keeps what it
    /// holds. If no stream opens, retry until the lost grace ends.
    fn reopen<H: MicHost>(&mut self, host: &mut H, current: &mut Current<H::Stream>) {
        let Some((generation, _, old)) = current.take() else {
            return;
        };
        {
            let _c = lock(&self.control);
            let mut slot = lock(&self.sink);
            slot.live = None;
            slot.watch = None;
            drop(slot);
            drop(old);
        }
        let now = self.now_ms();
        self.retry = Some(Retry {
            generation,
            since_ms: now,
            next_ms: now,
        });
        self.try_again(host, current);
    }

    fn try_again<H: MicHost>(&mut self, host: &mut H, current: &mut Current<H::Stream>) {
        let Some(retry) = self.retry else {
            return;
        };
        match self.build_and_play(host, retry.generation) {
            Ok((epoch, stream)) => {
                self.retry = None;
                let _c = lock(&self.control);
                let now = self.now_ms();
                let mut slot = lock(&self.sink);
                slot.sink.event(SinkEvent::Gap {
                    gap_ms: now.saturating_sub(retry.since_ms),
                });
                slot.live = Some((retry.generation, epoch));
                slot.watch = Some(ZeroWatch::new(self.zero_window_ms, now));
                drop(slot);
                *current = Some((retry.generation, epoch, stream));
            }
            Err(_) => {
                let now = self.now_ms();
                if now.saturating_sub(retry.since_ms) >= self.lost_grace_ms {
                    self.give_up(true);
                } else {
                    self.retry = Some(Retry {
                        next_ms: now.saturating_add(RETRY_EVERY_MS),
                        ..retry
                    });
                }
            }
        }
    }

    /// End a waiting reopen: close the slot, and report the device lost unless a close asked.
    fn give_up(&mut self, report: bool) {
        let Some(retry) = self.retry.take() else {
            return;
        };
        {
            let mut c = lock(&self.control);
            if matches!(c.slot, Slot::Open { generation: g } if g == retry.generation) {
                c.slot = Slot::Closed;
            }
        }
        if report {
            let _ = self.notify.send((AudioSource::Mic, MicFault::Lost));
        }
    }

    /// Note the default input, set the preference, then build and start a stream with a new
    /// epoch. A failed play drops its stream before this returns.
    fn build_and_play<H: MicHost>(
        &mut self,
        host: &mut H,
        generation: u64,
    ) -> Result<(u64, H::Stream), DeviceError> {
        self.note_default(host);
        self.epoch += 1;
        let epoch = self.epoch;
        host.set_preference(DevicePreference {
            prefer_built_in: self.prefer_built_in.load(Ordering::SeqCst),
        });
        let stream = host.build(
            self.on_data(generation, epoch),
            self.on_fault(generation, epoch),
        )?;
        host.play(&stream)?;
        Ok((epoch, stream))
    }

    fn note_default<H: MicHost>(&mut self, host: &mut H) {
        self.default_id = host.default_input_id();
        self.next_poll_ms = self.now_ms().saturating_add(self.device_poll_ms);
    }

    fn open<H: MicHost>(
        &mut self,
        host: &mut H,
        generation: u64,
        current: &mut Current<H::Stream>,
    ) -> Result<(), DeviceError> {
        // `close` turns an open slot into a Close message, so no stream is current here.
        debug_assert!(current.is_none());
        self.note_default(host);
        self.epoch += 1;
        let epoch = self.epoch;
        host.set_preference(DevicePreference {
            prefer_built_in: self.prefer_built_in.load(Ordering::SeqCst),
        });
        let stream = match host.build(
            self.on_data(generation, epoch),
            self.on_fault(generation, epoch),
        ) {
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
        // a cancelled open never hands audio to the sink. The zero watch starts here.
        let now = self.now_ms();
        let mut slot = lock(&self.sink);
        slot.live = Some((generation, epoch));
        slot.watch = Some(ZeroWatch::new(self.zero_window_ms, now));
        drop(slot);
        *current = Some((generation, epoch, stream));
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
    fn stop<S>(&self, current: &mut Current<S>) {
        let Some((_, _, stream)) = current.take() else {
            return;
        };
        let mut c = lock(&self.control);
        {
            let mut slot = lock(&self.sink);
            slot.live = None;
            slot.watch = None;
        }
        drop(stream);
        c.slot = Slot::Closed;
    }

    fn on_data(&self, generation: u64, epoch: u64) -> DataFn {
        let sink = Arc::clone(&self.sink);
        let base = self.base;
        let mut mono: Vec<f32> = Vec::new();
        Box::new(move |block| {
            let mut slot = lock(&sink);
            if slot.live != Some((generation, epoch)) {
                return;
            }
            mix_to_mono(block.samples, block.channels, &mut mono);
            let frame = MicFrame {
                samples: &mono,
                sample_rate: block.sample_rate,
            };
            let silent = frame.is_digital_silence();
            if let Some(watch) = slot.watch.as_mut() {
                watch.block(silent, ms_since(base));
            }
            slot.sink.frame(frame);
        })
    }

    fn on_fault(&self, generation: u64, epoch: u64) -> FaultFn {
        let tx = self.tx.clone();
        let mut ended = false;
        Box::new(move |fault| {
            // A fatal or invalidated stream reports once; a reroute reports each time.
            if ended || fault == StreamFault::Transient {
                return;
            }
            ended = matches!(fault, StreamFault::Fatal | StreamFault::Invalidated);
            let _ = tx.send(Msg::Fault {
                generation,
                epoch,
                fault,
            });
        })
    }
}
