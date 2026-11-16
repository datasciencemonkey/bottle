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
    /// The CLI failed. Only the exit code and a fixed class are kept: the CLI
    /// text can name the host, the workspace ID or the profile, so it is dropped.
    #[error("databricks auth token failed (exit {code:?}, class {class})")]
    Exit {
        code: Option<i32>,
        class: CliErrorClass,
    },
    #[error("databricks auth token printed no access_token")]
    BadOutput,
    #[error("the token source has no more tokens")]
    Exhausted,
}

/// A fixed category for a failed `databricks auth token`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliErrorClass {
    LoginRequired,
    NotU2m,
    ProfileNotFound,
    Timeout,
    Other,
}

impl std::fmt::Display for CliErrorClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            CliErrorClass::LoginRequired => "login_required",
            CliErrorClass::NotU2m => "not_u2m",
            CliErrorClass::ProfileNotFound => "profile_not_found",
            CliErrorClass::Timeout => "timeout",
            CliErrorClass::Other => "other",
        })
    }
}

/// Map CLI stderr to a class. The text itself is never kept.
pub fn classify_cli_error(stderr: &str) -> CliErrorClass {
    let t = stderr.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if has(&["timeout", "deadline exceeded", "timed out"]) {
        CliErrorClass::Timeout
    } else if has(&[
        "m2m",
        "u2m",
        "not supported",
        "pat ",
        "personal access token",
    ]) {
        CliErrorClass::NotU2m
    } else if t.contains("profile")
        && has(&[
            "not found",
            "no such",
            "does not exist",
            "cannot find",
            "has no",
        ])
    {
        CliErrorClass::ProfileNotFound
    } else if has(&[
        "invalid_grant",
        "login",
        "refresh",
        "expired",
        "reauth",
        "unauthorized",
    ]) {
        CliErrorClass::LoginRequired
    } else {
        CliErrorClass::Other
    }
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
            return Err(TokenError::Exit {
                code: out.status.code(),
                class: classify_cli_error(&String::from_utf8_lossy(&out.stderr)),
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

fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~' | '+' | '/')
}

fn secret_like(run: &str) -> bool {
    let digits = run.chars().filter(char::is_ascii_digit).count();
    run.starts_with("eyJ")
        || run.starts_with("dapi")
        || run.starts_with("dose")
        || run.len() >= 32
        // A workspace ID or another long number.
        || (digits >= 10 && digits * 2 >= run.len())
        // A host name.
        || run.contains("databricks.")
        || run.contains(".azuredatabricks.")
        || run.contains("cloud.databricks")
}

/// Replace token-like, host-like and long-number runs, wherever they sit:
/// after `:` or `=`, inside JSON strings, after a tab or inside a URL.
/// The report does not use CLI text at all; this is a second guard for logs.
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if !run.is_empty() {
            out.push_str(if secret_like(run) { "<redacted>" } else { run });
            run.clear();
        }
    };
    for c in text.chars() {
        if token_char(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
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
