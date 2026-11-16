//! Tests for THIRD_PARTY_NOTICES.md and the ported-file MIT header check.

use std::fs;
use std::path::PathBuf;

use artifact_verify::notices::{
    check_ported_file, check_ported_text, has_antigravity_mit_header, is_marked_ported,
    missing_notices, normalize, scan_ported_files, PortedCheck, ANTIGRAVITY_MIT_HEADER,
    PARAKEET_ATTRIBUTION, PORTED_MARKER, REQUIRED_NOTICES,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn notices_text() -> String {
    let p = repo_root().join("THIRD_PARTY_NOTICES.md");
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

/// Prefixes each line of `text` with `prefix` (a comment marker).
fn commented(prefix: &str, text: &str) -> String {
    text.lines()
        .map(|l| {
            if l.is_empty() {
                prefix.trim_end().to_string()
            } else {
                format!("{prefix}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn ported_rust_file(header: &str) -> String {
    format!(
        "{}\n{}\n\nfn bridge() {{}}\n",
        commented("// ", PORTED_MARKER),
        commented("// ", header)
    )
}

/// [SEC-21b] [AT-37] the required-notice list covers every component of section 20.1
#[test]
fn required_notice_list_covers_section_20_1() {
    let names: Vec<&str> = REQUIRED_NOTICES.iter().map(|n| n.component).collect();
    for want in [
        "antigravity-bricks-plugin",
        "NVIDIA Parakeet TDT 0.6B v3",
        "sherpa-onnx",
        "onnxruntime",
        "Silero VAD v6",
        "Rive runtimes",
        "Tauri",
        "tauri-nspanel",
        "tauri-plugin-global-shortcut",
        "Gemini on Databricks",
        "Databricks CLI",
    ] {
        assert!(names.contains(&want), "REQUIRED_NOTICES lacks {want}");
    }
    assert_eq!(missing_notices("").len(), REQUIRED_NOTICES.len());
}

/// [SEC-21b] [AT-37] THIRD_PARTY_NOTICES.md at the repo root has every notice of section 20.1
#[test]
fn third_party_notices_file_has_every_notice() {
    let text = notices_text();
    assert_eq!(missing_notices(&text), Vec::<&str>::new());
}

/// [SEC-21b] the CC-BY-4.0 Parakeet sentence and the MIT text are verbatim, not paraphrased
#[test]
fn notices_are_verbatim() {
    let text = normalize(&notices_text());
    assert!(text.contains(&normalize(PARAKEET_ATTRIBUTION)));
    assert!(text.contains(&normalize(ANTIGRAVITY_MIT_HEADER)));
    assert!(text.contains("Apache License Version 2.0, January 2004"));
    assert!(text.contains("CC BY 4.0"));
}

/// [SEC-21b] removing one notice is detected
#[test]
fn missing_notice_is_detected() {
    let text = notices_text().replace(PARAKEET_ATTRIBUTION, "Speech recognition uses Parakeet.");
    assert_eq!(missing_notices(&text), vec!["NVIDIA Parakeet TDT 0.6B v3"]);
    let text = notices_text().replace("Silero", "S.");
    assert!(missing_notices(&text).contains(&"Silero VAD v6"));
}

/// [SEC-21a] a ported file with the MIT header in comments passes
#[test]
fn ported_file_with_header_passes() {
    let rs = ported_rust_file(ANTIGRAVITY_MIT_HEADER);
    assert!(is_marked_ported(&rs));
    assert!(has_antigravity_mit_header(&rs));
    assert_eq!(check_ported_text(&rs), PortedCheck::HeaderPresent);

    let py = format!(
        "{}\n{}\n\ndef bridge():\n    pass\n",
        commented("# ", PORTED_MARKER),
        commented("# ", ANTIGRAVITY_MIT_HEADER)
    );
    assert_eq!(check_ported_text(&py), PortedCheck::HeaderPresent);

    let block = format!(
        "/*\n * {PORTED_MARKER}\n{}\n */\n",
        commented(" * ", ANTIGRAVITY_MIT_HEADER)
    );
    assert_eq!(check_ported_text(&block), PortedCheck::HeaderPresent);

    let md = format!("<!-- {PORTED_MARKER} -->\n<!--\n{ANTIGRAVITY_MIT_HEADER}-->\n# Skill\n");
    assert_eq!(check_ported_text(&md), PortedCheck::HeaderPresent);

    let crlf = ported_rust_file(ANTIGRAVITY_MIT_HEADER).replace('\n', "\r\n");
    assert_eq!(check_ported_text(&crlf), PortedCheck::HeaderPresent);
}

/// [SEC-21a] a ported file without the header, or with a changed header, fails
#[test]
fn ported_file_without_header_fails() {
    let bare = format!("// {PORTED_MARKER}\nfn bridge() {{}}\n");
    assert_eq!(check_ported_text(&bare), PortedCheck::HeaderMissing);

    let changed =
        ported_rust_file(&ANTIGRAVITY_MIT_HEADER.replace("Sathish Gangichetty", "Somebody Else"));
    assert_eq!(check_ported_text(&changed), PortedCheck::HeaderMissing);

    let truncated = ported_rust_file(&ANTIGRAVITY_MIT_HEADER[..400]);
    assert_eq!(check_ported_text(&truncated), PortedCheck::HeaderMissing);
}

/// [SEC-21a] files not marked as ported are ignored; a mention of the marker inside code is not a mark
#[test]
fn unmarked_files_are_not_ported() {
    assert_eq!(check_ported_text("fn main() {}\n"), PortedCheck::NotPorted);
    let mention = format!("let m = \"{PORTED_MARKER}\";\n");
    assert!(!is_marked_ported(&mention));
}

/// [SEC-21a] the scanner finds ported files that lack the header and skips build output
#[test]
fn scanner_reports_ported_files_without_header() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("src/genie")).unwrap();
    fs::create_dir_all(root.join("target/debug")).unwrap();
    fs::create_dir_all(root.join("node_modules/x")).unwrap();
    let good = root.join("src/genie/bridge.rs");
    let bad = root.join("src/genie/skills.ts");
    fs::write(&good, ported_rust_file(ANTIGRAVITY_MIT_HEADER)).unwrap();
    fs::write(&bad, format!("// {PORTED_MARKER}\nexport {{}};\n")).unwrap();
    fs::write(root.join("src/plain.rs"), "fn x() {}\n").unwrap();
    fs::write(root.join("src/blob.bin"), [0xff, 0xfe, 0x00, 0x81]).unwrap();
    fs::write(
        root.join("target/debug/copy.rs"),
        format!("// {PORTED_MARKER}\n"),
    )
    .unwrap();
    fs::write(
        root.join("node_modules/x/i.js"),
        format!("// {PORTED_MARKER}\n"),
    )
    .unwrap();

    assert_eq!(
        check_ported_file(&good).unwrap(),
        PortedCheck::HeaderPresent
    );
    assert_eq!(check_ported_file(&bad).unwrap(), PortedCheck::HeaderMissing);
    assert_eq!(
        check_ported_file(&root.join("src/blob.bin")).unwrap(),
        PortedCheck::NotPorted
    );
    assert_eq!(scan_ported_files(root).unwrap(), vec![bad]);
}

/// [SEC-21a] [AT-37] every ported file in this repository keeps the MIT header
#[test]
fn repository_ported_files_keep_header() {
    let missing = scan_ported_files(&repo_root()).unwrap();
    assert!(
        missing.is_empty(),
        "ported files without the MIT header: {missing:?}"
    );
}
