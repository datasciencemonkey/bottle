//! #381 NFR-13a: on Windows every executable of this crate (the app and each test executable)
//! carries the Common Controls v6 application manifest. `tauri` imports `TaskDialogIndirect`
//! from comctl32 v6; without the manifest the loader picks comctl32 v5 and the executable does
//! not start (STATUS_ENTRYPOINT_NOT_FOUND, 0xc0000139). See ADR 0036.

use std::path::PathBuf;

fn app_file(rel: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)).expect(rel)
}

const COMMON_CONTROLS: &str = r#"name="Microsoft.Windows.Common-Controls""#;

#[test]
fn nfr_13a_windows_manifest_declares_common_controls_v6() {
    let manifest = app_file("windows-app-manifest.xml");
    assert!(manifest.contains(COMMON_CONTROLS), "{manifest}");
    assert!(manifest.contains(r#"version="6.0.0.0""#), "{manifest}");
    assert!(
        manifest.contains(r#"publicKeyToken="6595b64144ccf1df""#),
        "{manifest}"
    );
}

#[test]
fn nfr_13a_build_script_embeds_one_manifest_in_every_windows_executable() {
    let build = app_file("build.rs");
    // The linker embeds the manifest in every linked target (bin, lib tests, integration tests).
    assert!(
        build.contains("cargo:rustc-link-arg=/MANIFEST:EMBED"),
        "{build}"
    );
    assert!(
        build.contains("cargo:rustc-link-arg=/MANIFESTINPUT:"),
        "{build}"
    );
    assert!(build.contains("windows-app-manifest.xml"), "{build}");
    // tauri-build must not add a second manifest resource to the app binary.
    assert!(build.contains("new_without_app_manifest()"), "{build}");
    // Linker warnings (for example a manifest conflict) fail the build.
    assert!(build.contains("cargo:rustc-link-arg=/WX"), "{build}");
}

#[test]
fn nfr_13a_build_script_ignores_only_the_missing_vendored_pdb_warning() {
    // The vendored OpenSSL of SQLCipher (#35, SP-21) is a static library whose objects point at
    // an `ossl_static.pdb` that openssl-src does not ship, so link.exe gives LNK4099 for each one.
    // /WX must stay on for every other warning, including a manifest conflict.
    let build = app_file("build.rs");
    assert!(build.contains("cargo:rustc-link-arg=/WX"), "{build}");
    assert!(
        build.contains("cargo:rustc-link-arg=/IGNORE:4099"),
        "{build}"
    );
    let ignores: Vec<&str> = build
        .match_indices("/IGNORE:")
        .map(|(i, _)| &build[i..build[i..].find('"').map_or(build.len(), |e| i + e)])
        .collect();
    assert_eq!(ignores, vec!["/IGNORE:4099"], "only LNK4099 is ignored");
}

/// Reads the manifest resource (ID 1, type RT_MANIFEST = 24) of the running test executable.
#[cfg(windows)]
#[test]
fn nfr_13a_windows_test_executable_carries_the_common_controls_manifest() {
    use windows_sys::Win32::System::LibraryLoader::{
        FindResourceW, LoadResource, LockResource, SizeofResource,
    };
    const CREATEPROCESS_MANIFEST_RESOURCE_ID: usize = 1;
    const RT_MANIFEST: usize = 24;
    // SAFETY: a null module handle means this executable; the IDs are integer resource names.
    let text = unsafe {
        let module = std::ptr::null_mut();
        let res = FindResourceW(
            module,
            CREATEPROCESS_MANIFEST_RESOURCE_ID as *const u16,
            RT_MANIFEST as *const u16,
        );
        assert!(
            !res.is_null(),
            "the test executable has no manifest resource"
        );
        let data = LockResource(LoadResource(module, res)) as *const u8;
        let len = SizeofResource(module, res) as usize;
        assert!(!data.is_null() && len > 0);
        String::from_utf8_lossy(std::slice::from_raw_parts(data, len)).into_owned()
    };
    assert!(text.contains(COMMON_CONTROLS), "{text}");
    assert!(text.contains(r#"version="6.0.0.0""#), "{text}");
}
