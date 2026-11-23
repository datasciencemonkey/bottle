//! The recording mixer: mic plus system audio to one 16 kHz mono stream (ADR 0103).
//!
//! ADR 0103 records the deviation from CAP-07b ("never mix"): a call recording is one mixed
//! stream, and `ai_transcribe` diarizes the speakers.
//!
//! - Each track is resampled to 16 kHz on its own ([`Resampler`], the R4 resampler).
//! - Output = `soft_clip(mic + system)`, sample by sample, while both tracks have audio.
//! - **One track silent or absent.** When a track is more than `max_lag` samples ahead of the
//!   other, its surplus goes out alone (the other track counts as silence). So mic-only
//!   recording works, and the output lags the input by at most `max_lag` samples.
//! - **Clock drift.** The mic and the system output run on different clocks. The track that
//!   runs fast builds up a lead; past `max_lag` the lead goes out padded with silence on the
//!   slow track. No sample is dropped, and each buffer stays under `max_lag` plus one block.
//! - [`Mixer::flush`] (at a stop) writes out what is left, padding the shorter track.

use std::collections::VecDeque;

use crate::stt_engine::Resampler;

/// One input of the mixer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Track {
    /// The user's microphone.
    Mic,
    /// System audio (the other side of the call).
    System,
}

/// Where the soft clip starts to bend.
const KNEE: f32 = 0.75;

/// A soft clip: linear up to |0.75|, then a tanh curve that stays below 1.0. Continuous in
/// value and slope, monotonic and odd. NaN is 0.
pub fn soft_clip(x: f32) -> f32 {
    if x.is_nan() {
        return 0.0;
    }
    let a = x.abs();
    if a <= KNEE {
        return x;
    }
    let room = 1.0 - KNEE;
    let y = KNEE + room * ((a - KNEE) / room).tanh();
    // tanh reaches 1.0 in f32 for large inputs: keep the result under full scale.
    y.min(0.999_9).copysign(x)
}

struct Input {
    resampler: Option<Resampler>,
    buf: VecDeque<f32>,
    scratch: Vec<f32>,
    /// 16 kHz samples pushed in since the start.
    total: u64,
}

impl Input {
    fn new() -> Self {
        Self {
            resampler: None,
            buf: VecDeque::new(),
            scratch: Vec::new(),
            total: 0,
        }
    }

    fn push(&mut self, samples: &[f32], rate: u32) {
        // A new rate (a device change) starts a new resampler.
        if self.resampler.as_ref().map(Resampler::from_rate) != Some(rate) {
            self.resampler = Some(Resampler::new(rate));
        }
        self.scratch.clear();
        if let Some(r) = self.resampler.as_mut() {
            r.process(samples, &mut self.scratch);
        }
        self.total += self.scratch.len() as u64;
        self.buf.extend(self.scratch.iter().copied());
    }
}

/// The two-track mixer. See the module docs.
pub struct Mixer {
    max_lag: usize,
    mic: Input,
    system: Input,
    gate: GateTrack,
}

/// Follows the gated mic samples (ADR 0118) from the mic queue to the output timeline. The
/// two tracks pair up in queue order, so a mic sample can leave later than its index; the
/// ranges are therefore counted in output samples, at the moment the mixer emits them.
#[derive(Default)]
struct GateTrack {
    /// Gate changes by index of the first mic sample after the change.
    marks: VecDeque<(u64, bool)>,
    last_mark: bool,
    mic_emitted: u64,
    out_pos: u64,
    open: Option<u64>,
    ranges: Vec<(u64, u64)>,
}

impl GateTrack {
    fn mic_sample(&mut self) {
        while self.marks.front().is_some_and(|m| m.0 <= self.mic_emitted) {
            let Some((_, gated)) = self.marks.pop_front() else {
                break;
            };
            match (gated, self.open) {
                (true, None) => self.open = Some(self.out_pos),
                (false, Some(start)) => {
                    self.ranges.push((start, self.out_pos - start));
                    self.open = None;
                }
                _ => {}
            }
        }
        self.mic_emitted += 1;
    }
}

impl Mixer {
    /// A mixer whose output trails the leading track by at most `max_lag` 16 kHz samples.
    pub fn new(max_lag: usize) -> Self {
        Self {
            max_lag,
            mic: Input::new(),
            system: Input::new(),
            gate: GateTrack::default(),
        }
    }

    /// Say whether the next mic block is gated (zeros that the tap sent for a closed gate).
    /// Call it before [`Mixer::push`] of each mic block.
    pub fn mark_mic(&mut self, gated: bool) {
        if gated != self.gate.last_mark {
            self.gate.last_mark = gated;
            self.gate.marks.push_back((self.mic.total, gated));
        }
    }

    /// The gated ranges so far as (start, length) in output samples. Call it after
    /// [`Mixer::flush`]: it closes a range that is still open.
    pub fn finish_ranges(&mut self) -> Vec<(u64, u64)> {
        if let Some(start) = self.gate.open.take() {
            self.gate.ranges.push((start, self.gate.out_pos - start));
        }
        std::mem::take(&mut self.gate.ranges)
    }

    /// Add one mono block of `track` at `rate` Hz.
    pub fn push(&mut self, track: Track, samples: &[f32], rate: u32) {
        match track {
            Track::Mic => self.mic.push(samples, rate),
            Track::System => self.system.push(samples, rate),
        }
    }

    /// The 16 kHz samples of `track` that wait for the other track.
    pub fn buffered(&self, track: Track) -> usize {
        match track {
            Track::Mic => self.mic.buf.len(),
            Track::System => self.system.buf.len(),
        }
    }

    /// Append the mixed samples that are ready to `out`.
    pub fn drain(&mut self, out: &mut Vec<f32>) {
        let both = self.mic.buf.len().min(self.system.buf.len());
        self.emit(both, out);
        let lead = self.mic.buf.len().max(self.system.buf.len());
        if lead > self.max_lag {
            self.emit(lead - self.max_lag, out);
        }
    }

    /// Append everything that is left to `out`, padding the shorter track with silence.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        let lead = self.mic.buf.len().max(self.system.buf.len());
        self.emit(lead, out);
    }

    /// Emit `n` samples; a track with fewer samples counts as silence for the rest.
    fn emit(&mut self, n: usize, out: &mut Vec<f32>) {
        out.reserve(n);
        for _ in 0..n {
            let a = match self.mic.buf.pop_front() {
                Some(v) => {
                    self.gate.mic_sample();
                    v
                }
                None => 0.0,
            };
            let b = self.system.buf.pop_front().unwrap_or(0.0);
            out.push(soft_clip(a + b));
            self.gate.out_pos += 1;
        }
    }
}
