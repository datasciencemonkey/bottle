//! Test doubles for R2 CaptureService (#4): a fake [`AudioDevice`] and a recording
//! [`CaptureEvents`] sink. They hold no audio and touch no hardware, so tests run on CI.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use crate::capture::{AudioDevice, AudioSource, CaptureEvents, DeviceError, Stopped};
use crate::types::CaptureState;

/// What the fake device did, and what it does on the next open.
#[derive(Debug, Default)]
pub struct FakeDeviceLog {
    /// The mic stream is open.
    pub mic_open: bool,
    /// The system-audio stream is open.
    pub system_open: bool,
    /// How many times the mic opened.
    pub mic_opens: u32,
    /// How many times system audio opened.
    pub system_opens: u32,
    /// Fail the next open of this source (`None`: any source) with this code.
    pub fail_next_open: Option<(Option<AudioSource>, &'static str)>,
    /// Panic in the next open, before the stream opens.
    pub panic_next_open: bool,
    /// Every open waits this long before it returns (a slow CoreAudio or WASAPI open).
    pub open_delay: Duration,
    /// An open is in progress now.
    pub opening: bool,
    /// Opens in progress now, over all sources.
    pub in_flight: u32,
    /// The most opens that were in progress at the same time.
    pub max_in_flight: u32,
}

/// A fake audio device. Clones share one [`FakeDeviceLog`]. It panics if a source opens twice.
#[derive(Debug, Default, Clone)]
pub struct FakeAudioDevice(Arc<Mutex<FakeDeviceLog>>);

impl FakeAudioDevice {
    /// A closed fake device.
    pub fn new() -> Self {
        Self::default()
    }

    /// Read or change the log.
    pub fn log(&self) -> MutexGuard<'_, FakeDeviceLog> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl AudioDevice for FakeAudioDevice {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        let delay = {
            let mut log = self.log();
            if log.panic_next_open {
                log.panic_next_open = false;
                drop(log);
                panic!("fake device panic");
            }
            if let Some((only, code)) = log.fail_next_open {
                if only.is_none() || only == Some(source) {
                    log.fail_next_open = None;
                    return Err(DeviceError { code });
                }
            }
            log.opening = true;
            log.in_flight += 1;
            log.max_in_flight = log.max_in_flight.max(log.in_flight);
            log.open_delay
        };
        if !delay.is_zero() {
            std::thread::sleep(delay);
        }
        let mut log = self.log();
        log.in_flight -= 1;
        log.opening = log.in_flight > 0;
        match source {
            AudioSource::Mic => {
                assert!(!log.mic_open, "the mic must not open twice");
                log.mic_open = true;
                log.mic_opens += 1;
            }
            AudioSource::SystemAudio => {
                assert!(!log.system_open, "system audio must not open twice");
                log.system_open = true;
                log.system_opens += 1;
            }
        }
        Ok(())
    }

    fn close(&self, source: AudioSource) {
        let mut log = self.log();
        match source {
            AudioSource::Mic => log.mic_open = false,
            AudioSource::SystemAudio => log.system_open = false,
        }
    }
}

/// A [`CaptureEvents`] sink that keeps every event.
#[derive(Debug, Default)]
pub struct RecordingCaptureEvents {
    states: Mutex<Vec<CaptureState>>,
    stops: Mutex<Vec<Stopped>>,
}

impl RecordingCaptureEvents {
    /// Every `capture.state` payload so far.
    pub fn states(&self) -> Vec<CaptureState> {
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Every stop so far.
    pub fn stops(&self) -> Vec<Stopped> {
        self.stops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The last stop, if any.
    pub fn last_stop(&self) -> Option<Stopped> {
        self.stops().last().cloned()
    }
}

impl CaptureEvents for RecordingCaptureEvents {
    fn state(&self, state: &CaptureState) {
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(state.clone());
    }

    fn stopped(&self, stopped: &Stopped) {
        self.stops
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(stopped.clone());
    }
}
