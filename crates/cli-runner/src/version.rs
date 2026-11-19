//! CLI version parsing and comparison.

use std::fmt;

/// A `major.minor.patch` CLI version. Pre-release and build suffixes are ignored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

/// Lowest CLI version Bottle accepts (D39; MDM `CliMinVersion` default).
pub const MIN_VERSION: Version = Version::new(1, 17, 0);
/// Recommended CLI version. Older but supported versions get a warning.
pub const RECOMMENDED_VERSION: Version = Version::new(1, 25, 0);

impl Version {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    /// Parses `1.17`, `1.17.0`, `v1.17.0` or `1.17.0-dev+abc`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let s = s.strip_prefix(['v', 'V']).unwrap_or(s);
        let core = s.split(['-', '+']).next()?;
        let parts = core
            .split('.')
            .map(|p| {
                if !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()) {
                    p.parse::<u64>().ok()
                } else {
                    None
                }
            })
            .collect::<Option<Vec<u64>>>()?;
        match parts[..] {
            [major, minor] => Some(Self::new(major, minor, 0)),
            [major, minor, patch] => Some(Self::new(major, minor, patch)),
            _ => None,
        }
    }

    /// Parses the output of `databricks --version`, e.g. `Databricks CLI v1.25.0`.
    pub fn parse_cli_output(output: &str) -> Option<Self> {
        output.split_whitespace().find_map(Self::parse)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}
