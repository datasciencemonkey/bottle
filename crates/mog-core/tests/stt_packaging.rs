//! #45 local Parakeet STT packaging: CPU EP and P-core threads (CAP-48), consented, resumable,
//! hash-pinned model download from the allowed sources only (CAP-49a, CAP-49b, CAP-49c), and the
//! vendored, hash-pinned sherpa-onnx archives (CAP-50a, CAP-50b). AT-37 inspects these.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use mog_core::stt::{
    p_cores_from_efficiency_classes, perf_core_count, LocalSttConfig, LOCAL_STT_PROVIDER,
};
use mog_core::stt_model::{
    is_allowed_url, sha256_hex, ConsentGate, FetchError, InstallError, ModelConsent,
    ModelInstaller, ModelSource, ModelSpec, ModelStatus, PinnedFile, RangeFetch,
    SHERPA_ONNX_ARCHIVE_PINS,
};

const GH: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/";
const HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/";

struct Gate(ModelConsent);
impl ConsentGate for Gate {
    fn model_download_consent(&self) -> ModelConsent {
        self.0
    }
}

/// A test double for the network: serves fixed bodies by URL and records every request.
#[derive(Default)]
struct FakeNet {
    bodies: BTreeMap<String, Vec<u8>>,
    /// Ignore `Range` and always send the whole body (an HTTP 200 reply).
    ignore_range: bool,
    /// Stop the body after this many bytes and return an error (a broken download).
    break_after: RefCell<Option<usize>>,
    calls: RefCell<Vec<(String, u64)>>,
}

impl RangeFetch for FakeNet {
    fn open(&self, url: &str, offset: u64) -> Result<(u64, Box<dyn Read + '_>), FetchError> {
        self.calls.borrow_mut().push((url.to_string(), offset));
        let body = self
            .bodies
            .get(url)
            .ok_or(FetchError::Status(404))?
            .clone();
        let start = if self.ignore_range { 0 } else { offset };
        let mut rest = body[start as usize..].to_vec();
        if let Some(n) = self.break_after.borrow_mut().take() {
            rest.truncate(n);
            return Ok((start, Box::new(Broken(io::Cursor::new(rest)))));
        }
        Ok((start, Box::new(io::Cursor::new(rest))))
    }
}

/// A reader that returns an error at the end of its data, like a dropped connection.
struct Broken(io::Cursor<Vec<u8>>);
impl Read for Broken {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.0.read(buf)? {
            0 => Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset")),
            n => Ok(n),
        }
    }
}

fn pinned(name: &'static str, body: &[u8]) -> PinnedFile {
    PinnedFile {
        name,
        size: body.len() as u64,
        sha256: Box::leak(sha256_hex(body).into_boxed_str()),
    }
}

/// A small model spec with the real file names, fake bodies and their real SHA-256.
fn fake_spec() -> (ModelSpec, BTreeMap<&'static str, Vec<u8>>) {
    let mut bodies = BTreeMap::new();
    for name in [
        "encoder.int8.onnx",
        "decoder.int8.onnx",
        "joiner.int8.onnx",
        "tokens.txt",
    ] {
        bodies.insert(name, format!("fake {name} ").repeat(500).into_bytes());
    }
    let files = bodies.iter().map(|(n, b)| pinned(n, b)).collect::<Vec<_>>();
    let archive = b"fake archive bytes".repeat(400);
    let spec = ModelSpec {
        name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        archive: pinned("sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2", &archive),
        files,
    };
    bodies.insert("archive", archive);
    (spec, bodies)
}

fn hf_net(bodies: &BTreeMap<&'static str, Vec<u8>>) -> FakeNet {
    let mut net = FakeNet::default();
    for (name, body) in bodies {
        if *name != "archive" {
            net.bodies.insert(format!("{HF}{name}"), body.clone());
        }
    }
    net
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mog-stt-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn no_extract(_archive: &Path, _dest: &Path) -> io::Result<()> {
    panic!("the HF source never extracts");
}

fn install(
    installer: &ModelInstaller,
    gate: &Gate,
    net: &FakeNet,
    source: ModelSource,
    extract: &dyn Fn(&Path, &Path) -> io::Result<()>,
) -> Result<PathBuf, InstallError> {
    let cancel = AtomicBool::new(false);
    installer.install(gate, net, source, extract, &cancel, &mut |_| {})
}

#[test]
fn cap_48_local_stt_uses_the_cpu_execution_provider_and_p_core_threads() {
    let cfg = LocalSttConfig::for_this_device();
    assert_eq!(cfg.provider, "cpu");
    assert_eq!(LOCAL_STT_PROVIDER, "cpu");
    assert_eq!(cfg.num_threads, perf_core_count());
    assert!(cfg.num_threads >= 1);
    assert_eq!(cfg.model_type, "nemo_transducer");
    assert_eq!(cfg.decoding_method, "greedy_search");
}

#[test]
fn cap_48_p_cores_are_the_cores_of_the_highest_efficiency_class() {
    // i5-1235U: 2 P-cores (class 1) and 8 E-cores (class 0).
    let classes = [1, 1, 0, 0, 0, 0, 0, 0, 0, 0];
    assert_eq!(p_cores_from_efficiency_classes(&classes), 2);
    // A CPU with one core type reports one class for every core: all are P-cores.
    assert_eq!(p_cores_from_efficiency_classes(&[0; 6]), 6);
    assert_eq!(p_cores_from_efficiency_classes(&[]), 1);
}

#[cfg(target_os = "macos")]
#[test]
fn cap_48_macos_thread_count_is_the_perflevel0_core_count() {
    let out = std::process::Command::new("sysctl")
        .args(["-n", "hw.perflevel0.physicalcpu"])
        .output()
        .unwrap();
    let want: usize = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap();
    assert_eq!(perf_core_count(), want);
}

#[test]
fn cap_49a_no_download_starts_before_consent() {
    let (spec, bodies) = fake_spec();
    let net = hf_net(&bodies);
    for consent in [ModelConsent::NotAsked, ModelConsent::Declined] {
        let dir = tmp("consent");
        let installer = ModelInstaller::new(&dir, spec.clone());
        let err = install(
            &installer,
            &Gate(consent),
            &net,
            ModelSource::HuggingFaceMirror,
            &no_extract,
        )
        .unwrap_err();
        assert_eq!(err, InstallError::ConsentRequired);
        assert_eq!(err.code(), "stt_consent_required");
        assert!(net.calls.borrow().is_empty(), "a request went out before consent");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0, "a file was written");
        assert_eq!(installer.status(), ModelStatus::Missing);
    }
}

#[test]
fn cap_49a_install_runs_after_consent_and_reports_installed() {
    let (spec, bodies) = fake_spec();
    let net = hf_net(&bodies);
    let dir = tmp("granted");
    let installer = ModelInstaller::new(&dir, spec.clone());
    let model = install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    )
    .unwrap();
    assert_eq!(model, dir.join(spec.name));
    for f in &spec.files {
        assert_eq!(
            fs::read(model.join(f.name)).unwrap(),
            bodies[f.name].as_slice()
        );
    }
    assert_eq!(installer.status(), ModelStatus::Installed);
    // A second install is a no-op: no new request.
    let n = net.calls.borrow().len();
    install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    )
    .unwrap();
    assert_eq!(net.calls.borrow().len(), n);
}

#[test]
fn cap_49b_downloads_only_from_the_sherpa_onnx_release_or_the_hf_mirror() {
    let spec = ModelSpec::parakeet_tdt_v3_int8();
    let gh = ModelSource::GithubRelease.urls(&spec);
    assert_eq!(
        gh,
        vec![format!(
            "{GH}sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2"
        )]
    );
    let hf = ModelSource::HuggingFaceMirror.urls(&spec);
    assert_eq!(hf.len(), 4);
    for url in gh.iter().chain(hf.iter()) {
        assert!(url.starts_with(GH) || url.starts_with(HF), "{url}");
        assert!(is_allowed_url(url), "{url}");
    }
    for bad in [
        "http://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/x.tar.bz2",
        "https://github.com/evil/sherpa-onnx/releases/download/asr-models/x.tar.bz2",
        "https://github.com.evil.example/k2-fsa/sherpa-onnx/releases/download/asr-models/x",
        "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/main/encoder.int8.onnx",
        "https://huggingface.co/someone/else/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/x",
        "https://blob.handy.computer/parakeet-v3-int8.tar.gz",
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/../../x",
    ] {
        assert!(!is_allowed_url(bad), "{bad}");
    }
}

#[test]
fn cap_49b_a_broken_download_resumes_from_the_partial_file() {
    let (spec, bodies) = fake_spec();
    let net = hf_net(&bodies);
    let dir = tmp("resume");
    let installer = ModelInstaller::new(&dir, spec.clone());
    *net.break_after.borrow_mut() = Some(700);
    let err = install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    )
    .unwrap_err();
    assert_eq!(err.code(), "stt_download_failed");
    match installer.status() {
        ModelStatus::Partial { bytes } => assert_eq!(bytes, 700),
        other => panic!("want Partial, got {other:?}"),
    }
    net.calls.borrow_mut().clear();
    install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    )
    .unwrap();
    let calls = net.calls.borrow();
    assert_eq!(calls[0].1, 700, "the retry must ask for the rest only");
    assert_eq!(installer.status(), ModelStatus::Installed);
}

#[test]
fn cap_49b_a_server_without_range_support_restarts_cleanly() {
    let (spec, bodies) = fake_spec();
    let mut net = hf_net(&bodies);
    let dir = tmp("norange");
    let installer = ModelInstaller::new(&dir, spec.clone());
    *net.break_after.borrow_mut() = Some(300);
    let _ = install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    );
    net.ignore_range = true;
    let model = install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    )
    .unwrap();
    let first = &spec.files[0];
    assert_eq!(
        fs::read(model.join(first.name)).unwrap(),
        bodies[first.name].as_slice()
    );
}

#[test]
fn cap_49c_a_changed_hash_stops_the_install() {
    let (spec, mut bodies) = fake_spec();
    // The server sends other bytes of the same size for one file.
    let enc = bodies.get_mut("encoder.int8.onnx").unwrap();
    enc[10] ^= 0xff;
    let net = hf_net(&bodies);
    let dir = tmp("hash");
    let installer = ModelInstaller::new(&dir, spec.clone());
    let err = install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::HuggingFaceMirror,
        &no_extract,
    )
    .unwrap_err();
    assert_eq!(
        err,
        InstallError::HashMismatch {
            file: "encoder.int8.onnx"
        }
    );
    assert_eq!(err.code(), "stt_hash_mismatch");
    assert!(!dir.join(spec.name).join("encoder.int8.onnx").exists());
    assert!(
        !dir.join("encoder.int8.onnx.partial").exists(),
        "a bad partial must not be resumed"
    );
    assert_ne!(installer.status(), ModelStatus::Installed);
}

#[test]
fn cap_49c_the_archive_and_the_extracted_files_are_checked() {
    let (spec, bodies) = fake_spec();
    let mut net = FakeNet::default();
    net.bodies.insert(
        format!("{GH}{}", spec.archive.name),
        bodies["archive"].clone(),
    );
    let good = |_a: &Path, dest: &Path| -> io::Result<()> {
        for f in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"] {
            fs::write(dest.join(f), &fake_spec().1[f])?;
        }
        Ok(())
    };
    let dir = tmp("archive");
    let installer = ModelInstaller::new(&dir, spec.clone());
    install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::GithubRelease,
        &good,
    )
    .unwrap();
    assert_eq!(installer.status(), ModelStatus::Installed);
    assert!(!dir.join(spec.archive.name).exists(), "the archive is removed after the extract");

    // An archive whose contents do not match the pinned file hashes stops the install.
    let bad = |_a: &Path, dest: &Path| -> io::Result<()> {
        for f in ["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx"] {
            fs::write(dest.join(f), &fake_spec().1[f])?;
        }
        fs::write(dest.join("tokens.txt"), b"changed")?;
        Ok(())
    };
    let dir = tmp("archive-bad");
    let installer = ModelInstaller::new(&dir, spec.clone());
    let err = install(
        &installer,
        &Gate(ModelConsent::Granted),
        &net,
        ModelSource::GithubRelease,
        &bad,
    )
    .unwrap_err();
    assert_eq!(err.code(), "stt_hash_mismatch");
    assert!(!dir.join(spec.name).exists());
}

#[test]
fn cap_49c_the_real_model_pins_match_the_sp_04_hashes() {
    let spec = ModelSpec::parakeet_tdt_v3_int8();
    assert_eq!(spec.archive.size, 487_211_443);
    assert_eq!(
        spec.archive.sha256,
        "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9"
    );
    let enc = spec
        .files
        .iter()
        .find(|f| f.name == "encoder.int8.onnx")
        .unwrap();
    assert_eq!(
        enc.sha256,
        "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479"
    );
    assert_eq!(spec.files.len(), 4);
}

#[test]
fn cap_50a_sherpa_onnx_archive_pins_cover_the_ci_and_release_targets() {
    let pins = SHERPA_ONNX_ARCHIVE_PINS;
    for name in [
        "sherpa-onnx-v2.0.1-osx-arm64-static-lib.tar.bz2",
        "sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2",
        "sherpa-onnx-v2.0.1-linux-x64-static-lib.tar.bz2",
    ] {
        let (_, sha) = pins
            .iter()
            .find(|(n, _)| *n == name)
            .unwrap_or_else(|| panic!("no pin for {name}"));
        assert_eq!(sha.len(), 64);
        assert!(sha.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }
    // The build script and the vendor script read the same pin file.
    let file = include_str!("../sherpa-onnx-archives.sha256");
    for (name, sha) in pins {
        assert!(file.contains(&format!("{sha}  {name}")), "{name}");
    }
}

#[test]
fn cap_50b_the_core_does_not_link_sherpa_onnx_without_the_local_stt_feature() {
    // Without `local-stt` the build fetches nothing; with it, build.rs refuses a build that has no
    // vendored, hash-checked archive (see docs/stt-local.md).
    let manifest = include_str!("../Cargo.toml");
    assert!(manifest.contains("local-stt = ["));
    assert!(manifest.contains("sherpa-onnx = { version = \"=2.0.1\", optional = true"));
    let build = include_str!("../build.rs");
    assert!(build.contains("SHERPA_ONNX_ARCHIVE_DIR"));
    assert!(build.contains("CARGO_FEATURE_LOCAL_STT"));
}

#[test]
fn cap_49_errors_are_content_free_codes() {
    for (err, code) in [
        (InstallError::ConsentRequired, "stt_consent_required"),
        (InstallError::UrlNotAllowed, "stt_url_not_allowed"),
        (InstallError::Cancelled, "stt_download_cancelled"),
        (InstallError::HashMismatch { file: "tokens.txt" }, "stt_hash_mismatch"),
    ] {
        assert_eq!(err.code(), code);
    }
}
