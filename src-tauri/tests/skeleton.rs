//! Skeleton contract tests for #221: ARC-1, ARC-3, ARC-4 (config part) and MB-10a/MB-10c.

use bottle_lib::windows::{WindowKind, CARD_HIDDEN_TTL, MAX_ACTIVE_MOGS};
use serde_json::Value;
use std::path::{Path, PathBuf};

fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    app_dir().parent().expect("repo root").to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn json(path: &Path) -> Value {
    serde_json::from_str(&read(path)).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn tauri_conf() -> Value {
    json(&app_dir().join("tauri.conf.json"))
}

fn capabilities() -> Vec<Value> {
    let dir = app_dir().join("capabilities");
    let mut caps: Vec<Value> = std::fs::read_dir(&dir)
        .expect("capabilities dir")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .map(|p| json(&p))
        .collect();
    caps.sort_by_key(|c| c["identifier"].as_str().unwrap_or_default().to_owned());
    caps
}

// ---------- ARC-1: one process, the app links mog-core, the FFI crate exists ----------

#[test]
fn arc_1_app_links_mog_core() {
    assert_eq!(bottle_lib::linked_core_version(), mog_core::core_version());
}

#[test]
fn arc_1_workspace_has_core_ffi_and_app_members() {
    let manifest = read(&repo_root().join("Cargo.toml"));
    for member in ["crates/mog-core", "crates/mog-core-ffi", "src-tauri"] {
        assert!(
            manifest.contains(&format!("\"{member}\"")),
            "workspace member {member} missing"
        );
    }
    let ffi = read(&repo_root().join("crates/mog-core-ffi/Cargo.toml"));
    assert!(
        ffi.contains("cdylib"),
        "mog-core-ffi must build a C ABI library"
    );
    assert!(ffi.contains("mog-core"), "mog-core-ffi must link mog-core");
}

// ---------- ARC-3: the four window kinds ----------

#[test]
fn arc_3_brain_is_hidden_and_single() {
    let spec = WindowKind::Brain.spec();
    assert_eq!(spec.label, "brain");
    assert!(!spec.visible, "brain must be hidden");
    assert!(spec.skip_taskbar);
    let starts = bottle_lib::startup_windows(false);
    assert_eq!(
        starts.iter().filter(|k| **k == WindowKind::Brain).count(),
        1
    );
}

#[test]
fn arc_3_hub_is_a_normal_window() {
    let spec = WindowKind::Hub.spec();
    assert_eq!(spec.label, "hub");
    assert!(spec.decorations && spec.focusable && !spec.transparent && !spec.always_on_top);
}

#[test]
fn arc_3_mog_is_transparent_frameless_on_top_non_activating() {
    let spec = WindowKind::mog("7b0c1f3e-0000-4000-8000-000000000001")
        .expect("uuid id is valid")
        .spec();
    assert_eq!(spec.label, "mog-7b0c1f3e-0000-4000-8000-000000000001");
    assert!(spec.transparent, "MOG window must be transparent");
    assert!(!spec.decorations, "MOG window must be frameless");
    assert!(spec.always_on_top, "MOG window must stay on top");
    assert!(!spec.focusable, "MOG window must not take focus");
    assert!(spec.skip_taskbar);
}

#[test]
fn arc_3_card_is_floating_and_key_capable() {
    let spec = WindowKind::card("m1").expect("valid").spec();
    assert_eq!(spec.label, "card-m1");
    assert!(spec.always_on_top && spec.focusable && !spec.decorations);
    assert_eq!(
        CARD_HIDDEN_TTL.as_secs(),
        300,
        "cards die after 5 min hidden"
    );
}

#[test]
fn arc_3_mog_and_card_open_on_demand_not_at_start() {
    let starts = bottle_lib::startup_windows(false);
    assert_eq!(starts, vec![WindowKind::Brain, WindowKind::Hub]);
    assert_eq!(MAX_ACTIVE_MOGS, 8);
}

#[test]
fn arc_3_labels_round_trip_and_reject_unknown() {
    for kind in [
        WindowKind::Brain,
        WindowKind::Hub,
        WindowKind::Mog("a-1".into()),
        WindowKind::Card("a-1".into()),
    ] {
        assert_eq!(WindowKind::from_label(&kind.label()), Some(kind));
    }
    for bad in [
        "", "main", "mog-", "card-", "mog-a/b", "mog-a b", "Brain", "mog-x:y",
    ] {
        assert_eq!(WindowKind::from_label(bad), None, "label {bad:?}");
    }
    assert_eq!(WindowKind::mog(&"x".repeat(65)), None);
}

#[test]
fn arc_3_each_kind_has_its_own_html_entry() {
    let urls: Vec<&str> = [
        WindowKind::Brain,
        WindowKind::Hub,
        WindowKind::Mog("a".into()),
        WindowKind::Card("a".into()),
    ]
    .iter()
    .map(|k| k.spec().url)
    .collect();
    assert_eq!(urls, ["brain.html", "hub.html", "mog.html", "card.html"]);
    for url in urls {
        assert!(repo_root().join(url).exists(), "{url} missing at repo root");
    }
}

// ---------- ARC-4: CSP and per-window capabilities, no forbidden plugins ----------

const CSP: &str =
    "default-src 'self'; connect-src ipc: http://ipc.localhost; script-src 'self' 'wasm-unsafe-eval'";

#[test]
fn arc_4_csp_is_the_strict_policy() {
    let conf = tauri_conf();
    assert_eq!(conf["app"]["security"]["csp"].as_str(), Some(CSP));
    let dev = conf["app"]["security"]["devCsp"].as_str().expect("devCsp");
    assert!(dev.starts_with("default-src 'self'; connect-src ipc: http://ipc.localhost"));
    assert!(
        !dev.contains("https:"),
        "dev CSP must not allow external hosts"
    );
    assert!(!dev.contains('*'), "dev CSP must not use wildcards");
}

#[test]
fn arc_4_capabilities_are_scoped_per_window_label() {
    let caps = capabilities();
    let ids: Vec<&str> = caps
        .iter()
        .map(|c| c["identifier"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["brain", "card", "hub", "mog"]);
    let conf = tauri_conf();
    let listed: Vec<&str> = conf["app"]["security"]["capabilities"]
        .as_array()
        .expect("capabilities listed explicitly in tauri.conf.json")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(
        listed.len(),
        4,
        "only the 4 per-window capabilities are enabled"
    );
    for cap in &caps {
        let windows: Vec<&str> = cap["windows"]
            .as_array()
            .expect("windows")
            .iter()
            .map(|w| w.as_str().unwrap())
            .collect();
        let expected = match cap["identifier"].as_str().unwrap() {
            "brain" => "brain",
            "hub" => "hub",
            "mog" => "mog-*",
            "card" => "card-*",
            other => panic!("unexpected capability {other}"),
        };
        assert_eq!(windows, [expected]);
        assert!(cap.get("webviews").is_none() && cap.get("remote").is_none());
    }
}

#[test]
fn arc_4_no_window_gets_plugin_permissions() {
    for cap in capabilities() {
        for perm in cap["permissions"].as_array().expect("permissions") {
            let id = perm
                .as_str()
                .or_else(|| perm["identifier"].as_str())
                .expect("permission id");
            assert!(id.starts_with("core:"), "non-core permission {id}");
        }
    }
}

#[test]
fn arc_4_no_forbidden_plugin_crates() {
    // Cargo.lock covers every crate of the workspace, including mog-core and its deps.
    let lock = read(&repo_root().join("Cargo.lock"));
    for name in lock
        .lines()
        .filter_map(|l| l.strip_prefix("name = \""))
        .map(|l| l.trim_end_matches('"'))
    {
        assert!(
            !name.starts_with("tauri-plugin-"),
            "no Tauri plugin crates in the skeleton: {name}"
        );
    }
}

#[test]
fn arc_4_views_get_no_brain_permissions() {
    for cap in capabilities() {
        let id = cap["identifier"].as_str().unwrap();
        if id == "brain" {
            continue;
        }
        for perm in cap["permissions"].as_array().unwrap() {
            let p = perm.as_str().unwrap();
            assert!(
                p == "core:event:allow-listen" || p == "core:event:allow-unlisten",
                "view {id} has {p}"
            );
        }
    }
}

// ---------- MB-10a / MB-10c: product name Bottle, no MOG in package or bundle IDs ----------
//
// MB-10c scope (ADR 0001): the bundle ID, the product name, the npm package and the app crate.
// The internal crates `mog-core` and `mog-core-ffi` are named by ARC-1, are `publish = false`
// and never ship as a package or bundle ID, so they are out of scope.

fn has_banned_name(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("mother bricks") || l.contains("motherbricks")
}

#[test]
fn mb_10a_product_name_is_bottle() {
    let conf = tauri_conf();
    assert_eq!(conf["productName"].as_str(), Some("Bottle"));
    let package = json(&repo_root().join("package.json"));
    assert_eq!(package["name"].as_str(), Some("bottle"));
    for text in [
        read(&app_dir().join("tauri.conf.json")),
        read(&repo_root().join("package.json")),
        read(&app_dir().join("Cargo.toml")),
    ] {
        assert!(!has_banned_name(&text));
    }
}

#[test]
fn mb_10c_no_mog_in_bundle_or_package_ids() {
    let conf = tauri_conf();
    let identifier = conf["identifier"].as_str().unwrap();
    // B0 and Appendix C: the bundle ID equals the macOS MDM domain.
    assert_eq!(identifier, "com.databricks.bottle");
    let package = json(&repo_root().join("package.json"));
    let app_manifest = read(&app_dir().join("Cargo.toml"));
    for id in [
        identifier,
        package["name"].as_str().unwrap(),
        conf["productName"].as_str().unwrap(),
    ] {
        assert!(!id.to_lowercase().contains("mog"), "{id} contains MOG");
    }
    assert!(app_manifest.contains("name = \"bottle\""));
    for internal in [
        "crates/mog-core/Cargo.toml",
        "crates/mog-core-ffi/Cargo.toml",
    ] {
        assert!(
            read(&repo_root().join(internal)).contains("publish.workspace = true"),
            "{internal} must stay unpublished to be out of MB-10c scope"
        );
    }
    assert!(app_manifest.contains("name = \"bottle_lib\""));
}
