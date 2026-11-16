//! CAP-50a, CAP-50b (#45): with the `local-stt` feature, the build must use the vendored,
//! hash-pinned sherpa-onnx archive and must not download libraries.
//!
//! `sherpa-onnx-sys` reads `SHERPA_ONNX_ARCHIVE_DIR` and copies the archive from there; without it,
//! it downloads the archive with no hash check. This script makes that path fail: it requires
//! `SHERPA_ONNX_ARCHIVE_DIR`, checks the archive for the target against
//! `sherpa-onnx-archives.sha256` and prints the result in the build log. Without `local-stt` it
//! does nothing.

use std::env;
use std::fs::File;
use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=sherpa-onnx-archives.sha256");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_ARCHIVE_DIR");
    if env::var_os("CARGO_FEATURE_LOCAL_STT").is_none() {
        return;
    }
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let name = match (os.as_str(), arch.as_str()) {
        ("macos", "aarch64") => "sherpa-onnx-v2.0.1-osx-arm64-static-lib.tar.bz2",
        ("macos", "x86_64") => "sherpa-onnx-v2.0.1-osx-x64-static-lib.tar.bz2",
        ("windows", "x86_64") => "sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2",
        ("linux", "x86_64") => "sherpa-onnx-v2.0.1-linux-x64-static-lib.tar.bz2",
        _ => panic!("CAP-50a: no pinned sherpa-onnx archive for {os}/{arch}"),
    };
    let pins = include_str!("sherpa-onnx-archives.sha256");
    let want = pins
        .lines()
        .filter_map(|l| l.split_once("  "))
        .find(|(_, n)| n.trim() == name)
        .map(|(h, _)| h.trim())
        .unwrap_or_else(|| panic!("CAP-50a: {name} is not in sherpa-onnx-archives.sha256"));
    let Some(dir) = env::var_os("SHERPA_ONNX_ARCHIVE_DIR") else {
        panic!(
            "CAP-50b: the local-stt build needs SHERPA_ONNX_ARCHIVE_DIR (the vendored, hash-pinned \
             sherpa-onnx archives). Run `uv run --no-project python scripts/vendor_sherpa_onnx.py \
             --dest <dir>` once, then set SHERPA_ONNX_ARCHIVE_DIR=<dir>. See docs/adr/0049-local-stt-packaging.md."
        )
    };
    let path = Path::new(&dir).join(name);
    let mut file = File::open(&path)
        .unwrap_or_else(|e| panic!("CAP-50a: cannot open {}: {e}", path.display()));
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).expect("CAP-50a: read the archive");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        panic!("CAP-50a: SHA-256 mismatch for {name}: got {got}, want {want}");
    }
    println!("cargo:rerun-if-changed={}", path.display());
    println!("cargo:warning=CAP-50a vendored sherpa-onnx archive {name} sha256 {got} OK");
}
