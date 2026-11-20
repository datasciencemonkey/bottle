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
/// The part length in minutes. The capture cuts a new Ogg Opus part every 5 minutes of audio
/// (counted in samples, so it is exact and needs no clock), and the cloud side uploads each
/// finished part while the call goes on. A crash then loses at most the part that is open.
/// `None` is one part per recording. At 16 kbps a part is about 600 KB.
pub const PART_MAX_MINUTES: Option<u64> = Some(5);
/// [`PART_MAX_MINUTES`] in 16 kHz samples.
pub const PART_MAX_SAMPLES: Option<u64> = part_max_samples(PART_MAX_MINUTES);

/// A part cap in minutes as 16 kHz samples.
pub const fn part_max_samples(minutes: Option<u64>) -> Option<u64> {
    match minutes {
        Some(m) => Some(m * 60 * SAMPLE_RATE as u64),
        None => None,
    }
}
