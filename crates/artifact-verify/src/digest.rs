//! A SHA-256 digest value with hex parsing and constant-time equality.

use std::fmt;

/// A 32-byte SHA-256 digest.
#[derive(Clone, Copy)]
pub struct Sha256Digest([u8; 32]);

/// Error returned when a hex string is not a valid SHA-256 digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DigestParseError {
    /// The string does not have 64 hex characters.
    BadLength(usize),
    /// The string contains a character that is not a hex digit.
    BadChar,
}

impl Sha256Digest {
    /// Wraps raw digest bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parses 64 hex characters (upper or lower case).
    pub fn from_hex(_hex: &str) -> Result<Self, DigestParseError> {
        unimplemented!()
    }

    /// The raw digest bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lower-case hex form.
    pub fn to_hex(&self) -> String {
        unimplemented!()
    }

    /// Constant-time equality.
    pub fn ct_eq(&self, _other: &Self) -> bool {
        unimplemented!()
    }
}

impl PartialEq for Sha256Digest {
    fn eq(&self, other: &Self) -> bool {
        self.ct_eq(other)
    }
}

impl Eq for Sha256Digest {}

impl fmt::Debug for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Sha256Digest({})", self.to_hex())
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for DigestParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadLength(n) => write!(f, "expected 64 hex characters, got {n}"),
            Self::BadChar => f.write_str("digest contains a non-hex character"),
        }
    }
}

impl std::error::Error for DigestParseError {}
