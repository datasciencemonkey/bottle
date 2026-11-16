//! R4 local Parakeet on sherpa-onnx (#45, feature `local-stt`).
//!
//! - [`ParakeetRecognizer`]: the sherpa-onnx offline transducer recognizer with the
//!   [`LocalSttConfig`] settings (CAP-48: CPU EP, one thread per P-core).
//! - [`HttpRangeFetch`]: the real [`RangeFetch`] for [`crate::stt_model::ModelInstaller`]
//!   (CAP-49b). It sends no credentials and no cookies.
//! - [`extract_model_archive`]: unpacks only the pinned model files of the release archive.
//!
//! Model residency, preload on key-down and VAD decode are #46.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::time::Duration;

use crate::stt::LocalSttConfig;
use crate::stt_model::{content_range_start, is_allowed_url, FetchError, ModelSpec, RangeFetch};

/// The runtime of local STT, for `Transcript.runtime` (CAP-72).
pub const RUNTIME: &str = "sherpa-onnx 2.0.1 (onnxruntime CPU EP)";
/// The model of local STT, for `Transcript.model` (CAP-72).
pub const MODEL: &str = "parakeet-tdt-0.6b-v3-int8";
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
