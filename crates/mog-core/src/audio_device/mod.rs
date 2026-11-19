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
//! - **Close during an open.** R2 runs one device call at a time, so it never does this. If
//!   another caller does, `close` does not wait for the slow open (the 200 ms panic stop,
//!   MB-05). It cancels the open: the open drops its stream and returns
//!   [`codes::OPEN_CANCELLED`].
//! - **Failure.** A fatal stream error after open stops the stream on the audio thread. Then a
//!   notifier thread calls [`CaptureCore::device_failed`](crate::capture::CaptureCore::device_failed)
//!   through the [`FailureLink`]. Transient errors (a route change, an overrun) do not stop
//!   capture.
//! - **Audio stays in RAM** (CAP-01b, CAP-12c, #21). Frames go only to the in-RAM [`FrameSink`]:
//!   the level sink and [`ptt_buffer::PttBuffer`] (ADR 0078). Nothing goes to disk or to a log,
//!   and there is no pre-roll. No frame arrives after `close` returns.
//! - **System audio** (the Others track) is not in this backend: `open(SystemAudio)` returns
//!   [`codes::SYSTEM_AUDIO_UNSUPPORTED`]. TODO(#108) TODO(#109).
//!
//! - **Permission (CAP-17a, #22).** Nothing asks the OS for the mic before the first `open`,
//!   and only a user action (`capture_start`) opens. On macOS the first stream build shows the
//!   Microphone prompt. A denied mic gives digital silence there, which the zero watch
//!   catches. Windows refuses the build (`permission_denied`).
//! - **Zero buffers (CAP-17b, #22).** While a stream is open, the audio thread runs a
//!   [`ZeroWatch`] with a 2 s window ([`MicConfig`]). If no block with sound comes in the
//!   window, the stream stops, the sink gets [`SinkEvent::Discard`] (no transcript), and the
//!   [`FailureLink`] reports [`MicFault::ZeroBuffers`] so the MOG shows the fix-it card.
//! - **Default device change (CAP-18, #22).** A cpal input stream stays on the device it was
//!   built on (CoreAudio and WASAPI). So while a stream is open the audio thread reads
//!   [`MicHost::default_input_id`] every 200 ms ([`MicConfig::device_poll`]). On a change, or
//!   on a lost stream ([`StreamFault::Invalidated`] or [`StreamFault::Fatal`]), it drops the
//!   stream and builds and plays a new one on the default device at once, under the same
//!   lease, then sends a [`SinkEvent::Gap`]. If no stream opens, it tries again every 250 ms
//!   for 10 s ([`MicConfig::lost_grace`], section 7.2), then reports [`MicFault::Lost`].
//! - **Built-in mic preference (CAP-22, #22).** [`MicDevice::set_prefer_built_in`] reaches
//!   the host through [`MicHost::set_preference`] before each build; [`choose_input`] is the
//!   rule.
//!
//! - **Level stream and peak (CAP-16a, CAP-16b, CAP-20, #23).** [`level::LevelSink`] reads the
//!   frames and sends the RMS level at 30 Hz or less while a capture records;
//!   [`normalize::normalize_peak`] sets each segment's peak to -3 dBFS before STT (ADR 0067).
//!
//! Device errors are the content-free codes in [`codes`]. ADR 0038 records the design, and
//! the #22 ADR the zero watch, the reopen and the preference.

pub mod codes;
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub mod cpal_backend;
pub mod level;
mod mic;
pub mod normalize;
pub mod ptt_buffer;
pub mod sink;
pub mod system;

pub use mic::{FailureLink, MicConfig, MicDevice, MicFault};
pub use sink::{mix_to_mono, DiscardSink, FrameSink, MicFrame, SinkEvent, ZeroWatch};

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
    /// The stream still runs (for example an overrun). Capture goes on.
    Transient,
    /// The OS moved the stream to the new default device and it still runs (CAP-18).
    /// The sink gets a gap marker.
    Rerouted,
    /// The default device changed and this stream cannot go on (CAP-18). The backend opens a
    /// new stream on the new default under the same lease.
    Invalidated,
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

    /// The device preference for the next [`MicHost::build`] (CAP-22). The audio thread calls
    /// it just before each build, never at launch. The default ignores it.
    fn set_preference(&mut self, _preference: DevicePreference) {}

    /// An opaque ID of the current default input device, or `None` if the host cannot tell
    /// (CAP-18). While a stream is open the audio thread reads it every
    /// [`MicConfig::device_poll`]; a change opens the stream again on the new default. The
    /// ID is compared only: never logged, stored or sent. The default is `None` (no poll).
    fn default_input_id(&mut self) -> Option<String> {
        None
    }
}

/// The user's input device preference (CAP-22).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DevicePreference {
    /// "Prefer the built-in mic when a Bluetooth headset connects". Default off.
    pub prefer_built_in: bool,
}

/// The kind of an input device, for [`choose_input`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// The mic in the computer.
    BuiltIn,
    /// A Bluetooth headset (for example AirPods).
    Bluetooth,
    /// Any other device (USB, line in, virtual).
    Other,
}

/// Which input device to build on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputChoice {
    /// The system default input device.
    Default,
    /// The built-in mic.
    BuiltIn,
}

/// The CAP-22 rule. With the preference on, a Bluetooth default (or no default) gives way to
/// the built-in mic, if there is one. So the headset stays in its A2DP (music) profile and
/// does not drop to the hands-free profile. Any other default is kept.
pub fn choose_input(
    prefer_built_in: bool,
    default: Option<InputKind>,
    built_in_present: bool,
) -> InputChoice {
    match default {
        Some(InputKind::Bluetooth) | None if prefer_built_in && built_in_present => {
            InputChoice::BuiltIn
        }
        _ => InputChoice::Default,
    }
}
