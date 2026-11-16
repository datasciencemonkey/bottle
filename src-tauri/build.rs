fn main() {
    // ARC-4: every IPC command is ACL-gated. One source: mog_core::ipc::COMMANDS.
    let commands: Vec<&'static str> = mog_core::ipc::COMMANDS.iter().map(|c| c.name).collect();
    let commands: &'static [&'static str] = Box::leak(commands.into_boxed_slice());
    let mut attributes = tauri_build::Attributes::new()
        .app_manifest(tauri_build::AppManifest::new().commands(commands));
    if is_windows_msvc() {
        // #381 NFR-13a (ADR 0036): the linker embeds the Common Controls v6 manifest in every
        // executable, so tauri-build must not add a second one to the app binary.
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        embed_manifest_in_every_executable();
    }
    tauri_build::try_build(attributes).expect("tauri build script");
}

/// The build script reads the target from Cargo (not `cfg!`), so it also works cross-compiled.
fn is_windows_msvc() -> bool {
    let var = |k: &str| std::env::var(k).unwrap_or_default();
    var("CARGO_CFG_TARGET_OS") == "windows" && var("CARGO_CFG_TARGET_ENV") == "msvc"
}

/// `tauri` imports `TaskDialogIndirect` from comctl32 v6. Test executables get no manifest from
/// tauri-build, so Windows loads comctl32 v5 and they fail with STATUS_ENTRYPOINT_NOT_FOUND.
/// `rustc-link-arg` applies to every linked target: the app, the lib tests and the integration
/// tests. This is the same fix that tauri uses for its own tests (tauri `build.rs`).
fn embed_manifest_in_every_executable() {
    let manifest =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    // Linker warnings (for example a manifest conflict) fail the build.
    println!("cargo:rustc-link-arg=/WX");
    // Except LNK4099 only: the vendored OpenSSL of SQLCipher (#35, SP-21, ADR 0033) is a static
    // library whose objects name an `ossl_static.pdb` that openssl-src does not ship. The warning
    // is about third-party debug info only; every other warning still fails the link.
    println!("cargo:rustc-link-arg=/IGNORE:4099");
}
