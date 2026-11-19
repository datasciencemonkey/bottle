use std::ffi::OsString;

use cli_runner::{clean_env, clean_env_for, is_allowed};

fn polluted() -> Vec<(OsString, OsString)> {
    [
        ("PATH", "/usr/bin"),
        ("HOME", "/home/someone"),
        ("DATABRICKS_CONFIG_FILE", "/home/someone/.databrickscfg"),
        ("HTTPS_PROXY", "http://proxy.example.invalid:3128"),
        ("no_proxy", "localhost"),
        ("SSL_CERT_FILE", "/etc/ssl/ca.pem"),
        ("REQUESTS_CA_BUNDLE", "/etc/ssl/ca.pem"),
        // Fake values only: they must be dropped.
        ("DATABRICKS_TOKEN", "test-token-not-real"),
        ("DATABRICKS_HOST", "https://fake.example.invalid"),
        ("DATABRICKS_CLIENT_ID", "fake-client-id"),
        ("DATABRICKS_CLIENT_SECRET", "fake-client-secret"),
        ("DATABRICKS_AUTH_TYPE", "pat"),
        ("DATABRICKS_CONFIG_PROFILE", "other"),
        ("AWS_SECRET_ACCESS_KEY", "fake"),
        ("LD_PRELOAD", "/fake/evil.so"),
        ("USERPROFILE", "C:\\Users\\someone"),
    ]
    .iter()
    .map(|(k, v)| (OsString::from(k), OsString::from(v)))
    .collect()
}

fn names(v: &[(OsString, OsString)]) -> Vec<String> {
    v.iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect()
}

/// [W-09] drops DATABRICKS_TOKEN, DATABRICKS_HOST, DATABRICKS_CLIENT_* and DATABRICKS_AUTH_TYPE
#[test]
fn w_09_drops_databricks_auth_vars() {
    for windows in [false, true] {
        let out = names(&clean_env_for(polluted(), windows));
        for bad in [
            "DATABRICKS_TOKEN",
            "DATABRICKS_HOST",
            "DATABRICKS_CLIENT_ID",
            "DATABRICKS_CLIENT_SECRET",
            "DATABRICKS_AUTH_TYPE",
            "DATABRICKS_CONFIG_PROFILE",
            "AWS_SECRET_ACCESS_KEY",
            "LD_PRELOAD",
        ] {
            assert!(
                !out.contains(&bad.to_string()),
                "{bad} kept (windows={windows})"
            );
        }
        let joined = format!("{:?}", clean_env_for(polluted(), windows));
        assert!(
            !joined.contains("test-token-not-real"),
            "token value leaked"
        );
    }
}

/// [W-09] keeps only PATH, HOME, config, proxy and CA variables (unix branch)
#[test]
fn w_09_keeps_only_allowed_vars_unix() {
    let out = names(&clean_env_for(polluted(), false));
    assert_eq!(
        out,
        vec![
            "PATH",
            "HOME",
            "DATABRICKS_CONFIG_FILE",
            "HTTPS_PROXY",
            "no_proxy",
            "SSL_CERT_FILE",
            "REQUESTS_CA_BUNDLE",
        ]
    );
    let kept = clean_env_for(polluted(), false);
    assert_eq!(
        kept[0].1,
        OsString::from("/usr/bin"),
        "values are kept as-is"
    );
}

/// [W-09] Windows branch: case-insensitive names, plus home/config folders and SYSTEMROOT
#[test]
fn w_09_keeps_only_allowed_vars_windows() {
    let vars: Vec<(OsString, OsString)> = [
        ("Path", "C:\\Windows"),
        ("SystemRoot", "C:\\Windows"),
        ("APPDATA", "C:\\Users\\someone\\AppData\\Roaming"),
        ("Databricks_Token", "fake"),
        ("databricks_host", "fake"),
        ("TEMP", "C:\\Temp"),
    ]
    .iter()
    .map(|(k, v)| (OsString::from(k), OsString::from(v)))
    .collect();
    let out = names(&clean_env_for(vars, true));
    assert_eq!(out, vec!["Path", "SystemRoot", "APPDATA"]);

    assert!(is_allowed("USERPROFILE", true));
    assert!(!is_allowed("USERPROFILE", false));
    assert!(!is_allowed("Path", false), "unix names are case-sensitive");
    assert!(is_allowed("https_proxy", false));
    assert!(!is_allowed("DATABRICKS_CLIENT_SECRET", true));
}

/// [W-09] clean_env uses the branch for the OS it was built for
#[test]
fn w_09_clean_env_matches_build_os() {
    assert_eq!(
        clean_env(polluted()),
        clean_env_for(polluted(), cfg!(windows))
    );
}

/// [W-09] keeps the CLI's own config variables that pick where U2M tokens live and the
/// `auth login` callback port. Dropping `DATABRICKS_AUTH_STORAGE` makes a sign-in kept in
/// plaintext storage look like "not signed in" (found live in antigravity-bricks-plugin #33),
/// which would hide the W-10a plaintext warning behind a wrong sign-in prompt.
#[test]
fn w_09_keeps_the_cli_token_storage_and_login_callback_config() {
    for windows in [false, true] {
        let mut env = polluted();
        env.push(("DATABRICKS_AUTH_STORAGE".into(), "plaintext".into()));
        env.push(("DATABRICKS_OAUTH_CALLBACK_PORT".into(), "8020".into()));
        let out = names(&clean_env_for(env, windows));
        for keep in ["DATABRICKS_AUTH_STORAGE", "DATABRICKS_OAUTH_CALLBACK_PORT"] {
            assert!(
                out.iter().any(|n| n == keep),
                "windows={windows}: {keep} dropped: {out:?}"
            );
        }
        for drop in [
            "DATABRICKS_TOKEN",
            "DATABRICKS_HOST",
            "DATABRICKS_AUTH_TYPE",
            "DATABRICKS_CONFIG_PROFILE",
        ] {
            assert!(
                !out.iter().any(|n| n == drop),
                "windows={windows}: {drop} kept"
            );
        }
    }
}
