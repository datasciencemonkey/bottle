//! Call recording, the capture half (ADR 0103).
pub mod chunker;
pub mod flac;
pub mod mixer;
pub mod permissions;
pub mod session;
pub mod tap;

/// The rate of the recorded stream: 16 kHz mono.
pub const SAMPLE_RATE: u32 = 16_000;
/// A new part starts after 55 min of audio.
pub const PART_MAX_SAMPLES: u64 = 0;
