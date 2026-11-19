//! A WAV writer for the recording stream: 16 kHz, mono, 16-bit PCM (ADR 0103).
//!
//! The fallback part format if a serving endpoint refuses FLAC data URLs. The header is the
//! canonical 44-byte RIFF/WAVE header; [`WavWriter::finish`] seeks back and fills in the sizes.

use std::io::{self, Seek, SeekFrom, Write};

use super::flac::to_i16;
use super::SAMPLE_RATE;

/// A streaming WAV writer over `W`.
pub struct WavWriter<W: Write + Seek> {
    out: W,
    samples: u64,
    buf: Vec<u8>,
}

fn header(data_bytes: u32) -> [u8; 44] {
    let mut h = [0u8; 44];
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&data_bytes.saturating_add(36).to_le_bytes());
    h[8..16].copy_from_slice(b"WAVEfmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    h[32..34].copy_from_slice(&2u16.to_le_bytes()); // block align
    h[34..36].copy_from_slice(&16u16.to_le_bytes()); // bits per sample
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_bytes.to_le_bytes());
    h
}

impl<W: Write + Seek> WavWriter<W> {
    /// Write a header with zero sizes to `out`.
    pub fn new(mut out: W) -> io::Result<Self> {
        out.write_all(&header(0))?;
        Ok(Self {
            out,
            samples: 0,
            buf: Vec::new(),
        })
    }

    /// Append mono samples at 16 kHz.
    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        self.buf.clear();
        for s in samples {
            self.buf.extend_from_slice(&to_i16(*s).to_le_bytes());
        }
        self.out.write_all(&self.buf)?;
        self.samples += samples.len() as u64;
        Ok(())
    }

    /// The samples written so far.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    /// Fill in the sizes and flush. WAV sizes are 32-bit: a part must stay under 4 GiB
    /// (about 37 h at 16 kHz; recordings stop at 120 min).
    pub fn finish(mut self) -> io::Result<W> {
        let data = u32::try_from(self.samples * 2).unwrap_or(u32::MAX);
        let end = self.out.stream_position()?;
        self.out.seek(SeekFrom::Start(0))?;
        self.out.write_all(&header(data))?;
        self.out.seek(SeekFrom::Start(end))?;
        self.out.flush()?;
        Ok(self.out)
    }
}
