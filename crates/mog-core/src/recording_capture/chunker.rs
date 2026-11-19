//! The part chunker: the mixed stream as audio parts (ADR 0103).
//!
//! Parts are `<name>.part01.flac`, `<name>.part02.flac`, ... in the per-recording temp dir.
//! By default a recording is one part ([`super::PART_MAX_MINUTES`] is `None`); with a cap, a
//! part ends exactly at the cap (a sample count, so tests can pass a small one). The encoder
//! sits behind [`PartEncoder`]: [`PartFormat::Flac`] (default) or [`PartFormat::Wav`].
//! The dir is user-only (0700) and each part file is user-only (0600) on unix; on Windows the
//! dir is under the user's own local app data, which only the user can read by default.
//! A part file is created on its first sample, so a recording with no audio has no part.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use super::flac::FlacWriter;
use super::wav::WavWriter;
use super::SAMPLE_RATE;

/// One finished part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartFile {
    /// 1 for the first part.
    pub index: u32,
    /// `<name>.partNN.flac`.
    pub file_name: String,
    /// The full local path.
    pub path: PathBuf,
    /// 16 kHz samples in the part.
    pub samples: u64,
    /// Where the part starts in the recording, in ms.
    pub offset_ms: u64,
}

impl PartFile {
    /// The length of the part in ms (rounded down).
    pub fn duration_ms(&self) -> u64 {
        self.samples * 1_000 / u64::from(SAMPLE_RATE)
    }
}

/// `<name>.partNN.flac`, with at least two digits.
pub fn part_file_name(name: &str, index: u32) -> String {
    part_file_name_as(name, index, PartFormat::Flac)
}

/// `<name>.partNN.<ext>` for `format`.
pub fn part_file_name_as(name: &str, index: u32, format: PartFormat) -> String {
    format!("{name}.part{index:02}.{}", format.extension())
}

/// Create `path` (and missing parents) with user-only permissions on the leaf.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn create_private_file(path: &Path) -> io::Result<File> {
    let mut o = OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

type Current = (Box<dyn PartEncoder>, PartFile);

/// The audio format of the part files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PartFormat {
    /// FLAC 16 kHz mono 16-bit ([`FlacWriter`]).
    #[default]
    Flac,
    /// WAV PCM 16 kHz mono 16-bit ([`WavWriter`]).
    Wav,
    /// Red skeleton.
    Opus,
}

impl PartFormat {
    /// The file extension, without the dot.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Wav => "wav",
            Self::Opus => unimplemented!("red"),
        }
    }

    /// The MIME type (for a data URL or an upload).
    pub fn mime(&self) -> &'static str {
        match self {
            Self::Flac => "audio/flac",
            Self::Wav => "audio/wav",
            Self::Opus => unimplemented!("red"),
        }
    }

    fn open(&self, file: File) -> io::Result<Box<dyn PartEncoder>> {
        let out = BufWriter::new(file);
        Ok(match self {
            Self::Flac => Box::new(FlacWriter::new(out)?),
            Self::Wav => Box::new(WavWriter::new(out)?),
            Self::Opus => unimplemented!("red"),
        })
    }
}

/// One part file's encoder. A new format implements this and gets a [`PartFormat`] variant.
pub trait PartEncoder: Send {
    /// Append mono 16 kHz samples.
    fn write(&mut self, samples: &[f32]) -> io::Result<()>;
    /// The samples written so far.
    fn samples(&self) -> u64;
    /// Finish the file and sync it to disk.
    fn finish(self: Box<Self>) -> io::Result<()>;
}

fn sync(buf: BufWriter<File>) -> io::Result<()> {
    buf.into_inner().map_err(|e| e.into_error())?.sync_all()
}

impl PartEncoder for FlacWriter<BufWriter<File>> {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        FlacWriter::write(self, samples)
    }
    fn samples(&self) -> u64 {
        FlacWriter::samples(self)
    }
    fn finish(self: Box<Self>) -> io::Result<()> {
        sync(FlacWriter::finish(*self)?)
    }
}

impl PartEncoder for WavWriter<BufWriter<File>> {
    fn write(&mut self, samples: &[f32]) -> io::Result<()> {
        WavWriter::write(self, samples)
    }
    fn samples(&self) -> u64 {
        WavWriter::samples(self)
    }
    fn finish(self: Box<Self>) -> io::Result<()> {
        sync(WavWriter::finish(*self)?)
    }
}

/// Writes the mixed stream into parts.
pub struct PartWriter {
    dir: PathBuf,
    name: String,
    max_samples: u64,
    format: PartFormat,
    current: Option<Current>,
    done: Vec<PartFile>,
    written: u64,
}

impl PartWriter {
    /// FLAC parts of at most `max_samples` samples (at least 1; `u64::MAX` for one part)
    /// named after `name`, in `dir`.
    pub fn new(dir: &Path, name: &str, max_samples: u64) -> io::Result<Self> {
        Self::with_format(dir, name, max_samples, PartFormat::Flac)
    }

    /// As [`PartWriter::new`], in `format`.
    pub fn with_format(
        dir: &Path,
        name: &str,
        max_samples: u64,
        format: PartFormat,
    ) -> io::Result<Self> {
        if !dir.is_dir() {
            create_private_dir(dir)?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            name: name.to_string(),
            max_samples: max_samples.max(1),
            format,
            current: None,
            done: Vec::new(),
            written: 0,
        })
    }

    /// Append mixed 16 kHz samples. A part that reaches the limit is finished and the rest
    /// goes to the next part.
    pub fn write(&mut self, mut samples: &[f32]) -> io::Result<()> {
        while !samples.is_empty() {
            if self.current.is_none() {
                self.open_next()?;
            }
            let Some((w, _)) = self.current.as_mut() else {
                return Ok(());
            };
            let room = usize::try_from(self.max_samples - w.samples()).unwrap_or(usize::MAX);
            let n = room.min(samples.len());
            w.write(&samples[..n])?;
            self.written += n as u64;
            samples = &samples[n..];
            if w.samples() >= self.max_samples {
                self.close_current()?;
            }
        }
        Ok(())
    }

    /// The parts finished so far (not the open one).
    pub fn finished(&self) -> &[PartFile] {
        &self.done
    }

    /// Finish the open part and return every part, in order.
    pub fn finish(mut self) -> io::Result<Vec<PartFile>> {
        self.close_current()?;
        Ok(self.done)
    }

    fn open_next(&mut self) -> io::Result<()> {
        let index = u32::try_from(self.done.len() + 1).unwrap_or(u32::MAX);
        let file_name = part_file_name_as(&self.name, index, self.format);
        let path = self.dir.join(&file_name);
        let file = create_private_file(&path)?;
        let writer = self.format.open(file)?;
        let part = PartFile {
            index,
            file_name,
            path,
            samples: 0,
            offset_ms: self.written * 1_000 / u64::from(SAMPLE_RATE),
        };
        self.current = Some((writer, part));
        Ok(())
    }

    fn close_current(&mut self) -> io::Result<()> {
        if let Some((w, mut part)) = self.current.take() {
            part.samples = w.samples();
            w.finish()?;
            self.done.push(part);
        }
        Ok(())
    }
}
