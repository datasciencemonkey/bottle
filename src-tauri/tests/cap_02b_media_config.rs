//! #387: CAP-02b / AT-17 (getUserMedia part). The config side of the WebView media block.
//!
//! CSP has no directive for `getUserMedia`, so these tests check the three layers that do
//! block it (ADR 0037): the app-wide deny handler in `run()`, the `Permissions-Policy` header
//! on every app page, and the CSP and capabilities that keep remote code and plugins out.

use serde_json::Value;
use std::path::{Path, PathBuf};

fn app_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn tauri_conf() -> Value {
    serde_json::from_str(&read(&app_dir().join("tauri.conf.json"))).expect("parse tauri.conf.json")
}

/// Every `.rs` file under `src-tauri/src`, with its text.
fn app_sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(dir).expect("read src dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|x| x == "rs") {
                let text = read(&path);
                out.push((path, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&app_dir().join("src"), &mut out);
    out
}

/// The media features that the header must turn off for every origin.
const MEDIA_FEATURES: [&str; 3] = ["camera", "microphone", "display-capture"];

#[test]
fn cap_02b_permissions_policy_denies_media() {
    let conf = tauri_conf();
    let header = &conf["app"]["security"]["headers"]["Permissions-Policy"];
    let policy = header
        .as_str()
        .expect("app.security.headers has a Permissions-Policy string (CAP-02b)");
    let directives: Vec<&str> = policy.split(',').map(str::trim).collect();
    for feature in MEDIA_FEATURES {
        assert!(
            directives.contains(&format!("{feature}=()").as_str()),
            "Permissions-Policy must set {feature}=() (no origin), got {policy:?}"
        );
    }
}

#[test]
fn cap_02b_app_installs_one_app_wide_deny_handler() {
    // The app-wide handler covers every WebView. A per-WebView handler runs first and can
    // override it, so no other file may set one.
    let hits: Vec<(PathBuf, usize)> = app_sources()
        .into_iter()
        .map(|(p, t)| {
            let n = t.matches(".on_permission_request(").count();
            (p, n)
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    let lib = app_dir().join("src").join("lib.rs");
    assert_eq!(
        hits,
        vec![(lib.clone(), 1)],
        "exactly one on_permission_request call, in lib.rs (CAP-02b)"
    );
    let text = read(&lib);
    assert!(
        text.contains(".on_permission_request(webview_media::on_permission_request)"),
        "run() must install webview_media::on_permission_request on the app builder"
    );
    // The call must be on the app builder in run(), not on a WebviewWindowBuilder.
    let run = text.split("pub fn run()").nth(1).expect("run() in lib.rs");
    assert!(run.contains(".on_permission_request(webview_media::on_permission_request)"));
}

#[test]
fn cap_02b_csp_and_capabilities_allow_no_media() {
    let conf = tauri_conf();
    let security = &conf["app"]["security"];
    for key in ["csp", "devCsp"] {
        let csp = security[key].as_str().expect("CSP string");
        assert!(csp.starts_with("default-src 'self'"), "{key}: {csp}");
        // A media stream source or a remote host would widen what a page can load or run.
        for banned in ["mediastream:", "blob:", "https:", "http:*", "*"] {
            assert!(
                !csp.contains(banned),
                "{key} must not contain {banned}: {csp}"
            );
        }
    }
    // Tauri has no media permission, so a media grant can only come from a plugin
    // or a remote URL in a capability. None may exist.
    let dir = app_dir().join("capabilities");
    for entry in std::fs::read_dir(&dir).expect("capabilities dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let text = read(&path).to_lowercase();
        for banned in [
            "media",
            "microphone",
            "camera",
            "getusermedia",
            "\"remote\"",
        ] {
            assert!(
                !text.contains(banned),
                "{} must not mention {banned}",
                path.display()
            );
        }
    }
}
