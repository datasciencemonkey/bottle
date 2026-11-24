//! ADR 0122: the old session host, its shim and its install tooling are gone. No file of the
//! repo may name them again, comments, docs and UI text included. The ADRs in `docs/adr` are the
//! historical log, and `research/director-decisions.md` and `research/kimi-snapshot` are records.
//! They are not scanned.
//!
//! The words are built from pieces, so this file does not match its own scan. A word matches only
//! as a whole word: the characters next to it are no letters or digits.

use std::path::{Path, PathBuf};

/// This file and its TypeScript twin hold the pattern, so they are left out of the scan. The
/// paths are relative to the repo root, so a file with the same bare name elsewhere is scanned.
const OWN: [&str; 2] = [
    "crates/mog-core/tests/no_removed_host_left.rs",
    "tests/no-removed-host.test.ts",
];

/// The paths, relative to the repo root, that the scan skips with everything below them.
const SKIP: [&str; 7] = [
    "docs/adr",
    "research/director-decisions.md",
    "research/kimi-snapshot",
    "target",
    "node_modules",
    ".git",
    // Generated test reports (git-ignored); they echo test names.
    "reports",
];

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

/// True when `word` occurs in `lower` with no letter or digit on either side.
fn has_whole_word(lower: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_alphanumeric();
    lower.match_indices(word).any(|(i, _)| {
        let before = lower[..i].chars().next_back();
        let after = lower[i + word.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

fn scan(root: &Path, dir: &Path, banned: &[String], hits: &mut Vec<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let path = e.path();
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if SKIP.contains(&rel.as_str()) || OWN.contains(&rel.as_str()) {
            continue;
        }
        if path.is_dir() {
            // Build output is not source.
            if !matches!(e.file_name().to_str(), Some("gen" | "dist")) {
                scan(root, &path, banned, hits);
            }
            continue;
        }
        if rel == "Cargo.lock" {
            continue;
        }
        // Binary and unreadable files are skipped: only text can name a feature.
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let lower = text.to_lowercase();
        for w in banned {
            if has_whole_word(&lower, w) {
                hits.push(format!("{rel}: {w}"));
            }
        }
    }
}

#[test]
fn adr_0122_no_file_names_the_removed_session_host_its_shim_or_its_tooling() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize().unwrap_or(root);
    let mut hits = Vec::new();
    scan(&root, &root, &banned(), &mut hits);
    assert!(
        hits.is_empty(),
        "removed names remain:\n{}",
        hits.join("\n")
    );
}

#[test]
fn a_banned_word_matches_only_as_a_whole_word() {
    let shim = &banned()[9];
    assert!(has_whole_word("a shim here", shim));
    assert!(has_whole_word("the shim.", shim));
    assert!(has_whole_word("shim", shim));
    assert!(!has_whole_word("a shimmer", shim));
    assert!(!has_whole_word("ashim", shim));
}

#[test]
fn the_scan_skips_the_own_files_by_full_path_only() {
    assert!(OWN.iter().all(|p| p.contains('/')));
    assert!(SKIP.contains(&"docs/adr"));
}
