//! Call recording, the capture half (ADR 0103).
//!
//! The mic and system audio go through the [`tap`] to the [`session`]'s writer thread, which
//! mixes them ([`mixer`]) and writes parts ([`chunker`]) with a part encoder ([`opus`] by
//! default; [`flac`] and [`wav`] stay as fallbacks). [`permissions`] asks the OS when the knob turns ON.
pub mod chunker;
pub mod flac;
pub mod mixer;
pub mod opus;
pub mod permissions;
pub mod session;
pub mod tap;
pub mod wav;

/// The rate of the recorded stream: 16 kHz mono.
pub const SAMPLE_RATE: u32 = 16_000;
/// The part cap in minutes. `None`: no split, one part per recording (recordings stop at
/// 120 min). The cloud side sends parts to a serving endpoint as data URLs; set a cap here if
/// one request cannot take a whole recording (for example `Some(10)`).
pub const PART_MAX_MINUTES: Option<u64> = None;
/// [`PART_MAX_MINUTES`] in 16 kHz samples.
pub const PART_MAX_SAMPLES: Option<u64> = part_max_samples(PART_MAX_MINUTES);

/// A part cap in minutes as 16 kHz samples.
pub const fn part_max_samples(minutes: Option<u64>) -> Option<u64> {
    match minutes {
        Some(m) => Some(m * 60 * SAMPLE_RATE as u64),
        None => None,
    }
}
