// Adapted from cjpais/Handy src-tauri/src/audio_toolkit/audio/sink.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! The in-RAM frame sink of the mic backend: the one place where mic audio leaves the
//! backend (CAP-01b, #21).
//!
//! Like Handy's `CaptureSink`, the sink gets mono audio in order, one block per data callback,
//! and the [`MicFrame::is_digital_silence`] check tells a muted or not-permitted mic from
//! quiet speech. Handy's VAD, visualiser, history, logging and WAV writing are not ported: the
//! consumers come later.
//!
//! - The RAM-only PTT buffer (no pre-roll, nothing to disk, CAP-12c, #21) is
//!   [`super::ptt_buffer::PttBufferSink`].
//! - The level stream (CAP-16a, #23) is [`super::level::LevelSink`]; the host tees it with
//!   the PTT buffer (#476).
//! - R4 speech recognition (`crate::stt_engine::SttFrameSink`, #46) reads the frames on the
//!   host tee and resamples them to 16 kHz.
//! - The 2 s zero-buffer watch (CAP-17b, #22) is [`ZeroWatch`]; it uses
//!   [`MicFrame::is_digital_silence`].

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

    /// A stream event between frames, in order with them (#22). The default ignores it.
    fn event(&mut self, _event: SinkEvent) {}
}

/// A stream event that the sink gets between frames (#22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SinkEvent {
    /// The default device changed (CAP-18). Frames before the marker came from the old
    /// device, frames after it from the new one. Nothing that the sink holds is dropped. The
    /// transcript shows a gap marker here. `gap_ms` is the time from the change to the new
    /// stream, 0 when the OS rerouted the stream by itself. TODO(#116): the segment pipeline
    /// carries the marker into the transcript.
    Gap {
        /// Milliseconds without audio.
        gap_ms: u64,
    },
    /// The mic gave only zero buffers for the zero window (CAP-17b), for example because the
    /// mic permission is denied. The sink drops the audio of this capture, so no transcript
    /// is made. The PTT buffer wipes its audio here (#21).
    Discard,
}

/// The zero-buffer watch (CAP-17b): it expires when no block with sound came for `window_ms`.
/// A block of digital silence counts as no sound, and so does no block at all. Times are
/// milliseconds on one monotonic clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZeroWatch {
    window_ms: u64,
    last_sound_ms: u64,
}

impl ZeroWatch {
    /// A watch for a stream that opened at `opened_ms`.
    pub fn new(window_ms: u64, opened_ms: u64) -> Self {
        Self {
            window_ms,
            last_sound_ms: opened_ms,
        }
    }

    /// One block came at `now_ms`. `silent` is [`MicFrame::is_digital_silence`].
    pub fn block(&mut self, silent: bool, now_ms: u64) {
        if !silent {
            self.last_sound_ms = self.last_sound_ms.max(now_ms);
        }
    }

    /// The window passed without sound.
    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms.saturating_sub(self.last_sound_ms) >= self.window_ms
    }

    /// The time at which the watch expires if no sound comes.
    pub fn deadline_ms(&self) -> u64 {
        self.last_sound_ms.saturating_add(self.window_ms)
    }
}

/// A sink that drops every frame. For a host or a test that has no consumer.
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
