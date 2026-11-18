//! Voice activity and the segment cut rule of R4 (CAP-52, ADR 0082).
//!
//! The engine gives the VAD one window of [`VAD_WINDOW`] samples at 16 kHz (32 ms, the Silero
//! frame) at a time. [`SegmentCutter`] then decides where a segment closes, so that R4 decodes
//! closed segments while the user still speaks and only the tail after key-up.
//!
//! - [`EnergyVad`] is the fallback detector: the RMS of a window over a fixed floor. It needs no
//!   model file.
//! - The Silero VAD on sherpa-onnx is `crate::stt_local::SileroVad` (feature `local-stt`).
//!   Its pinned model download is TODO(#513).
//!
//! The onset and hangover idea follows Handy's `SmoothedVad`
//! (`src-tauri/src/audio_toolkit/vad/smoothed.rs` @ aca8985): a pause counts only after a run
//! of non-speech windows. No Handy code is copied: Handy smooths a speech stream for one
//! decode, and Bottle cuts segments for a decode during speech.

/// The STT sample rate of Parakeet and of the VAD.
pub const STT_SAMPLE_RATE: u32 = 16_000;

/// One VAD window: 512 samples at 16 kHz, 32 ms.
pub const VAD_WINDOW: usize = 512;

/// The length of one window in ms.
pub const VAD_WINDOW_MS: u64 = VAD_WINDOW as u64 * 1_000 / STT_SAMPLE_RATE as u64;

/// A voice activity detector over one [`VAD_WINDOW`] of 16 kHz mono audio.
pub trait Vad: Send {
    /// The window holds speech.
    fn is_speech(&mut self, window: &[f32]) -> bool;
    /// Forget the state of the last capture.
    fn reset(&mut self) {}
}

/// The fallback VAD: a window is speech when its RMS is over `floor_dbfs`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnergyVad {
    /// The RMS floor in dBFS.
    pub floor_dbfs: f32,
}

impl Default for EnergyVad {
    /// A floor of -50 dBFS: a quiet voice is over it, a quiet room is under it.
    fn default() -> Self {
        Self { floor_dbfs: -50.0 }
    }
}

impl Vad for EnergyVad {
    fn is_speech(&mut self, window: &[f32]) -> bool {
        if window.is_empty() {
            return false;
        }
        let sum: f32 = window.iter().filter(|s| s.is_finite()).map(|s| s * s).sum();
        let rms = (sum / window.len() as f32).sqrt();
        rms > 0.0 && 20.0 * rms.log10() > self.floor_dbfs
    }
}

/// Where a segment closes (CAP-52). The default is the #283 proposal: close a segment at a
/// pause of 300 ms or more after 3 s of audio, and always at 30 s (the R2 segment limit and
/// the ASR request limit of CAP-53a). TODO(#283): the requirements owner sets the final rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentRule {
    /// A pause cut needs at least this much audio in the segment.
    pub min_segment_ms: u64,
    /// The non-speech run that counts as a pause.
    pub pause_ms: u64,
    /// A segment never grows past this.
    pub max_segment_ms: u64,
}

impl Default for SegmentRule {
    fn default() -> Self {
        Self {
            min_segment_ms: 3_000,
            pause_ms: 300,
            max_segment_ms: 30_000,
        }
    }
}

/// Applies a [`SegmentRule`] to a stream of VAD decisions, one per window.
#[derive(Debug, Clone)]
pub struct SegmentCutter {
    rule: SegmentRule,
    windows: u64,
    pause_windows: u64,
    speech: bool,
}

impl SegmentCutter {
    /// A cutter at the start of a segment.
    pub fn new(rule: SegmentRule) -> Self {
        Self {
            rule,
            windows: 0,
            pause_windows: 0,
            speech: false,
        }
    }

    /// Add one window. True when the segment closes after this window; the cutter then starts
    /// a new segment. A pause cut needs speech in the segment, so a quiet input that the VAD
    /// misses is not cut into pieces (it closes at the hard limit or at key-up).
    pub fn push(&mut self, speech: bool) -> bool {
        self.windows += 1;
        if speech {
            self.speech = true;
            self.pause_windows = 0;
        } else {
            self.pause_windows += 1;
        }
        let len_ms = self.windows * VAD_WINDOW_MS;
        let pause_ms = self.pause_windows * VAD_WINDOW_MS;
        let full = len_ms + VAD_WINDOW_MS > self.rule.max_segment_ms;
        let pause =
            self.speech && len_ms >= self.rule.min_segment_ms && pause_ms >= self.rule.pause_ms;
        if full || pause {
            *self = Self::new(self.rule);
            return true;
        }
        false
    }
}
