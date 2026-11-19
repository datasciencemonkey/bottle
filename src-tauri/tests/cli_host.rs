//! The host's one R5 CliRunner (#16). Source checks and fake seams only: no test runs a real CLI.
//!
//! Requirement IDs: W-02a, W-02c, W-02d, W-04a, W-09.

use std::path::{Path, PathBuf};

use bottle_lib::cli_host;
use mog_core::cli_status::RECORD_FILE;

fn src(rel: &str) -> String {
    std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

/// Every `.rs` file under `src-tauri/src`.
fn host_sources() -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "rs") {
                let text = std::fs::read_to_string(&p).unwrap();
                out.push((p, text));
            }
        }
    }
    out
}

#[test]
fn w_04a_w_09_the_host_builds_one_shared_r5_on_the_real_system() {
    let builds: Vec<_> = host_sources()
        .into_iter()
        .filter(|(_, t)| t.contains("DatabricksCliRunner::system("))
        .map(|(p, t)| (p, t.matches("DatabricksCliRunner::system(").count()))
        .collect();
    assert_eq!(builds.len(), 1, "one R5 per process: {builds:?}");
    assert_eq!(builds[0].1, 1, "one R5 per process: {builds:?}");
    assert!(builds[0].0.ends_with("cli_host.rs"), "{builds:?}");
}

#[test]
fn w_04a_t7_and_r6_take_the_shared_r5_not_the_stub() {
    let t7 = src("src/profiles_host.rs");
    assert!(!t7.contains("StubCliRunner"), "T7 runs on the shared R5");
    assert!(t7.contains("try_state::<SharedCli>()"));
    let r8 = src("src/mcp_host.rs");
    let install = &r8[r8.find("pub fn install").expect("mcp_host::install")..];
    assert!(
        install.contains("try_state::<SharedCli>()"),
        "R6 runs on the shared R5"
    );
}

#[test]
fn w_02d_the_app_runs_the_launch_check_at_start_and_never_fails_on_it() {
    let lib = src("src/lib.rs");
    let r5 = lib.find("cli_host::install(handle)").expect("R5 installed");
    let t7 = lib.find("profiles_host::install(handle)").expect("T7");
    let start = lib.find("cli_host::start(handle)").expect("launch check");
    let r8 = lib.find("mcp_host::install(handle)").expect("R8");
    assert!(
        r5 < t7 && t7 < start && start < r8,
        "R5, T7, launch check, R8"
    );
    // Neither returns a Result, so a missing CLI cannot fail the app start.
    let host = src("src/cli_host.rs");
    assert!(host.contains("pub fn install(handle: &tauri::AppHandle<tauri::Wry>) {"));
    assert!(host.contains("pub fn start(handle: &tauri::AppHandle<tauri::Wry>) {"));
    // The launch check runs off the main thread: a hung CLI cannot block the app start.
    assert!(host.contains("std::thread::Builder::new()"));
}

#[test]
fn w_02d_the_record_lives_in_the_app_config_dir() {
    let dir = Path::new("app-config");
    assert_eq!(cli_host::record_path(dir), dir.join(RECORD_FILE));
    assert!(src("src/cli_host.rs").contains("app_config_dir()"));
}
