//! R2 CaptureService (section 4.3).
//!
//! Responsibilities: The only owner of the mic and system audio. Leases, You and Others tracks, Silero VAD, segments (15-20 s target, 30 s hard), level stream, encrypted journal (60 s rotation, crypto-shred), raw-audio deletion.
//!
//! This module is the device-free core (#4): one owner of the mic and system-audio streams
//! (CAP-01a), capture leases with IDs (CAP-02a), the three modes (CAP-03), the locked 120 s
//! hold and 5 min hands-free caps (CAP-04a, CAP-04b), ARC-F9 priority and the panic stop
//! (MB-05). Time comes from an injectable [`Clock`], audio streams from an [`AudioDevice`]
//! and state changes go to [`CaptureEvents`].
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

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
        Ok(no) if no > 0 && wire.bytes().all(|b| b.is_ascii_digit()) => Ok(no),
        _ => Err(CaptureError::UnknownLease),
    }
}

/// A time source. Tests use a fake clock; [`SystemClock`] is the real one.
pub trait Clock: Send + Sync {
    /// Monotonic milliseconds. The caps use this time.
    fn monotonic_ms(&self) -> u64;
    /// Epoch milliseconds, for `CaptureState.startedAt`.
    fn epoch_ms(&self) -> u64;
}

/// The real clock: [`Instant`] for the caps, [`SystemTime`] for `startedAt`.
#[derive(Debug)]
pub struct SystemClock {
    base: Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        Self {
            base: Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn monotonic_ms(&self) -> u64 {
        u64::try_from(self.base.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn epoch_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
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

/// The audio device backend. CaptureService is its only caller, and it opens each source
/// at most once at a time. A cpal stream is not `Send`, so a real backend owns its streams on
/// its own audio thread and takes open and close as messages.
pub trait AudioDevice: Send + Sync {
    /// Open the stream of `source`.
    fn open(&self, source: AudioSource) -> Result<(), DeviceError>;
    /// Close the stream of `source`. Closing a closed stream does nothing.
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
/// calls it with its state lock held, so it must not call back into the service.
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
    /// Start a capture. Returns a new lease, or `mic_busy` (ARC-F9 priority).
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
}

#[derive(Default)]
struct Inner {
    /// The lease that owns the streams.
    primary: Option<LeaseRec>,
    /// A PTT on top of the primary lease: it suspends a hands-free lease (ARC-F9) or is a
    /// directed segment of a session.
    overlay: Option<LeaseRec>,
    next_id: LeaseNo,
    mic_open: bool,
    system_open: bool,
    last_state: Option<CaptureState>,
}

/// The CaptureService core over an [`AudioDevice`], a [`Clock`] and [`CaptureEvents`].
pub struct CaptureCore {
    device: Box<dyn AudioDevice>,
    clock: Arc<dyn Clock>,
    events: Arc<dyn CaptureEvents>,
    inner: Mutex<Inner>,
}

impl CaptureCore {
    /// Build the core. No stream opens until a lease starts (MOG-09).
    pub fn new(
        device: Box<dyn AudioDevice>,
        clock: Arc<dyn Clock>,
        events: Arc<dyn CaptureEvents>,
    ) -> Self {
        Self {
            device,
            clock,
            events,
            inner: Mutex::new(Inner {
                last_state: Some(Self::state_of(&Inner::default())),
                ..Inner::default()
            }),
        }
    }

    /// Lock the state. A panic in another call must not keep the mic locked or held: a
    /// poisoned lock is taken over, every lease stops with [`StopReason::Panic`] and both
    /// streams close.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                let mut guard = poisoned.into_inner();
                self.inner.clear_poison();
                self.release_all(&mut guard);
                guard
            }
        }
    }

    /// Stop every lease with [`StopReason::Panic`] and close both streams, even if a panic
    /// left the open flags out of line.
    fn release_all(&self, inner: &mut Inner) -> CaptureState {
        for rec in [inner.overlay.take(), inner.primary.take()]
            .into_iter()
            .flatten()
        {
            self.report_stop(rec, StopReason::Panic);
        }
        self.device.close(AudioSource::SystemAudio);
        self.device.close(AudioSource::Mic);
        inner.system_open = false;
        inner.mic_open = false;
        self.publish(inner)
    }

    fn state_of(inner: &Inner) -> CaptureState {
        let others_track = if inner.system_open {
            OthersTrack::Ok
        } else {
            OthersTrack::Off
        };
        let idle = CaptureState {
            owner: None,
            mode: None,
            lease_id: None,
            recording: false,
            paused: false,
            session_id: None,
            started_at: None,
            directed_to: None,
            others_track,
        };
        let Some(primary) = &inner.primary else {
            return idle;
        };
        match (&inner.overlay, primary.mode) {
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
                recording: !primary.paused || overlay.is_some(),
                paused: primary.paused,
                session_id: (mode == CaptureMode::Session)
                    .then(|| format!("session-{}", primary.id)),
                started_at: Some(primary.started_epoch),
                directed_to: overlay.as_ref().map(|o| o.mog_id.clone()),
                ..idle
            },
        }
    }

    /// Open or close the streams so they match the leases.
    fn reconcile(&self, inner: &mut Inner) -> Result<(), DeviceError> {
        let primary_live = inner.primary.as_ref().is_some_and(|p| !p.paused);
        let want_mic = primary_live || inner.overlay.is_some();
        let want_system = primary_live && inner.primary.as_ref().is_some_and(|p| p.others_track);

        if !want_system && inner.system_open {
            self.device.close(AudioSource::SystemAudio);
            inner.system_open = false;
        }
        if !want_mic && inner.mic_open {
            self.device.close(AudioSource::Mic);
            inner.mic_open = false;
        }
        if want_mic && !inner.mic_open {
            self.device.open(AudioSource::Mic)?;
            inner.mic_open = true;
        }
        if want_system && !inner.system_open {
            self.device.open(AudioSource::SystemAudio)?;
            inner.system_open = true;
        }
        Ok(())
    }

    fn publish(&self, inner: &mut Inner) -> CaptureState {
        let state = Self::state_of(inner);
        if inner.last_state.as_ref() != Some(&state) {
            self.events.state(&state);
            inner.last_state = Some(state.clone());
        }
        state
    }

    fn report_stop(&self, rec: LeaseRec, reason: StopReason) {
        self.events.stopped(&Stopped {
            lease_id: rec.id,
            mog_id: rec.mog_id,
            mode: rec.mode,
            reason,
        });
    }

    /// Take a lease out. When the primary goes, the overlay PTT (if any) becomes primary.
    fn remove(inner: &mut Inner, lease_id: LeaseNo) -> Option<LeaseRec> {
        if inner.overlay.as_ref().is_some_and(|o| o.id == lease_id) {
            return inner.overlay.take();
        }
        if inner.primary.as_ref().is_some_and(|p| p.id == lease_id) {
            let rec = inner.primary.take();
            inner.primary = inner.overlay.take();
            return rec;
        }
        None
    }

    /// Stop every lease whose cap has passed.
    fn expire(&self, inner: &mut Inner) {
        let now = self.clock.monotonic_ms();
        let due: Vec<(LeaseNo, StopReason)> = [&inner.overlay, &inner.primary]
            .into_iter()
            .flatten()
            .filter_map(|rec| match rec.deadline() {
                Some((at, reason)) if now >= at => Some((rec.id, reason)),
                _ => None,
            })
            .collect();
        for (id, reason) in due {
            if let Some(rec) = Self::remove(inner, id) {
                self.report_stop(rec, reason);
            }
        }
    }

    /// Close the streams and settle the state; device errors on close cannot happen.
    fn settle(&self, inner: &mut Inner) -> CaptureState {
        // Leases only went away, so this closes streams and opens none.
        let _ = self.reconcile(inner);
        self.publish(inner)
    }

    fn set_paused(&self, lease_id: LeaseNo, paused: bool) -> Result<CaptureState, CaptureError> {
        let mut inner = self.lock();
        self.expire(&mut inner);
        self.settle(&mut inner);
        let rec = Self::find(&mut inner, lease_id).ok_or(CaptureError::UnknownLease)?;
        if rec.mode != CaptureMode::Session {
            return Err(CaptureError::InvalidMode);
        }
        rec.paused = paused;
        if let Err(e) = self.reconcile(&mut inner) {
            // The streams did not reopen: keep the session paused.
            if let Some(rec) = Self::find(&mut inner, lease_id) {
                rec.paused = true;
            }
            self.settle(&mut inner);
            return Err(CaptureError::Device(e));
        }
        Ok(self.publish(&mut inner))
    }

    fn find(inner: &mut Inner, lease_id: LeaseNo) -> Option<&mut LeaseRec> {
        [&mut inner.primary, &mut inner.overlay]
            .into_iter()
            .flatten()
            .find(|rec| rec.id == lease_id)
    }
}

impl CaptureService for CaptureCore {
    fn start(&self, request: CaptureRequest) -> Result<Lease, CaptureError> {
        if request.others_track && request.mode != CaptureMode::Session {
            return Err(CaptureError::InvalidMode);
        }
        let mut inner = self.lock();
        self.expire(&mut inner);
        self.settle(&mut inner);

        // ARC-F9: one lease owns the streams; priority PTT > session > hands-free.
        let primary_mode = inner.primary.as_ref().map(|p| p.mode);
        let as_overlay = match (primary_mode, inner.overlay.is_some(), request.mode) {
            (None, _, _) => false,
            // A PTT suspends a hands-free capture, or is a directed segment of a session.
            (Some(CaptureMode::HandsFree | CaptureMode::Session), false, CaptureMode::Ptt) => true,
            // A session takes the mic from a hands-free capture.
            (Some(CaptureMode::HandsFree), false, CaptureMode::Session) => {
                if let Some(rec) = inner.primary.take() {
                    self.report_stop(rec, StopReason::Preempted);
                }
                false
            }
            _ => return Err(CaptureError::MicBusy),
        };

        inner.next_id += 1;
        let rec = LeaseRec {
            id: inner.next_id,
            mog_id: request.mog_id,
            mode: request.mode,
            started_mono: self.clock.monotonic_ms(),
            started_epoch: self.clock.epoch_ms(),
            others_track: request.others_track,
            paused: false,
        };
        let lease = Lease {
            id: rec.id,
            mog_id: rec.mog_id.clone(),
            mode: rec.mode,
        };
        if as_overlay {
            inner.overlay = Some(rec);
        } else {
            inner.primary = Some(rec);
        }

        if let Err(e) = self.reconcile(&mut inner) {
            // Roll back: the failed lease never existed.
            Self::remove(&mut inner, lease.id);
            self.settle(&mut inner);
            return Err(CaptureError::Device(e));
        }
        self.publish(&mut inner);
        Ok(lease)
    }

    fn stop(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError> {
        let mut inner = self.lock();
        self.expire(&mut inner);
        let result = match Self::remove(&mut inner, lease_id) {
            Some(rec) => {
                self.report_stop(rec, StopReason::Manual);
                Ok(())
            }
            None => Err(CaptureError::UnknownLease),
        };
        let state = self.settle(&mut inner);
        result.map(|()| state)
    }

    fn pause(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError> {
        self.set_paused(lease_id, true)
    }

    fn resume(&self, lease_id: LeaseNo) -> Result<CaptureState, CaptureError> {
        self.set_paused(lease_id, false)
    }

    fn status(&self) -> CaptureState {
        let mut inner = self.lock();
        self.expire(&mut inner);
        self.settle(&mut inner)
    }

    fn stop_all(&self) -> CaptureState {
        let mut inner = self.lock();
        self.release_all(&mut inner)
    }

    fn tick(&self) -> Option<u64> {
        let mut inner = self.lock();
        self.expire(&mut inner);
        self.settle(&mut inner);
        [&inner.overlay, &inner.primary]
            .into_iter()
            .flatten()
            .filter_map(|rec| rec.deadline().map(|(at, _)| at))
            .min()
    }
}

impl Drop for CaptureCore {
    /// The mic never stays open after the service goes, also when a panic unwinds it.
    fn drop(&mut self) {
        let inner = self.inner.get_mut().unwrap_or_else(|e| e.into_inner());
        if inner.system_open {
            self.device.close(AudioSource::SystemAudio);
        }
        if inner.mic_open {
            self.device.close(AudioSource::Mic);
        }
    }
}
