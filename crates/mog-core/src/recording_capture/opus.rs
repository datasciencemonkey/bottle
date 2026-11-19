//! The call recording codec: Opus in Ogg, 16 kHz mono, 16 kbps (ADR 0103).
//!
//! The cloud side sends one whole recording to a serving endpoint as a base64 data URL, and
//! the request cap is about 30 MB. At 16 kbps one hour is about 7.2 MB (9.6 MB as base64), so
//! a 120 min recording is one part.
//!
//! - **Encoder:** `unsafe-libopus` 0.2.0, a c2rust port of libopus 1.3.1 (BSD-3-Clause). It
//!   needs no C toolchain or cmake on any OS. `OPUS_APPLICATION_VOIP`, 16 kbps VBR,
//!   20 ms frames (320 samples at 16 kHz).
//! - **Container:** the `ogg` 0.9.2 crate writes the pages (RFC 7845): page 1 holds only the
//!   `OpusHead` header (version 1, mono, the pre-skip, 16 kHz input rate, gain 0, mapping 0),
//!   page 2 only `OpusTags`, then the audio. The granule position counts 48 kHz samples from
//!   the stream start, so after audio packet `k` it is `k * 960`. The last packet ends the
//!   stream with granule `pre_skip + input_samples * 3`, so a decoder trims the zero padding
//!   of the last frame.
//! - The writer holds back one packet, so it can mark the real last packet as the stream end
//!   in [`OpusWriter::finish`].
//!
//! Samples are clamped and scaled like the FLAC writer ([`super::flac::to_i16`]). Nothing is
//! logged.

use std::io::{self, Seek, Write};

use ogg::writing::{PacketWriteEndInfo, PacketWriter};
use unsafe_libopus::{
    opus_encode, opus_encoder_create, opus_encoder_ctl, opus_encoder_destroy, OpusEncoder,
    OPUS_APPLICATION_VOIP, OPUS_GET_LOOKAHEAD_REQUEST, OPUS_SET_BITRATE_REQUEST,
    OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_VOICE,
};

use super::flac::to_i16;
use super::SAMPLE_RATE;

/// The bit rate: 16 kbps.
pub const BITRATE: i32 = 16_000;
/// Samples per Opus frame: 20 ms at 16 kHz.
pub const FRAME: usize = 320;
/// The 48 kHz granule step of one frame.
const GRANULE_PER_FRAME: u64 = 960;
/// The largest packet we let libopus write (the RFC 6716 maximum is 1275 bytes per frame).
const MAX_PACKET: usize = 1_500;
/// The Ogg stream serial number (one logical stream per file).
const SERIAL: u32 = 0x426f_7474; // "Bott"

fn opus_error(code: i32) -> io::Error {
    io::Error::other(format!("opus error {code}"))
}

/// The libopus encoder state. It is plain memory that only this writer touches.
struct Encoder(*mut OpusEncoder);

// SAFETY: libopus encoder state has no thread affinity; it is used from one thread at a time
// (the writer is owned by the recording's writer thread).
unsafe impl Send for Encoder {}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: made by `opus_encoder_create`, freed once.
            unsafe { opus_encoder_destroy(self.0) };
        }
    }
}

/// A streaming Ogg Opus writer over `W`.
pub struct OpusWriter<W: Write + Seek> {
    ogg: PacketWriter<'static, W>,
    enc: Encoder,
    pre_skip: u64,
    pending: Vec<i16>,
    /// The last encoded packet, held back until the next one (or the finish).
    held: Option<Vec<u8>>,
    packets: u64,
    total: u64,
    scratch: Vec<u8>,
}

impl<W: Write + Seek> OpusWriter<W> {
    /// Make the encoder and write the two header pages to `out`.
    pub fn new(out: W) -> io::Result<Self> {
        let mut err = 0i32;
        // SAFETY: valid arguments; the result is checked.
        let st =
            unsafe { opus_encoder_create(SAMPLE_RATE as i32, 1, OPUS_APPLICATION_VOIP, &mut err) };
        if st.is_null() || err != 0 {
            return Err(opus_error(err));
        }
        let enc = Encoder(st);
        let mut lookahead = 0i32;
        // SAFETY: a live encoder and the documented argument types of each request.
        unsafe {
            let r = opus_encoder_ctl!(enc.0, OPUS_SET_BITRATE_REQUEST, BITRATE);
            if r != 0 {
                return Err(opus_error(r));
            }
            let _ = opus_encoder_ctl!(enc.0, OPUS_SET_SIGNAL_REQUEST, OPUS_SIGNAL_VOICE);
            let r = opus_encoder_ctl!(enc.0, OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead);
            if r != 0 {
                return Err(opus_error(r));
            }
        }
        // The pre-skip is in 48 kHz samples.
        let pre_skip = u64::try_from(lookahead).unwrap_or(0) * 3;
        let mut ogg = PacketWriter::new(out);
        ogg.write_packet(
            opus_head(pre_skip as u16),
            SERIAL,
            PacketWriteEndInfo::EndPage,
            0,
        )?;
        ogg.write_packet(opus_tags(), SERIAL, PacketWriteEndInfo::EndPage, 0)?;
        Ok(Self {
            ogg,
            enc,
            pre_skip,
            pending: Vec::with_capacity(FRAME),
            held: None,
            packets: 0,
            total: 0,
            scratch: vec![0u8; MAX_PACKET],
        })
    }

    /// Append mono samples at 16 kHz.
    pub fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        for s in samples {
            self.pending.push(to_i16(*s));
            if self.pending.len() == FRAME {
                self.encode_frame()?;
            }
        }
        self.total += samples.len() as u64;
        Ok(())
    }

    /// The samples written so far.
    pub fn samples(&self) -> u64 {
        self.total
    }

    /// Encode the last (zero-padded) frame, end the stream and flush. Returns the writer.
    pub fn finish(mut self) -> io::Result<W> {
        if !self.pending.is_empty() || self.packets == 0 && self.held.is_none() {
            // A short last frame, or an empty stream (one silent frame keeps it valid).
            self.pending.resize(FRAME, 0);
            self.encode_frame()?;
        }
        let end = self.pre_skip + self.total * 3;
        if let Some(last) = self.held.take() {
            // The final granule never passes the samples the packets hold.
            let gp = end.min(self.packets * GRANULE_PER_FRAME);
            self.ogg
                .write_packet(last, SERIAL, PacketWriteEndInfo::EndStream, gp)?;
        }
        let mut out = self.ogg.into_inner();
        out.flush()?;
        Ok(out)
    }

    fn encode_frame(&mut self) -> io::Result<()> {
        // SAFETY: `pending` holds FRAME samples; `scratch` holds MAX_PACKET bytes.
        let n = unsafe {
            opus_encode(
                self.enc.0,
                self.pending.as_ptr(),
                FRAME as i32,
                self.scratch.as_mut_ptr(),
                MAX_PACKET as i32,
            )
        };
        self.pending.clear();
        let n = usize::try_from(n).map_err(|_| opus_error(n))?;
        let packet = self.scratch[..n].to_vec();
        if let Some(prev) = self.held.replace(packet) {
            // `prev` is packet number `packets` (1-based after this increment).
            let gp = self.packets * GRANULE_PER_FRAME;
            self.ogg
                .write_packet(prev, SERIAL, PacketWriteEndInfo::NormalPacket, gp)?;
        }
        self.packets += 1;
        Ok(())
    }
}

/// The 19-byte `OpusHead` (RFC 7845 section 5.1), mapping family 0.
fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut h = Vec::with_capacity(19);
    h.extend_from_slice(b"OpusHead");
    h.push(1); // version
    h.push(1); // channels
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&SAMPLE_RATE.to_le_bytes()); // input sample rate (information only)
    h.extend_from_slice(&0i16.to_le_bytes()); // output gain
    h.push(0); // mapping family
    h
}

/// `OpusTags` with the vendor string and no user comments (RFC 7845 section 5.2).
fn opus_tags() -> Vec<u8> {
    let vendor = b"Bottle (libopus 1.3.1)";
    let mut t = Vec::with_capacity(16 + vendor.len());
    t.extend_from_slice(b"OpusTags");
    t.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    t.extend_from_slice(vendor);
    t.extend_from_slice(&0u32.to_le_bytes());
    t
}
