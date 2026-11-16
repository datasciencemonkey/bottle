//! ABI contract test from a real C host (NFR-18a, NFR-18c, AT-17).
//!
//! Compiles `tests/c/smoke.c` against `include/mog_core.h` with the system C compiler, links
//! the cdylib that cargo built for this test run, and runs it. Unix only: the Windows host
//! test comes with the CI matrix (#227).
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Build the cdylib and return the directory that holds it. Cargo reports the artifact path,
/// so the test does not depend on the target directory layout.
fn cdylib_dir() -> PathBuf {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = Command::new(cargo)
        .args([
            "build",
            "-p",
            "mog-core-ffi",
            "--lib",
            "--message-format=json",
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("run cargo build");
    assert!(
        out.status.success(),
        "cargo build failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let suffix = if cfg!(target_os = "macos") {
        "libmog_core_ffi.dylib"
    } else {
        "libmog_core_ffi.so"
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|msg| msg["reason"] == "compiler-artifact")
        .flat_map(|msg| msg["filenames"].as_array().cloned().unwrap_or_default())
        .filter_map(|f| f.as_str().map(PathBuf::from))
        .find(|f| f.ends_with(suffix))
        .and_then(|f| f.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| panic!("cargo did not report {suffix}"))
}

#[test]
fn nfr_18a_c_host_calls_the_abi_through_the_header() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib_dir = cdylib_dir();
    let exe = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mog_c_smoke");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_owned());
    let status = Command::new(&cc)
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(crate_dir.join("include"))
        .arg(crate_dir.join("tests/c/smoke.c"))
        .arg("-L")
        .arg(&lib_dir)
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()))
        .arg("-lmog_core_ffi")
        .arg("-o")
        .arg(&exe)
        .status()
        .unwrap_or_else(|e| panic!("cannot run the C compiler `{cc}`: {e}"));
    assert!(status.success(), "the C smoke host does not compile");
    let out = Command::new(&exe)
        .env("LD_LIBRARY_PATH", &lib_dir)
        .env("DYLD_LIBRARY_PATH", &lib_dir)
        .output()
        .expect("run the C smoke host");
    assert!(
        out.status.success(),
        "the C smoke host failed at step {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
}
