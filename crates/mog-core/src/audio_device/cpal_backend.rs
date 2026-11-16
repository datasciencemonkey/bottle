// Adapted from cjpais/Handy src-tauri/src/managers/audio.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! The cpal [`MicHost`] for macOS (CoreAudio) and Windows (WASAPI) (#388).
//!
//! From Handy: the error-kind to content-free code map (Handy's `recording_notice_code`:
//! permission denied, no microphone, other) and the per-stream failure handler that marks the
//! stream for a reopen on the next start. Handy's mic modes, lazy close, device cache,
//! mute handling and logging are not ported. Handy's cpal recorder itself is the separate
//! `handy-recorder` crate, which is not in the reference copy; nothing of it is copied.
//!
//! The stream is built on the default input device at each open, with its default input
//! config, and converted to `f32`. A device change therefore takes effect at the next open
//! (SP-08 follow-up 3). TODO(#22): the device picker and the permission check.
//!
//! No test builds a [`CpalMicHost`] stream: tests never open the user's mic. The device
//! checks are in the `needs:human` issue of #388.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::capture::DeviceError;

use super::{codes, DataFn, FaultFn, Interleaved, MicHost, StreamFault};

/// The cpal host of this OS: CoreAudio on macOS, WASAPI on Windows. Make it on the audio
/// thread (see [`MicDevice::spawn`](super::MicDevice::spawn)).
pub struct CpalMicHost {
    host: cpal::Host,
}

impl CpalMicHost {
    /// The default cpal host. This opens no device.
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }
}

impl Default for CpalMicHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MicHost for CpalMicHost {
    type Stream = cpal::Stream;

    fn build(
        &mut self,
        on_data: DataFn,
        mut on_fault: FaultFn,
    ) -> Result<cpal::Stream, DeviceError> {
        let device = self.host.default_input_device().ok_or(codes::NO_DEVICE)?;
        let supported = device
            .default_input_config()
            .map_err(|e| code_for(e.kind()))?;
        let format = supported.sample_format();
        let config = supported.config();
        let on_error = move |e: cpal::Error| on_fault(fault_for(e.kind()));
        match format {
            SampleFormat::F32 => build_as::<f32>(&device, config, on_data, on_error),
            SampleFormat::F64 => build_as::<f64>(&device, config, on_data, on_error),
            SampleFormat::I8 => build_as::<i8>(&device, config, on_data, on_error),
            SampleFormat::I16 => build_as::<i16>(&device, config, on_data, on_error),
            SampleFormat::I32 => build_as::<i32>(&device, config, on_data, on_error),
            SampleFormat::U8 => build_as::<u8>(&device, config, on_data, on_error),
            SampleFormat::U16 => build_as::<u16>(&device, config, on_data, on_error),
            SampleFormat::U32 => build_as::<u32>(&device, config, on_data, on_error),
            _ => Err(codes::UNSUPPORTED_FORMAT),
        }
    }

    fn play(&mut self, stream: &cpal::Stream) -> Result<(), DeviceError> {
        stream.play().map_err(|e| code_for(e.kind()))
    }
}

/// Build an input stream of sample type `T` that hands `f32` blocks to `on_data`.
fn build_as<T>(
    device: &cpal::Device,
    config: StreamConfig,
    mut on_data: DataFn,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, DeviceError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels;
    let sample_rate = config.sample_rate;
    let mut block: Vec<f32> = Vec::new();
    device
        .build_input_stream::<T, _, _>(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                block.clear();
                block.extend(data.iter().map(|s| f32::from_sample_(*s)));
                on_data(Interleaved {
                    samples: &block,
                    channels,
                    sample_rate,
                });
            },
            on_error,
            None,
        )
        .map_err(|e| code_for(e.kind()))
}

/// The content-free code of a cpal error (Handy `recording_notice_code`).
pub fn code_for(kind: ErrorKind) -> DeviceError {
    let _ = kind;
    todo!("#388 green")
}

/// Whether a cpal stream error after open ends the stream.
pub fn fault_for(kind: ErrorKind) -> StreamFault {
    let _ = kind;
    todo!("#388 green")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_01b_cpal_errors_map_to_content_free_codes() {
        assert_eq!(
            code_for(ErrorKind::PermissionDenied),
            codes::PERMISSION_DENIED
        );
        assert_eq!(code_for(ErrorKind::DeviceNotAvailable), codes::NO_DEVICE);
        assert_eq!(code_for(ErrorKind::HostUnavailable), codes::NO_DEVICE);
        assert_eq!(code_for(ErrorKind::DeviceBusy), codes::DEVICE_BUSY);
        assert_eq!(
            code_for(ErrorKind::UnsupportedConfig),
            codes::UNSUPPORTED_FORMAT
        );
        assert_eq!(
            code_for(ErrorKind::UnsupportedOperation),
            codes::UNSUPPORTED_FORMAT
        );
        assert_eq!(code_for(ErrorKind::InvalidInput), codes::UNSUPPORTED_FORMAT);
        for kind in [
            ErrorKind::BackendError,
            ErrorKind::Other,
            ErrorKind::StreamInvalidated,
            ErrorKind::ResourceExhausted,
            ErrorKind::DeviceChanged,
            ErrorKind::Xrun,
            ErrorKind::RealtimeDenied,
        ] {
            assert_eq!(code_for(kind), codes::DEVICE_FAILED, "{kind:?}");
        }
    }

    #[test]
    fn cap_01a_only_transient_cpal_errors_keep_the_stream() {
        for kind in [
            ErrorKind::DeviceChanged,
            ErrorKind::Xrun,
            ErrorKind::RealtimeDenied,
        ] {
            assert_eq!(fault_for(kind), StreamFault::Transient, "{kind:?}");
        }
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::PermissionDenied,
            ErrorKind::StreamInvalidated,
            ErrorKind::BackendError,
            ErrorKind::Other,
            ErrorKind::DeviceBusy,
            ErrorKind::HostUnavailable,
        ] {
            assert_eq!(fault_for(kind), StreamFault::Fatal, "{kind:?}");
        }
    }

    /// Type check only: the cpal host plugs into the mic backend. It is never called, so no
    /// test makes a cpal host or opens a stream.
    #[allow(dead_code)]
    fn cpal_host_plugs_into_the_mic_device() -> std::io::Result<super::super::MicDevice> {
        super::super::MicDevice::spawn(CpalMicHost::new, Box::new(super::super::DiscardSink))
            .map(|(device, _link)| device)
    }
}
