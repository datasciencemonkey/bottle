//! Session (red skeleton).
use std::path::PathBuf;
use std::sync::Arc;
use crate::audio_device::system::SystemAudioStatus;
use crate::capture::{CaptureError, CaptureService, LeaseNo, StopReason};
use crate::clock::{MonotonicClock, WallClock};
use crate::token_events::SessionEvent;
use super::chunker::PartFile;
use super::tap::RecordingTap;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAudioState { Ok, Unavailable, SilentOrDenied }
impl SystemAudioState { pub fn code(&self) -> &'static str { unimplemented!("red") } }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopCause { User, LengthReached, Sleep, Quit, MicLost, Panic }
impl StopCause {
    pub fn code(&self) -> &'static str { unimplemented!("red") }
    pub fn from_lease_stop(_r: StopReason) -> Self { unimplemented!("red") }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingCaptureError { Disabled, Busy, NotRecording, InvalidLength, Capture(CaptureError), Io }
impl RecordingCaptureError { pub fn code(&self) -> &'static str { unimplemented!("red") } }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartOptions { pub name: String, pub length_min: u32 }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveInfo { pub id: String, pub name: String, pub started_at_ms: i64, pub ends_at_ms: i64, pub system_audio: SystemAudioState, pub lease: LeaseNo }
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Snapshot { Idle, Recording(ActiveInfo) }
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinishedRecording { pub id: String, pub name: String, pub dir: PathBuf, pub parts: Vec<PartFile>, pub started_at_ms: i64, pub duration_ms: u64, pub cause: StopCause, pub system_audio: SystemAudioState }
pub trait PartsHandoff: Send + Sync { fn finished(&self, rec: FinishedRecording); }
pub type Observer = Arc<dyn Fn(&Snapshot) + Send + Sync>;
pub type Enabled = Arc<dyn Fn() -> bool + Send + Sync>;
#[derive(Debug, Clone)]
pub struct SessionConfig { pub temp_root: PathBuf, pub part_max_samples: u64 }
impl Default for SessionConfig { fn default() -> Self { Self { temp_root: PathBuf::new(), part_max_samples: 0 } } }
pub struct RecordingSession;
impl RecordingSession {
    #[allow(clippy::too_many_arguments)]
    pub fn new(_c: Arc<dyn CaptureService>, _t: RecordingTap, _s: SystemAudioStatus, _clock: Arc<dyn MonotonicClock>, _wall: Arc<dyn WallClock>, _en: Enabled, _h: Arc<dyn PartsHandoff>, _cfg: SessionConfig) -> Arc<Self> { Arc::new(Self) }
    pub fn start(&self, _o: StartOptions) -> Result<ActiveInfo, RecordingCaptureError> { unimplemented!("red") }
    pub fn stop(&self, _c: StopCause) -> Result<FinishedRecording, RecordingCaptureError> { unimplemented!("red") }
    pub fn snapshot(&self) -> Snapshot { unimplemented!("red") }
    pub fn tick(&self) -> Option<u64> { unimplemented!("red") }
    pub fn lease_stopped(&self, _l: LeaseNo, _r: StopReason) { unimplemented!("red") }
    pub fn session_event(&self, _e: SessionEvent) { unimplemented!("red") }
    pub fn set_observer(&self, _o: Observer) { unimplemented!("red") }
}
