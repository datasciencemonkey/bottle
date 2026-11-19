//! System audio (red skeleton).
use std::time::Duration;
use crate::capture::{AudioDevice, AudioSource, DeviceError};
use super::{FrameSink, MicHost};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemAudioAvailability { Off, Live, Unavailable }
#[derive(Debug, Clone, Default)]
pub struct SystemAudioStatus;
impl SystemAudioStatus { pub fn get(&self) -> SystemAudioAvailability { unimplemented!("red") } }
pub struct SystemAudioDevice;
impl SystemAudioDevice {
    pub fn spawn<H: MicHost, F: FnOnce() -> H + Send + 'static>(_f: F, _s: Box<dyn FrameSink>, _t: Duration) -> std::io::Result<(Self, SystemAudioStatus)> { unimplemented!("red") }
    pub fn unavailable() -> (Self, SystemAudioStatus) { unimplemented!("red") }
}
impl AudioDevice for SystemAudioDevice {
    fn open(&self, _s: AudioSource) -> Result<(), DeviceError> { unimplemented!("red") }
    fn close(&self, _s: AudioSource) {}
}
pub struct RecordingDevice;
impl RecordingDevice { pub fn new(_m: Box<dyn AudioDevice>, _s: Box<dyn AudioDevice>) -> Self { Self } }
impl AudioDevice for RecordingDevice {
    fn open(&self, _s: AudioSource) -> Result<(), DeviceError> { unimplemented!("red") }
    fn close(&self, _s: AudioSource) {}
}
