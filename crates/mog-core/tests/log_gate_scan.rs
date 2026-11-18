//! SEC-16c, AT-17 (#184, ADR 0079): production Rust code writes logs only through
//! `mog_core::telemetry::AppLog`. It lives in `tests/` so the SEC-03 print scan does not see
//! the banned list itself.

use std::path::{Path, PathBuf};

/// Production Rust code under these roots must write logs only through `mog_core::telemetry::AppLog`.
const SCAN_ROOTS: [&str; 3] = ["src", "../mog-core-ffi/src", "../../src-tauri/src"];
const BANNED: [&str; 8] = [
    "println!",
    "eprintln!",
    "print!(",
    "eprint!(",
    "dbg!",
    "log::",
    "tracing::",
    "std::io::stderr",
];

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n == "tests") {
                continue;
            }
            rust_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n != "tests.rs" && !n.ends_with("_tests.rs"))
        {
            out.push(p);
        }
    }
}

/// `banned` at the start of a path or call, so `catalog::` does not count as `log::`.
fn uses(line: &str, banned: &str) -> bool {
    line.match_indices(banned).any(|(i, _)| {
        line[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_alphanumeric() || c == '_'))
    })
}

#[test]
fn sec_16c_scan_matches_paths_not_suffixes() {
    assert!(uses("log::info!(x)", "log::"));
    assert!(uses("    ::log::warn!(x)", "log::"));
    assert!(uses("x; println!(\"{}\", y)", "println!"));
    assert!(!uses("catalog::find(id)", "log::"));
    assert!(!uses("my_println!()", "println!"));
}

#[test]
fn sec_16c_at_17_no_production_code_logs_outside_the_gate() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for root in SCAN_ROOTS {
        rust_files(&base.join(root), &mut files);
    }
    assert!(files.len() > 40, "the scan found the sources");
    let mut found = Vec::new();
    for f in files {
        // `testkit` is test support: its `live_or_skip!` prints why a live test skipped.
        if f.ends_with("testkit.rs") {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        // Only the test module at the end is skipped; a `#[cfg(test)]` item elsewhere is scanned.
        let code = src.split("#[cfg(test)]\nmod ").next().unwrap_or_default();
        for (i, line) in code.lines().enumerate() {
            let t = line.trim_start();
            if t.starts_with("//") {
                continue;
            }
            for b in BANNED {
                if uses(t, b) {
                    found.push(format!("{}:{} {b}", f.display(), i + 1));
                }
            }
        }
    }
    assert!(found.is_empty(), "logs outside the gate: {found:#?}");
}
