//! Join the Ogg Opus parts of one call into one Ogg Opus stream, with no re-encode.
//!
//! The capture writes a part about every 5 minutes (`recording_capture::PART_MAX_MINUTES`) and
//! the upload stage sends each part to the volume during the call. The transcribe call must
//! hear the whole call as one stream, so the speaker labels stay the same from the start to the
//! end. This module reads the Opus packets of every part and writes them again as one stream:
//!
//! - one `OpusHead` and one `OpusTags` (those of the first part; the later headers are dropped);
//! - one serial number and page numbers that count from 0;
//! - the granule position of packet `k` is `k * 960` (48 kHz samples, the pre-skip included),
//!   as [`super::super::recording_capture::opus::OpusWriter`] writes it. The last packet has the
//!   exact end: `pre_skip` plus the samples of the earlier parts plus the real samples of the
//!   last part (never more than the packets hold);
//! - the end-of-stream flag only on the last page.
//!
//! This is not a chained Ogg (several logical streams in one file): a model provider may read
//! only the first logical stream of a chain. A part that was cut short by a crash (no end of
//! stream, maybe a torn last page) still gives its whole packets. Every part must have the same
//! `OpusHead` (mono, the same pre-skip), which holds for parts made by one encoder.

use std::io::Cursor;

use ogg::reading::PacketReader;
use ogg::writing::{PacketWriteEndInfo, PacketWriter};

/// The Ogg stream serial of the joined stream (the same one the encoder uses).
const SERIAL: u32 = 0x426f_7474;
/// The 48 kHz granule step of one 20 ms frame.
const GRANULE_PER_FRAME: u64 = 960;
/// A 48 kHz granule step per second.
const GRANULE_RATE: f64 = 48_000.0;

/// Why a part cannot be read or joined. No upstream text, no audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StitchError {
    /// Not Ogg Opus (no `OpusHead` and `OpusTags` first), or no audio packet.
    NotOpus,
    /// The parts have different `OpusHead` values.
    Mismatch,
    /// No part given.
    Empty,
}

/// The packets of one Ogg Opus part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOpus {
    head: Vec<u8>,
    tags: Vec<u8>,
    audio: Vec<Vec<u8>>,
    /// The pre-skip (48 kHz samples) from `OpusHead`.
    pub pre_skip: u64,
    /// The granule position of the last page, when it ends the stream.
    end_gp: Option<u64>,
}

impl ParsedOpus {
    /// The audio packets (20 ms each).
    pub fn packets(&self) -> usize {
        self.audio.len()
    }

    /// The samples that a decoder gives after the trim, at 48 kHz. A part with no end of stream
    /// (a crash) counts every whole packet.
    pub fn samples_48k(&self) -> u64 {
        let all = self.audio.len() as u64 * GRANULE_PER_FRAME;
        match self.end_gp {
            Some(gp) => gp.saturating_sub(self.pre_skip).min(all),
            None => all,
        }
    }

    /// The length of the part in seconds.
    pub fn duration_s(&self) -> f64 {
        self.samples_48k() as f64 / GRANULE_RATE
    }
}

/// Read an Ogg Opus part. A torn or missing tail is not an error: the packets read before it
/// are kept.
pub fn parse(bytes: &[u8]) -> Result<ParsedOpus, StitchError> {
    let mut rd = PacketReader::new(Cursor::new(bytes));
    let head = match rd.read_packet() {
        Ok(Some(p)) if p.data.starts_with(b"OpusHead") && p.data.len() >= 19 => p.data,
        _ => return Err(StitchError::NotOpus),
    };
    let tags = match rd.read_packet() {
        Ok(Some(p)) if p.data.starts_with(b"OpusTags") => p.data,
        _ => return Err(StitchError::NotOpus),
    };
    let pre_skip = u64::from(u16::from_le_bytes([head[10], head[11]]));
    let mut audio = Vec::new();
    let mut end_gp = None;
    // A read error ends the loop: a crash can tear the last page.
    while let Ok(Some(p)) = rd.read_packet() {
        if p.last_in_stream() {
            end_gp = Some(p.absgp_page());
        }
        audio.push(p.data);
    }
    if audio.is_empty() {
        return Err(StitchError::NotOpus);
    }
    Ok(ParsedOpus {
        head,
        tags,
        audio,
        pre_skip,
        end_gp,
    })
}

/// The length of an Ogg Opus part in seconds, or `None` when it is not one.
pub fn duration_s(bytes: &[u8]) -> Option<f64> {
    parse(bytes).ok().map(|p| p.duration_s())
}

/// Join `parts` (in order) into one Ogg Opus stream. One part comes back as it is.
pub fn join(parts: &[&[u8]]) -> Result<Vec<u8>, StitchError> {
    match parts {
        [] => return Err(StitchError::Empty),
        [one] => {
            parse(one)?;
            return Ok((*one).to_vec());
        }
        _ => {}
    }
    let parsed = parts
        .iter()
        .map(|p| parse(p))
        .collect::<Result<Vec<_>, _>>()?;
    let first = &parsed[0];
    // The channel count, the rate field and the pre-skip must agree (bytes 9..16 of OpusHead).
    if parsed.iter().any(|p| p.head[9..16] != first.head[9..16]) {
        return Err(StitchError::Mismatch);
    }
    let total: u64 = parsed.iter().map(|p| p.audio.len() as u64).sum();
    let before_last: u64 = parsed[..parsed.len() - 1]
        .iter()
        .map(|p| p.audio.len() as u64 * GRANULE_PER_FRAME)
        .sum();
    let last = &parsed[parsed.len() - 1];
    let end = (first.pre_skip + before_last + last.samples_48k()).min(total * GRANULE_PER_FRAME);

    let mut out = PacketWriter::new(Vec::new());
    let io = |_| StitchError::NotOpus;
    out.write_packet(first.head.clone(), SERIAL, PacketWriteEndInfo::EndPage, 0)
        .map_err(io)?;
    out.write_packet(first.tags.clone(), SERIAL, PacketWriteEndInfo::EndPage, 0)
        .map_err(io)?;
    let mut k = 0u64;
    for part in &parsed {
        for packet in &part.audio {
            k += 1;
            let (info, gp) = if k == total {
                (PacketWriteEndInfo::EndStream, end)
            } else {
                (PacketWriteEndInfo::NormalPacket, k * GRANULE_PER_FRAME)
            };
            out.write_packet(packet.clone(), SERIAL, info, gp)
                .map_err(io)?;
        }
    }
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recording_capture::opus::{OpusWriter, FRAME};
    use unsafe_libopus::{opus_decode, opus_decoder_create, opus_decoder_destroy};

    /// A real part from our own encoder: `n` samples of a tone.
    fn part(n: usize, hz: f32) -> Vec<u8> {
        let mut w = OpusWriter::new(Cursor::new(Vec::new())).unwrap();
        let s: Vec<f32> = (0..n)
            .map(|i| 0.3 * (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin())
            .collect();
        w.write(&s).unwrap();
        w.finish().unwrap().into_inner()
    }

    /// One Ogg page header, read by hand (not by the `ogg` crate), for the structure checks.
    struct Page {
        flags: u8,
        gp: u64,
        serial: u32,
        seq: u32,
        crc_ok: bool,
        packets: Vec<Vec<u8>>,
    }

    fn crc32_ogg(data: &[u8]) -> u32 {
        let mut crc = 0u32;
        for &b in data {
            crc ^= u32::from(b) << 24;
            for _ in 0..8 {
                crc = if crc & 0x8000_0000 != 0 {
                    (crc << 1) ^ 0x04c1_1db7
                } else {
                    crc << 1
                };
            }
        }
        crc
    }

    fn pages(bytes: &[u8]) -> Vec<Page> {
        let mut out = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            assert_eq!(&bytes[at..at + 4], b"OggS", "a page starts with OggS");
            let nseg = bytes[at + 26] as usize;
            let table = &bytes[at + 27..at + 27 + nseg];
            let body_len: usize = table.iter().map(|&s| s as usize).sum();
            let end = at + 27 + nseg + body_len;
            let stored = u32::from_le_bytes(bytes[at + 22..at + 26].try_into().unwrap());
            let mut copy = bytes[at..end].to_vec();
            copy[22..26].copy_from_slice(&[0; 4]);
            let mut packets = Vec::new();
            let mut cur = Vec::new();
            let mut off = at + 27 + nseg;
            for &s in table {
                cur.extend_from_slice(&bytes[off..off + s as usize]);
                off += s as usize;
                if s < 255 {
                    packets.push(std::mem::take(&mut cur));
                }
            }
            out.push(Page {
                flags: bytes[at + 5],
                gp: u64::from_le_bytes(bytes[at + 6..at + 14].try_into().unwrap()),
                serial: u32::from_le_bytes(bytes[at + 14..at + 18].try_into().unwrap()),
                seq: u32::from_le_bytes(bytes[at + 18..at + 22].try_into().unwrap()),
                crc_ok: crc32_ogg(&copy) == stored,
                packets,
            });
            at = end;
        }
        out
    }

    fn decode_all(bytes: &[u8]) -> usize {
        let p = parse(bytes).unwrap();
        let mut err = 0;
        let dec = unsafe { opus_decoder_create(16_000, 1, &mut err) };
        assert!(!dec.is_null() && err == 0);
        let mut pcm = vec![0i16; FRAME];
        let mut n = 0;
        for pk in &p.audio {
            let r = unsafe {
                opus_decode(
                    dec,
                    pk.as_ptr(),
                    pk.len() as i32,
                    pcm.as_mut_ptr(),
                    FRAME as i32,
                    0,
                )
            };
            assert_eq!(r, FRAME as i32, "every packet decodes to one frame");
            n += r as usize;
        }
        unsafe { opus_decoder_destroy(dec) };
        n
    }

    #[test]
    fn joined_file_is_one_valid_stream() {
        let a = part(16_000 * 3 + 100, 220.0);
        let b = part(16_000 * 2, 330.0);
        let c = part(16_000 + 7, 440.0);
        let pa = parse(&a).unwrap();
        let pc = parse(&c).unwrap();
        let joined = join(&[&a, &b, &c]).unwrap();

        let pg = pages(&joined);
        assert!(pg.iter().all(|p| p.crc_ok), "every page CRC is valid");
        assert!(pg.iter().all(|p| p.serial == SERIAL), "one serial number");
        for (i, p) in pg.iter().enumerate() {
            assert_eq!(p.seq, i as u32, "page numbers count from 0 with no gap");
        }
        assert_eq!(pg[0].flags & 0x02, 0x02, "the first page is BOS");
        assert_eq!(
            pg.iter().filter(|p| p.flags & 0x02 != 0).count(),
            1,
            "BOS once"
        );
        assert_eq!(
            pg.iter().filter(|p| p.flags & 0x04 != 0).count(),
            1,
            "EOS once"
        );
        assert_eq!(
            pg.last().unwrap().flags & 0x04,
            0x04,
            "EOS on the last page"
        );
        let all: Vec<&Vec<u8>> = pg.iter().flat_map(|p| p.packets.iter()).collect();
        assert_eq!(
            all.iter().filter(|p| p.starts_with(b"OpusHead")).count(),
            1,
            "one OpusHead"
        );
        assert_eq!(
            all.iter().filter(|p| p.starts_with(b"OpusTags")).count(),
            1,
            "one OpusTags"
        );
        assert!(all[0].starts_with(b"OpusHead") && all[1].starts_with(b"OpusTags"));
        let mut prev = 0;
        for p in &pg {
            assert!(p.gp >= prev, "granule positions never go back");
            prev = p.gp;
        }

        let joined_parsed = parse(&joined).unwrap();
        let expect_packets: usize = [&a, &b, &c]
            .iter()
            .map(|x| parse(x).unwrap().packets())
            .sum();
        assert_eq!(joined_parsed.packets(), expect_packets);
        assert_eq!(
            joined_parsed.pre_skip, pa.pre_skip,
            "the pre-skip of part 1"
        );
        // The end: pre-skip + whole frames of parts 1 and 2 + the real samples of part 3.
        let whole = (pa.packets() + parse(&b).unwrap().packets()) as u64 * GRANULE_PER_FRAME;
        assert_eq!(
            pg.last().unwrap().gp,
            pa.pre_skip + whole + pc.samples_48k()
        );
        assert_eq!(decode_all(&joined), expect_packets * FRAME);
    }

    #[test]
    fn one_part_comes_back_unchanged() {
        let a = part(16_000, 220.0);
        assert_eq!(join(&[&a]).unwrap(), a);
    }

    #[test]
    fn a_torn_last_part_keeps_its_whole_packets() {
        let a = part(16_000 * 2, 220.0);
        let b = part(16_000 * 6, 330.0);
        // A crash: the file ends in the middle of a page, with no end of stream.
        let torn = &b[..b.len() - 40];
        let pb = parse(torn).unwrap();
        assert!(pb.packets() > 0 && pb.end_gp.is_none());
        let joined = join(&[&a, torn]).unwrap();
        let pg = pages(&joined);
        assert!(pg.iter().all(|p| p.crc_ok));
        assert_eq!(pg.last().unwrap().flags & 0x04, 0x04);
        assert_eq!(
            parse(&joined).unwrap().packets(),
            parse(&a).unwrap().packets() + pb.packets()
        );
    }

    #[test]
    fn not_opus_is_refused() {
        assert_eq!(
            join(&[b"nope".as_slice(), b"no".as_slice()]),
            Err(StitchError::NotOpus)
        );
        assert_eq!(join(&[]), Err(StitchError::Empty));
    }

    #[test]
    fn duration_reads_the_end_granule() {
        let a = part(16_000 * 4 + 13, 220.0);
        let d = duration_s(&a).unwrap();
        assert!((d - (4.0 + 13.0 / 16_000.0)).abs() < 0.001, "{d}");
    }
}
