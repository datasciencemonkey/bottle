//! The R10 secret check of a memory write (local memory plan 6.2, ADR 0113).
//!
//! A memory must never hold a token, a key or a password: the store is on disk, and a later
//! slice sends memories to a model. So R10 refuses the whole write (`secret_refused`) when the
//! title, the summary or the body holds a secret. R10 stores nothing of the text, and the refusal
//! holds no text.
//!
//! The check is the 7 outbound patterns of 4.6 rule 7 ([`crate::token_guard::contains_secret`])
//! plus the memory patterns below. It runs on the raw text and on a folded form (NFKC, no
//! zero-width code points), so a full-width or zero-width form of a token is found too.

use std::sync::OnceLock;

use regex::Regex;

use crate::token_guard::contains_secret;

/// The memory patterns that rule 7 does not have. Each one is a fixed-shape credential, so a
/// match in normal prose is rare. The tests use fake values only.
const EXTRA: &[(&str, &str)] = &[
    // `password: x`, `passwd = x`, `pwd=x` (rule 7 has only `password=`).
    (
        "password_form",
        r"(?i)\b(?:password|passwd|pwd|passphrase)\s*[:=]\s*\S",
    ),
    // GitHub tokens: classic `ghp_` and the other prefixes, and fine-grained `github_pat_`.
    (
        "github_token",
        r"\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})",
    ),
    // Slack tokens.
    ("slack_token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
    // OpenAI and Anthropic style API keys.
    ("api_key", r"\bsk-(?:ant-|proj-)?[A-Za-z0-9_-]{20,}"),
    // Google API keys.
    ("google_key", r"\bAIza[0-9A-Za-z_-]{35}"),
    // An AWS secret key named in the text (the access key id is in rule 7).
    (
        "aws_secret",
        r"(?i)\baws_secret_access_key\s*[:=]?\s*[A-Za-z0-9/+=]{20,}",
    ),
    // AWS temporary access key ids (rule 7 has `AKIA`).
    ("aws_temp_key", r"\bASIA[0-9A-Z]{16}"),
    // A Databricks OAuth secret (`dose` is in rule 7) or an Azure-style `client_secret=`.
    ("client_secret", r"(?i)\bclient[_-]?secret\s*[:=]\s*\S{8,}"),
];

fn extra() -> &'static [Regex] {
    static P: OnceLock<Vec<Regex>> = OnceLock::new();
    P.get_or_init(|| {
        EXTRA
            .iter()
            // The sources are constants; a test compiles each one.
            .filter_map(|(_, src)| Regex::new(src).ok())
            .collect()
    })
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{2060}'..='\u{2064}' | '\u{feff}' | '\u{00ad}')
}

/// NFKC, then drop zero-width code points. Case and white space stay: the token patterns are
/// case-sensitive.
fn fold(text: &str) -> String {
    icu_normalizer::ComposingNormalizerBorrowed::new_nfkc()
        .normalize(text)
        .chars()
        .filter(|c| !is_zero_width(*c))
        .collect()
}

fn matches(text: &str) -> bool {
    contains_secret(text) || extra().iter().any(|re| re.is_match(text))
}

/// True if `text` holds a token, a key or a password. R10 then refuses the write.
pub fn has_secret(text: &str) -> bool {
    if matches(text) {
        return true;
    }
    let folded = fold(text);
    folded != text && matches(&folded)
}

/// True if any of `texts` holds a secret.
pub fn any_secret<'a>(texts: impl IntoIterator<Item = &'a str>) -> bool {
    texts.into_iter().any(has_secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Fake values only: each one has the shape of a credential and is not one.
    // Built at run time, so no key-shaped literal is in the source.
    fn fake_aws(prefix: &str) -> String {
        format!("{prefix}{}", "FAKE".repeat(4))
    }

    fn fake_dapi() -> String {
        format!("dapi{}", "0123456789abcdef".repeat(2))
    }

    #[test]
    fn every_extra_pattern_compiles() {
        assert_eq!(extra().len(), EXTRA.len());
        for (code, src) in EXTRA {
            assert!(Regex::new(src).is_ok(), "{code}");
        }
    }

    #[test]
    fn ac_1_5_a_databricks_token_is_a_secret() {
        assert!(has_secret(&format!("my token is {}", fake_dapi())));
    }

    #[test]
    fn rule_7_forms_are_secrets() {
        for t in [
            "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ4In0.",
            fake_aws("AKIA").as_str(),
            "-----BEGIN PRIVATE KEY-----",
            "password=hunter2",
            "doseabcdef0123",
        ] {
            assert!(has_secret(t), "{t}");
        }
    }

    #[test]
    fn memory_forms_are_secrets() {
        let ghp = format!("ghp_{}", "A1b2C3d4E5".repeat(3));
        let pat = format!("github_pat_{}", "A1b2C3d4E5".repeat(3));
        let slack = format!("xoxb-{}", "1234567890-abc");
        let sk = format!("sk-ant-{}", "FAKEfake0123".repeat(2));
        let google = format!("AIza{}", "F".repeat(35));
        let aws = format!("aws_secret_access_key = {}", "FAKE/fake+".repeat(3));
        for t in [
            "Password: hunter2",
            "the pwd = x9",
            ghp.as_str(),
            pat.as_str(),
            slack.as_str(),
            sk.as_str(),
            google.as_str(),
            aws.as_str(),
            fake_aws("ASIA").as_str(),
            "client_secret=FAKEFAKEFAKE",
        ] {
            assert!(has_secret(t), "{t}");
        }
    }

    #[test]
    fn folded_forms_are_found() {
        // Full-width `ｄａｐｉ` and a zero-width space inside the token.
        let full_width: String = fake_dapi()
            .chars()
            .map(|c| char::from_u32(c as u32 + 0xfee0).unwrap_or(c))
            .collect();
        assert!(has_secret(&full_width));
        let zw = fake_dapi().replacen("dapi", "da\u{200b}pi", 1);
        assert!(has_secret(&zw));
    }

    #[test]
    fn normal_prose_is_not_a_secret() {
        for t in [
            "The sales mart refreshes at 06:00 UTC.",
            "An active user logged in at least once this month.",
            "Reset your password in the portal if it expires.",
            "Ask Pat which skew of the task queue is in use.",
            "The API key rotation happens every quarter.",
            "Revenue for Q3 was 4.2 M EUR, up 12 %.",
        ] {
            assert!(!has_secret(t), "{t}");
        }
    }

    #[test]
    fn any_secret_checks_each_field() {
        assert!(any_secret(["a title", "a summary", "pwd: x"]));
        assert!(!any_secret(["a title", "a summary", "a body"]));
    }
}
