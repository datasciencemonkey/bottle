//! The input peak normalisation of R2 (CAP-20, #23, ADR 0067).
//!
//! Before STT, R2 scales each segment so that its peak is -3 dBFS. A quiet mic then
//! transcribes as well as a loud one, and a loud segment comes down to the same peak. One gain
//! applies to the whole segment, so the shape of the audio does not change. Digital silence
//! (every sample 0) stays silence. There is no gain limit: the requirement sets the peak, and
//! the VAD trim (#116, CAP-54b) keeps non-speech out of STT. A sample that is not finite
//! becomes 0 first.
//!
//! The segment is a RAM buffer (CAP-01b, #21). This module only scales it: nothing is logged
//! or kept.
//!
//! The PTT buffer ([`super::ptt_buffer`], #21) calls [`normalize_peak`] on each utterance just
//! before it hands it to R4 (TODO(#46)). The segment pipeline (TODO(#116)) does the same for
//! each session segment.

use super::level::LEVEL_FLOOR_DBFS;

/// The target input peak before STT: -3 dBFS (CAP-20).
pub const TARGET_PEAK_DBFS: f32 = -3.0;

/// The peak of `samples` in dBFS (`20 * log10(max |s|)`). Silence gives [`LEVEL_FLOOR_DBFS`].
/// Non-finite samples are not counted.
pub fn peak_dbfs(samples: &[f32]) -> f32 {
    let peak = peak(samples);
    if peak <= 0.0 {
        return LEVEL_FLOOR_DBFS;
    }
    (20.0 * peak.log10()).max(LEVEL_FLOOR_DBFS)
}

fn peak(samples: &[f32]) -> f32 {
    samples
        .iter()
        .filter(|s| s.is_finite())
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

/// Scale `samples` in place so that the peak is [`TARGET_PEAK_DBFS`] (CAP-20). Returns the
/// linear gain. Non-finite samples become 0. Silence and an empty segment get gain 1.
pub fn normalize_peak(samples: &mut [f32]) -> f32 {
    for s in samples.iter_mut() {
        if !s.is_finite() {
            *s = 0.0;
        }
    }
    let peak = peak(samples);
    if peak <= 0.0 {
        return 1.0;
    }
    let target = 10f32.powf(TARGET_PEAK_DBFS / 20.0);
    let gain = target / peak;
    for s in samples.iter_mut() {
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
    gain
}
