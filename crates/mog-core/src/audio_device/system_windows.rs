//! The Windows system audio backend: WASAPI loopback through cpal (ADR 0103, #108).
//!
//! cpal opens a WASAPI loopback stream when an input stream is built on an output device. So
//! [`LoopbackHost`] builds an input stream on the default output device with its default
//! output config. Windows shows no prompt for loopback. Bottle's own sounds are in the
//! loopback mix (WASAPI process loopback exclusion is not in cpal); Bottle plays none while
//! it records.
//!
//! A default output change invalidates the stream; the system audio device builds it again
//! once on the new default (`StreamFault::Invalidated`).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use crate::capture::DeviceError;

use super::cpal_backend::{build_input_as, code_for, fault_for};
use super::{codes, DataFn, FaultFn, MicHost};

/// The WASAPI loopback host. Make it on the system audio thread.
pub struct LoopbackHost {
    host: cpal::Host,
}

impl LoopbackHost {
    /// The default cpal host. This opens no device.
    pub fn new() -> Self {
        Self {
            host: cpal::default_host(),
        }
    }
}

impl Default for LoopbackHost {
    fn default() -> Self {
        Self::new()
    }
}

impl MicHost for LoopbackHost {
    type Stream = cpal::Stream;

    fn build(
        &mut self,
        on_data: DataFn,
        mut on_fault: FaultFn,
    ) -> Result<cpal::Stream, DeviceError> {
        let device = self.host.default_output_device().ok_or(codes::NO_DEVICE)?;
        let supported = device
            .default_output_config()
            .map_err(|e| code_for(e.kind()))?;
        let format = supported.sample_format();
        let config = supported.config();
        let on_error = move |e: cpal::Error| on_fault(fault_for(e.kind()));
        match format {
            SampleFormat::F32 => build_input_as::<f32>(&device, config, on_data, on_error),
            SampleFormat::I16 => build_input_as::<i16>(&device, config, on_data, on_error),
            SampleFormat::I32 => build_input_as::<i32>(&device, config, on_data, on_error),
            SampleFormat::U16 => build_input_as::<u16>(&device, config, on_data, on_error),
            _ => Err(codes::UNSUPPORTED_FORMAT),
        }
    }

    fn play(&mut self, stream: &cpal::Stream) -> Result<(), DeviceError> {
        stream.play().map_err(|e| code_for(e.kind()))
    }
}
