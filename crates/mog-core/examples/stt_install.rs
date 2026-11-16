//! Dev tool for #45 (feature `local-stt`): install the Parakeet model into a folder and
//! transcribe a 16 kHz mono 16-bit WAV file.
//!
//! ```text
//! cargo run -p mog-core --features local-stt --release --example stt_install -- \
//!     --models <dir> --source github|hf --i-consent [--wav <file>]
//! ```
//!
//! `--i-consent` stands in for the onboarding wizard's consent step (CAP-49a, #43). Without it,
//! the tool shows that the install refuses to start.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use mog_core::stt::LocalSttConfig;
use mog_core::stt_local::{extract_model_archive, HttpRangeFetch, ParakeetRecognizer};
use mog_core::stt_model::{ConsentGate, ModelConsent, ModelInstaller, ModelSource, ModelSpec};

struct CliConsent(bool);
impl ConsentGate for CliConsent {
    fn model_download_consent(&self) -> ModelConsent {
        if self.0 {
            ModelConsent::Granted
        } else {
            ModelConsent::NotAsked
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn read_wav_16k_mono(path: &PathBuf) -> Vec<f32> {
    let bytes = std::fs::read(path).expect("read the WAV file");
    let data = bytes
        .windows(4)
        .position(|w| w == b"data")
        .expect("a WAV data chunk")
        + 8;
    bytes[data..]
        .chunks_exact(2)
        .map(|c| f32::from(i16::from_le_bytes([c[0], c[1]])) / 32768.0)
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let root = PathBuf::from(arg(&args, "--models").expect("--models <dir>"));
    let source = match arg(&args, "--source").as_deref() {
        Some("hf") => ModelSource::HuggingFaceMirror,
        _ => ModelSource::GithubRelease,
    };
    let spec = ModelSpec::parakeet_tdt_v3_int8();
    let installer = ModelInstaller::new(&root, spec.clone());
    println!("status before: {:?}", installer.status());
    let cancel = AtomicBool::new(false);
    let mut last = 0u64;
    let t = Instant::now();
    let res = installer.install(
        &CliConsent(args.iter().any(|a| a == "--i-consent")),
        &HttpRangeFetch::default(),
        source,
        &|a, d| extract_model_archive(&spec, a, d),
        &cancel,
        &mut |p| {
            if p.done - last >= 50 << 20 || p.done == p.total {
                last = p.done;
                println!("{} {} / {} bytes", p.file, p.done, p.total);
            }
        },
    );
    println!(
        "install: {:?} in {:.1} s",
        res.as_ref().map_err(|e| e.code()),
        t.elapsed().as_secs_f64()
    );
    println!("status after: {:?}", installer.status());
    let (Ok(dir), Some(wav)) = (res, arg(&args, "--wav")) else {
        return;
    };
    let cfg = LocalSttConfig::for_this_device();
    println!("config: {cfg:?}");
    let t = Instant::now();
    let rec = ParakeetRecognizer::load(&dir, cfg).expect("load the model");
    println!("load_ms={:.1}", t.elapsed().as_secs_f64() * 1000.0);
    let samples = read_wav_16k_mono(&PathBuf::from(wav));
    let t = Instant::now();
    let text = rec.transcribe(&samples);
    println!(
        "decode_ms={:.1} audio_s={:.2}",
        t.elapsed().as_secs_f64() * 1000.0,
        samples.len() as f64 / 16000.0
    );
    println!("text: {text}");
}
