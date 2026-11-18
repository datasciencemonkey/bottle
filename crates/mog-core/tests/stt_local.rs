//! #45 with the `local-stt` feature: the archive extract and the sherpa-onnx recognizer
//! (CAP-48, CAP-49c). The recognizer test needs the installed model: set `BOTTLE_STT_MODEL_DIR`
//! to the folder that `ModelInstaller::install` returns (see docs/adr/0049-local-stt-packaging.md).
#![cfg(feature = "local-stt")]

use std::fs;
use std::path::PathBuf;

use mog_core::stt::LocalSttConfig;
use mog_core::stt_local::{extract_model_archive, ParakeetRecognizer, MODEL, RUNTIME};
use mog_core::stt_model::{sha256_hex, ModelSpec, PinnedFile};

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mog-stt-local-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn tar_bz2(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let enc = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    let mut b = tar::Builder::new(enc);
    // A symlink named like a model file comes first; it must not be followed or written.
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    b.append_link(&mut link, "m/tokens.txt", "/etc/passwd")
        .unwrap();
    for (path, body) in entries {
        let mut h = tar::Header::new_gnu();
        h.set_size(body.len() as u64);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, path, *body).unwrap();
    }
    b.into_inner().unwrap().finish().unwrap()
}

#[test]
fn cap_49c_extract_writes_only_the_pinned_model_files() {
    let body = b"tokens";
    let spec = ModelSpec {
        name: "m",
        archive: PinnedFile {
            name: "m.tar.bz2",
            size: 0,
            sha256: "",
        },
        files: vec![PinnedFile {
            name: "tokens.txt",
            size: body.len() as u64,
            sha256: Box::leak(sha256_hex(body).into_boxed_str()),
        }],
    };
    let dir = tmp("extract");
    let archive = dir.join("m.tar.bz2");
    fs::write(
        &archive,
        tar_bz2(&[
            ("m/tokens.txt", body),
            ("m/README.md", b"skip"),
            ("other/tokens.txt", b"wrong top folder"),
            ("m/sub/tokens.txt", b"too deep"),
            (
                "./m/joiner.int8.onnx",
                b"dot prefix, not a pinned name here",
            ),
        ]),
    )
    .unwrap();
    let dest = dir.join("out");
    fs::create_dir_all(&dest).unwrap();
    extract_model_archive(&spec, &archive, &dest).unwrap();
    let names: Vec<_> = fs::read_dir(&dest)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["tokens.txt".to_string()]);
    assert_eq!(fs::read(dest.join("tokens.txt")).unwrap(), body);
    assert!(!fs::symlink_metadata(dest.join("tokens.txt"))
        .unwrap()
        .file_type()
        .is_symlink());
}

#[test]
fn cap_48_recognizer_runs_on_the_cpu_ep_with_p_core_threads() {
    let Some(dir) = std::env::var_os("BOTTLE_STT_MODEL_DIR") else {
        // TODO(#414): this return exists because the 487 MB model is never downloaded in a test.
        // The #414 `local-stt` CI job sets BOTTLE_STT_MODEL_DIR to an installed model and fails
        // if this note prints, so the recognizer part runs there.
        eprintln!("BOTTLE_STT_MODEL_DIR is not set: the model part of this test did not run");
        return;
    };
    let cfg = LocalSttConfig::for_this_device();
    let rec = ParakeetRecognizer::load(PathBuf::from(dir).as_path(), cfg.clone()).unwrap();
    assert_eq!(rec.config().provider, "cpu");
    assert_eq!(rec.config().num_threads, mog_core::stt::perf_core_count());
    // One second of silence gives no words.
    assert_eq!(rec.transcribe(&vec![0.0; 16_000]), "");
    assert!(MODEL.contains("parakeet"));
    assert!(RUNTIME.starts_with("sherpa-onnx 2.0."));
}

#[test]
fn cap_51c_cap_52_cap_72_engine_decodes_with_the_real_model() {
    use mog_core::capture::StopReason;
    use mog_core::stt_engine::{EngineParts, LocalSttEngine, PowerSource, SttDone, TranscriptSink};
    use mog_core::stt_local::ParakeetLoader;
    use mog_core::types::{CaptureMode, TranscriptProvider};
    use std::sync::{Arc, Mutex};
    let Some(dir) = std::env::var_os("BOTTLE_STT_MODEL_DIR") else {
        // TODO(#414): the #414 `local-stt` CI job sets BOTTLE_STT_MODEL_DIR and fails if this
        // note prints, so the real-model part of #46 runs there.
        eprintln!("BOTTLE_STT_MODEL_DIR is not set: the #46 real-model engine test did not run");
        return;
    };
    #[derive(Default)]
    struct Out(Mutex<Vec<SttDone>>);
    impl TranscriptSink for Out {
        fn done(&self, d: SttDone) {
            self.0.lock().unwrap().push(d);
        }
    }
    struct Battery;
    impl mog_core::stt_engine::PowerProbe for Battery {
        fn source(&self) -> PowerSource {
            PowerSource::Battery
        }
    }
    let out = Arc::new(Out::default());
    let mut parts = EngineParts::new(
        Arc::new(ParakeetLoader::new(PathBuf::from(dir))),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    parts.power = Arc::new(Battery);
    let r4 = LocalSttEngine::spawn(parts);
    r4.preload();
    r4.sync();
    // A loud failure in the #414 job: the variable is set, so the model must load.
    assert!(
        r4.status().loaded,
        "the model at BOTTLE_STT_MODEL_DIR did not load: {:?}",
        r4.status()
    );
    r4.begin(1, "mog-a", CaptureMode::Ptt);
    r4.frames(&vec![0.0; 48_000], 48_000);
    r4.commit(1, StopReason::Released);
    r4.sync();
    let done = out.0.lock().unwrap().clone();
    assert_eq!(done.len(), 1);
    let t = done[0].result.clone().expect("a transcript");
    assert_eq!(t.provider, TranscriptProvider::LocalParakeet);
    assert_eq!(t.model, MODEL);
    assert_eq!(t.runtime, RUNTIME);
}
