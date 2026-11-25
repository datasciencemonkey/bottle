//! The content-free device error codes of the mic backend. Only the code crosses IPC (as
//! `host_failed`, see [`CaptureError::to_ipc`](crate::capture::CaptureError::to_ipc)); no
//! device name, OS message or audio is in an error.

use crate::capture::DeviceError;

/// The OS denied access to the mic (macOS Microphone permission, Windows privacy settings).
pub const PERMISSION_DENIED: DeviceError = DeviceError {
    code: "permission_denied",
};
/// There is no input device, or the device went away.
pub const NO_DEVICE: DeviceError = DeviceError { code: "no_device" };
/// Another app holds the device in exclusive mode.
pub const DEVICE_BUSY: DeviceError = DeviceError {
    code: "device_busy",
};
/// The device has no input format that the backend can read.
pub const UNSUPPORTED_FORMAT: DeviceError = DeviceError {
    code: "unsupported_format",
};
/// Any other audio API failure.
pub const DEVICE_FAILED: DeviceError = DeviceError {
    code: "device_failed",
};
/// A `close` cancelled this `open` while it ran. The stream is not open.
pub const OPEN_CANCELLED: DeviceError = DeviceError {
    code: "open_cancelled",
};
/// The audio thread is gone (it panicked). The mic cannot open.
pub const AUDIO_THREAD_GONE: DeviceError = DeviceError {
    code: "audio_thread_gone",
};
/// System audio (the Others track) is not in this backend. TODO(#108) TODO(#109).
pub const SYSTEM_AUDIO_UNSUPPORTED: DeviceError = DeviceError {
    code: "system_audio_unsupported",
};
/// The macOS process tap needs a real device to lend the aggregate a clock, and the system
/// has no default output device at all (owner review 2026-11-26, "9"): distinct from
/// [`DEVICE_FAILED`] so this specific, recoverable-by-plugging-something-in case is not lost
/// in the generic code.
pub const NO_OUTPUT_DEVICE: DeviceError = DeviceError {
    code: "no_output_device",
};
