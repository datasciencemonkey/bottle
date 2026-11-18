//! CAP-50a, CAP-50b (#45, ADR 0049): the `local-stt` build check. `build.rs` includes this file,
//! and `tests/stt_build_check.rs` tests it. It has no dependency except `sha2`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// The pin file: `<sha256>  <archive name>` lines.
pub const PINS: &str = include_str!("../sherpa-onnx-archives.sha256");

/// The vendored archive that `sherpa-onnx-sys` 2.0.1 (feature `static`) links for a target.
pub fn archive_name(os: &str, arch: &str) -> Result<&'static str, String> {
    match (os, arch) {
        ("macos", "aarch64") => Ok("sherpa-onnx-v2.0.1-osx-arm64-static-lib.tar.bz2"),
        ("macos", "x86_64") => Ok("sherpa-onnx-v2.0.1-osx-x64-static-lib.tar.bz2"),
        ("windows", "x86_64") => Ok("sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2"),
        ("linux", "x86_64") => Ok("sherpa-onnx-v2.0.1-linux-x64-static-lib.tar.bz2"),
        _ => Err(format!(
            "CAP-50a: no pinned sherpa-onnx archive for {os}/{arch}"
        )),
    }
}

/// The pinned SHA-256 of `name` in `pins`.
pub fn pinned_sha(pins: &str, name: &str) -> Result<String, String> {
    pins.lines()
        .filter_map(|l| l.split_once("  "))
        .find(|(_, n)| n.trim() == name)
        .map(|(h, _)| h.trim().to_string())
        .ok_or_else(|| format!("CAP-50a: {name} is not in sherpa-onnx-archives.sha256"))
}

/// The vendored archive folder. Refuses `SHERPA_ONNX_LIB_DIR` (it makes `sherpa-onnx-sys` link
/// any folder, with no hash check) and a missing `SHERPA_ONNX_ARCHIVE_DIR` (it makes
/// `sherpa-onnx-sys` download the archive).
pub fn check_env(archive_dir: Option<&OsStr>, lib_dir_set: bool) -> Result<PathBuf, String> {
    if lib_dir_set {
        return Err(
            "CAP-50a: SHERPA_ONNX_LIB_DIR is set; the local-stt build links only the \
                    vendored, hash-pinned archive. Unset it."
                .into(),
        );
    }
    archive_dir.map(PathBuf::from).ok_or_else(|| {
        "CAP-50b: the local-stt build needs SHERPA_ONNX_ARCHIVE_DIR (the vendored, hash-pinned \
         sherpa-onnx archives; .cargo/config.toml points it at vendor/sherpa-onnx). Run `uv run \
         --no-project python scripts/vendor_sherpa_onnx.py` once. See \
         docs/adr/0049-local-stt-packaging.md."
            .into()
    })
}

/// The SHA-256 of a file, lower-case hex.
pub fn file_sha(path: &Path) -> Result<String, String> {
    let mut file =
        File::open(path).map_err(|e| format!("CAP-50a: cannot open {}: {e}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("CAP-50a: cannot read {}: {e}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Check the vendored archive against its pin. Returns its SHA-256.
pub fn check_archive(path: &Path, want: &str) -> Result<String, String> {
    let got = file_sha(path)?;
    if got != want {
        return Err(format!(
            "CAP-50a: SHA-256 mismatch for {}: got {got}, want {want}",
            path.display()
        ));
    }
    Ok(got)
}

/// The library folder that `sherpa-onnx-sys` 2.0.1 links:
/// `<target dir>/sherpa-onnx-prebuilt/<archive stem>/lib` (its `target_dir_from_out_dir`).
pub fn prebuilt_lib_dir(
    out_dir: &Path,
    cargo_target_dir: Option<&OsStr>,
    archive: &str,
) -> PathBuf {
    let target = match cargo_target_dir {
        Some(t) => PathBuf::from(t),
        None => out_dir
            .ancestors()
            .find(|p| p.file_name() == Some(OsStr::new("target")))
            .unwrap_or(out_dir)
            .to_path_buf(),
    };
    let stem = archive.strip_suffix(".tar.bz2").unwrap_or(archive);
    target.join("sherpa-onnx-prebuilt").join(stem).join("lib")
}

/// Check that `lib` holds exactly the files of `want` (name to SHA-256): the `lib/` files of the
/// pinned archive. A cached folder from an earlier, unchecked download fails here.
pub fn check_lib_dir(want: &BTreeMap<String, String>, lib: &Path) -> Result<(), String> {
    let entries = fs::read_dir(lib).map_err(|e| format!("CAP-50a: {}: {e}", lib.display()))?;
    let mut seen = 0usize;
    for entry in entries {
        let entry = entry.map_err(|e| format!("CAP-50a: {}: {e}", lib.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(sha) = want.get(&name) else {
            return Err(format!(
                "CAP-50a: {} is not in the pinned archive; remove {} and rebuild",
                name,
                lib.display()
            ));
        };
        if &file_sha(&entry.path())? != sha {
            return Err(format!(
                "CAP-50a: {name} in {} differs from the pinned archive; remove the folder and rebuild",
                lib.display()
            ));
        }
        seen += 1;
    }
    if seen != want.len() {
        return Err(format!(
            "CAP-50a: {} lacks files of the pinned archive; remove it and rebuild",
            lib.display()
        ));
    }
    Ok(())
}
