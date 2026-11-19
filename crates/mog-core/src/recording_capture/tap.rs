//! Tap (red skeleton).
use std::sync::Arc;
use crate::audio_device::{FrameSink, MicFrame};
use crate::clock::MonotonicClock;
#[derive(Clone)]
pub struct RecordingTap;
pub struct TapSink;
impl FrameSink for TapSink { fn frame(&mut self, _f: MicFrame<'_>) {} }
impl RecordingTap {
    pub fn new(_clock: Arc<dyn MonotonicClock>) -> Self { Self }
    pub fn mic_sink(&self) -> TapSink { TapSink }
    pub fn system_sink(&self) -> TapSink { TapSink }
}
