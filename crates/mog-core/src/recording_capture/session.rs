//! The recording session: start, stop and the stop rules of one call recording (ADR 0103).
//!
//! - **Knob gate.** [`RecordingSession::start`] asks the `enabled` check (the
//!   `settings.enabled` knob) first. When it is off, nothing starts: no mic lease, no system
//!   audio tap and no temp file.
//! - **One recording at a time.** A second start gets `recording_busy`.
//! - **Capture.** The recording holds one R2 `Session` lease with the Others track
//!   (`others_track: true`), so CaptureService is still the only owner of the mic and system
//!   audio (CAP-01a). A PTT during a recording is an R2 overlay on that lease (R2 shares the
//!   open mic); hands-free gives way to the recording (ARC-F9).
//! - **Frames** reach the writer thread through the [`RecordingTap`]. The writer mixes
//!   ([`Mixer`]) and writes the parts ([`PartWriter`], one FLAC part by default) in a
//!   user-only temp dir.
//! - **Stop rules.** The user's stop, the length preset (`length_min`, 5..=180,
//!   [`RecordingSession::tick`]), OS sleep ([`RecordingSession::session_event`]), app quit
//!   (the host calls `stop(Quit)`), and the end of the lease (mic loss, CAP-17b zero buffers,
//!   the MB-05 panic stop; [`RecordingSession::lease_stopped`]). Every stop finalizes the open
//!   part and hands the parts to the cloud side ([`PartsHandoff`]) at once.
//! - **CAP-10a.** When system audio has had no block with sound for 5 s, `system_audio` is
//!   `silent_or_denied` (a denied System Audio Recording permission gives digital silence on
//!   macOS). Sound clears it. The recording never stops for it.
//!
//! Time: the length preset and the 5 s check run on the injected [`MonotonicClock`];
//! `started_at_ms` / `ends_at_ms` are wall clock epoch ms for the UI. Nothing here logs audio,
//! a path with the user name, or a token.

use std::fmt;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

use crate::audio_device::system::{SystemAudioAvailability, SystemAudioStatus};
use crate::capture::{CaptureError, CaptureRequest, CaptureService, LeaseNo, StopReason};
use crate::clock::{MonotonicClock, WallClock};
use crate::token_events::SessionEvent;
use crate::types::{CaptureMode, MogId};

use super::chunker::{create_private_dir, PartFile, PartFormat, PartWriter};
use super::mixer::Mixer;
use super::tap::{Block, RecordingTap};
use super::PART_MAX_SAMPLES;

/// The shortest length preset, in minutes.
pub const LENGTH_MIN_MIN: u32 = 5;
/// The longest length preset, in minutes.
pub const LENGTH_MIN_MAX: u32 = 180;
/// The default length preset, in minutes.
pub const LENGTH_MIN_DEFAULT: u32 = 60;
/// CAP-10a: system audio with no sound for this long is `silent_or_denied`.
pub const SYSTEM_SILENCE_MS: u64 = 5_000;
/// The MOG ID of the recording's R2 lease (a recording belongs to no MOG).
pub const RECORDING_MOG_ID: &str = "bottle-recording";
/// The mixer's lag bound: 250 ms at 16 kHz.
pub const MIX_LAG_SAMPLES: usize = 4_000;

/// The `system_audio` field of the recording state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAudioState {
    /// System audio is open and had sound in the last 5 s.
    Ok,
    /// No system audio source (an older OS, no backend, a refused or failed tap).
    Unavailable,
    /// CAP-10a: 5 s with no sound, for example a denied System Audio Recording permission.
    SilentOrDenied,
}

impl SystemAudioState {
    /// The stable wire code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unavailable => "unavailable",
            Self::SilentOrDenied => "silent_or_denied",
        }
    }
}

/// Why a recording stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause {
    /// The user stopped it.
    User,
    /// The length preset ran out.
    LengthReached,
    /// The computer went to sleep.
    Sleep,
    /// The app quit.
    Quit,
    /// The mic went away or gave only zero buffers (CAP-17b), or another capture took it.
    MicLost,
    /// The panic stop (MB-05).
    Panic,
}

impl StopCause {
    /// The stable wire code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::LengthReached => "length_reached",
            Self::Sleep => "sleep",
            Self::Quit => "quit",
            Self::MicLost => "mic_lost",
            Self::Panic => "panic",
        }
    }

    /// The cause of a recording whose R2 lease stopped for `reason`.
    pub fn from_lease_stop(reason: StopReason) -> Self {
        match reason {
            StopReason::Panic => Self::Panic,
            StopReason::Manual => Self::User,
            _ => Self::MicLost,
        }
    }
}

/// A refused start or stop. Only [`RecordingCaptureError::code`] crosses IPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingCaptureError {
    /// The recording knob is off.
    Disabled,
    /// A recording runs already.
    Busy,
    /// No recording runs.
    NotRecording,
    /// `length_min` is outside 5..=180.
    InvalidLength,
    /// R2 refused the mic lease (`mic_busy`, a device error).
    Capture(CaptureError),
    /// The temp dir or a part file could not be written.
    Io,
}

impl RecordingCaptureError {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Disabled => "recording_disabled",
            Self::Busy => "recording_busy",
            Self::NotRecording => "not_recording",
            Self::InvalidLength => "invalid_args",
            Self::Capture(e) => e.code(),
            Self::Io => "host_failed",
        }
    }
}

impl fmt::Display for RecordingCaptureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "recording error: {}", self.code())
    }
}

impl std::error::Error for RecordingCaptureError {}

/// A start request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartOptions {
    /// The recording name (`YYYY-MM-DD_HHMM`, `_2`.. on a clash); the cloud side picks it.
    pub name: String,
    /// The length preset in minutes (5..=180).
    pub length_min: u32,
}

/// The running recording, for the state event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveInfo {
    /// The recording ID (random hex).
    pub id: String,
    /// The recording name.
    pub name: String,
    /// Wall clock epoch ms of the start.
    pub started_at_ms: i64,
    /// Wall clock epoch ms at which the length preset stops it.
    pub ends_at_ms: i64,
    /// The system audio state.
    pub system_audio: SystemAudioState,
    /// The R2 lease of the recording.
    pub lease: LeaseNo,
}

/// The session state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot {
    /// No recording runs.
    Idle,
    /// A recording runs.
    Recording(ActiveInfo),
}

/// A finished recording, handed to the cloud side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRecording {
    /// The recording ID.
    pub id: String,
    /// The recording name.
    pub name: String,
    /// The local temp dir that holds the parts. The cloud side deletes it after the upload.
    pub dir: PathBuf,
    /// The parts, in order. Empty if no audio came in.
    pub parts: Vec<PartFile>,
    /// Wall clock epoch ms of the start.
    pub started_at_ms: i64,
    /// The recording length on the monotonic clock, in ms.
    pub duration_ms: u64,
    /// Why it stopped.
    pub cause: StopCause,
    /// The system audio state at the stop.
    pub system_audio: SystemAudioState,
}

/// The receiver of finished recordings (the upload pipeline of the cloud side). It must not
/// block for long: it runs on the thread that stopped the recording.
pub trait PartsHandoff: Send + Sync {
    /// One recording finished.
    fn finished(&self, rec: FinishedRecording);
}

/// A state observer (the `recording.state` relay).
pub type Observer = Arc<dyn Fn(&Snapshot) + Send + Sync>;
/// The knob check (`settings.enabled`).
pub type Enabled = Arc<dyn Fn() -> bool + Send + Sync>;

/// The session settings.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Where per-recording temp dirs go (the app's cache dir in the host).
    pub temp_root: PathBuf,
    /// The part cap in samples; `None` (the default, [`PART_MAX_SAMPLES`]) is one part.
    pub part_max_samples: Option<u64>,
    /// The CAP-10a window.
    pub silence_ms: u64,
    /// The part encoder (FLAC by default).
    pub part_format: PartFormat,
    /// The mixer lag bound.
    pub mix_lag_samples: usize,
    /// The MOG ID of the R2 lease.
    pub mog_id: MogId,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            temp_root: std::env::temp_dir().join("bottle-recordings"),
            part_max_samples: PART_MAX_SAMPLES,
            silence_ms: SYSTEM_SILENCE_MS,
            part_format: PartFormat::Flac,
            mix_lag_samples: MIX_LAG_SAMPLES,
            mog_id: RECORDING_MOG_ID.to_string(),
        }
    }
}

struct Active {
    info: ActiveInfo,
    dir: PathBuf,
    started_mono: u64,
    ends_mono: u64,
    writer: JoinHandle<std::io::Result<Vec<PartFile>>>,
}

#[derive(Default)]
struct State {
    active: Option<Active>,
    /// A start is between its checks and its lease.
    starting: bool,
}

/// The recording session. See the [module docs](self).
pub struct RecordingSession {
    capture: Arc<dyn CaptureService>,
    tap: RecordingTap,
    system: SystemAudioStatus,
    clock: Arc<dyn MonotonicClock>,
    wall: Arc<dyn WallClock>,
    enabled: Enabled,
    handoff: Arc<dyn PartsHandoff>,
    config: SessionConfig,
    state: Mutex<State>,
    observer: Mutex<Option<Observer>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn random_id() -> String {
    let mut bytes = [0u8; 8];
    if getrandom::fill(&mut bytes).is_err() {
        // No OS randomness: fall back to the time, still unique per process run.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos() as u64);
        bytes = t.to_be_bytes();
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl RecordingSession {
    /// A session over R2 `capture`. `tap` must be the tap whose sinks the mic and the system
    /// audio device feed; `system` the status of that system audio device.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        capture: Arc<dyn CaptureService>,
        tap: RecordingTap,
        system: SystemAudioStatus,
        clock: Arc<dyn MonotonicClock>,
        wall: Arc<dyn WallClock>,
        enabled: Enabled,
        handoff: Arc<dyn PartsHandoff>,
        config: SessionConfig,
    ) -> Arc<Self> {
        Arc::new(Self {
            capture,
            tap,
            system,
            clock,
            wall,
            enabled,
            handoff,
            config,
            state: Mutex::new(State::default()),
            observer: Mutex::new(None),
        })
    }

    /// Send each state change to `observer`.
    pub fn set_observer(&self, observer: Observer) {
        *lock(&self.observer) = Some(observer);
    }

    fn now_ms(&self) -> u64 {
        u64::try_from(self.clock.now().as_millis()).unwrap_or(u64::MAX)
    }

    fn notify(&self, snap: &Snapshot) {
        let observer = lock(&self.observer).clone();
        if let Some(o) = observer {
            o(snap);
        }
    }

    /// Whether the recording knob is on.
    pub fn enabled(&self) -> bool {
        (self.enabled)()
    }

    /// Start a recording. See the module docs for the checks.
    pub fn start(&self, opts: StartOptions) -> Result<ActiveInfo, RecordingCaptureError> {
        if !(self.enabled)() {
            return Err(RecordingCaptureError::Disabled);
        }
        if !(LENGTH_MIN_MIN..=LENGTH_MIN_MAX).contains(&opts.length_min) {
            return Err(RecordingCaptureError::InvalidLength);
        }
        {
            let mut st = lock(&self.state);
            if st.active.is_some() || st.starting {
                return Err(RecordingCaptureError::Busy);
            }
            st.starting = true;
        }
        let result = self.start_inner(opts);
        let mut st = lock(&self.state);
        st.starting = false;
        let active = result?;
        let info = active.info.clone();
        st.active = Some(active);
        drop(st);
        self.notify(&Snapshot::Recording(info.clone()));
        Ok(info)
    }

    fn start_inner(&self, opts: StartOptions) -> Result<Active, RecordingCaptureError> {
        let id = random_id();
        let dir = self.config.temp_root.join(&id);
        create_private_dir(&self.config.temp_root)
            .and_then(|()| create_private_dir(&dir))
            .map_err(|_| RecordingCaptureError::Io)?;
        let parts = match PartWriter::with_format(
            &dir,
            &opts.name,
            self.config.part_max_samples.unwrap_or(u64::MAX),
            self.config.part_format,
        ) {
            Ok(p) => p,
            Err(_) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(RecordingCaptureError::Io);
            }
        };
        let lease = match self.capture.start(CaptureRequest {
            mog_id: self.config.mog_id.clone(),
            mode: CaptureMode::Session,
            others_track: true,
        }) {
            Ok(lease) => lease,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(RecordingCaptureError::Capture(e));
            }
        };
        // Frames count from here: the streams are open.
        let rx = self.tap.arm();
        let lag = self.config.mix_lag_samples;
        let writer = thread::Builder::new()
            .name("bottle-rec-writer".into())
            .spawn(move || write_loop(rx, Mixer::new(lag), parts));
        let writer = match writer {
            Ok(w) => w,
            Err(_) => {
                self.tap.disarm();
                let _ = self.capture.stop(lease.id);
                let _ = std::fs::remove_dir_all(&dir);
                return Err(RecordingCaptureError::Io);
            }
        };
        let started_mono = self.now_ms();
        let length_ms = u64::from(opts.length_min) * 60_000;
        let started_at_ms = self.wall.now_ms();
        let system_audio = match self.system.get() {
            SystemAudioAvailability::Live => SystemAudioState::Ok,
            _ => SystemAudioState::Unavailable,
        };
        Ok(Active {
            info: ActiveInfo {
                id,
                name: opts.name,
                started_at_ms,
                ends_at_ms: started_at_ms.saturating_add(length_ms as i64),
                system_audio,
                lease: lease.id,
            },
            dir,
            started_mono,
            ends_mono: started_mono.saturating_add(length_ms),
            writer,
        })
    }

    /// Stop the recording, finalize its parts and hand them off.
    pub fn stop(&self, cause: StopCause) -> Result<FinishedRecording, RecordingCaptureError> {
        let active = lock(&self.state)
            .active
            .take()
            .ok_or(RecordingCaptureError::NotRecording)?;
        // The lease is gone already after a mic loss or a panic stop. The stop event of our
        // own stop finds no active recording, so it does nothing.
        if !matches!(cause, StopCause::MicLost | StopCause::Panic) {
            let _ = self.capture.stop(active.info.lease);
        }
        self.tap.disarm();
        let duration_ms = self.now_ms().saturating_sub(active.started_mono);
        let system_audio = self.system_state(&active.info);
        let parts = match active.writer.join() {
            Ok(Ok(parts)) => parts,
            // A write error: hand off what was finished (the open part may be cut short).
            Ok(Err(_)) | Err(_) => existing_parts(&active.dir),
        };
        let rec = FinishedRecording {
            id: active.info.id,
            name: active.info.name,
            dir: active.dir,
            parts,
            started_at_ms: active.info.started_at_ms,
            duration_ms,
            cause,
            system_audio,
        };
        self.notify(&Snapshot::Idle);
        self.handoff.finished(rec.clone());
        Ok(rec)
    }

    /// The state now.
    pub fn snapshot(&self) -> Snapshot {
        match &lock(&self.state).active {
            Some(a) => Snapshot::Recording(a.info.clone()),
            None => Snapshot::Idle,
        }
    }

    fn system_state(&self, info: &ActiveInfo) -> SystemAudioState {
        if info.system_audio == SystemAudioState::Unavailable
            || self.system.get() != SystemAudioAvailability::Live
        {
            return SystemAudioState::Unavailable;
        }
        let quiet = self
            .now_ms()
            .saturating_sub(self.tap.last_system_sound_ms());
        if quiet >= self.config.silence_ms {
            SystemAudioState::SilentOrDenied
        } else {
            SystemAudioState::Ok
        }
    }

    /// Apply the length preset and the CAP-10a check at the clock's time now. Returns the
    /// next deadline in monotonic ms, or `None` when no recording runs. A host timer calls it
    /// (every 250 ms is enough); tests call it after they move the clock.
    pub fn tick(&self) -> Option<u64> {
        let now = self.now_ms();
        let mut st = lock(&self.state);
        let active = st.active.as_mut()?;
        if now >= active.ends_mono {
            drop(st);
            let _ = self.stop(StopCause::LengthReached);
            return None;
        }
        let next = self.system_state(&active.info);
        let changed = next != active.info.system_audio
            && active.info.system_audio != SystemAudioState::Unavailable;
        if changed {
            active.info.system_audio = next;
        }
        let silence_at = self
            .tap
            .last_system_sound_ms()
            .saturating_add(self.config.silence_ms);
        let deadline = if active.info.system_audio == SystemAudioState::Ok {
            active.ends_mono.min(silence_at)
        } else {
            active.ends_mono
        };
        let snap = Snapshot::Recording(active.info.clone());
        drop(st);
        if changed {
            self.notify(&snap);
        }
        Some(deadline)
    }

    /// R2 stopped `lease` for `reason` (the host's `capture.stopped` chain). If it is the
    /// recording's lease, the recording stops and finalizes.
    pub fn lease_stopped(&self, lease: LeaseNo, reason: StopReason) {
        let ours = lock(&self.state)
            .active
            .as_ref()
            .is_some_and(|a| a.info.lease == lease);
        if ours {
            let _ = self.stop(StopCause::from_lease_stop(reason));
        }
    }

    /// An OS event: sleep stops the recording; a screen lock does not.
    pub fn session_event(&self, event: SessionEvent) {
        if event == SessionEvent::Sleep {
            let _ = self.stop(StopCause::Sleep);
        }
    }
}

/// The writer thread: mix each block, write parts, flush at the end of the queue.
fn write_loop(
    rx: Receiver<Block>,
    mut mixer: Mixer,
    mut parts: PartWriter,
) -> std::io::Result<Vec<PartFile>> {
    let mut out = Vec::new();
    for block in rx {
        mixer.push(block.track, &block.samples, block.rate);
        out.clear();
        mixer.drain(&mut out);
        parts.write(&out)?;
    }
    out.clear();
    mixer.flush(&mut out);
    parts.write(&out)?;
    parts.finish()
}

/// The part files in `dir`, by name, after a failed write (best effort).
fn existing_parts(dir: &std::path::Path) -> Vec<PartFile> {
    let mut found: Vec<PartFile> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let file_name = e.file_name().into_string().ok()?;
            let index = file_name
                .rsplit_once('.')?
                .0
                .rsplit_once(".part")?
                .1
                .parse()
                .ok()?;
            Some(PartFile {
                index,
                path: e.path(),
                file_name,
                samples: 0,
                offset_ms: 0,
            })
        })
        .collect();
    found.sort_by_key(|p| p.index);
    found
}
