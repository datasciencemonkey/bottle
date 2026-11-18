//! #413 the wizard STT step backend (MB-07): the W-17e models folder, the stored download
//! consent that the CAP-49a gate reads, and the install service with cancel and resume
//! (CAP-49b) and the hash stop (CAP-49c). The network is a test double: no test opens a socket.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use mog_core::stt_model::{
    sha256_hex, ConsentGate, FetchError, ModelConsent, ModelSource, ModelSpec, ModelStatus,
    PinnedFile, RangeFetch,
};
use mog_core::stt_setup::{
    models_dir, Extractor, FileConsent, SetupError, SttSetup, TargetOs, CONSENT_FILE,
};

const HF: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/";

/// A network double: fixed bodies by URL, a log of every request. `Send + Sync`, like the
/// real `HttpRangeFetch`.
#[derive(Default)]
struct FakeNet {
    bodies: BTreeMap<String, Vec<u8>>,
    calls: Mutex<Vec<(String, u64)>>,
}

impl FakeNet {
    fn calls(&self) -> Vec<(String, u64)> {
        self.calls.lock().unwrap().clone()
    }
}

impl RangeFetch for FakeNet {
    fn open(&self, url: &str, offset: u64) -> Result<(u64, Box<dyn Read + '_>), FetchError> {
        self.calls.lock().unwrap().push((url.to_string(), offset));
        let body = self.bodies.get(url).ok_or(FetchError::Status(404))?;
        Ok((
            offset,
            Box::new(io::Cursor::new(body[offset as usize..].to_vec())),
        ))
    }
}

fn pinned(name: &'static str, body: &[u8]) -> PinnedFile {
    PinnedFile {
        name,
        size: body.len() as u64,
        sha256: Box::leak(sha256_hex(body).into_boxed_str()),
    }
}

/// The real file names with fake bodies. The encoder is larger than one 64 KiB chunk, so a
/// cancel after the first chunk leaves a partial file.
fn fake_spec() -> (ModelSpec, BTreeMap<&'static str, Vec<u8>>) {
    let mut bodies = BTreeMap::new();
    bodies.insert("encoder.int8.onnx", b"enc-".repeat(60_000));
    bodies.insert("decoder.int8.onnx", b"dec-".repeat(100));
    bodies.insert("joiner.int8.onnx", b"joi-".repeat(100));
    bodies.insert("tokens.txt", b"tok-".repeat(100));
    let files = bodies.iter().map(|(n, b)| pinned(n, b)).collect();
    let spec = ModelSpec {
        name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
        archive: pinned(
            "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
            &b"fake archive".repeat(10),
        ),
        files,
    };
    (spec, bodies)
}

fn hf_net(bodies: &BTreeMap<&'static str, Vec<u8>>) -> Arc<FakeNet> {
    let mut net = FakeNet::default();
    for (name, body) in bodies {
        net.bodies.insert(format!("{HF}{name}"), body.clone());
    }
    Arc::new(net)
}

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mog-stt-setup-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn no_extract() -> Arc<Extractor> {
    Arc::new(|_, _| panic!("the HF source never extracts"))
}

fn setup(root: &Path, net: Arc<FakeNet>) -> SttSetup {
    let (spec, _) = fake_spec();
    SttSetup::new(root, spec, net, no_extract())
}

fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<OsString> {
    move |k| {
        pairs
            .iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| OsString::from(v))
    }
}

// ---------- W-17e: the user-only models folder ----------

#[test]
fn w_17e_macos_models_dir_is_under_application_support() {
    let dir = models_dir(TargetOs::MacOs, &env(&[("HOME", "/Users/u")]));
    assert_eq!(
        dir,
        Some(PathBuf::from(
            "/Users/u/Library/Application Support/Bottle/models"
        ))
    );
}

#[test]
fn w_17e_windows_models_dir_is_under_local_app_data() {
    const LOCAL: &str = r"C:\Users\u\AppData\Local";
    let dir = models_dir(TargetOs::Windows, &env(&[("LOCALAPPDATA", LOCAL)]));
    assert_eq!(
        dir,
        Some(PathBuf::from(LOCAL).join("Bottle").join("models"))
    );
    // Not the roaming folder and not the home folder.
    let roaming_only = models_dir(
        TargetOs::Windows,
        &env(&[
            ("APPDATA", r"C:\Users\u\AppData\Roaming"),
            ("USERPROFILE", r"C:\Users\u"),
        ]),
    );
    assert_eq!(roaming_only, None);
}

#[test]
fn w_17e_a_missing_or_relative_base_gives_no_models_dir() {
    assert_eq!(models_dir(TargetOs::MacOs, &env(&[])), None);
    assert_eq!(models_dir(TargetOs::MacOs, &env(&[("HOME", "")])), None);
    assert_eq!(
        models_dir(TargetOs::MacOs, &env(&[("HOME", "relative/home")])),
        None
    );
    assert_eq!(
        models_dir(
            TargetOs::Windows,
            &env(&[("LOCALAPPDATA", r"AppData\Local")])
        ),
        None
    );
}

// ---------- CAP-49a: the stored answer and the gate ----------

#[test]
fn cap_49a_a_new_install_has_no_answer_yet() {
    let root = tmp("not-asked");
    let consent = FileConsent::new(&root);
    assert_eq!(consent.model_download_consent(), ModelConsent::NotAsked);
    assert_eq!(
        setup(&root, Arc::new(FakeNet::default())).status().consent,
        ModelConsent::NotAsked
    );
}

#[test]
fn cap_49a_the_stored_answer_lasts_across_launches() {
    let root = tmp("stored");
    let net = Arc::new(FakeNet::default());
    setup(&root, net.clone())
        .set_consent(ModelConsent::Granted)
        .unwrap();
    // A new process reads the same answer from the models folder.
    assert_eq!(
        FileConsent::new(&root).model_download_consent(),
        ModelConsent::Granted
    );
    assert_eq!(
        setup(&root, net.clone()).status().consent,
        ModelConsent::Granted
    );
    setup(&root, net.clone())
        .set_consent(ModelConsent::Declined)
        .unwrap();
    assert_eq!(setup(&root, net).status().consent, ModelConsent::Declined);
    // The file holds one word only: no path, no time, no user data.
    let text = fs::read_to_string(root.join(CONSENT_FILE)).unwrap();
    assert_eq!(text.trim(), "declined");
}

#[test]
fn cap_49a_an_unknown_stored_value_counts_as_not_asked() {
    let root = tmp("garbage");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(CONSENT_FILE), "yes please").unwrap();
    assert_eq!(
        FileConsent::new(&root).model_download_consent(),
        ModelConsent::NotAsked
    );
}

#[test]
fn cap_49a_install_sends_no_request_and_writes_no_model_file_before_granted() {
    let (_, bodies) = fake_spec();
    for answer in [None, Some(ModelConsent::Declined)] {
        let root = tmp(&format!("gate-{answer:?}"));
        let net = hf_net(&bodies);
        let s = setup(&root, net.clone());
        if let Some(a) = answer {
            s.set_consent(a).unwrap();
        }
        let err = s
            .install(ModelSource::HuggingFaceMirror, &mut |_| {})
            .unwrap_err();
        assert_eq!(err.code(), "stt_consent_required");
        assert!(net.calls().is_empty(), "no request before consent");
        assert!(!s.model_dir().exists(), "no model file before consent");
        assert_eq!(s.status().model, ModelStatus::Missing);
    }
}

#[test]
fn cap_49a_mb_07_install_runs_after_granted_and_status_shows_installed() {
    let (_, bodies) = fake_spec();
    let root = tmp("granted");
    let net = hf_net(&bodies);
    let s = setup(&root, net.clone());
    assert_eq!(s.status().model, ModelStatus::Missing);
    s.set_consent(ModelConsent::Granted).unwrap();
    let mut seen = Vec::new();
    let dir = s
        .install(ModelSource::HuggingFaceMirror, &mut |p| seen.push(p.file))
        .unwrap();
    assert_eq!(dir, s.model_dir());
    assert_eq!(net.calls().len(), 4);
    assert!(seen.contains(&"encoder.int8.onnx"));
    let st = s.status();
    assert_eq!(st.model, ModelStatus::Installed);
    assert!(!st.installing);
    assert_eq!(st.total_bytes, fake_spec().0.archive.size);
    // A second install (the wizard's check before it marks the step done) needs no request.
    s.install(ModelSource::HuggingFaceMirror, &mut |_| {})
        .unwrap();
    assert_eq!(net.calls().len(), 4);
}

// ---------- CAP-49b: cancel, partial status and resume ----------

#[test]
fn cap_49b_cancel_keeps_the_partial_file_and_a_retry_resumes() {
    let (_, bodies) = fake_spec();
    let root = tmp("cancel");
    let net = hf_net(&bodies);
    let s = setup(&root, net.clone());
    s.set_consent(ModelConsent::Granted).unwrap();
    let mut chunks = 0;
    let err = s
        .install(ModelSource::HuggingFaceMirror, &mut |p| {
            if p.file == "encoder.int8.onnx" {
                chunks += 1;
                assert!(s.status().installing);
                assert!(s.cancel(), "cancel reports that an install runs");
            }
        })
        .unwrap_err();
    assert_eq!(err.code(), "stt_download_cancelled");
    assert_eq!(chunks, 1, "the install stops at the next chunk");
    let st = s.status();
    assert!(!st.installing);
    let ModelStatus::Partial { bytes } = st.model else {
        panic!("want Partial, got {:?}", st.model);
    };
    assert!(bytes > 0);
    assert!(!s.cancel(), "no install runs now");

    // Retry: the encoder request starts at the kept bytes.
    s.install(ModelSource::HuggingFaceMirror, &mut |_| {})
        .unwrap();
    let enc = format!("{HF}encoder.int8.onnx");
    let offsets: Vec<u64> = net
        .calls()
        .into_iter()
        .filter(|(u, _)| *u == enc)
        .map(|(_, o)| o)
        .collect();
    assert_eq!(offsets, vec![0, bytes]);
    assert_eq!(s.status().model, ModelStatus::Installed);
}

#[test]
fn cap_49b_a_second_install_while_one_runs_is_refused() {
    let (_, bodies) = fake_spec();
    let root = tmp("busy");
    let s = setup(&root, hf_net(&bodies));
    s.set_consent(ModelConsent::Granted).unwrap();
    let mut inner = None;
    s.install(ModelSource::HuggingFaceMirror, &mut |_| {
        if inner.is_none() {
            inner = Some(s.install(ModelSource::HuggingFaceMirror, &mut |_| {}));
        }
    })
    .unwrap();
    let inner = inner.unwrap().unwrap_err();
    assert_eq!(inner, SetupError::Busy);
    assert_eq!(inner.code(), "stt_install_busy");
}

// ---------- CAP-49c: a changed hash stops the install ----------

#[test]
fn cap_49c_a_changed_file_stops_the_install_and_is_not_kept() {
    let (_, bodies) = fake_spec();
    let root = tmp("hash");
    let mut changed = bodies.clone();
    let tok = changed.get_mut("tokens.txt").unwrap();
    tok[0] ^= 0xff; // same size, other bytes
    let net = hf_net(&changed);
    let s = setup(&root, net);
    s.set_consent(ModelConsent::Granted).unwrap();
    let err = s
        .install(ModelSource::HuggingFaceMirror, &mut |_| {})
        .unwrap_err();
    assert_eq!(err.code(), "stt_hash_mismatch");
    assert_ne!(s.status().model, ModelStatus::Installed);
    assert!(!root.join("tokens.txt.partial").exists());
    assert!(!s.model_dir().join("tokens.txt").exists());
}

#[test]
fn cap_49c_every_setup_error_has_a_content_free_code() {
    use mog_core::stt_model::InstallError;
    let all = [
        SetupError::Install(InstallError::ConsentRequired),
        SetupError::Install(InstallError::UrlNotAllowed),
        SetupError::Install(InstallError::Download),
        SetupError::Install(InstallError::HashMismatch { file: "tokens.txt" }),
        SetupError::Install(InstallError::Extract),
        SetupError::Install(InstallError::Io),
        SetupError::Install(InstallError::Cancelled),
        SetupError::Busy,
        SetupError::ModelsDirUnavailable,
    ];
    let codes: Vec<&str> = all.iter().map(SetupError::code).collect();
    assert_eq!(
        codes,
        [
            "stt_consent_required",
            "stt_url_not_allowed",
            "stt_download_failed",
            "stt_hash_mismatch",
            "stt_extract_failed",
            "stt_io_failed",
            "stt_download_cancelled",
            "stt_install_busy",
            "stt_models_dir_unavailable",
        ]
    );
    for (e, c) in all.iter().zip(codes) {
        // The file name of a hash mismatch does not leak into the text.
        assert_eq!(e.to_string(), c);
    }
}

// ---------- MB-07: the app's R4 loader reads the W-17e folder ----------

#[test]
fn mb_07_app_loader_reports_model_missing_without_an_installed_model() {
    use mog_core::stt_engine::RecognizerLoader;
    use mog_core::stt_setup::AppModelLoader;
    let (spec, _) = fake_spec();
    let root = tmp("loader-empty");
    for loader in [
        AppModelLoader::new(Some(root.clone()), spec.clone()),
        AppModelLoader::new(None, spec),
    ] {
        assert_eq!(loader.installed_model_dir(), None);
        assert_eq!(loader.load().err(), Some("stt_model_missing"));
    }
}

#[test]
fn mb_07_app_loader_finds_the_model_that_the_wizard_installed() {
    use mog_core::stt_setup::AppModelLoader;
    let (spec, bodies) = fake_spec();
    let root = tmp("loader-installed");
    let s = setup(&root, hf_net(&bodies));
    let loader = AppModelLoader::new(Some(root.clone()), spec);
    assert_eq!(loader.installed_model_dir(), None);
    // The loader checks at each load, so an install during the session needs no restart.
    s.set_consent(ModelConsent::Granted).unwrap();
    s.install(ModelSource::HuggingFaceMirror, &mut |_| {})
        .unwrap();
    assert_eq!(loader.installed_model_dir(), Some(s.model_dir()));
}
