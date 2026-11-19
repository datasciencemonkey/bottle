//! Recording host (red skeleton).
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use mog_core::audio_device::system::SystemAudioStatus;
use mog_core::clock::{MonotonicClock, WallClock};
use mog_core::recording_capture::permissions::PermissionAsker;
use mog_core::recording_capture::session::{PartsHandoff, RecordingSession, SessionConfig};
use mog_core::recording_capture::tap::RecordingTap;
use mog_core::token_events::SessionEventSource;
use crate::capture_host::CaptureHost;
pub const TICK_EVERY: Duration = Duration::from_millis(250);
pub struct RecordingHost { session: Arc<RecordingSession>, _p: PathBuf }
impl RecordingHost {
    #[allow(clippy::too_many_arguments)]
    pub fn new(_c: &CaptureHost, _t: RecordingTap, _s: SystemAudioStatus, _clock: Arc<dyn MonotonicClock>, _w: Arc<dyn WallClock>, _h: Arc<dyn PartsHandoff>, _cfg: SessionConfig) -> std::io::Result<Self> { unimplemented!("red") }
    pub fn session(&self) -> &Arc<RecordingSession> { &self.session }
    pub fn enabled(&self) -> bool { unimplemented!("red") }
    pub fn set_enabled(&self, _on: bool, _a: &dyn PermissionAsker) { unimplemented!("red") }
    pub fn watch_sleep(&self, _s: &dyn SessionEventSource) -> mog_core::error::CoreResult<()> { unimplemented!("red") }
    pub fn shutdown(&self) { unimplemented!("red") }
}
