//! The mic [`AudioDevice`](crate::capture::AudioDevice) backend of R2 CaptureService (#388).
//!
//! [`MicDevice`] implements the `AudioDevice` contract of [`crate::capture`] over a
//! [`MicHost`]. On macOS and Windows the host is [`cpal_backend::CpalMicHost`]; tests use a
//! fake host and never open a real mic.
//!
//! - **One audio thread.** A cpal stream is not `Send`. The audio thread makes the host and
//!   owns every stream. `open` and `close` are messages to that thread.
//! - **Open** builds the stream of the default input device, then starts it. No stream exists
//!   before `open` (MOG-09). This is the SP-08 fallback ("cold" path): SP-08 (#15) is not run
//!   by a person yet, so the stream is not pre-built. TODO(#15): pre-build if SP-08 passes.
//! - **Close** drops the stream on the audio thread and waits for that. The stream, and with it
//!   the OS mic indicator, is off before `close` returns (MOG-09, CAP-13b). There is no idle
//!   timeout, no lazy close and no always-on mode.
//! - **Close during an open** does not wait for the slow open (the 200 ms panic stop, MB-05).
//!   It cancels the open: the open drops its stream and returns [`codes::OPEN_CANCELLED`].
//!   R2 calls `close` again after the open returns.
//! - **Failure.** A fatal stream error after open stops the stream on the audio thread. Then a
//!   notifier thread calls [`CaptureCore::device_failed`](crate::capture::CaptureCore::device_failed)
//!   through the [`FailureLink`]. Transient errors (a route change, an overrun) do not stop
//!   capture.
//! - **Audio stays in RAM** (CAP-01b, #21). Frames go only to the in-RAM [`FrameSink`]. Nothing
//!   goes to disk or to a log, and there is no pre-roll. No frame arrives after `close` returns.
//! - **System audio** (the Others track) is not in this backend: `open(SystemAudio)` returns
//!   [`codes::SYSTEM_AUDIO_UNSUPPORTED`]. TODO(#108) TODO(#109).
//!
//! Device errors are the content-free codes in [`codes`]. ADR 0038 records the design.

pub mod codes;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod cpal_backend;
mod mic;
pub mod sink;

pub use mic::{FailureLink, MicDevice};
pub use sink::{mix_to_mono, DiscardSink, FrameSink, MicFrame};

use crate::capture::DeviceError;

/// One block of interleaved input samples, as the host's data callback receives it.
#[derive(Debug, Clone, Copy)]
pub struct Interleaved<'a> {
    /// The samples, converted to `f32` in -1.0..=1.0, channel by channel per frame.
    pub samples: &'a [f32],
    /// The number of channels per frame. 0 is treated as 1.
    pub channels: u16,
    /// The sample rate in Hz.
    pub sample_rate: u32,
}

/// How bad a stream error after open is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFault {
    /// The stream still runs (for example a route change or an overrun). Capture goes on.
    Transient,
    /// The stream is gone (for example an unplugged mic). R2 stops the leases that use it.
    Fatal,
}

/// The data callback of one stream. It runs on the host's real-time audio thread.
pub type DataFn = Box<dyn FnMut(Interleaved<'_>) + Send + 'static>;
/// The error callback of one stream. It may run on any host thread.
pub type FaultFn = Box<dyn FnMut(StreamFault) + Send + 'static>;

/// The seam between [`MicDevice`] and an audio API. Only the audio thread calls it, so neither
/// the host nor its stream needs to be `Send`.
pub trait MicHost: 'static {
    /// A built input stream. Dropping it stops it and releases the device (and the OS mic
    /// indicator) before the drop returns.
    type Stream: 'static;

    /// Build an input stream on the current default input device, without starting it.
    fn build(&mut self, on_data: DataFn, on_fault: FaultFn) -> Result<Self::Stream, DeviceError>;

    /// Start a built stream. The OS mic indicator comes on here.
    fn play(&mut self, stream: &Self::Stream) -> Result<(), DeviceError>;
}
