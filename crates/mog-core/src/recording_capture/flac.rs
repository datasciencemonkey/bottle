//! A small FLAC writer for the recording stream: 16 kHz, mono, 16-bit (ADR 0103).
//!
//! The format is the FLAC spec (RFC 9639). The writer is Bottle's own code, so no new crate
//! sits in the audio path:
//!
//! - one STREAMINFO block (the last metadata block); [`FlacWriter::finish`] seeks back and
//!   fills in the total sample count and the frame sizes. The MD5 field is all zeros, which
//!   the spec allows ("not computed");
//! - fixed-size frames of [`BLOCK_SIZE`] samples (the last frame can be shorter);
//! - per frame one subframe: CONSTANT for a block of one value (digital silence), else the
//!   cheapest FIXED predictor of order 0 to 4 with one Rice partition, else VERBATIM when
//!   that is smaller.
//!
//! Speech compresses to about half of raw PCM this way (about 25-60 MB per hour), well below
//! the part limits of `ai_transcribe`. Samples are clamped to -1.0..=1.0 and scaled by 32767;
//! NaN and infinities become 0.
//!
//! The writer only writes audio to the `W` it was given; it logs nothing.

use std::io::{self, Seek, SeekFrom, Write};

use super::SAMPLE_RATE;

/// Samples per frame (block size code 12: 256 * 2^4).
pub const BLOCK_SIZE: usize = 4096;
/// The largest Rice parameter of the 4-bit Rice coding method (15 is the escape code).
const MAX_RICE: u32 = 14;
/// The byte offset of the STREAMINFO body: "fLaC" plus the 4-byte block header.
const STREAMINFO_AT: u64 = 8;

/// Convert one sample to 16-bit PCM: clamp, scale by 32767, round. Non-finite is 0.
pub fn to_i16(x: f32) -> i16 {
    if !x.is_finite() {
        return 0;
    }
    // In range after the clamp, so the cast cannot wrap.
    (x.clamp(-1.0, 1.0) * 32_767.0).round() as i16
}

/// A streaming FLAC encoder over `W`.
pub struct FlacWriter<W: Write + Seek> {
    out: W,
    pending: Vec<i32>,
    total: u64,
    frame_no: u64,
    min_frame: u32,
    max_frame: u32,
    bits: BitWriter,
}

impl<W: Write + Seek> FlacWriter<W> {
    /// Write the stream header and STREAMINFO (with zero counts) to `out`.
    pub fn new(mut out: W) -> io::Result<Self> {
        out.write_all(b"fLaC")?;
        // Last metadata block, type 0 (STREAMINFO), length 34.
        out.write_all(&[0x80, 0, 0, 34])?;
        out.write_all(&streaminfo(0, 0, 0))?;
        Ok(Self {
            out,
            pending: Vec::with_capacity(BLOCK_SIZE),
            total: 0,
            frame_no: 0,
            min_frame: 0,
            max_frame: 0,
            bits: BitWriter::default(),
        })
    }

    /// Append mono samples at 16 kHz.
    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for s in samples {
            self.pending.push(i32::from(to_i16(*s)));
            if self.pending.len() == BLOCK_SIZE {
                self.flush_frame()?;
            }
        }
        self.total += samples.len() as u64;
        Ok(())
    }

    /// The samples written so far.
    pub fn samples(&self) -> u64 {
        self.total
    }

    /// Write the last frame, fill in STREAMINFO and flush. Returns the inner writer.
    pub fn finish(mut self) -> io::Result<W> {
        if !self.pending.is_empty() {
            self.flush_frame()?;
        }
        let end = self.out.stream_position()?;
        self.out.seek(SeekFrom::Start(STREAMINFO_AT))?;
        self.out
            .write_all(&streaminfo(self.total, self.min_frame, self.max_frame))?;
        self.out.seek(SeekFrom::Start(end))?;
        self.out.flush()?;
        Ok(self.out)
    }

    fn flush_frame(&mut self) -> io::Result<()> {
        let frame = encode_frame(&mut self.bits, &self.pending, self.frame_no);
        self.out.write_all(&frame)?;
        let size = u32::try_from(frame.len()).unwrap_or(u32::MAX);
        self.min_frame = if self.frame_no == 0 {
            size
        } else {
            self.min_frame.min(size)
        };
        self.max_frame = self.max_frame.max(size);
        self.frame_no += 1;
        self.pending.clear();
        Ok(())
    }
}

/// The 34-byte STREAMINFO body.
fn streaminfo(total: u64, min_frame: u32, max_frame: u32) -> [u8; 34] {
    let mut b = BitWriter::default();
    b.put(BLOCK_SIZE as u64, 16); // min block size
    b.put(BLOCK_SIZE as u64, 16); // max block size
    b.put(u64::from(min_frame), 24);
    b.put(u64::from(max_frame), 24);
    b.put(u64::from(SAMPLE_RATE), 20);
    b.put(0, 3); // channels - 1
    b.put(15, 5); // bits per sample - 1
    b.put(total.min((1 << 36) - 1), 36);
    for _ in 0..16 {
        b.put(0, 8); // MD5 not computed
    }
    let mut out = [0u8; 34];
    out.copy_from_slice(&b.take());
    out
}

/// Encode one frame of `block` (1..=BLOCK_SIZE samples).
fn encode_frame(bits: &mut BitWriter, block: &[i32], frame_no: u64) -> Vec<u8> {
    let n = block.len();
    bits.clear();
    // Sync code, reserved 0, fixed block size strategy.
    bits.put(0b11_1111_1111_1110, 14);
    bits.put(0, 1);
    bits.put(0, 1);
    // Block size: code 12 is 4096; code 7 is "16-bit (n - 1) at the header end".
    let size_code = if n == BLOCK_SIZE { 0b1100 } else { 0b0111 };
    bits.put(size_code, 4);
    bits.put(0b0101, 4); // 16 kHz
    bits.put(0, 4); // mono
    bits.put(0b100, 3); // 16 bits per sample
    bits.put(0, 1);
    put_utf8(bits, frame_no);
    if n != BLOCK_SIZE {
        bits.put((n - 1) as u64, 16);
    }
    let crc = crc8(bits.bytes_so_far());
    bits.put(u64::from(crc), 8);
    put_subframe(bits, block);
    bits.align();
    let mut frame = bits.take();
    let crc = crc16(&frame);
    frame.extend_from_slice(&crc.to_be_bytes());
    frame
}

/// The frame number in the UTF-8-like coding of the FLAC frame header.
fn put_utf8(bits: &mut BitWriter, v: u64) {
    if v < 0x80 {
        bits.put(v, 8);
        return;
    }
    // Number of continuation bytes for the value.
    let extra = match v {
        0..=0x7FF => 1,
        0x800..=0xFFFF => 2,
        0x1_0000..=0x1F_FFFF => 3,
        0x20_0000..=0x3FF_FFFF => 4,
        0x400_0000..=0x7FFF_FFFF => 5,
        _ => 6,
    };
    let lead_mark: u64 = (0xFF00u64 >> (extra + 1)) & 0xFF;
    bits.put(lead_mark | (v >> (6 * extra)), 8);
    for i in (0..extra).rev() {
        bits.put(0x80 | ((v >> (6 * i)) & 0x3F), 8);
    }
}

fn put_subframe(bits: &mut BitWriter, block: &[i32]) {
    let n = block.len();
    if block.iter().all(|s| *s == block[0]) {
        bits.put(0, 1);
        bits.put(0b000000, 6); // CONSTANT
        bits.put(0, 1);
        bits.put_signed(block[0], 16);
        return;
    }
    let verbatim_bits = 16 * n as u64;
    // The cheapest fixed predictor, if the block is long enough for its warm-up.
    let mut best: Option<(usize, u32, u64)> = None;
    let mut residual = Vec::with_capacity(n);
    for order in 0..=4usize.min(n - 1) {
        fixed_residual(block, order, &mut residual);
        let (k, cost) = best_rice(&residual);
        let total = 16 * order as u64 + 2 + 4 + 4 + cost;
        if best.is_none_or(|b| total < b.2) {
            best = Some((order, k, total));
        }
    }
    match best {
        Some((order, k, total)) if total < verbatim_bits => {
            bits.put(0, 1);
            bits.put(0b001000 | order as u64, 6); // FIXED, order
            bits.put(0, 1);
            for s in &block[..order] {
                bits.put_signed(*s, 16);
            }
            fixed_residual(block, order, &mut residual);
            bits.put(0b00, 2); // Rice, 4-bit parameter
            bits.put(0, 4); // partition order 0
            bits.put(u64::from(k), 4);
            for r in &residual {
                bits.put_rice(zigzag(*r), k);
            }
        }
        _ => {
            bits.put(0, 1);
            bits.put(0b000001, 6); // VERBATIM
            bits.put(0, 1);
            for s in block {
                bits.put_signed(*s, 16);
            }
        }
    }
}

/// The residual of the fixed predictor of `order` (0..=4), after the warm-up samples.
fn fixed_residual(x: &[i32], order: usize, out: &mut Vec<i32>) {
    out.clear();
    for i in order..x.len() {
        let r = match order {
            0 => x[i],
            1 => x[i] - x[i - 1],
            2 => x[i] - 2 * x[i - 1] + x[i - 2],
            3 => x[i] - 3 * x[i - 1] + 3 * x[i - 2] - x[i - 3],
            _ => x[i] - 4 * x[i - 1] + 6 * x[i - 2] - 4 * x[i - 3] + x[i - 4],
        };
        out.push(r);
    }
}

fn zigzag(r: i32) -> u64 {
    ((i64::from(r) << 1) ^ (i64::from(r) >> 63)) as u64
}

/// The Rice parameter with the fewest bits for `residual`, and that bit count.
fn best_rice(residual: &[i32]) -> (u32, u64) {
    let n = residual.len() as u64;
    let mut best = (0u32, u64::MAX);
    for k in 0..=MAX_RICE {
        let cost: u64 =
            n * (1 + u64::from(k)) + residual.iter().map(|r| zigzag(*r) >> k).sum::<u64>();
        if cost < best.1 {
            best = (k, cost);
        }
    }
    best
}

/// A big-endian bit writer.
#[derive(Default)]
struct BitWriter {
    bytes: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn clear(&mut self) {
        self.bytes.clear();
        self.acc = 0;
        self.n = 0;
    }

    /// Append the low `width` bits of `v` (width <= 56 per call).
    fn put(&mut self, v: u64, width: u32) {
        if width == 0 {
            return;
        }
        if width > 32 {
            self.put(v >> 32, width - 32);
            self.put(v & 0xFFFF_FFFF, 32);
            return;
        }
        let mask = (1u64 << width) - 1;
        self.acc = (self.acc << width) | (v & mask);
        self.n += width;
        while self.n >= 8 {
            self.n -= 8;
            self.bytes.push((self.acc >> self.n) as u8);
        }
        self.acc &= (1u64 << self.n) - 1;
    }

    fn put_signed(&mut self, v: i32, width: u32) {
        self.put(v as u32 as u64, width);
    }

    /// Rice code: `u >> k` zeros, a one, then the low `k` bits.
    fn put_rice(&mut self, u: u64, k: u32) {
        let mut q = u >> k;
        while q >= 32 {
            self.put(0, 32);
            q -= 32;
        }
        self.put(1, q as u32 + 1);
        self.put(u, k);
    }

    fn align(&mut self) {
        if self.n > 0 {
            self.put(0, 8 - self.n);
        }
    }

    /// The whole bytes so far (the header before its CRC-8 is byte-aligned).
    fn bytes_so_far(&self) -> &[u8] {
        &self.bytes
    }

    fn take(&mut self) -> Vec<u8> {
        self.align();
        std::mem::take(&mut self.bytes)
    }
}

/// CRC-8, polynomial x^8 + x^2 + x + 1, initial 0 (the frame header check).
fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// CRC-16, polynomial x^16 + x^15 + x^2 + 1, initial 0 (the frame check).
fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for b in data {
        crc ^= u16::from(*b) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adr_0103_crcs_match_the_spec_check_values() {
        // The "123456789" check values of CRC-8/SMBUS and CRC-16/UMTS (BUYPASS).
        assert_eq!(crc8(b"123456789"), 0xF4);
        assert_eq!(crc16(b"123456789"), 0xFEE8);
    }

    #[test]
    fn adr_0103_frame_numbers_use_the_utf8_coding() {
        let enc = |v: u64| {
            let mut b = BitWriter::default();
            put_utf8(&mut b, v);
            b.take()
        };
        assert_eq!(enc(0x00), vec![0x00]);
        assert_eq!(enc(0x7F), vec![0x7F]);
        assert_eq!(enc(0x80), vec![0xC2, 0x80]);
        assert_eq!(enc(0x7FF), vec![0xDF, 0xBF]);
        assert_eq!(enc(0x800), vec![0xE0, 0xA0, 0x80]);
        assert_eq!(enc(0x1_0000), vec![0xF0, 0x90, 0x80, 0x80]);
    }

    #[test]
    fn adr_0103_samples_clamp_and_scale() {
        assert_eq!(to_i16(0.0), 0);
        assert_eq!(to_i16(1.0), 32_767);
        assert_eq!(to_i16(-1.0), -32_767);
        assert_eq!(to_i16(5.0), 32_767);
        assert_eq!(to_i16(f32::NAN), 0);
        assert_eq!(to_i16(f32::INFINITY), 0);
    }
}
