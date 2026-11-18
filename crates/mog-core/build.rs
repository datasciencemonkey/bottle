//! CAP-50a, CAP-50b (#45, ADR 0049): with the `local-stt` feature, the build must link only the
//! vendored, hash-pinned sherpa-onnx archive and must not download libraries.
//!
//! - `.cargo/config.toml` sets `SHERPA_ONNX_ARCHIVE_DIR` to `vendor/sherpa-onnx`, so
//!   `sherpa-onnx-sys` never takes its download branch (with no archive there, it fails).
//! - `sherpa-onnx-sys` is a direct dependency with `links`, so its build script runs before this one.
//! - This script refuses `SHERPA_ONNX_LIB_DIR`, checks the archive against
//!   `sherpa-onnx-archives.sha256`, and checks that the library folder `sherpa-onnx-sys` links is
//!   byte-equal to the archive's `lib/` folder. It prints the result in the build log.
//!
//! Without `local-stt` it does nothing.
//!
//! TODO(#414): the no-download guarantee rests on the `.cargo/config.toml` `[env]` line, because
//! `sherpa-onnx-sys` runs first (`scripts/tests/test_vendor_sherpa_onnx.py` pins that line). The
//! #414 CI job proves it on Windows and Linux: a clean absolute CARGO_TARGET_DIR, the network
//! blocked at the OS level, and the full `linked lib dir ... matches it` log line.

#[path = "build_support/sherpa_vendor.rs"]
#[allow(dead_code)]
mod sherpa_vendor;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_support/sherpa_vendor.rs");
    println!("cargo:rerun-if-changed=sherpa-onnx-archives.sha256");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_ARCHIVE_DIR");
    println!("cargo:rerun-if-env-changed=SHERPA_ONNX_LIB_DIR");
    #[cfg(feature = "local-stt")]
    if let Err(e) = local_stt::check() {
        panic!("{e}");
    }
}

#[cfg(feature = "local-stt")]
mod local_stt {
    use std::collections::BTreeMap;
    use std::env;
    use std::fs::File;
    use std::io;
    use std::path::{Component, PathBuf};

    use sha2::{Digest, Sha256};

    use super::sherpa_vendor::{
        archive_name, check_archive, check_env, check_lib_dir, pinned_sha, prebuilt_lib_dir, PINS,
    };

    pub fn check() -> Result<(), String> {
        let os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
        let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
        let name = archive_name(&os, &arch)?;
        let want = pinned_sha(PINS, name)?;
        let dir = check_env(
            env::var_os("SHERPA_ONNX_ARCHIVE_DIR").as_deref(),
            env::var_os("SHERPA_ONNX_LIB_DIR").is_some(),
        )?;
        let archive = dir.join(name);
        let got = check_archive(&archive, &want)?;
        println!("cargo:rerun-if-changed={}", archive.display());

        // The `lib/` files of the pinned archive, by name and SHA-256.
        let stem = name.strip_suffix(".tar.bz2").unwrap_or(name);
        let file = File::open(&archive).map_err(|e| format!("CAP-50a: {e}"))?;
        let mut tar = tar::Archive::new(bzip2::read::BzDecoder::new(file));
        let mut libs = BTreeMap::new();
        for entry in tar.entries().map_err(|e| format!("CAP-50a: {e}"))? {
            let mut entry = entry.map_err(|e| format!("CAP-50a: {e}"))?;
            let path: PathBuf = entry.path().map_err(|e| format!("CAP-50a: {e}"))?.into();
            let parts: Vec<_> = path
                .components()
                .filter(|c| !matches!(c, Component::CurDir))
                .collect();
            if let [top, Component::Normal(lib), Component::Normal(file)] = parts.as_slice() {
                if top.as_os_str() == stem && *lib == "lib" && entry.header().entry_type().is_file()
                {
                    let mut hasher = Sha256::new();
                    io::copy(&mut entry, &mut hasher).map_err(|e| format!("CAP-50a: {e}"))?;
                    let sha: String = hasher
                        .finalize()
                        .iter()
                        .map(|b| format!("{b:02x}"))
                        .collect();
                    libs.insert(file.to_string_lossy().into_owned(), sha);
                }
            }
        }
        if libs.is_empty() {
            return Err(format!("CAP-50a: {name} has no {stem}/lib files"));
        }
        let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or("CAP-50a: OUT_DIR is not set")?);
        let lib_dir = prebuilt_lib_dir(&out_dir, env::var_os("CARGO_TARGET_DIR").as_deref(), name);
        check_lib_dir(&libs, &lib_dir)?;
        println!("cargo:rerun-if-changed={}", lib_dir.display());
        println!(
            "cargo:warning=CAP-50a vendored sherpa-onnx archive {name} sha256 {got} OK; linked lib dir \
             {} matches it ({} files)",
            lib_dir.display(),
            libs.len()
        );
        Ok(())
    }
}
