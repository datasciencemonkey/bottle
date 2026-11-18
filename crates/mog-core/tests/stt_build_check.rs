//! #45 CAP-50a, CAP-50b: the `local-stt` build check in `build_support/sherpa_vendor.rs`, which
//! `build.rs` includes. It refuses `SHERPA_ONNX_LIB_DIR`, a missing or changed vendored archive,
//! and a cached library folder that is not byte-equal to the pinned archive's `lib/` folder.

#[path = "../build_support/sherpa_vendor.rs"]
#[allow(dead_code)]
mod sherpa_vendor;

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};
use sherpa_vendor::{
    archive_name, check_archive, check_env, check_lib_dir, pinned_sha, prebuilt_lib_dir, PINS,
};

fn tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mog-stt-build-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn hex(b: &[u8]) -> String {
    Sha256::digest(b)
        .iter()
        .map(|x| format!("{x:02x}"))
        .collect()
}

#[test]
fn cap_50b_the_build_needs_the_vendored_archive_dir() {
    let err = check_env(None, false).unwrap_err();
    assert!(err.contains("CAP-50b"), "{err}");
    let dir = tmp("env");
    assert_eq!(check_env(Some(dir.as_os_str()), false).unwrap(), dir);
}

#[test]
fn cap_50a_sherpa_onnx_lib_dir_is_refused() {
    // SHERPA_ONNX_LIB_DIR makes sherpa-onnx-sys link any folder with no hash check.
    let dir = tmp("libdir");
    let err = check_env(Some(dir.as_os_str()), true).unwrap_err();
    assert!(err.contains("SHERPA_ONNX_LIB_DIR"), "{err}");
}

#[test]
fn cap_50a_the_archive_must_match_its_pin() {
    let dir = tmp("archive");
    let name = archive_name("macos", "aarch64").unwrap();
    let path = dir.join(name);
    fs::write(&path, b"vendored").unwrap();
    assert_eq!(
        check_archive(&path, &hex(b"vendored")).unwrap(),
        hex(b"vendored")
    );
    let err = check_archive(&path, &hex(b"other")).unwrap_err();
    assert!(err.contains("mismatch"), "{err}");
    assert!(check_archive(&dir.join("missing"), &hex(b"x")).is_err());
}

#[test]
fn cap_50a_every_ci_and_release_target_has_a_pin() {
    for (os, arch) in [
        ("macos", "aarch64"),
        ("macos", "x86_64"),
        ("windows", "x86_64"),
        ("linux", "x86_64"),
    ] {
        let name = archive_name(os, arch).unwrap();
        let sha = pinned_sha(PINS, name).unwrap();
        assert_eq!(sha.len(), 64);
    }
    assert!(archive_name("linux", "riscv64").is_err());
    assert!(pinned_sha(PINS, "not-pinned.tar.bz2").is_err());
    // The Rust table in stt_model.rs and the pin file agree.
    for (name, sha) in mog_core::stt_model::SHERPA_ONNX_ARCHIVE_PINS {
        assert_eq!(pinned_sha(PINS, name).unwrap(), *sha);
    }
}

#[test]
fn cap_50a_a_cached_lib_dir_must_equal_the_pinned_archive() {
    let lib = tmp("lib");
    fs::write(lib.join("libsherpa-onnx-c-api.a"), b"api").unwrap();
    fs::write(lib.join("libonnxruntime.a"), b"ort").unwrap();
    let want: BTreeMap<String, String> = [
        ("libsherpa-onnx-c-api.a".to_string(), hex(b"api")),
        ("libonnxruntime.a".to_string(), hex(b"ort")),
    ]
    .into();
    check_lib_dir(&want, &lib).unwrap();
    // A file that differs (for example an old unchecked download) stops the build.
    fs::write(lib.join("libonnxruntime.a"), b"downloaded").unwrap();
    assert!(check_lib_dir(&want, &lib)
        .unwrap_err()
        .contains("libonnxruntime.a"));
    fs::write(lib.join("libonnxruntime.a"), b"ort").unwrap();
    // An extra library in the folder stops the build too.
    fs::write(lib.join("libextra.a"), b"x").unwrap();
    assert!(check_lib_dir(&want, &lib)
        .unwrap_err()
        .contains("libextra.a"));
    fs::remove_file(lib.join("libextra.a")).unwrap();
    // A missing library stops the build.
    fs::remove_file(lib.join("libonnxruntime.a")).unwrap();
    assert!(check_lib_dir(&want, &lib).is_err());
}

#[test]
fn cap_50a_the_prebuilt_lib_dir_is_the_one_sherpa_onnx_sys_links() {
    let out = PathBuf::from("/w/target/release/build/mog-core-1/out");
    let name = "sherpa-onnx-v2.0.1-osx-arm64-static-lib.tar.bz2";
    assert_eq!(
        prebuilt_lib_dir(&out, None, name),
        PathBuf::from("/w/target/sherpa-onnx-prebuilt/sherpa-onnx-v2.0.1-osx-arm64-static-lib/lib")
    );
    assert_eq!(
        prebuilt_lib_dir(&out, Some(PathBuf::from("/t").as_os_str()), name),
        PathBuf::from("/t/sherpa-onnx-prebuilt/sherpa-onnx-v2.0.1-osx-arm64-static-lib/lib")
    );
}
