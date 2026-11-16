//! The clean environment of the harness runtime process (ARC-7, RTR-28).
//!
//! The harness runs outside the secret boundary. It gets an allowlist of non-secret variables
//! from the Bottle process, plus the loopback base URL of its adapter. It never gets a token, a
//! credential, a `DATABRICKS_*` variable or a model API key. The per-session secret goes to the
//! runtime on stdin in the session frame, not in the environment.

/// The variables that the harness may inherit. They locate the user's tools and temp folders;
/// none of them holds a credential.
pub const INHERIT: [&str; 14] = [
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "SHELL",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TEMP",
    "TMP",
    // Windows needs these to start a process at all.
    "USERPROFILE",
    "SYSTEMROOT",
    "COMSPEC",
    "PATHEXT",
];

/// True if `name` is on the inherit allowlist (Windows names are case-insensitive).
fn inherited(name: &str) -> bool {
    INHERIT.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// True if a value looks like a credential. An allowlisted variable with such a value is dropped.
fn looks_like_secret(value: &str) -> bool {
    let v = value.trim();
    v.starts_with("dapi") || v.starts_with("dose") || v.starts_with("eyJ") || v.starts_with("sk-")
}

/// Build the harness environment from `parent` (the Bottle process environment) and `extra`
/// (the adapter's base URL). Only allowlisted names pass; `extra` wins over `parent`.
pub fn harness_env<I>(parent: I, extra: &[(&str, String)]) -> Vec<(String, String)>
where
    I: IntoIterator<Item = (String, String)>,
{
    let mut env: Vec<(String, String)> = parent
        .into_iter()
        .filter(|(k, v)| inherited(k) && !looks_like_secret(v))
        .filter(|(k, _)| !extra.iter().any(|(e, _)| e.eq_ignore_ascii_case(k)))
        .collect();
    env.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), v.clone())));
    env
}

/// [`harness_env`] over the current process environment. Non-UTF-8 entries are dropped.
pub fn harness_env_from_process(extra: &[(&str, String)]) -> Vec<(String, String)> {
    let parent = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)));
    harness_env(parent, extra)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kv(k: &str, v: &str) -> (String, String) {
        (k.to_owned(), v.to_owned())
    }

    #[test]
    fn rtr_28_harness_env_keeps_only_the_allowlist_and_the_base_url() {
        let parent = vec![
            kv("PATH", "/usr/bin"),
            kv("HOME", "/Users/u"),
            kv("DATABRICKS_TOKEN", "dapi123"),
            kv("DATABRICKS_CONFIG_PROFILE", "bottle-home"),
            kv("ANTHROPIC_API_KEY", "sk-ant-1"),
            kv("ANTHROPIC_BASE_URL", "https://elsewhere.example"),
            kv("AWS_SECRET_ACCESS_KEY", "x"),
            kv("SSH_AUTH_SOCK", "/tmp/agent.sock"),
        ];
        let env = harness_env(
            parent,
            &[("ANTHROPIC_BASE_URL", "http://127.0.0.1:9/anthropic".into())],
        );
        assert_eq!(
            env,
            vec![
                kv("PATH", "/usr/bin"),
                kv("HOME", "/Users/u"),
                kv("ANTHROPIC_BASE_URL", "http://127.0.0.1:9/anthropic"),
            ]
        );
    }

    #[test]
    fn rtr_28_allowlisted_name_with_a_token_value_is_dropped() {
        let env = harness_env(vec![kv("TMPDIR", "dapi-oops"), kv("LANG", "C")], &[]);
        assert_eq!(env, vec![kv("LANG", "C")]);
    }
}
