//! Token sources. The real source runs `databricks auth token --profile <p>`
//! (binding rule: tokens come only from the CLI). The token stays in memory only:
//! `Token` has no `Display`, no `Serialize`, and its `Debug` output is redacted.
use std::future::Future;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct Token(String);

impl Token {
    pub fn new(secret: impl Into<String>) -> Self {
        Token(secret.into())
    }

    /// The only way to read the secret. Callers put it in an `Authorization`
    /// header and nowhere else.
    pub fn expose_secret(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(<redacted>)")
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

/// Gives a token for each request (G-04d). `force_refresh` asks for a new token
/// even if the cached one is still valid (used once after a 401).
pub trait TokenSource: Send + Sync {
    fn token(&self, force_refresh: bool) -> impl Future<Output = Result<Token, TokenError>> + Send;
}

/// `databricks auth token --profile <p> --output json [--force-refresh]`.
/// The CLI keeps its own token cache; this type keeps nothing between calls.
pub struct CliTokenSource {
    program: PathBuf,
    profile: String,
}

impl CliTokenSource {
    /// The profile is chosen by the person. It must not look like a flag.
    pub fn new(program: impl Into<PathBuf>, profile: &str) -> Result<Self, TokenError> {
        let ok = !profile.is_empty()
            && !profile.starts_with('-')
            && profile
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'));
        if !ok {
            return Err(TokenError::BadProfile);
        }
        Ok(CliTokenSource {
            program: program.into(),
            profile: profile.to_string(),
        })
    }

    pub fn args(&self, force_refresh: bool) -> Vec<String> {
        let mut a: Vec<String> = [
            "auth",
            "token",
            "--profile",
            &self.profile,
            "--output",
            "json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        if force_refresh {
            a.push("--force-refresh".into());
        }
        a
    }
}

impl TokenSource for CliTokenSource {
    async fn token(&self, force_refresh: bool) -> Result<Token, TokenError> {
        let out = tokio::process::Command::new(&self.program)
            .args(self.args(force_refresh))
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| TokenError::Spawn(e.kind()))?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let first: String = stderr.lines().take(3).collect::<Vec<_>>().join(" ");
            let mut s = scrub(&first);
            s.truncate(300);
            return Err(TokenError::Exit {
                code: out.status.code(),
                stderr: s,
            });
        }
        // stdout holds the token: parse it, never log or keep the raw bytes.
        let v: serde_json::Value =
            serde_json::from_slice(&out.stdout).map_err(|_| TokenError::BadOutput)?;
        match v.get("access_token").and_then(|t| t.as_str()) {
            Some(t) if !t.is_empty() => Ok(Token::new(t)),
            _ => Err(TokenError::BadOutput),
        }
    }
}

fn token_like(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let charset = w
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/' | '='));
    w.starts_with("eyJ")
        || w.starts_with("dapi")
        || w.starts_with("dose")
        || (w.len() >= 32 && charset)
}

/// Replace words that look like tokens (JWTs, PATs, long opaque strings).
pub fn scrub(text: &str) -> String {
    text.split(' ')
        .map(|w| if token_like(w) { "<redacted>" } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Test double: a list of tokens.
/// - `new`: returns the current token; a forced refresh moves to the next one
///   (like the CLI cache, which keeps a token until it expires or is forced).
/// - `fresh_each_call`: every call returns the next token.
pub struct RotatingTokens {
    tokens: Vec<String>,
    each_call: bool,
    state: Mutex<(usize, Vec<bool>)>,
}

impl RotatingTokens {
    pub fn new(tokens: &[&str]) -> Self {
        Self::build(tokens, false)
    }

    pub fn fresh_each_call(tokens: &[&str]) -> Self {
        Self::build(tokens, true)
    }

    fn build(tokens: &[&str], each_call: bool) -> Self {
        RotatingTokens {
            tokens: tokens.iter().map(|t| t.to_string()).collect(),
            each_call,
            state: Mutex::new((0, Vec::new())),
        }
    }

    /// The `force_refresh` flag of each call, in order.
    pub fn calls(&self) -> Vec<bool> {
        self.state.lock().expect("lock").1.clone()
    }
}

impl TokenSource for RotatingTokens {
    async fn token(&self, force_refresh: bool) -> Result<Token, TokenError> {
        let mut st = self.state.lock().expect("lock");
        let first = st.1.is_empty();
        st.1.push(force_refresh);
        if !first && (self.each_call || force_refresh) {
            st.0 += 1;
        } else if first && force_refresh {
            st.0 = 0;
        }
        self.tokens
            .get(st.0)
            .map(|t| Token::new(t.as_str()))
            .ok_or(TokenError::Exhausted)
    }
}
