//! SP-04 (#44) benchmark helpers: WAV reader, percentiles, WER, tail split, peak RSS
//! and a thin wrapper over the sherpa-onnx offline recognizer for Parakeet TDT 0.6B v3 int8.

use std::fmt;
use std::path::Path;

/// The only sample rate the benchmark accepts (Parakeet expects 16 kHz mono).
pub const SAMPLE_RATE: u32 = 16_000;

/// Summary statistics of a set of latency samples (milliseconds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    pub n: usize,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub mean: f64,
}

/// Nearest-rank percentile of an ascending slice.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    assert!(!sorted.is_empty(), "percentile of an empty slice");
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// Statistics of unsorted samples. `None` for an empty slice.
pub fn stats(samples: &[f64]) -> Option<Stats> {
    if samples.is_empty() {
        return None;
    }
    let mut v = samples.to_vec();
    v.sort_by(f64::total_cmp);
    Some(Stats {
        n: v.len(),
        p50: percentile(&v, 50.0),
        p95: percentile(&v, 95.0),
        max: v[v.len() - 1],
        mean: v.iter().sum::<f64>() / v.len() as f64,
    })
}

/// Reasons a WAV file is rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum WavError {
    Io(String),
    NotRiffWave,
    NoFmt,
    NoData,
    NotPcm16 { format: u16, bits: u16 },
    WrongChannels(u16),
    WrongRate(u32),
}

impl fmt::Display for WavError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

impl std::error::Error for WavError {}

/// Parse a 16-bit PCM, mono, 16 kHz WAV image into f32 samples in [-1, 1).
pub fn parse_wav(bytes: &[u8]) -> Result<Vec<f32>, WavError> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotRiffWave);
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32_at(bytes, pos + 4) as usize;
        let start = pos + 8;
        let end = start.saturating_add(size).min(bytes.len());
        let chunk = &bytes[start..end];
        if id == b"fmt " {
            if chunk.len() < 16 {
                return Err(WavError::NoFmt);
            }
            let mut format = u16_at(chunk, 0);
            // WAVE_FORMAT_EXTENSIBLE: the sub-format GUID starts with the real format tag.
            if format == 0xFFFE && chunk.len() >= 26 {
                format = u16_at(chunk, 24);
            }
            fmt = Some((
                format,
                u16_at(chunk, 2),
                u32_at(chunk, 4),
                u16_at(chunk, 14),
            ));
        } else if id == b"data" {
            let (format, channels, rate, bits) = fmt.ok_or(WavError::NoFmt)?;
            if format != 1 || bits != 16 {
                return Err(WavError::NotPcm16 { format, bits });
            }
            if channels != 1 {
                return Err(WavError::WrongChannels(channels));
            }
            if rate != SAMPLE_RATE {
                return Err(WavError::WrongRate(rate));
            }
            return Ok(chunk
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| f32::from(i16::from_le_bytes(*c)) / 32_768.0)
                .collect());
        }
        pos = start.saturating_add(size).saturating_add(size % 2);
    }
    Err(if fmt.is_some() {
        WavError::NoData
    } else {
        WavError::NoFmt
    })
}

/// Read a WAV file with [`parse_wav`].
pub fn read_wav(path: &Path) -> Result<Vec<f32>, WavError> {
    let bytes =
        std::fs::read(path).map_err(|e| WavError::Io(format!("{}: {e}", path.display())))?;
    parse_wav(&bytes)
}

/// Word error rate of `hypothesis` against `reference` after lower-casing and removing punctuation.
pub fn wer(reference: &str, hypothesis: &str) -> f64 {
    fn words(s: &str) -> Vec<String> {
        s.to_lowercase()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '\'' {
                    c
                } else {
                    ' '
                }
            })
            .collect::<String>()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
    let r = words(reference);
    let h = words(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    // Word-level Levenshtein distance with one rolling row.
    let mut row: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut prev = row[0];
        row[0] = i + 1;
        for (j, hw) in h.iter().enumerate() {
            let cur = row[j + 1];
            let sub = prev + usize::from(rw != hw);
            row[j + 1] = sub.min(row[j] + 1).min(cur + 1);
            prev = cur;
        }
    }
    row[h.len()] as f64 / r.len() as f64
}

/// Pad with trailing silence to `secs` seconds. Longer audio is returned unchanged.
pub fn pad_to_secs(samples: &[f32], secs: f64) -> Vec<f32> {
    let target = (secs * f64::from(SAMPLE_RATE)).round() as usize;
    let mut out = samples.to_vec();
    if out.len() < target {
        out.resize(target, 0.0);
    }
    out
}

/// Split into (head, tail): the tail is the last `tail_secs` seconds (or all audio if shorter).
pub fn split_tail(samples: &[f32], tail_secs: f64) -> (&[f32], &[f32]) {
    let tail = ((tail_secs * f64::from(SAMPLE_RATE)).round() as usize).min(samples.len());
    samples.split_at(samples.len() - tail)
}

/// Peak resident set size of this process in bytes, if the OS reports it.
pub fn peak_rss_bytes() -> Option<u64> {
    #[cfg(unix)]
    {
        // SAFETY: getrusage writes into a zeroed struct we own.
        let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
            return None;
        }
        let v = ru.ru_maxrss as u64;
        // macOS reports bytes; Linux reports KiB.
        Some(if cfg!(target_os = "macos") {
            v
        } else {
            v * 1024
        })
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        // SAFETY: the counters struct is zeroed and its size is passed in `cb`.
        unsafe {
            let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
            c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
            if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) == 0 {
                return None;
            }
            Some(c.PeakWorkingSetSize as u64)
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        None
    }
}

/// Physical performance-core count (macOS `hw.perflevel0.physicalcpu`). `None` elsewhere.
pub fn p_core_count() -> Option<usize> {
    #[cfg(target_os = "macos")]
    {
        let name = c"hw.perflevel0.physicalcpu";
        let mut v: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>();
        // SAFETY: `v` and `len` are valid for writes and `len` holds the buffer size.
        let rc = unsafe {
            libc::sysctlbyname(
                name.as_ptr(),
                (&mut v as *mut libc::c_int).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && v > 0).then_some(v as usize)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Windows hybrid CPUs need a per-model value: run.ps1 passes --threads.
        None
    }
}

/// Parakeet TDT 0.6B v3 int8 on the sherpa-onnx offline recognizer, CPU execution provider.
pub struct Engine {
    recognizer: sherpa_onnx::OfflineRecognizer,
}

impl Engine {
    /// Load encoder/decoder/joiner int8 graphs and tokens.txt from `model_dir`.
    pub fn load(model_dir: &Path, threads: i32) -> Result<Engine, String> {
        let file = |name: &str| -> Result<Option<String>, String> {
            let p = model_dir.join(name);
            if !p.is_file() {
                return Err(format!("missing model file {}", p.display()));
            }
            Ok(Some(p.to_string_lossy().into_owned()))
        };
        let mut config = sherpa_onnx::OfflineRecognizerConfig::default();
        config.model_config.transducer = sherpa_onnx::OfflineTransducerModelConfig {
            encoder: file("encoder.int8.onnx")?,
            decoder: file("decoder.int8.onnx")?,
            joiner: file("joiner.int8.onnx")?,
        };
        config.model_config.tokens = file("tokens.txt")?;
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.provider = Some("cpu".into());
        config.model_config.num_threads = threads;
        config.decoding_method = Some("greedy_search".into());
        sherpa_onnx::OfflineRecognizer::create(&config)
            .map(|recognizer| Engine { recognizer })
            .ok_or_else(|| {
                format!(
                    "sherpa-onnx could not create the recognizer from {}",
                    model_dir.display()
                )
            })
    }

    /// Decode one utterance (16 kHz mono f32) and return the text.
    pub fn transcribe(&self, samples: &[f32]) -> String {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, samples);
        self.recognizer.decode(&stream);
        stream
            .get_result()
            .map(|r| r.text.trim().to_owned())
            .unwrap_or_default()
    }
}
