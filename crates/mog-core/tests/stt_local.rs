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
}

#[test]
fn cap_48_recognizer_runs_on_the_cpu_ep_with_p_core_threads() {
    let Some(dir) = std::env::var_os("BOTTLE_STT_MODEL_DIR") else {
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
