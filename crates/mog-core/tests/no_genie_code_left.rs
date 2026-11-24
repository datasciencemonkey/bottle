//! ADR 0122: the Genie Code session host, its shim and its install tooling are gone. No file of
//! the Rust code, the app shell, the tools or the repo tests may name them again, comments
//! included. The ADRs in `docs/adr` are the historical log and are not scanned.
//!
//! The words are built from pieces, so this file does not match its own scan.

use std::path::{Path, PathBuf};

/// This file: it holds the pieces, not the words, but it is left out of the scan anyway.
const OWN: &str = "no_genie_code_left.rs";

/// The lower-case words that must not occur. A match is case-insensitive.
fn banned() -> Vec<String> {
    [
        ("genie", " code"),
        ("genie", "_code"),
        ("genie", "-code"),
        ("genie", "code"),
        ("genie", " app-server"),
        ("app", "-server"),
        ("agent-mcp-", "shim"),
        ("genie", "_home"),
        ("genie", "-lockdown"),
        ("sh", "im"),
    ]
    .iter()
    .map(|(a, b)| format!("{a}{b}"))
    .collect()
}

fn scan(dir: &Path, banned: &[String], hits: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                "target" | "node_modules" | ".git" | "gen" | "dist"
            ) {
                continue;
            }
            scan(&path, banned, hits);
            continue;
        }
        if name == OWN || name == "Cargo.lock" {
            continue;
        }
        // Binary and unreadable files are skipped: only text can name a feature.
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = text.to_lowercase();
        for w in banned {
            if lower.contains(w.as_str()) {
                hits.push(format!("{}: {w}", path.display()));
            }
        }
    }
}

#[test]
fn adr_0122_no_file_names_the_removed_session_host_its_shim_or_its_tooling() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let banned = banned();
    let mut hits = Vec::new();
    for dir in ["crates", "src-tauri", "tools", "tests"] {
        let mut found = Vec::new();
        scan(&root.join(dir), &banned, &mut found);
        hits.extend(found);
    }
    assert!(
        hits.is_empty(),
        "removed names remain:\n{}",
        hits.join("\n")
    );
}
