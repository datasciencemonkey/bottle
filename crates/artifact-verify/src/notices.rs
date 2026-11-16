//! Third-party notice checks (SEC-21a, SEC-21b).
//!
//! * [`missing_notices`] lists the notices of requirements section 20.1 that a
//!   `THIRD_PARTY_NOTICES.md` text lacks.
//! * [`has_antigravity_mit_header`] and [`scan_ported_files`] check that every file
//!   marked as ported from antigravity-bricks-plugin keeps its MIT header.
//!
//! Text is compared after comment markers are removed and white space is collapsed,
//! so the header can sit in a `//`, `#`, `/* */` or `<!-- -->` comment.

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

/// Every notice of requirements section 20.1.
pub const REQUIRED_NOTICES: &[RequiredNotice] = &[];

/// Components of [`REQUIRED_NOTICES`] whose texts are not all in `text`.
pub fn missing_notices(_text: &str) -> Vec<&'static str> {
    unimplemented!()
}

/// Collapses white space and removes leading comment markers on each line.
pub fn normalize(_text: &str) -> String {
    unimplemented!()
}

/// True if `text` carries the full antigravity-bricks-plugin MIT header.
pub fn has_antigravity_mit_header(_text: &str) -> bool {
    unimplemented!()
}

/// True if a line of `text` is the [`PORTED_MARKER`].
pub fn is_marked_ported(_text: &str) -> bool {
    unimplemented!()
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
pub fn check_ported_text(_text: &str) -> PortedCheck {
    unimplemented!()
}

/// Checks one file on disk. Files that are not UTF-8 are treated as not ported.
pub fn check_ported_file(_path: &Path) -> io::Result<PortedCheck> {
    unimplemented!()
}

/// Walks `root` and returns every file marked as ported that lacks the MIT header.
/// Skips `.git`, `target` and `node_modules` directories, and symbolic links.
pub fn scan_ported_files(_root: &Path) -> io::Result<Vec<PathBuf>> {
    unimplemented!()
}
