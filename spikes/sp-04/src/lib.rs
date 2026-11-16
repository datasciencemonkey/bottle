//! SP-04 (#44) benchmark helpers: WAV reader, percentiles, WER, tail split, peak RSS
//! and a thin wrapper over the sherpa-onnx offline recognizer for Parakeet TDT 0.6B v3 int8.

use std::fmt;
use std::path::Path;

/// The only sample rate the benchmark accepts (Parakeet expects 16 kHz mono).
pub const SAMPLE_RATE: u32 = 16_000;

/// Summary statistics of a set of latency samples (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub mean: f64,
}

/// Nearest-rank percentile of an ascending slice.
pub fn percentile(_sorted: &[f64], _p: f64) -> f64 {
    todo!("TODO(#44) red")
}

/// Statistics of unsorted samples. `None` for an empty slice.
pub fn stats(_samples: &[f64]) -> Option<Stats> {
    todo!("TODO(#44) red")
}

/// Reasons a WAV file is rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum WavError {
    Io(String),
    NotRiffWave,
    NoFmt,
    NoData,
    NotPcm16 { format: u16, bits: u16 },
    WrongChannels(u16),
    WrongRate(u32),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for WavError {}

/// Parse a 16-bit PCM, mono, 16 kHz WAV image into f32 samples in [-1, 1).
pub fn parse_wav(_bytes: &[u8]) -> Result<Vec<f32>, WavError> {
    todo!("TODO(#44) red")
}

/// Read a WAV file with [`parse_wav`].
pub fn read_wav(path: &Path) -> Result<Vec<f32>, WavError> {
    let bytes =
        std::fs::read(path).map_err(|e| WavError::Io(format!("{}: {e}", path.display())))?;
    parse_wav(&bytes)
}

/// Word error rate of `hypothesis` against `reference` after lower-casing and removing punctuation.
pub fn wer(_reference: &str, _hypothesis: &str) -> f64 {
    todo!("TODO(#44) red")
}

/// Pad with trailing silence to `secs` seconds. Longer audio is returned unchanged.
pub fn pad_to_secs(_samples: &[f32], _secs: f64) -> Vec<f32> {
    todo!("TODO(#44) red")
}

/// Split into (head, tail): the tail is the last `tail_secs` seconds (or all audio if shorter).
pub fn split_tail(_samples: &[f32], _tail_secs: f64) -> (&[f32], &[f32]) {
    todo!("TODO(#44) red")
}

/// Peak resident set size of this process in bytes, if the OS reports it.
pub fn peak_rss_bytes() -> Option<u64> {
    todo!("TODO(#44) red")
}

/// Physical performance-core count (macOS `hw.perflevel0.physicalcpu`). `None` elsewhere.
pub fn p_core_count() -> Option<usize> {
    todo!("TODO(#44) red")
}

/// Parakeet TDT 0.6B v3 int8 on the sherpa-onnx offline recognizer, CPU execution provider.
pub struct Engine {
    _private: (),
}

impl Engine {
    /// Load encoder/decoder/joiner int8 graphs and tokens.txt from `model_dir`.
    pub fn load(_model_dir: &Path, _threads: i32) -> Result<Engine, String> {
        todo!("TODO(#44) red")
    }

    /// Decode one utterance (16 kHz mono f32) and return the text.
    pub fn transcribe(&self, _samples: &[f32]) -> String {
        todo!("TODO(#44) red")
    }
}
