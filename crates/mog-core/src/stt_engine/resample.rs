//! The streaming resample from the device rate to 16 kHz (CAP-52, ADR 0082).
//!
//! The mic gives mono audio at its native rate (often 48 kHz or 44.1 kHz, 16 kHz or 8 kHz on a
//! Bluetooth headset). Parakeet and the VAD take 16 kHz. Down-sampling averages the input over
//! each output period (a box filter, which also removes most energy above the new Nyquist
//! rate). Up-sampling interpolates linearly. The state carries over between blocks, so a stream
//! cut into blocks gives the same output as one block.

use super::vad::STT_SAMPLE_RATE;

/// A resampler from one input rate to [`STT_SAMPLE_RATE`].
#[derive(Debug, Clone)]
pub struct Resampler {
    from: u32,
    /// Input samples per output sample.
    step: f64,
    /// Down: the input position of the end of the current output period.
    /// Up: the input position of the next output sample, from the previous input sample.
    pos: f64,
    /// Input samples seen so far (down) or the previous input sample (up).
    seen: f64,
    sum: f32,
    count: u32,
    last: Option<f32>,
}

impl Resampler {
    /// A resampler from `from` Hz. A rate of 0 is taken as 16 kHz.
    pub fn new(from: u32) -> Self {
        let from = if from == 0 { STT_SAMPLE_RATE } else { from };
        let step = f64::from(from) / f64::from(STT_SAMPLE_RATE);
        Self {
            from,
            step,
            pos: if step > 1.0 { step } else { 0.0 },
            seen: 0.0,
            sum: 0.0,
            count: 0,
            last: None,
        }
    }

    /// The input rate.
    pub fn from_rate(&self) -> u32 {
        self.from
    }

    /// Resample one block and append the 16 kHz samples to `out`. Non-finite samples become 0.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let clean = |s: f32| if s.is_finite() { s } else { 0.0 };
        if self.from == STT_SAMPLE_RATE {
            out.extend(input.iter().map(|s| clean(*s)));
            return;
        }
        if self.step > 1.0 {
            for s in input {
                self.sum += clean(*s);
                self.count += 1;
                self.seen += 1.0;
                if self.seen >= self.pos {
                    out.push(self.sum / self.count as f32);
                    self.sum = 0.0;
                    self.count = 0;
                    self.pos += self.step;
                }
            }
            return;
        }
        // Up-sampling: x[-1] is the last sample of the previous block.
        for s in input {
            let b = clean(*s);
            let a = self.last.unwrap_or(b);
            while self.pos < 1.0 {
                out.push(a + (b - a) * self.pos as f32);
                self.pos += self.step;
            }
            self.pos -= 1.0;
            self.last = Some(b);
        }
    }
}
