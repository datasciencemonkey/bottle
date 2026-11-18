//! R4 local Parakeet on sherpa-onnx (#45, feature `local-stt`).
//!
//! - [`ParakeetRecognizer`]: the sherpa-onnx offline transducer recognizer with the
//!   [`LocalSttConfig`] settings (CAP-48: CPU EP, one thread per P-core).
//! - [`HttpRangeFetch`]: the real [`RangeFetch`] for [`crate::stt_model::ModelInstaller`]
//!   (CAP-49b). It sends no credentials and no cookies.
//! - [`extract_model_archive`]: unpacks only the pinned model files of the release archive.
//!
//! - [`ParakeetLoader`]: the R4 [`RecognizerLoader`] over [`ParakeetRecognizer`] (#46,
//!   ADR 0082). The engine (`crate::stt_engine`) owns residency, preload and VAD decode.
//! - [`SileroVad`]: the Silero VAD on sherpa-onnx, for the R4 segment cut (CAP-52). Its model
//!   file has no pinned download yet: TODO(#513).

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::stt::LocalSttConfig;
use crate::stt_engine::{
    Recognizer, RecognizerLoader, SttIdentity, Vad, STT_SAMPLE_RATE, VAD_WINDOW,
};
use crate::stt_model::{content_range_start, is_allowed_url, FetchError, ModelSpec, RangeFetch};

/// The runtime of local STT, for `Transcript.runtime` (CAP-72).
pub const RUNTIME: &str = crate::stt::LOCAL_STT_RUNTIME;
/// The model of local STT, for `Transcript.model` (CAP-72).
pub const MODEL: &str = crate::stt::LOCAL_STT_MODEL;
/// The sample rate the recognizer takes.
pub const SAMPLE_RATE: u32 = 16_000;

/// Parakeet TDT 0.6B v3 int8 on the sherpa-onnx offline recognizer.
pub struct ParakeetRecognizer {
    recognizer: sherpa_onnx::OfflineRecognizer,
    config: LocalSttConfig,
}

impl std::fmt::Debug for ParakeetRecognizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParakeetRecognizer")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl ParakeetRecognizer {
    /// Load the model files from `model_dir` (the folder that
    /// [`crate::stt_model::ModelInstaller::install`] returns).
    pub fn load(model_dir: &Path, config: LocalSttConfig) -> Result<Self, &'static str> {
        let file = |name: &str| -> Result<Option<String>, &'static str> {
            let p = model_dir.join(name);
            if !p.is_file() {
                return Err("stt_model_missing");
            }
            Ok(Some(p.to_string_lossy().into_owned()))
        };
        let mut c = sherpa_onnx::OfflineRecognizerConfig::default();
        c.model_config.transducer = sherpa_onnx::OfflineTransducerModelConfig {
            encoder: file("encoder.int8.onnx")?,
            decoder: file("decoder.int8.onnx")?,
            joiner: file("joiner.int8.onnx")?,
        };
        c.model_config.tokens = file("tokens.txt")?;
        c.model_config.model_type = Some(config.model_type.into());
        c.model_config.provider = Some(config.provider.into());
        c.model_config.num_threads = i32::try_from(config.num_threads).unwrap_or(1);
        c.decoding_method = Some(config.decoding_method.into());
        let recognizer =
            sherpa_onnx::OfflineRecognizer::create(&c).ok_or("stt_model_load_failed")?;
        Ok(ParakeetRecognizer { recognizer, config })
    }

    /// The settings the recognizer runs with (CAP-48 inspection).
    pub fn config(&self) -> &LocalSttConfig {
        &self.config
    }

    /// Decode one utterance (16 kHz mono f32 samples) and return the text.
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

impl Recognizer for ParakeetRecognizer {
    fn transcribe(&mut self, samples: &[f32]) -> String {
        ParakeetRecognizer::transcribe(self, samples)
    }
}

/// The R4 loader of local Parakeet from an installed model folder (#46, CAP-51c).
#[derive(Debug, Clone)]
pub struct ParakeetLoader {
    model_dir: PathBuf,
    config: LocalSttConfig,
}

impl ParakeetLoader {
    /// A loader of the model in `model_dir` with the CAP-48 settings of this device.
    pub fn new(model_dir: impl Into<PathBuf>) -> Self {
        Self {
            model_dir: model_dir.into(),
            config: LocalSttConfig::for_this_device(),
        }
    }
}

impl RecognizerLoader for ParakeetLoader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        ParakeetRecognizer::load(&self.model_dir, self.config.clone())
            .map(|r| Box::new(r) as Box<dyn Recognizer>)
    }

    fn identity(&self) -> SttIdentity {
        SttIdentity::local_parakeet()
    }
}

/// The Silero VAD on sherpa-onnx (CAP-52). It reads one 32 ms window at 16 kHz at a time and
/// says whether speech is going on. The detector's own speech segments are dropped at once:
/// R4 cuts its segments with `SegmentCutter`. TODO(#513): pin and install `silero_vad.onnx`.
pub struct SileroVad {
    vad: sherpa_onnx::VoiceActivityDetector,
}

impl std::fmt::Debug for SileroVad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SileroVad").finish_non_exhaustive()
    }
}

impl SileroVad {
    /// Load the Silero model file `model`. CPU EP, one thread.
    pub fn load(model: &Path) -> Result<Self, &'static str> {
        if !model.is_file() {
            return Err("stt_vad_model_missing");
        }
        let c = sherpa_onnx::VadModelConfig {
            silero_vad: sherpa_onnx::SileroVadModelConfig {
                model: Some(model.to_string_lossy().into_owned()),
                threshold: 0.5,
                min_silence_duration: 0.1,
                min_speech_duration: 0.05,
                window_size: VAD_WINDOW as i32,
                max_speech_duration: 30.0,
            },
            sample_rate: STT_SAMPLE_RATE as i32,
            num_threads: 1,
            provider: Some(crate::stt::LOCAL_STT_PROVIDER.into()),
            ..Default::default()
        };
        let vad =
            sherpa_onnx::VoiceActivityDetector::create(&c, 31.0).ok_or("stt_vad_load_failed")?;
        Ok(Self { vad })
    }
}

impl Vad for SileroVad {
    fn is_speech(&mut self, window: &[f32]) -> bool {
        self.vad.accept_waveform(window);
        let speech = self.vad.detected();
        // The detector keeps a copy of each speech segment; drop it now (audio in RAM only,
        // and only as long as needed).
        while !self.vad.is_empty() {
            self.vad.pop();
        }
        speech
    }

    fn reset(&mut self) {
        self.vad.reset();
    }
}

/// The HTTPS client for the model download. It refuses any URL outside the pinned sources.
#[derive(Debug)]
pub struct HttpRangeFetch {
    agent: ureq::Agent,
}

impl Default for HttpRangeFetch {
    fn default() -> Self {
        HttpRangeFetch {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(20))
                .timeout_read(Duration::from_secs(60))
                .https_only(true)
                .build(),
        }
    }
}

impl RangeFetch for HttpRangeFetch {
    fn open(&self, url: &str, offset: u64) -> Result<(u64, Box<dyn Read + '_>), FetchError> {
        if !is_allowed_url(url) {
            return Err(FetchError::Status(0));
        }
        let mut req = self.agent.get(url);
        if offset > 0 {
            req = req.set("Range", &format!("bytes={offset}-"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Err(FetchError::Status(code)),
            Err(ureq::Error::Transport(_)) => return Err(FetchError::Transport),
        };
        let start = if resp.status() == 206 {
            resp.header("Content-Range")
                .and_then(content_range_start)
                .ok_or(FetchError::Status(206))?
        } else {
            0
        };
        Ok((start, Box::new(resp.into_reader())))
    }
}

/// Unpack the model files of `spec` from the release archive into `dest`. Only the pinned file
/// names under the archive's top folder are written; every other entry is skipped, so a crafted
/// path cannot leave `dest`.
pub fn extract_model_archive(spec: &ModelSpec, archive: &Path, dest: &Path) -> io::Result<()> {
    let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(File::open(archive)?));
    for entry in tar.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue; // no links, folders or devices
        }
        let path = entry.path()?.into_owned();
        let mut parts = path.components();
        let (Some(top), Some(name), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if top.as_os_str() != spec.name {
            continue;
        }
        let Some(pin) = spec.files.iter().find(|f| name.as_os_str() == f.name) else {
            continue;
        };
        let mut out = File::create(dest.join(pin.name))?;
        io::copy(&mut (&mut entry).take(pin.size + 1), &mut out)?;
    }
    Ok(())
}
