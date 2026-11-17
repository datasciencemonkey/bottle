//! Shared token guards of section 4.6 (#224, ARC-6, MB-02).
//!
//! - Rule 7: [`scrub`], [`scrub_json`] and [`contains_secret`] apply the 7 outbound secret
//!   patterns to IPC payloads, logs and errors.
//! - Rule 5: [`strip_profile_keys`] removes every `profile` key from `mcp_call` tool arguments.
//! - MB-02: [`check_config_json`] rejects a MOG config with a token-like value or a key named
//!   token, password, secret or pat.
//!
//! The matchers are hand-written (no regex crate). They follow the rule-7 patterns exactly;
//! ADR 0013 records the details (ASCII `\w`, how far a match is redacted, `password=` case).

use crate::ipc::JsonObject;
use serde_json::{Map, Value};

/// The 7 outbound secret patterns of 4.6 rule 7, as the requirements write them.
pub const PATTERNS: [&str; 7] = [
    r"eyJ[\w-]+\.",
    "dapi[0-9a-f]{32}",
    "dose[0-9a-f]+",
    "AKIA[0-9A-Z]{16}",
    "ghp_…",
    "-----BEGIN",
    "password=",
];

/// The text that replaces a secret.
pub const REDACTED: &str = "[REDACTED]";

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_lower_hex(b: u8) -> bool {
    b.is_ascii_digit() || (b'a'..=b'f').contains(&b)
}

fn is_upper_alnum(b: u8) -> bool {
    b.is_ascii_digit() || b.is_ascii_uppercase()
}

/// Count the bytes from `i` on that satisfy `f`.
fn run(s: &[u8], i: usize, f: impl Fn(u8) -> bool) -> usize {
    s[i.min(s.len())..].iter().take_while(|&&b| f(b)).count()
}

fn find(s: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    s.get(from..)?
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| from + p)
}

/// A left-to-right matcher over one string.
///
/// `eyj_fail_end` keeps the scan linear: when `eyJ` at `i` is followed by a word run that ends
/// at `e` without a dot, every later `eyJ` whose run starts at or before `e` ends at the same
/// byte and fails too, so it is not scanned again.
struct Matcher<'a> {
    s: &'a [u8],
    eyj_fail_end: usize,
}

impl<'a> Matcher<'a> {
    fn new(s: &'a str) -> Self {
        Self {
            s: s.as_bytes(),
            eyj_fail_end: 0,
        }
    }

    /// If a secret pattern starts at byte `i`, return the end of the span to redact. Call it
    /// with increasing `i`.
    ///
    /// Every pattern starts with an ASCII byte and every span ends after ASCII bytes or at the
    /// end of the string, so the span always lies on `char` boundaries.
    fn match_at(&mut self, i: usize) -> Option<usize> {
        let s = self.s;
        if s[i..].starts_with(b"eyJ") && i + 3 > self.eyj_fail_end {
            // eyJ[\w-]+\.  then the rest of the JWT (base64url segments, dots, padding).
            let n = run(s, i + 3, is_word);
            if n > 0 && s.get(i + 3 + n) == Some(&b'.') {
                let tail = run(s, i + 3 + n, |b| is_word(b) || b == b'.' || b == b'=');
                return Some(i + 3 + n + tail);
            }
            self.eyj_fail_end = i + 3 + n;
        }
        match_other(s, i)
    }
}

/// The patterns other than `eyJ`. A failed check scans a short run only: the byte after each
/// prefix (`p`/`i` of `dapi`, `o`/`s` of `dose`, `_` of `ghp_`) cannot be in the run, and an
/// `AKIA`, `-----BEGIN` or `password=` check that scans far succeeds and ends the span.
fn match_other(s: &[u8], i: usize) -> Option<usize> {
    let rest = &s[i..];
    if rest.starts_with(b"dapi") {
        let n = run(s, i + 4, is_lower_hex);
        if n >= 32 {
            let mut end = i + 4 + n;
            // Databricks PATs can end in `-<digits>`.
            if s.get(end) == Some(&b'-') {
                let d = run(s, end + 1, |b| b.is_ascii_digit());
                if d > 0 {
                    end += 1 + d;
                }
            }
            return Some(end);
        }
    }
    if rest.starts_with(b"dose") {
        let n = run(s, i + 4, is_lower_hex);
        if n > 0 {
            return Some(i + 4 + n);
        }
    }
    if rest.starts_with(b"AKIA") {
        let n = run(s, i + 4, is_upper_alnum);
        if n >= 16 {
            return Some(i + 4 + n);
        }
    }
    if rest.starts_with(b"ghp_") {
        let n = run(s, i + 4, |b| b.is_ascii_alphanumeric());
        if n > 0 {
            return Some(i + 4 + n);
        }
    }
    if rest.starts_with(b"-----BEGIN") {
        // Up to the end of the matching `-----END ...-----` line, or to the end of the text.
        if let Some(end) = find(s, i + 10, b"-----END") {
            return Some(match find(s, end + 8, b"-----") {
                Some(close) => close + 5,
                None => s.len(),
            });
        }
        return Some(s.len());
    }
    if rest.len() >= 9 && rest[..9].eq_ignore_ascii_case(b"password=") {
        let v = run(s, i + 9, |b| {
            !(b.is_ascii_whitespace() || matches!(b, b'&' | b';' | b',' | b'"' | b'\''))
        });
        return Some(i + 9 + v);
    }
    None
}

/// True if `s` contains any rule-7 secret pattern.
pub fn contains_secret(s: &str) -> bool {
    let mut m = Matcher::new(s);
    (0..s.len()).any(|i| m.match_at(i).is_some())
}

/// Replace every rule-7 secret in `s` with [`REDACTED`]. Use it for logs and error text.
pub fn scrub(s: &str) -> String {
    let mut m = Matcher::new(s);
    let mut out = String::with_capacity(s.len());
    let mut copied = 0;
    let mut i = 0;
    while i < s.len() {
        match m.match_at(i) {
            Some(end) => {
                out.push_str(&s[copied..i]);
                out.push_str(REDACTED);
                copied = end;
                i = end.max(i + 1);
            }
            None => i += 1,
        }
    }
    out.push_str(&s[copied..]);
    out
}

/// Scrub every string and every object key in an IPC payload.
pub fn scrub_json(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(scrub(s)),
        Value::Array(a) => Value::Array(a.iter().map(scrub_json).collect()),
        Value::Object(m) => {
            let mut out = Map::new();
            for (k, v) in m {
                let clean = scrub(k);
                // Two keys can scrub to the same text; keep both values.
                let mut key = clean.clone();
                let mut n = 1;
                while out.contains_key(&key) {
                    n += 1;
                    key = format!("{clean}#{n}");
                }
                out.insert(key, scrub_json(v));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

/// True if any string or key in `v` contains a rule-7 secret pattern.
pub fn json_contains_secret(v: &Value) -> bool {
    match v {
        Value::String(s) => contains_secret(s),
        Value::Array(a) => a.iter().any(json_contains_secret),
        Value::Object(m) => m
            .iter()
            .any(|(k, v)| contains_secret(k) || json_contains_secret(v)),
        _ => false,
    }
}

// ---------- rule 5 ----------

fn is_profile_key(k: &str) -> bool {
    k.eq_ignore_ascii_case("profile")
}

fn strip_value(v: &mut Value) -> usize {
    match v {
        Value::Object(m) => strip_map(m),
        Value::Array(a) => a.iter_mut().map(strip_value).sum(),
        _ => 0,
    }
}

fn strip_map(m: &mut Map<String, Value>) -> usize {
    let before = m.len();
    m.retain(|k, _| !is_profile_key(k));
    let removed = before - m.len();
    removed + m.values_mut().map(strip_value).sum::<usize>()
}

/// Remove every `profile` key (any ASCII case, at any depth) from tool arguments (4.6 rule 5).
/// Returns how many keys it removed. The profile of a call comes only from `mcp_call.profile`.
pub fn strip_profile_keys(args: &mut JsonObject) -> usize {
    let before = args.len();
    args.retain(|k, _| !is_profile_key(k));
    let removed = before - args.len();
    removed + args.values_mut().map(strip_value).sum::<usize>()
}

// ---------- MB-02 ----------

/// Why a MOG config was rejected. It carries no key or value, so it can cross IPC (ARC-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSecret {
    /// A key names token, password, secret or pat (see ADR 0013).
    SecretNamedKey,
    /// A key or a value matches a rule-7 secret pattern.
    TokenLikeValue,
}

impl ConfigSecret {
    /// The stable, content-free code.
    pub fn code(&self) -> &'static str {
        match self {
            ConfigSecret::SecretNamedKey => "secret_named_key",
            ConfigSecret::TokenLikeValue => "token_like_value",
        }
    }
}

impl std::fmt::Display for ConfigSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ConfigSecret::SecretNamedKey => "a config key names a secret",
            ConfigSecret::TokenLikeValue => "a config value looks like a token",
        })
    }
}

impl std::error::Error for ConfigSecret {}

const SECRET_KEY_WORDS: [&str; 5] = ["token", "password", "passwd", "secret", "pat"];

/// The words of a key, lowercase. A word ends at a non-alphanumeric character, at a change
/// from lower case to upper case, at a change between letter and digit, and before the last
/// capital of an acronym (`APIToken` is `api`, `token`).
fn key_words(k: &str) -> Vec<String> {
    let chars: Vec<char> = k.chars().collect();
    let mut words = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        if !c.is_alphanumeric() {
            if !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if let Some(&p) = i.checked_sub(1).and_then(|j| chars.get(j)) {
            let next_lower = chars.get(i + 1).is_some_and(|n| n.is_lowercase());
            let boundary = (p.is_lowercase() && c.is_uppercase())
                || (p.is_alphabetic() != c.is_alphabetic() && p.is_alphanumeric())
                || (p.is_uppercase() && c.is_uppercase() && next_lower);
            if boundary && !cur.is_empty() {
                words.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        words.push(cur);
    }
    words
}

/// True if the key names a secret: a word is in [`SECRET_KEY_WORDS`], or the key, run
/// together in lower case, holds `password`, `passwd`, `secret`, or `token` not followed by
/// `s` (so `accesstoken` fails and `maxTokens` passes).
fn names_a_secret(k: &str) -> bool {
    if key_words(k)
        .iter()
        .any(|w| SECRET_KEY_WORDS.contains(&w.as_str()))
    {
        return true;
    }
    let flat: String = k
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if ["password", "passwd", "secret"]
        .iter()
        .any(|w| flat.contains(w))
    {
        return true;
    }
    flat.match_indices("token")
        .any(|(i, _)| flat.as_bytes().get(i + 5) != Some(&b's'))
}

fn check_key(k: &str) -> Result<(), ConfigSecret> {
    if contains_secret(k) {
        return Err(ConfigSecret::TokenLikeValue);
    }
    if names_a_secret(k) {
        return Err(ConfigSecret::SecretNamedKey);
    }
    Ok(())
}

fn check_value(v: &Value) -> Result<(), ConfigSecret> {
    match v {
        Value::String(s) if contains_secret(s) => Err(ConfigSecret::TokenLikeValue),
        Value::Array(a) => a.iter().try_for_each(check_value),
        Value::Object(m) => m.iter().try_for_each(|(k, v)| {
            check_key(k)?;
            check_value(v)
        }),
        _ => Ok(()),
    }
}

/// Check a MOG config for `config_write` (MB-02, 4.5). `MogConfigComponent` calls it (#379).
pub fn check_config_json(json: &JsonObject) -> Result<(), ConfigSecret> {
    json.iter().try_for_each(|(k, v)| {
        check_key(k)?;
        check_value(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_6_rule_7_key_words_split_camel_and_separators() {
        assert_eq!(key_words("accessToken"), ["access", "token"]);
        assert_eq!(key_words("db-password"), ["db", "password"]);
        assert_eq!(key_words("PAT"), ["pat"]);
        assert_eq!(key_words("maxTokens"), ["max", "tokens"]);
        assert_eq!(key_words("APIToken"), ["api", "token"]);
        assert_eq!(key_words("PATValue"), ["pat", "value"]);
        assert_eq!(key_words("password1"), ["password", "1"]);
    }

    #[test]
    fn arc_6_rule_7_password_match_is_case_insensitive() {
        assert!(contains_secret("PassWord=x"));
        assert_eq!(scrub("a Password=x b"), format!("a {REDACTED} b"));
    }
}
