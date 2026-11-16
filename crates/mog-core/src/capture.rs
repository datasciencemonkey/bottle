//! R2 CaptureService (section 4.3).
//!
//! Responsibilities: The only owner of the mic and system audio. Leases, You and Others tracks, Silero VAD, segments (15-20 s target, 30 s hard), level stream, encrypted journal (60 s rotation, crypto-shred), raw-audio deletion.
//!
//! This module is the device-free core (#4): one owner of the mic and system-audio streams
//! (CAP-01a), capture leases with IDs (CAP-02a), the three modes (CAP-03), the locked 120 s
//! hold and 5 min hands-free caps (CAP-04a, CAP-04b), ARC-F9 priority and the panic stop
//! (MB-05). Time comes from the shared [`MonotonicClock`] (#229), audio streams from an
//! [`AudioDevice`] and state changes go to [`CaptureEvents`].
//!
//! Lock protocol (review R2). There are two locks:
//!
//! - the state lock (`inner`): leases, the pending start or resume, the published state;
//! - the device mutex (`dev`): every `open` and `close`, and the open flags of the two sources.
//!
//! The lock order is device, then state. No thread takes the device mutex while it holds the
//! state lock, and no lock is held while an event goes out. Every device call runs under the
//! device mutex, and the device mutex holder reads the leases under the state lock just before
//! each call. A start or resume grants its lease under the state lock while it still holds the
//! device mutex, after it saw its sources open. So no close can run between that check and the
//! grant, and a new grant cannot appear while a close decides.
//!
//! Only a start or resume waits for the device mutex: it waits for a bounded close, or for
//! one other device phase. Every other call (`stop`, `stop_all`, `status`, `tick`, `pause`,
//! `device_failed`) changes the state, sets a `dirty` flag and then only *tries* the device
//! mutex. If the mutex is free, the call closes the unneeded streams itself. If it is held, the
//! holder sees `dirty` before it lets go, or right after, and closes them. So the MB-05 panic
//! stop never waits for a slow CoreAudio or WASAPI open; a stream that a slow open is still
//! opening closes when that open returns. Handy keeps device work on one owner for the same
//! reason (cjpais/Handy `src-tauri/src/managers/audio.rs:495-500` @ aca8985; pattern only, no
//! Handy code is copied).
//!
//! Invariants:
//!
//! 1. Every granted lease has its sources open (checked under the device mutex at the grant,
//!    with a `debug_assert`).
//! 2. A source never opens twice at once: opens run only under the device mutex, and at most
//!    one start or resume is pending.
//! 3. After a stop, no source stays open with no lease: the stop sets `dirty`, and the device
//!    mutex holder (this thread or the current one) closes every stream that no lease needs.
//!
//! While one start or resume is in its device phase, a second start or resume gets `mic_busy`,
//! also a PTT that would win under ARC-F9. The window is one device open. After `stop_all` it
//! lasts until the cancelled open returns.
//!
//! Lease boundaries for the segment pipeline (#116): the device sees only open and close, and a
//! PTT over hands-free or a directed PTT in a session reuses the open mic. The pipeline cuts
//! audio per lease from the [`Lease`] that `start` returns, the [`Stopped`] events and the
//! `leaseId` / `directedTo` fields of each `capture.state` event.
//!
//! Audio never leaves Rust (CAP-01b): no method or event here carries a sample. The
//! [`AudioDevice`] only opens and closes streams; the frames stay inside the device backend
//! and the Rust pipeline behind it.
//!
//! Segments (`SegmentRef`, with its required `leaseId`) are not made here yet: the VAD and
//! segment pipeline (#116) builds them from [`Lease::lease_id`].
//!
//! Follow-up: this crate has no real [`AudioDevice`] yet. The cpal mic backend and the
//! system-audio backends (#15, #107, #108, #109) implement the trait. Until one lands, a host
//! cannot build a [`CaptureCore`] that opens a real mic.

use std::cell::Cell;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::clock::MonotonicClock;
use crate::ipc::IpcError;
use crate::types::{self, CaptureMode, CaptureState, MogId, OthersTrack};

/// The locked hold-to-talk safety stop: 120 s (section 7.1, D54, CAP-04a).
pub const HOLD_MAX_MS: u64 = 120_000;
/// The locked hands-free cap: 5 min (section 7.1, D54, CAP-04b).
pub const HANDS_FREE_MAX_MS: u64 = 300_000;

/// The number of a capture lease inside R2. Numbers start at 1 and are never reused in one
/// process. On the wire it is [`types::LeaseId`], the decimal text of this number (ADR 0007).
pub type LeaseNo = u64;

/// Read a wire lease ID ([`types::LeaseId`] or a checked `ipc::LeaseRef`) back into a
/// [`LeaseNo`]. Text that R2 never issued is [`CaptureError::UnknownLease`].
pub fn parse_lease(wire: &str) -> Result<LeaseNo, CaptureError> {
    match wire.parse::<LeaseNo>() {
        // `no.to_string() == wire` rejects "+7" and "007": R2 never issues them.
        Ok(no) if no > 0 && no.to_string() == wire => Ok(no),
        _ => Err(CaptureError::UnknownLease),
    }
}

/// An audio stream that CaptureService owns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    /// The microphone (the You track).
    Mic,
    /// System audio (the Others track, meeting sessions only).
    SystemAudio,
}

/// A content-free device error, for example `permission_denied` or `no_device`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceError {
    /// The stable error code that may cross IPC.
    pub code: &'static str,
}

/// The audio device backend. CaptureService is its only caller. It opens each source at most
/// once at a time and never holds its state lock during a call. A cpal stream is not `Send`, so
/// a real backend owns its streams on its own audio thread and takes open and close as messages.
///
/// When a stream fails after it opened (an unplugged mic), the backend calls
/// [`CaptureCore::device_failed`]. This is the place of Handy's failure handler and
/// `needs_reopen` (cjpais/Handy `src-tauri/src/managers/audio.rs:294-307`, `:413-423`,
/// `:1051-1101` @ aca8985): the next start opens the stream again.
pub trait AudioDevice: Send + Sync {
    /// Open the stream of `source`. It may be slow.
    fn open(&self, source: AudioSource) -> Result<(), DeviceError>;
    /// Close the stream of `source`. The stream, and with it the OS mic indicator, must be
    /// stopped before `close` returns: no idle timeout, no lazy close and no always-on mode
    /// (MOG-09, CAP-13b; Handy's `MicrophoneMode::AlwaysOn` and `schedule_lazy_close` are not
    /// ported). Closing a closed stream does nothing. R2 never runs two device calls at once.
    fn close(&self, source: AudioSource);
}

/// Why a lease stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// `capture_stop` with the lease ID.
    Manual,
    /// The 120 s hold safety stop (CAP-04a).
    HoldCap,
    /// The 5 min hands-free cap (CAP-04b).
    HandsFreeCap,
    /// A higher-priority capture took the mic (ARC-F9).
    Preempted,
    /// `capture_stop_all`, for example from the panic key (MB-05).
    Panic,
    /// The device reported a failed stream, for example an unplugged mic.
    DeviceLost,
}

impl StopReason {
    /// The stable reason code.
    pub fn code(&self) -> &'static str {
        match self {
            StopReason::Manual => "manual",
            StopReason::HoldCap => "hold_cap",
            StopReason::HandsFreeCap => "hands_free_cap",
            StopReason::Preempted => "preempted",
            StopReason::Panic => "panic",
            StopReason::DeviceLost => "device_lost",
        }
    }
}

/// A lease that stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// The lease that stopped.
    pub lease_id: LeaseNo,
    /// The MOG that held it.
    pub mog_id: MogId,
    /// Its mode.
    pub mode: CaptureMode,
    /// Why it stopped.
    pub reason: StopReason,
}

/// The receiver of CaptureService events (the `capture.state` event for the brain). The core
/// calls it after it releases its state lock, so a sink may call back into the service.
pub trait CaptureEvents: Send + Sync {
    /// The capture state changed.
    fn state(&self, state: &CaptureState);
    /// A lease stopped.
    fn stopped(&self, stopped: &Stopped);
}

/// A `capture_start` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureRequest {
    /// The MOG that asks for the mic.
    pub mog_id: MogId,
    /// The capture mode.
    pub mode: CaptureMode,
    /// Also capture system audio (the Others track). Sessions only.
    pub others_track: bool,
}

/// A granted lease.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lease {
    /// The lease ID. `capture_stop` and `capture_pause` need it.
    pub id: LeaseNo,
    /// The MOG that holds the lease.
    pub mog_id: MogId,
    /// The capture mode of the lease.
    pub mode: CaptureMode,
}

impl Lease {
    /// The wire lease ID of `CaptureState.leaseId`, `SegmentRef.leaseId` and the
    /// `capture_stop` / `capture_pause` arguments (ADR 0007).
    pub fn lease_id(&self) -> types::LeaseId {
        self.id.to_string()
    }
}

/// A CaptureService error. Only [`CaptureError::code`] crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureError {
    /// Another capture holds the mic and has the same or a higher priority (CAP-01a).
    MicBusy,
    /// No live lease has this ID.
    UnknownLease,
    /// The command does not apply to this mode.
    InvalidMode,
    /// The audio device failed.
    Device(DeviceError),
}

impl CaptureError {
    /// The stable, content-free error code.
    pub fn code(&self) -> &'static str {
        match self {
            CaptureError::MicBusy => "mic_busy",
            CaptureError::UnknownLease => "unknown_lease",
            CaptureError::InvalidMode => "invalid_mode",
            CaptureError::Device(e) => e.code,
        }
    }
}

impl CaptureError {
    /// The typed IPC error of `command` (ADR 0007). The codes are the same; a device fault
    /// is `host_failed`, so no device detail crosses IPC.
    pub fn to_ipc(&self, command: &str) -> IpcError {
        let command = command.to_string();
        match self {
            CaptureError::MicBusy => IpcError::MicBusy { command },
            CaptureError::UnknownLease => IpcError::UnknownLease { command },
            CaptureError::InvalidMode => IpcError::InvalidMode { command },
            CaptureError::Device(_) => IpcError::HostFailed { command },
        }
    }
}

impl fmt::Display for CaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "capture error: {}", self.code())
    }
}

impl std::error::Error for CaptureError {}

/// The only owner of the mic and system audio. Callers hold a lease while they capture.
pub trait CaptureService: Send + Sync {
    /// Start a capture. Returns a new lease, or `mic_busy` (ARC-F9 priority). A second start
    /// while a start or resume still opens the device also gets `mic_busy`. If `stop_all` runs
    /// while the device opens, the start returns `unknown_lease` and no lease is granted.
    fn start(&self, request: CaptureRequest) -> Result<Lease, CaptureError>;
    /// Stop the capture of a lease.
    fn stop(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError>;
    /// Pause a session lease. The streams close (CAP-13b).
    fn pause(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError>;
    /// Resume a paused session lease.
    fn resume(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError>;
    /// The current capture state.
    fn status(&self) -> CaptureState;
    /// Stop every lease and close every stream (the panic stop, MB-05).
    fn stop_all(&self) -> CaptureState;
    /// Apply the caps at the current time. Returns the next cap deadline in monotonic ms.
    /// A host timer calls this at the deadline.
    fn tick(&self) -> Option<u64>;
}

#[derive(Debug, Clone)]
struct LeaseRec {
    id: LeaseNo,
    mog_id: MogId,
    mode: CaptureMode,
    started_mono: u64,
    started_epoch: u64,
    others_track: bool,
    paused: bool,
}

impl LeaseRec {
    fn deadline(&self) -> Option<(u64, StopReason)> {
        match self.mode {
            CaptureMode::Ptt => Some((self.started_mono + HOLD_MAX_MS, StopReason::HoldCap)),
            CaptureMode::HandsFree => Some((
                self.started_mono + HANDS_FREE_MAX_MS,
                StopReason::HandsFreeCap,
            )),
            // The session stop policy belongs to R3 SessionGuard (#67, #127).
            CaptureMode::Session => None,
        }
    }

    fn live(&self) -> bool {
        !self.paused
    }

    fn stopped(self, reason: StopReason) -> Ev {
        Ev::Stopped(Stopped {
            lease_id: self.id,
            mog_id: self.mog_id,
            mode: self.mode,
            reason,
        })
    }
}

/// A start or resume whose streams are opening. There is at most one.
#[derive(Debug)]
enum Pending {
    /// A new lease. It owns nothing until the open succeeds; then it takes its place and, if
    /// `preempt` is set, the hands-free lease it preempts stops (ARC-F9, and no loss on a
    /// failed open).
    Start {
        rec: LeaseRec,
        overlay: bool,
        preempt: Option<LeaseNo>,
    },
    /// A paused session that resumes.
    Resume(LeaseNo),
    /// `stop_all` cancelled the start or resume, but its open is still running. The open slot
    /// stays taken until that open returns, so no second open of a source can run beside it.
    Cancelled,
}

/// The error of a start or resume whose stream the device lost before the grant.
const DEVICE_LOST: DeviceError = DeviceError {
    code: "device_lost",
};

/// An event, sent after the state lock is released.
enum Ev {
    State(CaptureState),
    Stopped(Stopped),
}

#[derive(Default)]
struct Inner {
    /// The lease that owns the streams.
    primary: Option<LeaseRec>,
    /// A PTT on top of the primary lease: it suspends a hands-free lease (ARC-F9) or is a
    /// directed segment of a session.
    overlay: Option<LeaseRec>,
    pending: Option<Pending>,
    /// The device reported a lost stream that the pending start or resume needs, before its
    /// lease was granted. The grant then fails with `device_lost`.
    pending_lost: bool,
    next_no: LeaseNo,
    last_state: Option<CaptureState>,
}

impl Inner {
    /// The streams that the leases (and a pending start) need: (mic, system audio).
    fn wanted(&self) -> (bool, bool) {
        let primary_live = self.primary.as_ref().filter(|p| p.live());
        let mut mic = primary_live.is_some() || self.overlay.is_some();
        let mut system = primary_live.is_some_and(|p| p.others_track);
        if let Some(Pending::Start { rec, .. }) = &self.pending {
            mic = true;
            system |= rec.others_track;
        }
        (mic, system)
    }

    /// Take a lease out. When the primary goes, the overlay PTT (if any) becomes primary.
    fn remove(&mut self, lease_id: LeaseNo) -> Option<LeaseRec> {
        if self.overlay.as_ref().is_some_and(|o| o.id == lease_id) {
            return self.overlay.take();
        }
        if self.primary.as_ref().is_some_and(|p| p.id == lease_id) {
            let rec = self.primary.take();
            self.primary = self.overlay.take();
            return rec;
        }
        None
    }

    fn find(&mut self, lease_id: LeaseNo) -> Option<&mut LeaseRec> {
        [&mut self.primary, &mut self.overlay]
            .into_iter()
            .flatten()
            .find(|rec| rec.id == lease_id)
    }

    fn state(&self) -> CaptureState {
        let idle = CaptureState {
            owner: None,
            mode: None,
            lease_id: None,
            recording: false,
            paused: false,
            session_id: None,
            started_at: None,
            directed_to: None,
            others_track: OthersTrack::Off,
        };
        let Some(primary) = &self.primary else {
            return idle;
        };
        match (&self.overlay, primary.mode) {
            (Some(ptt), CaptureMode::HandsFree) => CaptureState {
                owner: Some(ptt.mog_id.clone()),
                mode: Some(CaptureMode::Ptt),
                lease_id: Some(ptt.id.to_string()),
                recording: true,
                started_at: Some(ptt.started_epoch),
                ..idle
            },
            (overlay, mode) => CaptureState {
                owner: Some(primary.mog_id.clone()),
                mode: Some(mode),
                // A directed PTT uses the session stream; the session lease owns the mic.
                lease_id: Some(primary.id.to_string()),
                recording: primary.live() || overlay.is_some(),
                paused: primary.paused,
                session_id: (mode == CaptureMode::Session)
                    .then(|| format!("session-{}", primary.id)),
                started_at: Some(primary.started_epoch),
                directed_to: overlay.as_ref().map(|o| o.mog_id.clone()),
                others_track: if primary.live() && primary.others_track {
                    OthersTrack::Ok
                } else {
                    OthersTrack::Off
                },
            },
        }
    }

    /// Stop every lease whose cap has passed.
    fn expire(&mut self, now: u64, evs: &mut Vec<Ev>) {
        let due: Vec<(LeaseNo, StopReason)> = [&self.overlay, &self.primary]
            .into_iter()
            .flatten()
            .filter_map(|rec| match rec.deadline() {
                Some((at, reason)) if now >= at => Some((rec.id, reason)),
                _ => None,
            })
            .collect();
        for (id, reason) in due {
            if let Some(rec) = self.remove(id) {
                evs.push(rec.stopped(reason));
            }
        }
    }
}

/// The streams that are open now. Only a holder of the device mutex reads or changes it, and
/// every `open` and `close` runs under that mutex, so these flags are always the truth.
#[derive(Debug, Default)]
struct DevState {
    mic_open: bool,
    system_open: bool,
}

impl DevState {
    fn flag(&mut self, source: AudioSource) -> &mut bool {
        match source {
            AudioSource::Mic => &mut self.mic_open,
            AudioSource::SystemAudio => &mut self.system_open,
        }
    }
}

/// The CaptureService core over an [`AudioDevice`], a [`MonotonicClock`] and [`CaptureEvents`].
pub struct CaptureCore {
    device: Box<dyn AudioDevice>,
    clock: Arc<dyn MonotonicClock>,
    events: Arc<dyn CaptureEvents>,
    /// Epoch milliseconds at the clock's origin, for `CaptureState.startedAt`.
    epoch_origin_ms: u64,
    /// The state lock: leases, the pending start or resume, the last published state.
    inner: Mutex<Inner>,
    /// The device mutex: every device call, and the open flags. Lock order: device, then
    /// state. Never take the device mutex while the state lock is held.
    dev: Mutex<DevState>,
    /// The leases changed since the device mutex holder last made the streams match them.
    dirty: AtomicBool,
    /// The backend reported these streams as failed; the next device holder closes them.
    lost_mic: AtomicBool,
    lost_system: AtomicBool,
}

impl CaptureCore {
    /// Build the core. No stream opens until a lease starts (MOG-09).
    pub fn new(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn MonotonicClock>,
        events: Arc<dyn CaptureEvents>,
    ) -> Self {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let mono_ms = u64::try_from(clock.now().as_millis()).unwrap_or(u64::MAX);
        Self {
            device,
            clock,
            events,
            epoch_origin_ms: now_ms.saturating_sub(mono_ms),
            inner: Mutex::new(Inner {
                last_state: Some(Inner::default().state()),
                ..Inner::default()
            }),
            dev: Mutex::new(DevState::default()),
            dirty: AtomicBool::new(false),
            lost_mic: AtomicBool::new(false),
            lost_system: AtomicBool::new(false),
        }
    }

    /// Set the epoch time of the clock's origin, so `startedAt` is fixed in tests.
    pub fn with_epoch_origin_ms(mut self, epoch_ms: u64) -> Self {
        self.epoch_origin_ms = epoch_ms;
        self
    }

    /// The backend reports that the stream of `source` failed (for example an unplugged mic).
    /// Every live lease that uses it stops with [`StopReason::DeviceLost`], and the next start
    /// opens the stream again (Handy's `needs_reopen`, `managers/audio.rs:294-307` @ aca8985).
    /// It never waits for the device mutex, so a backend may call it from inside `open`.
    pub fn device_failed(&self, source: AudioSource) {
        self.locked(|inner, _, evs| {
            let ids: Vec<LeaseNo> = match source {
                AudioSource::Mic => [&inner.overlay, &inner.primary]
                    .into_iter()
                    .flatten()
                    .filter(|r| r.live())
                    .map(|r| r.id)
                    .collect(),
                AudioSource::SystemAudio => inner
                    .primary
                    .iter()
                    .filter(|p| p.live() && p.others_track)
                    .map(|p| p.id)
                    .collect(),
            };
            for id in ids {
                if let Some(rec) = inner.remove(id) {
                    evs.push(rec.stopped(StopReason::DeviceLost));
                }
            }
            // A pending start or resume that needs this source must not get a lease on a
            // closed stream.
            let needs = match &inner.pending {
                Some(Pending::Start { rec, .. }) => source == AudioSource::Mic || rec.others_track,
                Some(Pending::Resume(_)) => true,
                Some(Pending::Cancelled) | None => false,
            };
            if needs {
                inner.pending_lost = true;
            }
        });
        self.lost(source).store(true, Ordering::SeqCst);
        self.request_sync();
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.now().as_millis()).unwrap_or(u64::MAX)
    }

    fn lost(&self, source: AudioSource) -> &AtomicBool {
        match source {
            AudioSource::Mic => &self.lost_mic,
            AudioSource::SystemAudio => &self.lost_system,
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, Inner> {
        let guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        self.inner.clear_poison();
        guard
    }

    fn lock_dev(&self) -> MutexGuard<'_, DevState> {
        let guard = self.dev.lock().unwrap_or_else(PoisonError::into_inner);
        self.dev.clear_poison();
        guard
    }

    /// Run `f` under the state lock and publish a changed state. The events are returned, to
    /// be sent once no lock is held.
    fn locked_defer<R>(&self, f: impl FnOnce(&mut Inner, u64, &mut Vec<Ev>) -> R) -> (R, Vec<Ev>) {
        let mut evs = Vec::new();
        let mut inner = self.lock_state();
        let now = self.now_ms();
        let result = f(&mut inner, now, &mut evs);
        let state = inner.state();
        if inner.last_state.as_ref() != Some(&state) {
            inner.last_state = Some(state.clone());
            evs.push(Ev::State(state));
        }
        (result, evs)
    }

    fn emit(&self, evs: Vec<Ev>) {
        for ev in evs {
            match ev {
                Ev::State(s) => self.events.state(&s),
                Ev::Stopped(s) => self.events.stopped(&s),
            }
        }
    }

    /// [`Self::locked_defer`], then send the events. No lock is held when they go out.
    fn locked<R>(&self, f: impl FnOnce(&mut Inner, u64, &mut Vec<Ev>) -> R) -> R {
        let (result, evs) = self.locked_defer(f);
        self.emit(evs);
        result
    }

    /// Close every stream that no lease (and no pending start or resume) needs, and every stream
    /// the backend reported lost. Runs under the device mutex; reads the leases under the state
    /// lock just before each device call (lock order: device, then state).
    fn reconcile(&self, dev: &mut DevState) {
        for source in [AudioSource::SystemAudio, AudioSource::Mic] {
            let lost = self.lost(source).swap(false, Ordering::SeqCst);
            let wanted = {
                let (mic, system) = self.lock_state().wanted();
                match source {
                    AudioSource::Mic => mic,
                    AudioSource::SystemAudio => system,
                }
            };
            if *dev.flag(source) && (lost || !wanted) {
                // New grants need the device mutex, which this thread holds, so no lease can
                // start to use this stream before it closes.
                self.device.close(source);
                *dev.flag(source) = false;
            }
        }
    }

    /// Make the streams match the leases while `dirty` is set. Never waits for the device
    /// mutex: if another thread holds it, that thread sees `dirty` before or after it lets go.
    fn sync(&self) {
        loop {
            let mut dev = match self.dev.try_lock() {
                Ok(dev) => dev,
                Err(TryLockError::Poisoned(p)) => {
                    self.dev.clear_poison();
                    p.into_inner()
                }
                Err(TryLockError::WouldBlock) => return,
            };
            while self.dirty.swap(false, Ordering::SeqCst) {
                self.reconcile(&mut dev);
            }
            drop(dev);
            if !self.dirty.load(Ordering::SeqCst) {
                return;
            }
        }
    }

    fn request_sync(&self) {
        self.dirty.store(true, Ordering::SeqCst);
        self.sync();
    }

    /// Open, under the device mutex, every stream that the pending start or resume needs.
    fn open_needed(&self, dev: &mut DevState) -> Result<(), DeviceError> {
        for source in [AudioSource::Mic, AudioSource::SystemAudio] {
            let wanted = {
                let (mic, system) = self.lock_state().wanted();
                match source {
                    AudioSource::Mic => mic,
                    AudioSource::SystemAudio => system,
                }
            };
            if wanted && !*dev.flag(source) {
                self.device.open(source)?;
                *dev.flag(source) = true;
            }
        }
        Ok(())
    }

    /// The device phase of a start or resume: take the device mutex (it waits only for a
    /// bounded close or another device phase), open what is needed, then grant or refuse under
    /// the state lock while the device mutex is still held, so the grant sees the true open
    /// flags. Then close what is no longer needed, let the device mutex go and send the events.
    fn device_phase<R>(
        &self,
        commit: impl FnOnce(&mut Inner, Result<(), DeviceError>, &DevState, &mut Vec<Ev>) -> R,
    ) -> R {
        let mut dev = self.lock_dev();
        let opened = self.open_needed(&mut dev);
        let (result, evs) = self.locked_defer(|inner, _, evs| commit(inner, opened, &dev, evs));
        self.dirty.store(true, Ordering::SeqCst);
        while self.dirty.swap(false, Ordering::SeqCst) {
            self.reconcile(&mut dev);
        }
        drop(dev);
        self.sync();
        self.emit(evs);
        result
    }

    fn set_paused(&self, lease_id: LeaseNo, paused: bool) -> Result<CaptureState, CaptureError> {
        let guard = PendingGuard::new(self);
        let result = self.locked(|inner, now, evs| {
            inner.expire(now, evs);
            if !paused && inner.pending.is_some() {
                return Err(CaptureError::MicBusy);
            }
            let rec = inner.find(lease_id).ok_or(CaptureError::UnknownLease)?;
            if rec.mode != CaptureMode::Session {
                return Err(CaptureError::InvalidMode);
            }
            let resumes = rec.paused && !paused;
            rec.paused = paused;
            if resumes {
                inner.pending = Some(Pending::Resume(lease_id));
                inner.pending_lost = false;
                guard.mine.set(Some(lease_id));
            }
            Ok((resumes, inner.state()))
        });
        let (resumes, state) = match result {
            Ok(r) => r,
            Err(e) => {
                self.request_sync();
                return Err(e);
            }
        };
        if !resumes {
            self.request_sync();
            return Ok(state);
        }
        self.device_phase(|inner, opened, dev, _| {
            guard.mine.set(None);
            match inner.pending.take() {
                Some(Pending::Resume(id)) if id == lease_id => {}
                // A stop_all ran while the device opened; the open slot is free again.
                Some(Pending::Cancelled) => return Err(CaptureError::UnknownLease),
                other => {
                    inner.pending = other;
                    return Err(CaptureError::UnknownLease);
                }
            }
            let needs_system = inner.find(lease_id).is_some_and(|r| r.others_track);
            let opened = match opened {
                Ok(()) if inner.pending_lost => Err(DEVICE_LOST),
                Ok(()) if !dev.mic_open || (needs_system && !dev.system_open) => Err(DEVICE_LOST),
                other => other,
            };
            if let Err(e) = opened {
                // The streams did not reopen: keep the session paused.
                if let Some(rec) = inner.find(lease_id) {
                    rec.paused = true;
                }
                return Err(CaptureError::Device(e));
            }
            Ok(inner.state())
        })
    }
}

/// Clears this thread's pending start or resume if the thread unwinds while it is set: a
/// panic in the device open or in an event sink. Without it the open slot stays taken and
/// every later start gets `mic_busy` (review N14). It is armed from the moment the pending
/// entry exists, before any event goes out, and disarmed when the thread takes the entry back.
struct PendingGuard<'a> {
    core: &'a CaptureCore,
    mine: Cell<Option<LeaseNo>>,
}

impl<'a> PendingGuard<'a> {
    fn new(core: &'a CaptureCore) -> Self {
        Self {
            core,
            mine: Cell::new(None),
        }
    }
}

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        let Some(id) = self.mine.get() else {
            return;
        };
        if !std::thread::panicking() {
            return;
        }
        let core = self.core;
        {
            // No events here: a second panic in a sink would abort the process. The next
            // call publishes the changed state.
            let mut inner = core.lock_state();
            // While `mine` is set, the pending entry (if any) is this thread's: there is at
            // most one, and only this thread takes it back.
            if let Some(Pending::Resume(resumed)) = inner.pending.take() {
                if resumed == id {
                    if let Some(rec) = inner.find(id) {
                        rec.paused = true;
                    }
                }
            }
        }
        // Close every stream that no lease needs, also one that a panicking open opened but
        // did not mark. The device mutex was let go when the open unwound.
        let mut dev = core.lock_dev();
        let (mic, system) = core.lock_state().wanted();
        for (source, want) in [(AudioSource::SystemAudio, system), (AudioSource::Mic, mic)] {
            if !want {
                core.device.close(source);
                *dev.flag(source) = false;
            }
        }
    }
}

impl CaptureService for CaptureCore {
    fn start(&self, request: CaptureRequest) -> Result<Lease, CaptureError> {
        if request.others_track && request.mode != CaptureMode::Session {
            return Err(CaptureError::InvalidMode);
        }
        let guard = PendingGuard::new(self);
        let lease = self.locked(|inner, now, evs| {
            inner.expire(now, evs);
            if inner.pending.is_some() {
                return Err(CaptureError::MicBusy);
            }
            // ARC-F9: one lease owns the streams; priority PTT > session > hands-free.
            let primary = inner.primary.as_ref().map(|p| (p.id, p.mode));
            let (overlay, preempt) = match (primary, inner.overlay.is_some(), request.mode) {
                (None, _, _) => (false, None),
                // A PTT suspends a hands-free capture, or is a directed segment of a session.
                (
                    Some((_, CaptureMode::HandsFree | CaptureMode::Session)),
                    false,
                    CaptureMode::Ptt,
                ) => (true, None),
                // A session takes the mic from a hands-free capture, once its streams open.
                (Some((id, CaptureMode::HandsFree)), false, CaptureMode::Session) => {
                    (false, Some(id))
                }
                _ => return Err(CaptureError::MicBusy),
            };
            inner.next_no += 1;
            let rec = LeaseRec {
                id: inner.next_no,
                mog_id: request.mog_id,
                mode: request.mode,
                started_mono: now,
                started_epoch: self.epoch_origin_ms.saturating_add(now),
                others_track: request.others_track,
                paused: false,
            };
            let lease = Lease {
                id: rec.id,
                mog_id: rec.mog_id.clone(),
                mode: rec.mode,
            };
            inner.pending = Some(Pending::Start {
                rec,
                overlay,
                preempt,
            });
            inner.pending_lost = false;
            // Armed before the events of this section go out (N14).
            guard.mine.set(Some(lease.id));
            Ok(lease)
        });
        let lease = match lease {
            Ok(lease) => lease,
            Err(e) => {
                self.request_sync();
                return Err(e);
            }
        };

        // The device phase holds only the device mutex across the open, never the state lock
        // (Handy `managers/audio.rs:495-500`).
        self.device_phase(|inner, opened, dev, evs| {
            guard.mine.set(None);
            let (rec, overlay, preempt) = match inner.pending.take() {
                Some(Pending::Start {
                    rec,
                    overlay,
                    preempt,
                }) if rec.id == lease.id => (rec, overlay, preempt),
                // A stop_all ran while the device opened: no lease was granted, and the open
                // slot is free again now that this open has returned.
                Some(Pending::Cancelled) => return Err(CaptureError::UnknownLease),
                other => {
                    inner.pending = other;
                    return Err(CaptureError::UnknownLease);
                }
            };
            // A failed open drops only the new lease; a hands-free capture it would preempt
            // keeps the mic.
            opened.map_err(CaptureError::Device)?;
            if inner.pending_lost {
                return Err(CaptureError::Device(DEVICE_LOST));
            }
            // Invariant: a lease is granted only on open streams, checked under the device
            // mutex, so no close can run between this check and the grant.
            let open = dev.mic_open && (!rec.others_track || dev.system_open);
            debug_assert!(open, "a pending start must find its streams open");
            if !open {
                return Err(CaptureError::Device(DEVICE_LOST));
            }
            if let Some(id) = preempt {
                if let Some(hf) = inner.remove(id) {
                    evs.push(hf.stopped(StopReason::Preempted));
                }
            }
            match (overlay, inner.primary.is_some()) {
                (true, true) => inner.overlay = Some(rec),
                (_, false) => inner.primary = Some(rec),
                // Not reachable: one start at a time, and leases only go away meanwhile.
                (false, true) => return Err(CaptureError::MicBusy),
            }
            Ok(lease)
        })
    }

    fn stop(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError> {
        let result = self.locked(|inner, now, evs| {
            inner.expire(now, evs);
            let rec = inner.remove(lease_id).ok_or(CaptureError::UnknownLease)?;
            evs.push(rec.stopped(StopReason::Manual));
            Ok(inner.state())
        });
        self.request_sync();
        result
    }

    fn pause(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError> {
        self.set_paused(lease_id, true)
    }

    fn resume(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError> {
        self.set_paused(lease_id, false)
    }

    fn status(&self) -> CaptureState {
        let state = self.locked(|inner, now, evs| {
            inner.expire(now, evs);
            inner.state()
        });
        self.request_sync();
        state
    }

    fn stop_all(&self) -> CaptureState {
        let state = self.locked(|inner, _, evs| {
            for rec in [inner.overlay.take(), inner.primary.take()]
                .into_iter()
                .flatten()
            {
                evs.push(rec.stopped(StopReason::Panic));
            }
            // A start in progress gets no lease (it returns `unknown_lease`), but its open slot
            // stays taken until its open returns: a new start meanwhile gets `mic_busy`.
            if inner.pending.is_some() {
                inner.pending = Some(Pending::Cancelled);
            }
            inner.state()
        });
        // MB-05: the decision above never waits for the device. The streams close now if the
        // device mutex is free; else its holder closes them when its device call returns.
        self.request_sync();
        state
    }

    fn tick(&self) -> Option<u64> {
        let next = self.locked(|inner, now, evs| {
            inner.expire(now, evs);
            [&inner.overlay, &inner.primary]
                .into_iter()
                .flatten()
                .filter_map(|rec| rec.deadline().map(|(at, _)| at))
                .min()
        });
        self.request_sync();
        next
    }
}

impl Drop for CaptureCore {
    /// The mic never stays open after the service goes. Both sources close without a
    /// condition, so a stream that a panicking open left open also closes.
    fn drop(&mut self) {
        self.device.close(AudioSource::SystemAudio);
        self.device.close(AudioSource::Mic);
    }
}
