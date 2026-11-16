//! [`MicDevice`]: the audio thread, its messages and the failure notifier (#388).

use std::sync::{Arc, Weak};

use crate::capture::{AudioDevice, AudioSource, CaptureCore, DeviceError};

use super::{FrameSink, MicHost};

/// The mic backend. See the [module docs](super).
pub struct MicDevice {
    _private: (),
}

/// The link from the backend to [`CaptureCore::device_failed`]. The core owns the device, so
/// the link connects after the core is built.
#[derive(Clone)]
pub struct FailureLink {
    _private: (),
}

impl FailureLink {
    /// Report failed streams to `core`. The link holds a weak reference, so it never keeps the
    /// core alive.
    pub fn connect(&self, core: &Arc<CaptureCore>) {
        let weak: Weak<CaptureCore> = Arc::downgrade(core);
        let _ = weak;
        todo!("#388 green")
    }

    /// Report failed streams to `f`. `f` runs on the notifier thread, never on the audio thread.
    pub fn connect_fn(&self, f: impl Fn(AudioSource) + Send + Sync + 'static) {
        let _ = f;
        todo!("#388 green")
    }
}

impl MicDevice {
    /// Start the audio thread. `make_host` runs on that thread, so the host need not be `Send`.
    /// No stream opens here (MOG-09).
    pub fn spawn<H, F>(
        make_host: F,
        sink: Box<dyn FrameSink>,
    ) -> std::io::Result<(Self, FailureLink)>
    where
        H: MicHost,
        F: FnOnce() -> H + Send + 'static,
    {
        let _ = (make_host, sink);
        todo!("#388 green")
    }
}

impl AudioDevice for MicDevice {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        let _ = source;
        todo!("#388 green")
    }

    fn close(&self, source: AudioSource) {
        let _ = source;
        todo!("#388 green")
    }
}
