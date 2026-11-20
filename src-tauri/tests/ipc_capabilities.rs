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
    let core_of = |id: &str| -> BTreeSet<String> {
        permissions(&capability(id))
            .into_iter()
            .filter(|p| p.starts_with("core:"))
            .collect()
    };
    let listen = ["core:event:allow-listen", "core:event:allow-unlisten"];
    for id in ["hub", "mog"] {
        let want: BTreeSet<String> = listen.map(String::from).into();
        assert_eq!(core_of(id), want, "view {id} core permissions");
    }
    // #573 (ADR 0102): the card also fits its own window to its content: its size and place, and
    // the monitor and scale reads for that. It cannot show, focus, create or close a window.
    let card: BTreeSet<String> = listen
        .into_iter()
        .chain([
            "core:window:allow-set-size",
            "core:window:allow-set-position",
            "core:window:allow-outer-position",
            // lamp-alive: the card also reads the size and the title of its own window
            // (the side of its MOG, which the host sets, `card-placement.ts`); it reads no other window.
            "core:window:allow-outer-size",
            "core:window:allow-title",
            "core:window:allow-scale-factor",
            "core:window:allow-current-monitor",
        ])
        .map(String::from)
        .collect();
    assert_eq!(core_of("card"), card, "view card core permissions");
    // ARC-5: the brain sends to one view with emit_to; no global emit. #573 (ADR 0102): after a
    // double-click it finds the hub, shows it and focuses it. `allow-is-focused` is read-only: a ⌥G
    // follow-up into a thread (#145) asks it whether the hub window really has focus. It cannot create or close a window.
    // #ui-4 (review #603, findings 1 and 4): the brain also builds and pops up the MOG
    // right-click menu as a native Tauri menu (`mog-context-menu.ts`), on the MOG's own window
    // (`Window.getByLabel`, already covered by `allow-get-all-windows`): a MOG view itself stays
    // listen-and-view_emit-only (above), since only the brain holds `core:menu:*`.
    let want: BTreeSet<String> = [
        "core:event:allow-listen",
        "core:event:allow-unlisten",
        "core:event:allow-emit-to",
        "core:window:allow-get-all-windows",
        "core:window:allow-show",
        "core:window:allow-unminimize",
        "core:window:allow-set-focus",
        "core:window:allow-is-focused",
        "core:menu:allow-new",
        "core:menu:allow-popup",
    ]
    .map(String::from)
    .into();
    assert_eq!(core_of("brain"), want);
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
