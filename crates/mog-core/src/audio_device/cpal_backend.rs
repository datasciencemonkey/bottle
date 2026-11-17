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
//! (SP-08 follow-up 3). With the CAP-22 preference on, a Bluetooth default gives way to the
//! built-in mic (#22). There is no separate permission check: the first build, after a user
//! action, shows the OS prompt (CAP-17a), and a denied mic is caught by the zero watch
//! (CAP-17b) or by the `permission_denied` build error.
//!
//! No test builds a [`CpalMicHost`] stream: tests never open the user's mic. The device
//! checks are in the `needs:human` issue of #388.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, InterfaceType, SampleFormat, SizedSample, StreamConfig};

use crate::capture::DeviceError;

use super::{
    choose_input, codes, DataFn, DevicePreference, FaultFn, InputChoice, InputKind, Interleaved,
    MicHost, StreamFault,
};

/// The cpal host of this OS: CoreAudio on macOS, WASAPI on Windows. Make it on the audio
/// thread (see [`MicDevice::spawn`](super::MicDevice::spawn)).
pub struct CpalMicHost {
    host: cpal::Host,
    preference: DevicePreference,
}

impl CpalMicHost {
    /// The default cpal host. This opens no device.
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
            preference: DevicePreference::default(),
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
        let device = self.input_device().ok_or(codes::NO_DEVICE)?;
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

    fn set_preference(&mut self, preference: DevicePreference) {
        self.preference = preference;
    }
}

impl CpalMicHost {
    /// The input device for the next build (CAP-22). Device names are not logged or kept.
    fn input_device(&self) -> Option<cpal::Device> {
        let default = self.host.default_input_device();
        if !self.preference.prefer_built_in {
            return default;
        }
        let built_in = self
            .host
            .input_devices()
            .ok()
            .and_then(|mut all| all.find(|d| kind_of(d) == Some(InputKind::BuiltIn)));
        let default_kind = default
            .as_ref()
            .map(|d| kind_of(d).unwrap_or(InputKind::Other));
        match choose_input(true, default_kind, built_in.is_some()) {
            InputChoice::BuiltIn => built_in,
            InputChoice::Default => default,
        }
    }
}

/// The CAP-22 kind of a cpal device, from its interface type.
fn kind_of(device: &cpal::Device) -> Option<InputKind> {
    let interface = device.description().ok()?.interface_type();
    Some(kind_for(interface))
}

/// The CAP-22 kind of a cpal interface type.
pub fn kind_for(interface: InterfaceType) -> InputKind {
    match interface {
        InterfaceType::BuiltIn => InputKind::BuiltIn,
        InterfaceType::Bluetooth => InputKind::Bluetooth,
        _ => InputKind::Other,
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
    match kind {
        ErrorKind::PermissionDenied => codes::PERMISSION_DENIED,
        ErrorKind::DeviceNotAvailable | ErrorKind::HostUnavailable => codes::NO_DEVICE,
        ErrorKind::DeviceBusy => codes::DEVICE_BUSY,
        ErrorKind::UnsupportedConfig
        | ErrorKind::UnsupportedOperation
        | ErrorKind::InvalidInput => codes::UNSUPPORTED_FORMAT,
        // `ErrorKind` is non-exhaustive: a new kind is a plain device failure.
        _ => codes::DEVICE_FAILED,
    }
}

/// Whether a cpal stream error after open ends the stream.
pub fn fault_for(kind: ErrorKind) -> StreamFault {
    match kind {
        // CAP-18: CoreAudio rerouted the stream to the new default device; it still runs.
        ErrorKind::DeviceChanged => StreamFault::Rerouted,
        // CAP-18: WASAPI does not rebind; a new default exists, so open a new stream on it.
        ErrorKind::StreamInvalidated => StreamFault::Invalidated,
        // A glitch or a refused real-time priority: audio still flows.
        ErrorKind::Xrun | ErrorKind::RealtimeDenied => StreamFault::Transient,
        _ => StreamFault::Fatal,
    }
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
        for kind in [ErrorKind::Xrun, ErrorKind::RealtimeDenied] {
            assert_eq!(fault_for(kind), StreamFault::Transient, "{kind:?}");
        }
        for kind in [
            ErrorKind::DeviceNotAvailable,
            ErrorKind::PermissionDenied,
            ErrorKind::BackendError,
            ErrorKind::Other,
            ErrorKind::DeviceBusy,
            ErrorKind::HostUnavailable,
        ] {
            assert_eq!(fault_for(kind), StreamFault::Fatal, "{kind:?}");
        }
    }

    #[test]
    fn cap_18_a_default_device_change_reroutes_or_reopens() {
        assert_eq!(fault_for(ErrorKind::DeviceChanged), StreamFault::Rerouted);
        assert_eq!(
            fault_for(ErrorKind::StreamInvalidated),
            StreamFault::Invalidated
        );
    }

    #[test]
    fn cap_22_interface_types_map_to_input_kinds() {
        assert_eq!(kind_for(InterfaceType::BuiltIn), InputKind::BuiltIn);
        assert_eq!(kind_for(InterfaceType::Bluetooth), InputKind::Bluetooth);
        for t in [
            InterfaceType::Usb,
            InterfaceType::Virtual,
            InterfaceType::Unknown,
        ] {
            assert_eq!(kind_for(t), InputKind::Other, "{t:?}");
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
