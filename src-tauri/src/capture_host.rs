//! The host side of R2 CaptureService (#425, ADR 0055).
//!
//! One [`CaptureHost`] per process owns the one [`CaptureCore`]. It answers the `capture_*`
//! commands that the hardened gateway checked, keeps the owner MOG of each lease (so a MOG can
//! stop or pause only its own lease, ADR 0007), relays the core's events to the brain as
//! `capture:state` and `capture:stopped`, and runs the [`CapTimer`] that applies the 120 s hold
//! and 5 min hands-free caps (CAP-04a, CAP-04b).
//!
//! No audio, transcript or token is in any reply, event or log (CAP-01b): the replies and
//! events hold only the capture state, lease IDs, MOG IDs, modes and stop reasons.
//!
//! **The level stream (CAP-16a, CAP-16b, #476, ADR 0067).** [`spawn_level_mic`] gives the
//! [`MicDevice`] a `TeeSink` of the `LevelSink` and the PTT buffer. [`CaptureHost::with_levels`]
//! builds the core with `LevelCaptureEvents`, so the [`LevelGate`] is open only while a capture
//! records. `capture_subscribe` resolves its `onLevel` reference to a [`LevelOut`] (in the app a
//! [`ChannelLevelOut`] on the brain webview, [`brain_level_resolver`]) and puts it in the gate.
//! Only the dBFS number crosses IPC; no level value is logged. TODO(#475): a level carries no
//! capture ID, so a late level can reach the next target MOG.
//!
//! **The RAM-only PTT buffer (CAP-12c, CAP-13a, CAP-34, #21, ADR 0078).** The PTT half of the
//! `TeeSink` is a [`PttBuffer`] sink. [`CaptureHost::with_audio`] puts the same buffer in the
//! core's event chain with `PttCaptureEvents`, so a take starts only when a PTT or hands-free
//! lease records, and each stop hands one utterance to the buffer's consumer (R4
//! SttEngine, #46) or wipes it. [`CaptureHost::with_stt`] also puts R4 in the chain (ADR 0082).
//! [`BrainSegments`] sends one `capture:segment` to the brain for each R4 result (#514).

use mog_core::app_settings::AppSettings;
use mog_core::audio_device::level::{
    LevelCaptureEvents, LevelGate, LevelOut, LevelSink, NoLevelOut, TeeSink,
};
use mog_core::audio_device::ptt_buffer::{NoUtteranceSink, PttBuffer, PttCaptureEvents};
use mog_core::audio_device::{FailureLink, FrameSink, MicDevice, MicHost};
use mog_core::capture::{
    parse_lease, AudioDevice, AudioSource, CaptureCore, CaptureError, CaptureEvents,
    CaptureRequest, CaptureService, DeviceError, LeaseNo, Stopped,
};
use mog_core::clock::MonotonicClock;
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::{
    events, CaptureLeaseArgs, CaptureMode, CaptureStartArgs, CaptureSubscribeArgs, ChannelRef,
    IpcError, MogId,
};
use mog_core::ipc_gateway::ipc_error_code;
use mog_core::stt_engine::{self, LocalSttEngine, SttCaptureEvents, SttDone, TranscriptSink};
use mog_core::types::{CaptureState, CaptureStopped, SegmentRef, Track};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

/// The brain relay: deliver one core-to-brain item, `false` if no brain listens. The app
/// passes `BrainChannel::send`, which scrubs each payload (SEC-01).
pub type BrainRelay = Arc<dyn Fn(&str, Value) -> bool + Send + Sync>;

/// The error of [`UnwiredMic`]: no mic backend is built into this app yet.
pub const UNWIRED_MIC: DeviceError = DeviceError { code: "no_device" };

/// The mic device on a platform with no mic backend (the cpal backend of #388 is for macOS
/// and Windows only). Every open fails with [`UNWIRED_MIC`], so `capture_start` answers
/// `host_failed` and no lease is granted.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnwiredMic;

impl AudioDevice for UnwiredMic {
    fn open(&self, _source: AudioSource) -> Result<(), DeviceError> {
        Err(UNWIRED_MIC)
    }
    fn close(&self, _source: AudioSource) {}
}

/// The capture device from a spawned [`MicDevice`] (#388), with its failure link.
///
/// CAP-22 (#453, ADR 0063): `settings` gets the preference handle of the device, so each
/// write of `app.capture.preferBuiltInMic` reaches the device that the capture core holds, and
/// the device starts with the value in force. A mic that did not spawn is the [`UnwiredMic`]:
/// the app still runs, and every start answers `host_failed`.
pub fn mic_capture_device(
    spawned: std::io::Result<(MicDevice, FailureLink)>,
    settings: Option<&AppSettings>,
) -> (Box<dyn AudioDevice>, Option<FailureLink>) {
    match spawned {
        Ok((mic, link)) => {
            if let Some(settings) = settings {
                settings.set_mic_preference(Arc::new(mic.preference()));
            }
            (Box::new(mic), Some(link))
        }
        Err(_) => (Box::new(UnwiredMic), None),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn refused(e: IpcError) -> CoreError {
    CoreError::IpcRefused {
        code: ipc_error_code(&e),
    }
}

fn capture_error(command: &str, e: CaptureError) -> CoreError {
    refused(e.to_ipc(command))
}

fn invalid_args(command: &str) -> CoreError {
    refused(IpcError::InvalidArgs {
        command: command.to_owned(),
        reason: mog_core::ipc::INVALID_ARGS_REASON.to_owned(),
    })
}

fn args_of<T: serde::de::DeserializeOwned>(command: &str, args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| invalid_args(command))
}

fn to_value<T: serde::Serialize>(command: &str, v: &T) -> CoreResult<Value> {
    serde_json::to_value(v).map_err(|_| {
        refused(IpcError::HostFailed {
            command: command.to_owned(),
        })
    })
}

// ---------- lease owners ----------

/// The owner MOG of each live lease. The sink removes a lease when it stops. A stop can arrive
/// before `capture_start` records its grant (a `stop_all` or a cap on another thread), so such
/// a lease is kept in `gone` and the grant then records nothing.
#[derive(Default)]
struct LeaseOwners {
    live: HashMap<LeaseNo, MogId>,
    gone: HashSet<LeaseNo>,
    /// The pause number of the last `capture_pause` of each lease (ADR 0055, decision 7).
    paused_at: HashMap<LeaseNo, u64>,
}

impl LeaseOwners {
    fn granted(&mut self, lease: LeaseNo, mog: MogId) {
        if !self.gone.remove(&lease) {
            self.live.insert(lease, mog);
        }
    }

    fn stopped(&mut self, lease: LeaseNo) {
        self.paused_at.remove(&lease);
        if self.live.remove(&lease).is_none() {
            self.gone.insert(lease);
        }
    }

    fn owner(&self, lease: LeaseNo) -> Option<&str> {
        self.live.get(&lease).map(String::as_str)
    }
}

// ---------- the event sink ----------

/// The [`CaptureEvents`] sink of the host: `capture:state` and `capture:stopped` go to the brain
/// through the brain relay. It also forgets the owner of each stopped lease.
pub struct BrainCaptureEvents {
    relay: BrainRelay,
    owners: Arc<Mutex<LeaseOwners>>,
}

impl CaptureEvents for BrainCaptureEvents {
    fn state(&self, state: &CaptureState) {
        if let Ok(payload) = serde_json::to_value(state) {
            // No brain yet: the brain reads `capture_status` when it subscribes.
            let _ = (self.relay)(events::CAPTURE_STATE, payload);
        }
    }

    fn stopped(&self, stopped: &Stopped) {
        lock(&self.owners).stopped(stopped.lease_id);
        if let Ok(payload) = serde_json::to_value(CaptureStopped::from(stopped)) {
            let _ = (self.relay)(events::CAPTURE_STOPPED, payload);
        }
    }
}

// ---------- R4 results ----------

/// The [`TranscriptSink`] of R4 in the app (#514, ADR 0082 amendment 1): each R4 result sends
/// one final `capture:segment` for the user's track to the brain, which then calls
/// `stt_transcribe` with its `segmentId` (F1 steps 5 and 8). R4's store keeps the result before
/// this sink runs, so that call finds it. A failed result also gets the event, so
/// `stt_transcribe` answers its code. The event holds no text and no audio (CAP-01b).
pub struct BrainSegments {
    relay: BrainRelay,
}

impl BrainSegments {
    /// Send each R4 result on `relay` (the brain relay of the capture host).
    pub fn new(relay: BrainRelay) -> Self {
        Self { relay }
    }
}

impl TranscriptSink for BrainSegments {
    fn done(&self, done: SttDone) {
        let segment = SegmentRef {
            segment_id: stt_engine::segment_id(done.lease_id),
            mog_id: done.mog_id,
            lease_id: done.lease_id.to_string(),
            session_id: None,
            track: Track::You,
            // One segment per lease: its times are relative to the lease's first frame.
            t_start_ms: 0,
            t_end_ms: done.duration_ms,
            is_final: true,
            directed_to: None,
        };
        if let Ok(payload) = serde_json::to_value(segment) {
            // No brain yet: the result waits in R4's store, and the next result pushes it out.
            let _ = (self.relay)(events::CAPTURE_SEGMENT, payload);
        }
    }
}

// ---------- the level stream ----------

/// Turns a `capture_subscribe` `onLevel` reference into the receiver of the levels. `None`:
/// the reference does not resolve (for example the webview is gone).
pub type LevelResolver = Arc<dyn Fn(&ChannelRef) -> Option<Arc<dyn LevelOut>> + Send + Sync>;

/// A [`LevelOut`] over a Tauri `Channel`: each level crosses as one JSON number (CAP-01b).
/// After a failed send it is dead and sends nothing more, so the audio thread does not retry
/// a gone webview at 30 Hz. The next `capture_subscribe` replaces it.
pub struct ChannelLevelOut {
    channel: tauri::ipc::Channel<f32>,
    dead: AtomicBool,
}

impl ChannelLevelOut {
    /// Send the levels on `channel`.
    pub fn new(channel: tauri::ipc::Channel<f32>) -> Self {
        Self {
            channel,
            dead: AtomicBool::new(false),
        }
    }
}

impl LevelOut for ChannelLevelOut {
    fn level(&self, dbfs: f32) {
        if self.dead.load(Ordering::Relaxed) {
            return;
        }
        // A failed send means the webview is gone. Nothing is logged (no level value in logs).
        if self.channel.send(dbfs).is_err() {
            self.dead.store(true, Ordering::Relaxed);
        }
    }
}

/// The app resolver: the reference is a Channel of the `brain` webview. The gateway lets only
/// the brain call `capture_subscribe` (ADR 0040), so the caller is that webview.
pub fn brain_level_resolver<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> LevelResolver {
    use std::str::FromStr;
    use tauri::Manager;
    let brain = crate::windows::WindowKind::Brain.label();
    Arc::new(move |c: &ChannelRef| {
        let id = tauri::ipc::JavaScriptChannelId::from_str(c.as_str()).ok()?;
        let webview = app.get_webview_window(&brain)?;
        let channel = id.channel_on(webview.as_ref().clone());
        Some(Arc::new(ChannelLevelOut::new(channel)) as Arc<dyn LevelOut>)
    })
}

/// The level parts of the host: the gate that the mic's `LevelSink` sends through, and the
/// resolver of the `onLevel` reference.
pub struct HostLevels {
    gate: LevelGate,
    resolve: LevelResolver,
}

impl HostLevels {
    /// A closed gate with no receiver yet, and `resolve` for `capture_subscribe`.
    pub fn new(resolve: LevelResolver) -> Self {
        Self {
            gate: LevelGate::new(Arc::new(NoLevelOut)),
            resolve,
        }
    }

    /// The gate. [`spawn_level_mic`] gives it to the mic's `LevelSink`.
    pub fn gate(&self) -> &LevelGate {
        &self.gate
    }
}

/// Start the mic over `make_host`. Its frames go to a `TeeSink`: first the `LevelSink` on
/// `gate` (CAP-16a), then `ptt` (the app passes [`PttBuffer::sink`], CAP-12c, #21).
pub fn spawn_level_mic<H, F, B>(
    make_host: F,
    ptt: B,
    gate: &LevelGate,
) -> std::io::Result<(MicDevice, FailureLink)>
where
    H: MicHost,
    F: FnOnce() -> H + Send + 'static,
    B: FrameSink,
{
    MicDevice::spawn(
        make_host,
        Box::new(TeeSink(LevelSink::new(gate.clone()), ptt)),
    )
}

/// The brain events with or without R4 in front (#46).
enum SttEvents<E> {
    On(SttCaptureEvents<E>),
    Off(E),
}

impl<E: CaptureEvents> CaptureEvents for SttEvents<E> {
    fn state(&self, state: &CaptureState) {
        match self {
            SttEvents::On(e) => e.state(state),
            SttEvents::Off(e) => e.state(state),
        }
    }

    fn stopped(&self, stopped: &Stopped) {
        match self {
            SttEvents::On(e) => e.stopped(stopped),
            SttEvents::Off(e) => e.stopped(stopped),
        }
    }
}

// ---------- the cap timer ----------

/// What the timer thread waits for in its last wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimerWait {
    /// The thread has not waited yet.
    #[default]
    NotYet,
    /// No deadline: it waits until an arm, a settle or the stop.
    Forever,
    /// It waits up to this long: the time left to the armed deadline.
    For(Duration),
}

#[derive(Default)]
struct TimerState {
    /// The next cap deadline in monotonic ms of the shared clock.
    armed: Option<u64>,
    stop: bool,
    /// Bumped by [`CapTimer::settle`]; the thread copies it to `seen` before it waits.
    kicks: u64,
    seen: u64,
    last_wait: TimerWait,
}

struct TimerShared {
    state: Mutex<TimerState>,
    /// Wakes the timer thread: an earlier deadline, a settle or the stop.
    wake: Condvar,
    /// The thread is about to wait (for [`CapTimer::settle`]).
    idle: Condvar,
    ticks: AtomicU64,
}

/// The host cap timer: one thread that calls [`CaptureService::tick`] at the deadline that
/// `tick()` returns (CAP-04a, CAP-04b). It sleeps on a condition variable until the deadline,
/// or with no deadline until an arm, so it never busy-loops.
///
/// [`CapTimer::arm`] only ever moves the deadline earlier. Before each `tick()` the thread
/// clears the deadline, and after it keeps the earlier of the `tick()` result and any arm that
/// came meanwhile. So an arm that races a tick is never lost; a stale early deadline only
/// costs one extra `tick()`, which returns the true next deadline.
pub struct CapTimer {
    shared: Arc<TimerShared>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

/// After a `tick()` that panicked, the timer tries again this much later.
pub const TICK_RETRY_MS: u64 = 1_000;

fn min_deadline(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

impl CapTimer {
    /// Start the timer thread over `service`. Time comes from `clock`, the clock of the core.
    pub fn spawn(
        service: Arc<dyn CaptureService>,
        clock: Arc<dyn MonotonicClock>,
    ) -> std::io::Result<Self> {
        let shared = Arc::new(TimerShared {
            state: Mutex::default(),
            wake: Condvar::new(),
            idle: Condvar::new(),
            ticks: AtomicU64::new(0),
        });
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("bottle-capture-caps".into())
                .spawn(move || run_timer(&shared, &*service, &*clock))?
        };
        Ok(Self {
            shared,
            thread: Mutex::new(Some(thread)),
        })
    }

    /// A new deadline (from `tick()`, or after a `start()` or `resume()`). If it is earlier
    /// than the armed one, the thread wakes and waits for the new one.
    pub fn arm(&self, deadline: Option<u64>) {
        let Some(deadline) = deadline else {
            return;
        };
        let mut st = lock(&self.shared.state);
        if st.armed.is_none_or(|armed| deadline < armed) {
            st.armed = Some(deadline);
            self.shared.wake.notify_all();
        }
    }

    /// The deadline the thread waits for now.
    pub fn armed(&self) -> Option<u64> {
        lock(&self.shared.state).armed
    }

    /// How many times the thread called `tick()`.
    pub fn ticks(&self) -> u64 {
        self.shared.ticks.load(Ordering::SeqCst)
    }

    /// What the thread waits for now (or in its last wait). A bounded wait is always the time
    /// left to the armed deadline, so the thread never spins (no busy loop).
    pub fn last_wait(&self) -> TimerWait {
        lock(&self.shared.state).last_wait
    }

    /// Wake the thread to read the clock again, and return when it waits again, after any
    /// `tick()` that was due. Tests call it after they move a simulated clock; on the system
    /// clock it is not needed. It returns at once if the thread has stopped.
    pub fn settle(&self) {
        let mut st = lock(&self.shared.state);
        if st.stop {
            return;
        }
        st.kicks += 1;
        let kick = st.kicks;
        self.shared.wake.notify_all();
        while !st.stop && st.seen < kick {
            st = self
                .shared
                .idle
                .wait(st)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// True once the thread is told to stop.
    pub fn is_stopped(&self) -> bool {
        lock(&self.shared.state).stop
    }

    /// Stop the thread and wait for it. Safe to call more than once.
    pub fn shutdown(&self) {
        {
            let mut st = lock(&self.shared.state);
            st.stop = true;
            self.shared.wake.notify_all();
            self.shared.idle.notify_all();
        }
        let handle = lock(&self.thread).take();
        if let Some(handle) = handle {
            if handle.thread().id() != std::thread::current().id() {
                let _ = handle.join();
            }
        }
    }
}

impl Drop for CapTimer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn run_timer(shared: &TimerShared, service: &dyn CaptureService, clock: &dyn MonotonicClock) {
    let mut st = lock(&shared.state);
    loop {
        // Wait until the armed deadline is due.
        loop {
            if st.stop {
                return;
            }
            let now = clock.now();
            // `None`: due now. `Some(None)`: no deadline. `Some(Some(d))`: due in `d`.
            let wait = match st.armed.map(Duration::from_millis) {
                Some(at) if now >= at => None,
                Some(at) => Some(Some(at - now)),
                None => Some(None),
            };
            let Some(wait) = wait else {
                break;
            };
            st.seen = st.kicks;
            st.last_wait = wait.map_or(TimerWait::Forever, TimerWait::For);
            shared.idle.notify_all();
            st = match wait {
                Some(d) => {
                    shared
                        .wake
                        .wait_timeout(st, d)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => shared.wake.wait(st).unwrap_or_else(PoisonError::into_inner),
            };
        }
        st.armed = None;
        drop(st);
        // A panic in `tick()` (for example in an event sink) must not end the caps for good.
        // The core commits its state before it sends events, so a retry in 1 s is safe and
        // stays within the ±1 s of AT-03 (ADR 0055).
        let next =
            std::panic::catch_unwind(AssertUnwindSafe(|| service.tick())).unwrap_or_else(|_| {
                let now = u64::try_from(clock.now().as_millis()).unwrap_or(u64::MAX);
                Some(now.saturating_add(TICK_RETRY_MS))
            });
        shared.ticks.fetch_add(1, Ordering::SeqCst);
        st = lock(&shared.state);
        st.armed = min_deadline(st.armed, next);
    }
}

// ---------- the host ----------

/// The admission of one `capture_start` or `capture_resume`, taken on the IPC thread before
/// the call goes to a worker (ADR 0055, decision 7). A `capture_stop_all` (or a stop or pause)
/// that runs inline after the admission, but before the queued job reaches R2, makes the job
/// undo its grant. So a later stop always wins over an earlier start or resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureTicket {
    panic: u64,
    lease: u64,
}

/// The one CaptureService of the process, with its lease owners and its cap timer.
pub struct CaptureHost {
    service: Arc<CaptureCore>,
    owners: Arc<Mutex<LeaseOwners>>,
    timer: CapTimer,
    /// Bumped by every `capture_stop_all` and by `shutdown`.
    panic_gen: AtomicU64,
    /// Bumped by every `capture_pause`; the lease records the new number in `paused_at`.
    pause_seq: AtomicU64,
    /// Set by `shutdown`: no new lease after it.
    closed: AtomicBool,
    levels: HostLevels,
}

impl CaptureHost {
    /// Build the core over `device` and `clock`, relay its events with `relay`, and start the
    /// cap timer. No stream opens until a lease starts (MOG-09). This host has no level
    /// stream: `capture_subscribe` checks its Channels and every level is dropped.
    pub fn new(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn MonotonicClock>,
        relay: BrainRelay,
    ) -> std::io::Result<Self> {
        let none: LevelResolver = Arc::new(|_: &ChannelRef| Some(Arc::new(NoLevelOut) as _));
        Self::with_levels(device, clock, relay, HostLevels::new(none))
    }

    /// As [`CaptureHost::new`], with the level stream of `levels` (CAP-16a, CAP-16b): the core
    /// opens the level gate while a capture records, and `capture_subscribe` sets its receiver.
    /// The mic device must send its frames through the same gate ([`spawn_level_mic`]). This
    /// host's PTT buffer drops every utterance ([`NoUtteranceSink`]).
    pub fn with_levels(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn MonotonicClock>,
        relay: BrainRelay,
        levels: HostLevels,
    ) -> std::io::Result<Self> {
        let ptt = PttBuffer::new(Arc::new(NoUtteranceSink));
        Self::with_audio(device, clock, relay, levels, ptt)
    }

    /// As [`CaptureHost::with_levels`], with the RAM-only PTT buffer `ptt` (CAP-12c, CAP-13a,
    /// CAP-34, #21). The core drives the buffer from each `capture.state` and stop. The mic
    /// device must send its frames to [`PttBuffer::sink`] of the same buffer
    /// ([`spawn_level_mic`]).
    pub fn with_audio(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn MonotonicClock>,
        relay: BrainRelay,
        levels: HostLevels,
        ptt: PttBuffer,
    ) -> std::io::Result<Self> {
        Self::build(device, clock, relay, levels, ptt, None)
    }

    /// As [`CaptureHost::with_audio`], with R4 SttEngine `stt` in the event chain (#46,
    /// ADR 0082): a granted PTT or hands-free lease begins its take and preloads the model
    /// (CAP-51c), and each stop commits or drops it. The mic device must tee its frames to
    /// [`LocalSttEngine::frame_sink`] next to the PTT buffer ([`spawn_level_mic`]).
    pub fn with_stt(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn MonotonicClock>,
        relay: BrainRelay,
        levels: HostLevels,
        ptt: PttBuffer,
        stt: LocalSttEngine,
    ) -> std::io::Result<Self> {
        Self::build(device, clock, relay, levels, ptt, Some(stt))
    }

    fn build(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn MonotonicClock>,
        relay: BrainRelay,
        levels: HostLevels,
        ptt: PttBuffer,
        stt: Option<LocalSttEngine>,
    ) -> std::io::Result<Self> {
        let owners: Arc<Mutex<LeaseOwners>> = Arc::default();
        let brain = BrainCaptureEvents {
            relay,
            owners: Arc::clone(&owners),
        };
        let brain = match stt {
            Some(engine) => SttEvents::On(SttCaptureEvents::new(brain, engine)),
            None => SttEvents::Off(brain),
        };
        // The gate closes before the brain sees a stop state (CAP-16b). The PTT buffer ends its
        // take before the brain sees the stop (#21). R4 begins, commits or drops its take
        // before the brain sees the event (#46).
        let sink = Arc::new(LevelCaptureEvents::new(
            PttCaptureEvents::new(brain, ptt),
            levels.gate.clone(),
        ));
        let service = Arc::new(CaptureCore::new(device, Arc::clone(&clock), sink));
        let timer = CapTimer::spawn(Arc::clone(&service) as Arc<dyn CaptureService>, clock)?;
        Ok(Self {
            service,
            owners,
            timer,
            panic_gen: AtomicU64::new(0),
            pause_seq: AtomicU64::new(0),
            closed: AtomicBool::new(false),
            levels,
        })
    }

    /// Let the mic backend report a failed stream to the core (ADR 0038, decision 7).
    pub fn connect_failures(&self, link: &mog_core::audio_device::FailureLink) {
        link.connect(&self.service);
    }

    /// The cap timer.
    pub fn timer(&self) -> &CapTimer {
        &self.timer
    }

    /// Take the admission of a call now, on the IPC thread, before it goes to a worker.
    pub fn ticket(&self) -> CaptureTicket {
        CaptureTicket {
            panic: self.panic_gen.load(Ordering::SeqCst),
            lease: self.pause_seq.load(Ordering::SeqCst),
        }
    }

    /// How many leases the host still knows an owner for. Every stop forgets its lease.
    #[doc(hidden)]
    pub fn live_owner_count(&self) -> usize {
        lock(&self.owners).live.len()
    }

    /// On quit: stop every lease (the mic closes) and end the timer thread. A start that is
    /// already on a worker gets no lease after this.
    pub fn shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.panic_gen.fetch_add(1, Ordering::SeqCst);
        self.service.stop_all();
        self.timer.shutdown();
    }

    /// Run one `capture_*` command that the gateway checked. Its admission is now.
    pub fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        self.call_admitted(command, args, self.ticket())
    }

    /// Run one `capture_*` command that the gateway checked, with the admission `ticket` that
    /// the IPC thread took when the call arrived.
    pub fn call_admitted(
        &self,
        command: &'static str,
        args: Value,
        ticket: CaptureTicket,
    ) -> CoreResult<Value> {
        match command {
            "capture_start" => self.start(command, args, ticket),
            "capture_stop" | "capture_pause" | "capture_resume" => {
                let a: CaptureLeaseArgs = args_of(command, args)?;
                let lease = self.own_lease(command, &a)?;
                let state = match command {
                    // A stop first makes a queued resume of the lease `unknown_lease` in R2.
                    "capture_stop" => self.service.stop(lease),
                    "capture_pause" => {
                        self.note_pause(lease);
                        self.service.pause(lease)
                    }
                    _ => self.resume(lease, ticket),
                };
                to_value(command, &state.map_err(|e| capture_error(command, e))?)
            }
            "capture_status" => to_value(command, &self.service.status()),
            // MB-05: the decision never waits for the device.
            "capture_stop_all" => {
                self.panic_gen.fetch_add(1, Ordering::SeqCst);
                to_value(command, &self.service.stop_all())
            }
            "capture_subscribe" => {
                // The gateway checked both Channel references.
                let a: CaptureSubscribeArgs = args_of(command, args)?;
                // CAP-16a: the levels go to this `onLevel` Channel from now on. A later call
                // (a brain reload) replaces it. TODO(#547): R4's partial text feeds `onPartial`.
                let out = (self.levels.resolve)(&a.on_level).ok_or_else(|| {
                    refused(IpcError::HostFailed {
                        command: command.to_owned(),
                    })
                })?;
                self.levels.gate.set_out(out);
                Ok(json!({}))
            }
            _ => Err(refused(IpcError::NotImplemented {
                command: command.to_owned(),
            })),
        }
    }

    /// Record a `capture_pause` of `lease`, so a resume of that lease admitted earlier
    /// undoes itself.
    fn note_pause(&self, lease: LeaseNo) {
        let seq = self.pause_seq.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.owners).paused_at.insert(lease, seq);
    }

    /// True if a `stop_all` or a quit ran since `ticket`, or, for a resume of `resume_of`, a
    /// pause of that same lease. A queued start or resume must then not keep its grant.
    fn overtaken(&self, ticket: CaptureTicket, resume_of: Option<LeaseNo>) -> bool {
        let paused_later = resume_of.is_some_and(|lease| {
            lock(&self.owners)
                .paused_at
                .get(&lease)
                .is_some_and(|&at| at > ticket.lease)
        });
        self.closed.load(Ordering::SeqCst)
            || self.panic_gen.load(Ordering::SeqCst) != ticket.panic
            || paused_later
    }

    fn resume(&self, lease: LeaseNo, ticket: CaptureTicket) -> Result<CaptureState, CaptureError> {
        if self.overtaken(ticket, Some(lease)) {
            // A later pause or `stop_all` won before this resume reached R2.
            return Ok(self.service.status());
        }
        let state = self.service.resume(lease)?;
        if self.overtaken(ticket, Some(lease)) {
            // The user's later pause wins over this resume (CAP-13b, ADR 0055). The reply is
            // the paused state; the brain reads `recording` in it.
            // TODO(#65): run this branch in a test once a session can start.
            return match self.service.pause(lease) {
                Ok(paused) => Ok(paused),
                Err(_) => Ok(self.service.status()),
            };
        }
        // A session has no cap deadline today, so this arm has no effect until a capped
        // lease can resume. TODO(#65): test it once a session can start.
        self.timer.arm(self.service.tick());
        Ok(state)
    }

    fn start(
        &self,
        command: &'static str,
        args: Value,
        ticket: CaptureTicket,
    ) -> CoreResult<Value> {
        let a: CaptureStartArgs = args_of(command, args)?;
        if self.closed.load(Ordering::SeqCst) {
            // The app is quitting (`shutdown`): no new lease.
            return Err(refused(IpcError::HostFailed {
                command: command.to_owned(),
            }));
        }
        if a.mode == CaptureMode::Session {
            // TODO(#65): check the consent token of `session_attest`, and pass `policy` to R3
            // SessionGuard (#67). Until then no session starts (fail closed).
            return Err(refused(IpcError::ConsentRequired {
                command: command.to_owned(),
            }));
        }
        if self.overtaken(ticket, None) {
            // A `capture_stop_all` ran after this call arrived and before its job ran: the mic
            // never opens (MB-05). The answer is the one R2 gives a start that `stop_all`
            // cancels during its open.
            return Err(capture_error(command, CaptureError::UnknownLease));
        }
        let lease = self
            .service
            .start(CaptureRequest {
                mog_id: a.mog_id,
                mode: a.mode,
                others_track: a.others_track.unwrap_or(false),
            })
            .map_err(|e| capture_error(command, e))?;
        lock(&self.owners).granted(lease.id, lease.mog_id.clone());
        if self.overtaken(ticket, None) {
            // A `capture_stop_all` ran while R2 granted the lease: it wins (MB-05). The answer is
            // the one R2 gives a start that `stop_all` cancels during its open.
            let _ = self.service.stop(lease.id);
            return Err(capture_error(command, CaptureError::UnknownLease));
        }
        // The new lease may have the earliest cap (a PTT over hands-free).
        self.timer.arm(self.service.tick());
        // The reply names the caller's lease, also when that lease is a PTT directed into a
        // session, whose mic the session lease owns (ADR 0055).
        let mut state = self.service.status();
        state.lease_id = Some(lease.lease_id());
        to_value(command, &state)
    }

    /// The lease of `a`, if `a.mogId` owns it. Any other case is `unknown_lease` (ADR 0007).
    fn own_lease(&self, command: &str, a: &CaptureLeaseArgs) -> CoreResult<LeaseNo> {
        let unknown = || capture_error(command, CaptureError::UnknownLease);
        let lease = parse_lease(a.lease_id.as_str()).map_err(|_| unknown())?;
        match lock(&self.owners).owner(lease) {
            Some(owner) if owner == a.mog_id => Ok(lease),
            _ => Err(unknown()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_02a_owners_forget_a_lease_that_stopped_before_its_grant_was_recorded() {
        let mut owners = LeaseOwners::default();
        owners.stopped(3);
        owners.granted(3, "a1".into());
        assert_eq!(owners.owner(3), None);
        assert!(owners.gone.is_empty() && owners.live.is_empty());
        owners.granted(4, "a1".into());
        assert_eq!(owners.owner(4), Some("a1"));
        owners.stopped(4);
        assert_eq!(owners.owner(4), None);
        assert!(owners.gone.is_empty() && owners.live.is_empty());
    }

    fn host() -> CaptureHost {
        let fake = mog_core::testkit::capture::FakeAudioDevice::new();
        let clock = mog_core::clock::SimClock::new();
        CaptureHost::new(Box::new(fake), Arc::new(clock), Arc::new(|_: &str, _| true)).unwrap()
    }

    #[test]
    fn cap_13b_a_pause_after_a_queued_resume_wins() {
        // TODO(#65): no session can start yet, so the resume path is checked at the ticket.
        let h = host();
        let t = h.ticket();
        assert!(!h.overtaken(t, Some(7)) && !h.overtaken(t, None));
        h.note_pause(7);
        assert!(h.overtaken(t, Some(7)), "a later pause of the lease wins");
        assert!(
            !h.overtaken(t, Some(8)),
            "a pause of another lease does not"
        );
        assert!(!h.overtaken(t, None), "nor does it cancel a queued start");
        assert!(
            !h.overtaken(h.ticket(), Some(7)),
            "an earlier pause does not"
        );
        lock(&h.owners).stopped(7);
        assert!(!h.overtaken(t, Some(7)), "a stop forgets the pause mark");
        let t = h.ticket();
        h.panic_gen.fetch_add(1, Ordering::SeqCst);
        assert!(
            h.overtaken(t, Some(7)) && h.overtaken(t, None),
            "stop_all overtakes both"
        );
        let t = h.ticket();
        h.shutdown();
        assert!(
            h.overtaken(h.ticket(), None) && h.overtaken(t, Some(7)),
            "quit is final"
        );
    }

    #[test]
    fn cap_02a_every_stop_reason_has_the_same_wire_code_in_capture_stopped() {
        use mog_core::capture::StopReason::*;
        // A new R2 reason fails to build in `From<StopReason>`; this checks the wire codes.
        for r in [
            Manual,
            HoldCap,
            HandsFreeCap,
            Preempted,
            Panic,
            DeviceLost,
            Released,
            Discarded,
            Tap,
            Escape,
            VadEnd,
            ZeroBuffers,
        ] {
            let wire = serde_json::to_value(mog_core::types::CaptureStopReason::from(r)).unwrap();
            assert_eq!(wire, json!(r.code()), "{r:?}");
        }
    }

    #[test]
    fn cap_04a_min_deadline_keeps_the_earlier_one() {
        assert_eq!(min_deadline(None, None), None);
        assert_eq!(min_deadline(Some(5), None), Some(5));
        assert_eq!(min_deadline(None, Some(7)), Some(7));
        assert_eq!(min_deadline(Some(9), Some(7)), Some(7));
    }
}
