//! Token sources. The real source runs `databricks auth token --profile <p>`.
//! The token stays in memory only and its `Debug` output is redacted.
use std::future::Future;
use std::path::PathBuf;

pub struct Token(String);

impl Token {
    pub fn new(secret: impl Into<String>) -> Self {
        Token(secret.into())
    }
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, _f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unimplemented!("TODO(#29): green commit")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("the profile name is not valid")]
    BadProfile,
    #[error("could not start the Databricks CLI: {0:?}")]
    Spawn(std::io::ErrorKind),
    #[error("databricks auth token failed (exit {code:?}): {stderr}")]
    Exit { code: Option<i32>, stderr: String },
    #[error("databricks auth token printed no access_token")]
    BadOutput,
    #[error("the token source has no more tokens")]
    Exhausted,
}

pub trait TokenSource: Send + Sync {
    fn token(&self, force_refresh: bool) -> impl Future<Output = Result<Token, TokenError>> + Send;
}

pub struct CliTokenSource {
    #[allow(dead_code)]
    program: PathBuf,
    #[allow(dead_code)]
    profile: String,
}

impl CliTokenSource {
    pub fn new(_program: impl Into<PathBuf>, _profile: &str) -> Result<Self, TokenError> {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn args(&self, _force_refresh: bool) -> Vec<String> {
        unimplemented!("TODO(#29): green commit")
    }
}

impl TokenSource for CliTokenSource {
    async fn token(&self, _force_refresh: bool) -> Result<Token, TokenError> {
        unimplemented!("TODO(#29): green commit")
    }
}

pub fn scrub(_text: &str) -> String {
    unimplemented!("TODO(#29): green commit")
}

/// Test double: a list of tokens.
pub struct RotatingTokens {
    #[allow(dead_code)]
    tokens: Vec<String>,
}

impl RotatingTokens {
    pub fn new(_tokens: &[&str]) -> Self {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn fresh_each_call(_tokens: &[&str]) -> Self {
        unimplemented!("TODO(#29): green commit")
    }
    pub fn calls(&self) -> Vec<bool> {
        unimplemented!("TODO(#29): green commit")
    }
}

impl TokenSource for RotatingTokens {
    async fn token(&self, _force_refresh: bool) -> Result<Token, TokenError> {
        unimplemented!("TODO(#29): green commit")
    }
}
