//! Third-party notice checks (SEC-21a, SEC-21b).
//!
//! * [`missing_notices`] lists the notices of requirements section 20.1 that a
//!   `THIRD_PARTY_NOTICES.md` text lacks.
//! * [`has_antigravity_mit_header`] and [`scan_ported_files`] check that every file
//!   marked as ported from antigravity-bricks-plugin keeps its MIT header.
//!
//! Text is compared after comment markers are removed and white space is collapsed,
//! so the header can sit in a `//`, `#`, `/* */` or `<!-- -->` comment.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The MIT header that files ported from antigravity-bricks-plugin (commit a31fd2c)
/// must keep, verbatim from requirements section 20.1.
pub const ANTIGRAVITY_MIT_HEADER: &str = "\
MIT License

Copyright (c) 2026 Sathish Gangichetty

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
";

/// The Parakeet CC-BY-4.0 attribution sentence, verbatim from requirements section 20.1.
pub const PARAKEET_ATTRIBUTION: &str = "Speech recognition uses NVIDIA Parakeet TDT 0.6B v3 (https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3), © NVIDIA, licensed under CC BY 4.0 (https://creativecommons.org/licenses/by/4.0/), converted to ONNX and quantised to int8 by the k2-fsa/sherpa-onnx project. Modifications: ONNX export and int8 quantisation.";

/// A file is "marked as ported" when one of its lines, after the comment marker,
/// is exactly this text.
pub const PORTED_MARKER: &str = "Ported from antigravity-bricks-plugin (commit a31fd2c).";

/// One notice that `THIRD_PARTY_NOTICES.md` must contain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequiredNotice {
    /// Component name, as in requirements section 20.1.
    pub component: &'static str,
    /// Texts that must all be present (compared with white space collapsed).
    pub needles: &'static [&'static str],
}

/// The opening of the permission paragraph shared by every MIT licence text.
pub const MIT_PERMISSION: &str = "\
Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the \"Software\"), to deal
in the Software without restriction";

/// Every notice of requirements section 20.1.
pub const REQUIRED_NOTICES: &[RequiredNotice] = &[
    RequiredNotice {
        component: "antigravity-bricks-plugin",
        needles: &[
            "github.com/datasciencemonkey/antigravity-bricks-plugin",
            "a31fd2c",
            ANTIGRAVITY_MIT_HEADER,
        ],
    },
    RequiredNotice {
        component: "NVIDIA Parakeet TDT 0.6B v3",
        needles: &["CC-BY-4.0", PARAKEET_ATTRIBUTION],
    },
    RequiredNotice {
        component: "sherpa-onnx",
        needles: &[
            "sherpa-onnx",
            "Apache License",
            "Version 2.0, January 2004",
            "END OF TERMS AND CONDITIONS",
        ],
    },
    RequiredNotice {
        component: "onnxruntime",
        needles: &[
            "onnxruntime",
            "Copyright (c) Microsoft Corporation",
            MIT_PERMISSION,
        ],
    },
    RequiredNotice {
        component: "Silero VAD v6",
        needles: &["Silero VAD", "Copyright (c) 2020-present Silero Team"],
    },
    RequiredNotice {
        component: "Rive runtimes",
        needles: &[
            "@rive-app/webgl2",
            "@rive-app/canvas",
            "rive-wasm",
            "Copyright (c) 2020-2021 Rive",
        ],
    },
    RequiredNotice {
        component: "Tauri",
        needles: &[
            "Copyright (c) 2017 - Present Tauri Apps Contributors",
            "The Tauri Programme in the Commons Conservancy",
        ],
    },
    RequiredNotice {
        component: "tauri-nspanel",
        needles: &["tauri-nspanel", "Copyright (c) 2023 - Present Victor Aremu"],
    },
    RequiredNotice {
        component: "tauri-plugin-global-shortcut",
        needles: &["tauri-plugin-global-shortcut"],
    },
    RequiredNotice {
        component: "Gemini on Databricks",
        needles: &["Gemini on Databricks", "accepts the Gemini terms"],
    },
    RequiredNotice {
        component: "Databricks CLI",
        needles: &["Databricks CLI", "Databricks License", "not bundled"],
    },
];

/// Comment markers removed from the start (and end) of each line before comparing.
const COMMENT_MARKERS: &[&str] = &["<!--", "-->", "///", "//!", "//", "/*", "*/", "#", "*", ";"];

/// Components of [`REQUIRED_NOTICES`] whose texts are not all in `text`.
pub fn missing_notices(text: &str) -> Vec<&'static str> {
    let hay = normalize(text);
    REQUIRED_NOTICES
        .iter()
        .filter(|n| {
            !n.needles
                .iter()
                .all(|needle| hay.contains(&normalize(needle)))
        })
        .map(|n| n.component)
        .collect()
}

/// Removes comment markers at the start and end of one line, then trims it.
fn strip_comment(line: &str) -> &str {
    let mut l = line.trim();
    loop {
        let before = l;
        for m in COMMENT_MARKERS {
            if let Some(rest) = l.strip_prefix(m) {
                l = rest.trim_start();
            }
        }
        for m in ["-->", "*/"] {
            if let Some(rest) = l.strip_suffix(m) {
                l = rest.trim_end();
            }
        }
        if l == before {
            return l;
        }
    }
}

/// Collapses white space and removes leading comment markers on each line.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        for word in strip_comment(line).split_whitespace() {
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(word);
        }
    }
    out
}

/// True if `text` carries the full antigravity-bricks-plugin MIT header.
pub fn has_antigravity_mit_header(text: &str) -> bool {
    normalize(text).contains(&normalize(ANTIGRAVITY_MIT_HEADER))
}

/// True if a line of `text` is the [`PORTED_MARKER`].
pub fn is_marked_ported(text: &str) -> bool {
    text.lines().any(|l| strip_comment(l) == PORTED_MARKER)
}

/// Result of checking one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortedCheck {
    /// The file is not marked as ported.
    NotPorted,
    /// The file is marked as ported and keeps the MIT header.
    HeaderPresent,
    /// The file is marked as ported but the MIT header is missing or changed.
    HeaderMissing,
}

/// Checks the text of one file.
pub fn check_ported_text(text: &str) -> PortedCheck {
    if !is_marked_ported(text) {
        PortedCheck::NotPorted
    } else if has_antigravity_mit_header(text) {
        PortedCheck::HeaderPresent
    } else {
        PortedCheck::HeaderMissing
    }
}

/// Checks one file on disk. Files that are not UTF-8 are treated as not ported.
pub fn check_ported_file(path: &Path) -> io::Result<PortedCheck> {
    let bytes = fs::read(path)?;
    Ok(match std::str::from_utf8(&bytes) {
        Ok(text) => check_ported_text(text),
        Err(_) => PortedCheck::NotPorted,
    })
}

/// Directory names [`scan_ported_files`] does not enter.
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

/// Walks `root` and returns every file marked as ported that lacks the MIT header.
/// Skips `.git`, `target` and `node_modules` directories, and symbolic links.
/// The result is sorted.
pub fn scan_ported_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let path = entry.path();
            if kind.is_dir() {
                let skip = SKIP_DIRS
                    .iter()
                    .any(|d| entry.file_name().to_str() == Some(d));
                if !skip {
                    stack.push(path);
                }
            } else if kind.is_file() && check_ported_file(&path)? == PortedCheck::HeaderMissing {
                missing.push(path);
            }
        }
    }
    missing.sort();
    Ok(missing)
}
