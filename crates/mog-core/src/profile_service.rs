//! T7 ProfileService, host side (#17, ADR 0094): the `profiles_list` reply, the auth-type
//! filter, the token storage check and the R7 profile hosts.
//!
//! - W-01a: the Databricks CLI is the only auth source. T7 reads the CLI's own config file for
//!   the profile names, hosts and auth types, and asks R5 for `databricks auth describe`. It
//!   never reads a token, the CLI token cache or another credential source.
//! - W-03a, W-03b: only an `auth_type = databricks-cli` profile is `supported`. PAT and
//!   service-principal profiles are listed with `supported: false`, and
//!   [`ProfileHosts::workspace_url`] refuses them, so they never reach Genie.
//! - W-08: T7 never writes the config file or any other file.
//! - W-10a, W-10b: a `databricks-cli` profile whose `auth describe` shows plaintext token
//!   storage (or no storage mode) is not `supported` until the storage changes. A refresh checks
//!   it again.
//! - T7 never picks a profile, not even when only one exists. The user picks (#163).
//!
//! The parse keeps only the `[section]` names and the `host` and `auth_type` values. It finds
//! every other key by its name only and never parses its value, which may be a secret. The file
//! bytes are overwritten with zeros after the parse. The name-only key scan, the DEFAULT section
//! rule and the `databricks-cli` filter follow the reference plugin (`config_sections`, `U2M`).
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{compiler_fence, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;

use crate::cli_runner::CliRunner;
use crate::error::{CoreError, CoreResult};
use crate::http_transport::ProfileHosts;
use crate::ipc::ProfilesListArgs;
use crate::ipc_gateway::GatewayComponents;
use crate::types::Profile;

/// The auth type that `databricks auth login` writes: OAuth U2M, the only one Bottle uses.
pub const U2M_AUTH_TYPE: &str = "databricks-cli";
/// The CLI's own settings section (for example `default_profile`). It is not a profile.
const SETTINGS_SECTION: &str = "__settings__";
/// The `token_storage.mode` of `auth describe` for the OS key store.
const SECURE_STORAGE: &str = "secure";
/// [`ProfileHosts::workspace_url`] for a profile that is not listed or not supported.
pub const PROFILE_NOT_SUPPORTED: &str = "profile_not_supported";
const ID: &str = "T7";

/// One profile of the CLI config file: only the fields T7 needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigProfile {
    /// The section name, which is the profile name.
    pub name: String,
    /// The bare `https://host[:port]` of the `host` key.
    pub host: String,
    /// The `auth_type` value, or `""` when the section has none (often a PAT).
    pub auth_type: String,
}

/// Parse the CLI config text (INI, as the CLI reads it). Keys before the first header belong to
/// `DEFAULT`. A key ends at the first `=` or `:`. A repeated section merges into the first one.
/// `[__settings__]` and a section with no `host` are not profiles.
pub fn parse_config(text: &[u8]) -> Vec<ConfigProfile> {
    let text = text.strip_prefix(b"\xef\xbb\xbf").unwrap_or(text);
    let mut out: Vec<ConfigProfile> = Vec::new();
    let mut current = Some(section(&mut out, "DEFAULT"));
    for line in text.split(|&b| b == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() || line[0] == b'#' || line[0] == b';' {
            continue;
        }
        if line[0] == b'[' {
            current = line
                .iter()
                .position(|&b| b == b']')
                .and_then(|end| std::str::from_utf8(&line[1..end]).ok())
                .map(str::trim)
                .filter(|name| !name.is_empty() && *name != SETTINGS_SECTION)
                .map(|name| section(&mut out, name));
            continue;
        }
        let (Some(i), Some(split)) = (current, line.iter().position(|&b| b == b'=' || b == b':'))
        else {
            continue;
        };
        let key = line[..split].trim_ascii();
        // Only these two values are ever parsed. Any other value is skipped unread.
        if key.eq_ignore_ascii_case(b"host") {
            out[i].host = value(&line[split + 1..])
                .and_then(bare_host)
                .unwrap_or_default();
        } else if key.eq_ignore_ascii_case(b"auth_type") {
            out[i].auth_type = value(&line[split + 1..]).unwrap_or_default();
        }
    }
    out.retain(|p| !p.host.is_empty());
    out
}

/// The index of section `name` in `out`, added if new.
fn section(out: &mut Vec<ConfigProfile>, name: &str) -> usize {
    if let Some(i) = out.iter().position(|p| p.name == name) {
        return i;
    }
    out.push(ConfigProfile {
        name: name.to_owned(),
        host: String::new(),
        auth_type: String::new(),
    });
    out.len() - 1
}

/// A `host` or `auth_type` value: its first word, without quotes.
fn value(raw: &[u8]) -> Option<String> {
    let word = raw.trim_ascii().split(|b| b.is_ascii_whitespace()).next()?;
    let word = word
        .strip_prefix(b"\"")
        .and_then(|w| w.strip_suffix(b"\""))
        .unwrap_or(word);
    std::str::from_utf8(word)
        .ok()
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
}

/// The bare `https://host[:port]` of a `host` value (the CLI adds `https://` when it is missing).
fn bare_host(raw: String) -> Option<String> {
    let (scheme, rest) = match raw.split_once("://") {
        Some((s, r)) => (s.to_ascii_lowercase(), r),
        None => ("https".to_owned(), raw.as_str()),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    (!authority.is_empty()).then(|| format!("{scheme}://{authority}"))
}

/// The CLI config file: `~/.databrickscfg`, under `USERPROFILE` on Windows and `HOME` elsewhere.
/// Bottle reads only the file that the CLI reads with the clean environment of R5, so
/// `DATABRICKS_CONFIG_FILE` is not used. `None` when the home is not set.
pub fn config_path(env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = if cfg!(windows) {
        env("USERPROFILE")
    } else {
        env("HOME")
    }?;
    Some(PathBuf::from(home).join(".databrickscfg"))
}

/// The one storage check command (W-10a). It never asks for `--sensitive`.
pub fn describe_args(profile: &str) -> [&str; 6] {
    ["auth", "describe", "--profile", profile, "--output", "json"]
}

/// Where the CLI profiles come from.
pub trait ConfigSource: Send + Sync {
    /// The profiles, or none when the CLI has no config file.
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>>;
}

/// The CLI config file at a path. It is only read, never written (W-08).
#[derive(Debug, Clone)]
pub struct ConfigFile {
    path: Option<PathBuf>,
}

impl ConfigFile {
    /// The file at `path`; `None` lists no profiles.
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }

    /// The CLI's own config file for this user ([`config_path`]).
    pub fn of_user() -> Self {
        Self::new(config_path(|k| std::env::var_os(k)))
    }
}

impl ConfigSource for ConfigFile {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        let Some(path) = &self.path else {
            return Ok(Vec::new());
        };
        let mut bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => {
                return Err(CoreError::Unavailable {
                    component: ID,
                    reason: "profile_config_unreadable",
                })
            }
        };
        let profiles = parse_config(&bytes);
        // The file can hold secrets: overwrite the copy with zeros now.
        bytes.fill(0);
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut bytes);
        Ok(profiles)
    }
}

/// The token storage verdict of one `databricks-cli` profile (W-10a).
enum Storage {
    /// `token_storage.mode` is `secure` (the OS key store).
    Secure,
    /// Plaintext, no mode (an older CLI keeps tokens in a plaintext file) or a failed check.
    Blocked,
    /// R5 is not built, so the check cannot run. TODO(#16): R5 runs the CLI.
    Unchecked,
}

fn storage_of(cli: &dyn CliRunner, profile: &str) -> Storage {
    match cli.run_json(&describe_args(profile)) {
        Ok(reply) => {
            // Read only `token_storage.mode`; the rest of the reply is dropped.
            let mode = [
                "/details/token_storage/mode",
                "/token_storage/mode",
                "/status/token_storage/mode",
            ]
            .iter()
            .find_map(|p| reply.pointer(p).and_then(Value::as_str));
            if mode == Some(SECURE_STORAGE) {
                Storage::Secure
            } else {
                Storage::Blocked
            }
        }
        Err(CoreError::NotImplemented { .. }) => Storage::Unchecked,
        Err(_) => Storage::Blocked,
    }
}

/// T7 host side: the profile list over the CLI config file and R5.
pub struct ProfileService {
    config: Arc<dyn ConfigSource>,
    cli: Arc<dyn CliRunner>,
    cache: Mutex<Option<Vec<Profile>>>,
}

impl std::fmt::Debug for ProfileService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileService").finish_non_exhaustive()
    }
}

impl ProfileService {
    pub fn new(config: Arc<dyn ConfigSource>, cli: Arc<dyn CliRunner>) -> Self {
        Self {
            config,
            cli,
            cache: Mutex::new(None),
        }
    }

    /// Every profile with its `supported` verdict. The first call and `refresh` read the config
    /// file and check the storage again; other calls return the last list.
    pub fn list(&self, refresh: bool) -> CoreResult<Vec<Profile>> {
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        if let (false, Some(list)) = (refresh, cache.as_ref()) {
            return Ok(list.clone());
        }
        let list: Vec<Profile> = self
            .config
            .profiles()?
            .into_iter()
            .map(|p| {
                let supported = p.auth_type == U2M_AUTH_TYPE
                    && !matches!(storage_of(self.cli.as_ref(), &p.name), Storage::Blocked);
                Profile {
                    name: p.name,
                    host: p.host,
                    auth_type: p.auth_type,
                    supported,
                    workspace_id: None,
                }
            })
            .collect();
        *cache = Some(list.clone());
        Ok(list)
    }
}

impl ProfileHosts for ProfileService {
    /// The host of a supported profile only (W-03b, W-10a).
    fn workspace_url(&self, profile: &str) -> CoreResult<String> {
        self.list(false)?
            .into_iter()
            .find(|p| p.name == profile && p.supported)
            .map(|p| p.host)
            .ok_or(CoreError::Unavailable {
                component: ID,
                reason: PROFILE_NOT_SUPPORTED,
            })
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

impl GatewayComponents for ProfileService {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        if command != "profiles_list" {
            return Err(refused("not_implemented"));
        }
        let a: ProfilesListArgs =
            serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        let list = self
            .list(a.refresh.unwrap_or(false))
            .map_err(|_| refused("host_failed"))?;
        serde_json::to_value(list).map_err(|_| refused("host_failed"))
    }
}
