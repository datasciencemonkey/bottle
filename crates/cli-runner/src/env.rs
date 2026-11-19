//! The clean child env (W-09).

use std::ffi::OsString;

/// Variables kept on every OS: PATH, HOME, CLI config, proxy and CA variables.
pub const ALLOWED_VARS: &[&str] = &[
    "PATH",
    "HOME",
    "DATABRICKS_CONFIG_FILE",
    // CLI config, not credentials: where U2M tokens live (plaintext or the OS keyring) and the
    // `auth login` callback port. Without the first, a plaintext sign-in looks signed out.
    "DATABRICKS_AUTH_STORAGE",
    "DATABRICKS_OAUTH_CALLBACK_PORT",
    "XDG_CONFIG_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "REQUESTS_CA_BUNDLE",
    "CURL_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
];

/// Extra Windows variables: the home and config folders, and SYSTEMROOT, which
/// the Windows network stack needs.
pub const WINDOWS_ALLOWED_VARS: &[&str] = &["USERPROFILE", "APPDATA", "LOCALAPPDATA", "SYSTEMROOT"];

/// True if `name` may pass into a CLI child. Windows names are case-insensitive.
pub fn is_allowed(name: &str, windows: bool) -> bool {
    if windows {
        ALLOWED_VARS
            .iter()
            .chain(WINDOWS_ALLOWED_VARS)
            .any(|a| a.eq_ignore_ascii_case(name))
    } else {
        ALLOWED_VARS.contains(&name)
    }
}

/// Keeps only allowed variables, for the given OS family.
pub fn clean_env_for(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
    windows: bool,
) -> Vec<(OsString, OsString)> {
    vars.into_iter()
        .filter(|(k, _)| k.to_str().is_some_and(|k| is_allowed(k, windows)))
        .collect()
}

/// Keeps only allowed variables, for the OS this was built for.
pub fn clean_env(
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    clean_env_for(vars, cfg!(windows))
}
