// Adapted from cjpais/Handy src-tauri/src/audio_toolkit/audio/sink.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! The in-RAM frame sink of the mic backend: the one place where mic audio leaves the
//! backend (CAP-01b, #21).
//!
//! Like Handy's `CaptureSink`, the sink gets mono audio in order, one block per data callback,
//! and the [`MicFrame::is_digital_silence`] check tells a muted or not-permitted mic from
//! quiet speech. Handy's VAD, visualiser, history, logging and WAV writing are not ported: the
//! consumers come later.
//!
//! - TODO(#21): the RAM-only PTT buffer (no pre-roll, nothing to disk) implements [`FrameSink`].
//! - TODO(#23): the level meter reads the frames.
//! - TODO(#46): speech recognition, with the 16 kHz resample, reads the frames.
//! - TODO(#22): the 2 s zero-buffer detector uses [`MicFrame::is_digital_silence`].

/// One block of mono mic audio at the device's native rate. It borrows the backend's buffer:
/// a sink that keeps audio copies it into its own RAM buffer.
#[derive(Debug, Clone, Copy)]
pub struct MicFrame<'a> {
    /// Mono samples in -1.0..=1.0.
    pub samples: &'a [f32],
    /// The sample rate in Hz.
    pub sample_rate: u32,
}

impl MicFrame<'_> {
    /// Every sample is exactly 0. A muted mic, or a mic without permission on macOS, gives
    /// digital silence; a quiet room does not.
    pub fn is_digital_silence(&self) -> bool {
        self.samples.iter().all(|s| *s == 0.0)
    }
}

/// The receiver of mic frames. It runs on the host's real-time audio thread, so it must be
/// fast and must not block. It must keep audio in RAM only: no disk, no log, no telemetry.
/// It must not panic: it runs inside the host's C audio callback, where a panic aborts the
/// process.
pub trait FrameSink: Send + 'static {
    /// One block of mono audio, in order.
    fn frame(&mut self, frame: MicFrame<'_>);
}

/// A sink that drops every frame. For a host that has no consumer yet. TODO(#21).
#[derive(Debug, Default, Clone, Copy)]
pub struct DiscardSink;

impl FrameSink for DiscardSink {
    fn frame(&mut self, _frame: MicFrame<'_>) {}
}

/// Mix interleaved samples down to mono into `out` (cleared first): the mean of the channels
/// of each frame. A trailing part frame is dropped. `channels` 0 counts as 1.
pub fn mix_to_mono(interleaved: &[f32], channels: u16, out: &mut Vec<f32>) {
    out.clear();
    let channels = channels.max(1);
    if channels == 1 {
        out.extend_from_slice(interleaved);
        return;
    }
    let scale = f32::from(channels);
    out.extend(
        interleaved
            .chunks_exact(usize::from(channels))
            .map(|f| f.iter().sum::<f32>() / scale),
    );
}
