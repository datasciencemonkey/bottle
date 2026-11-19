//! Call recording, the capture half (ADR 0103).
pub mod chunker;
pub mod flac;
pub mod mixer;
pub mod permissions;
pub mod session;
pub mod tap;

/// The rate of the recorded stream: 16 kHz mono.
pub const SAMPLE_RATE: u32 = 16_000;
/// Red skeleton.
pub const PART_MAX_MINUTES: Option<u64> = Some(55);
/// Red skeleton.
pub const PART_MAX_SAMPLES: Option<u64> = Some(55 * 60 * SAMPLE_RATE as u64);
/// Red skeleton.
pub const fn part_max_samples(_minutes: Option<u64>) -> Option<u64> {
    None
}
