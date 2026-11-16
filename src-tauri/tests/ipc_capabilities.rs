//! #223: the per-window capability files match the IPC command table (ARC-4).

use mog_core::ipc::{WindowClass, COMMANDS};
use serde_json::Value;
use std::collections::BTreeSet;
use std::path::PathBuf;

fn capability(id: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("capabilities")
        .join(format!("{id}.json"));
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read")).expect("parse")
}

fn permissions(cap: &Value) -> BTreeSet<String> {
    cap["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .map(|p| p.as_str().expect("string permission").to_owned())
        .collect()
}

/// The Tauri permission for an app command (Tauri turns `_` into `-`).
fn allow(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

#[test]
fn arc_4_capability_files_match_the_command_table() {
    for (id, class) in [
        ("brain", WindowClass::Brain),
        ("hub", WindowClass::Hub),
        ("mog", WindowClass::Mog),
        ("card", WindowClass::Card),
    ] {
        let got: BTreeSet<String> = permissions(&capability(id))
            .into_iter()
            .filter(|p| !p.starts_with("core:"))
            .collect();
        let want: BTreeSet<String> = COMMANDS
            .iter()
            .filter(|c| c.windows.contains(&class))
            .map(|c| allow(c.name))
            .collect();
        assert_eq!(got, want, "capability {id}");
    }
}

#[test]
fn arc_4_views_only_listen_to_events() {
    for id in ["hub", "mog", "card"] {
        let core: BTreeSet<String> = permissions(&capability(id))
            .into_iter()
            .filter(|p| p.starts_with("core:"))
            .collect();
        let want: BTreeSet<String> = ["core:event:allow-listen", "core:event:allow-unlisten"]
            .map(String::from)
            .into();
        assert_eq!(core, want, "view {id} core permissions");
    }
    let brain = permissions(&capability("brain"));
    assert!(brain.contains("core:event:default"));
}

#[test]
fn arc_4_build_script_registers_the_table_in_the_app_manifest() {
    let build = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("build.rs"))
        .expect("build.rs");
    assert!(build.contains("app_manifest"), "commands must be ACL-gated");
    assert!(
        build.contains("mog_core::ipc::COMMANDS"),
        "one source for the command list"
    );
}

#[test]
fn arc_4_schema_file_is_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/ipc-commands.json");
    let generated = serde_json::to_string_pretty(&mog_core::ipc::schema_document()).unwrap() + "\n";
    if std::env::var("BOTTLE_UPDATE_SCHEMAS").is_ok_and(|v| v == "1") {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).unwrap();
    }
    let committed = std::fs::read_to_string(&path)
        .expect("run with BOTTLE_UPDATE_SCHEMAS=1 to write src-tauri/schemas/ipc-commands.json");
    assert_eq!(
        committed, generated,
        "schemas drifted: rerun with BOTTLE_UPDATE_SCHEMAS=1"
    );
}
