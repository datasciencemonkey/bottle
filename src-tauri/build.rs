fn main() {
    // ARC-4: every IPC command is ACL-gated. One source: mog_core::ipc::COMMANDS.
    let commands: Vec<&'static str> = mog_core::ipc::COMMANDS.iter().map(|c| c.name).collect();
    let commands: &'static [&'static str] = Box::leak(commands.into_boxed_slice());
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands)),
    )
    .expect("tauri build script");
}
