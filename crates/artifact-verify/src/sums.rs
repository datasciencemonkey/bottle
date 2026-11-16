//! Parser for a Databricks CLI release checksum file
//! (`databricks_cli_<ver>_SHA256SUMS`, lines `<hex>  <filename>`).

use std::fmt;

use crate::digest::{DigestParseError, Sha256Digest};

/// The parsed entries of a `SHA256SUMS` file.
#[derive(Debug, Clone, Default)]
pub struct Sha256Sums {
    entries: Vec<(String, Sha256Digest)>,
}

/// Errors from parsing or looking up a `SHA256SUMS` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SumsError {
    /// A line is not `<hex>  <filename>`.
    BadLine { line: usize },
    /// A line has a malformed digest.
    BadDigest {
        line: usize,
        source: DigestParseError,
    },
    /// The same file name is listed twice.
    Duplicate { file_name: String },
    /// The file does not list the requested artefact.
    NotListed { file_name: String },
}

/// The checksum file name of a Databricks CLI release, e.g.
/// `databricks_cli_0.250.0_SHA256SUMS` for version `0.250.0` (a leading `v` is removed).
pub fn sums_file_name(version: &str) -> String {
    let v = version.strip_prefix('v').unwrap_or(version);
    format!("databricks_cli_{v}_SHA256SUMS")
}

impl Sha256Sums {
    /// Parses the text of a `SHA256SUMS` file. Blank lines are ignored.
    pub fn parse(text: &str) -> Result<Self, SumsError> {
        let mut entries: Vec<(String, Sha256Digest)> = Vec::new();
        for (idx, raw) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw.trim_end_matches('\r');
            if line.trim().is_empty() {
                continue;
            }
            // `sha256sum` format: digest, one space, a mode char (' ' text or '*' binary),
            // then the name. "<hex>  *<name>" is accepted as binary mode too.
            let (hex, rest) = line
                .split_once(' ')
                .ok_or(SumsError::BadLine { line: line_no })?;
            let name = rest
                .strip_prefix(" *")
                .or_else(|| rest.strip_prefix(' '))
                .or_else(|| rest.strip_prefix('*'))
                .ok_or(SumsError::BadLine { line: line_no })?;
            if name.is_empty() || name.trim() != name {
                return Err(SumsError::BadLine { line: line_no });
            }
            let digest = Sha256Digest::from_hex(hex).map_err(|source| SumsError::BadDigest {
                line: line_no,
                source,
            })?;
            if entries.iter().any(|(n, _)| n == name) {
                return Err(SumsError::Duplicate {
                    file_name: name.to_string(),
                });
            }
            entries.push((name.to_string(), digest));
        }
        Ok(Self { entries })
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if the file has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The expected digest of `file_name`, if listed.
    pub fn get(&self, file_name: &str) -> Option<&Sha256Digest> {
        self.entries
            .iter()
            .find(|(n, _)| n == file_name)
            .map(|(_, d)| d)
    }

    /// The expected digest of `file_name`, or [`SumsError::NotListed`].
    pub fn expected_for(&self, file_name: &str) -> Result<&Sha256Digest, SumsError> {
        self.get(file_name).ok_or_else(|| SumsError::NotListed {
            file_name: file_name.to_string(),
        })
    }
}

impl fmt::Display for SumsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLine { line } => write!(f, "SHA256SUMS line {line} is not '<hex>  <file>'"),
            Self::BadDigest { line, source } => write!(f, "SHA256SUMS line {line}: {source}"),
            Self::Duplicate { file_name } => write!(f, "SHA256SUMS lists {file_name} twice"),
            Self::NotListed { file_name } => write!(f, "SHA256SUMS does not list {file_name}"),
        }
    }
}

impl std::error::Error for SumsError {}
