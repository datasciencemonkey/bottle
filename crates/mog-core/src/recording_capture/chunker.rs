//! The part chunker: the mixed stream as FLAC parts of at most 55 min (ADR 0103).
//!
//! Parts are `<name>.part01.flac`, `<name>.part02.flac`, ... in the per-recording temp dir.
//! A part ends exactly at the part limit (a sample count, so tests can pass a small one).
//! The dir is user-only (0700) and each part file is user-only (0600) on unix; on Windows the
//! dir is under the user's own local app data, which only the user can read by default.
//! A part file is created on its first sample, so a recording with no audio has no part.

use std::fs::{File, OpenOptions};
use std::io::{self, BufWriter};
use std::path::{Path, PathBuf};

use super::flac::FlacWriter;
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
    format!("{name}.part{index:02}.flac")
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

type Current = (FlacWriter<BufWriter<File>>, PartFile);

/// The audio format of the part files (red skeleton).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PartFormat {
    /// FLAC.
    #[default]
    Flac,
    /// WAV.
    Wav,
}

impl PartFormat {
    /// Red skeleton.
    pub fn extension(&self) -> &'static str {
        unimplemented!("red")
    }
}

/// Writes the mixed stream into parts.
pub struct PartWriter {
    dir: PathBuf,
    name: String,
    max_samples: u64,
    current: Option<Current>,
    done: Vec<PartFile>,
    written: u64,
}

impl PartWriter {
    /// Parts of at most `max_samples` samples (at least 1) named after `name`, in `dir`.
    pub fn new(dir: &Path, name: &str, max_samples: u64) -> io::Result<Self> {
        if !dir.is_dir() {
            create_private_dir(dir)?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            name: name.to_string(),
            max_samples: max_samples.max(1),
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

    /// Red skeleton.
    pub fn with_format(_dir: &Path, _name: &str, _max: u64, _f: PartFormat) -> io::Result<Self> {
        unimplemented!("red")
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
        let file_name = part_file_name(&self.name, index);
        let path = self.dir.join(&file_name);
        let file = create_private_file(&path)?;
        let writer = FlacWriter::new(BufWriter::new(file))?;
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
            let buf = w.finish()?;
            let file = buf.into_inner().map_err(|e| e.into_error())?;
            file.sync_all()?;
            self.done.push(part);
        }
        Ok(())
    }
}
