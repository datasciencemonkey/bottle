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
//! - The hub texts ("Not supported (PAT or service principal)", the plaintext-storage warning)
//!   read this list: TODO(#560).
//!
//! The parse keeps only the `[section]` names and the `host` and `auth_type` values. It finds
//! every other key by its name only and never parses its value, which may be a secret. The file
//! is read one line at a time into one buffer that is overwritten with zeros after each line. The name-only key scan, the DEFAULT section
//! rule and the `databricks-cli` filter follow the reference plugin (`config_sections`, `U2M`).
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{compiler_fence, AtomicBool, Ordering};
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
    let mut parser = ConfigParser::new();
    for line in text.split(|&b| b == b'\n') {
        parser.feed_line(line);
    }
    parser.finish()
}

/// The INI parse, one line at a time. It keeps only the section names and the `host` and
/// `auth_type` values; a line is dropped as soon as it is read.
struct ConfigParser {
    out: Vec<ConfigProfile>,
    current: Option<usize>,
    first: bool,
}

impl ConfigParser {
    fn new() -> Self {
        let mut out = Vec::new();
        let current = Some(section(&mut out, "DEFAULT"));
        Self {
            out,
            current,
            first: true,
        }
    }

    fn feed_line(&mut self, line: &[u8]) {
        let line = if std::mem::take(&mut self.first) {
            line.strip_prefix(b"\xef\xbb\xbf").unwrap_or(line)
        } else {
            line
        };
        let line = line.trim_ascii();
        if line.is_empty() || line[0] == b'#' || line[0] == b';' {
            return;
        }
        if line[0] == b'[' {
            let out = &mut self.out;
            self.current = line
                .iter()
                .position(|&b| b == b']')
                .and_then(|end| std::str::from_utf8(&line[1..end]).ok())
                .map(str::trim)
                .filter(|name| !name.is_empty() && *name != SETTINGS_SECTION)
                .map(|name| section(out, name));
            return;
        }
        let (Some(i), Some(split)) = (
            self.current,
            line.iter().position(|&b| b == b'=' || b == b':'),
        ) else {
            return;
        };
        let key = line[..split].trim_ascii();
        // Only these two values are ever parsed. Any other value is skipped unread.
        if key.eq_ignore_ascii_case(b"host") {
            self.out[i].host = value(&line[split + 1..])
                .and_then(bare_host)
                .unwrap_or_default();
        } else if key.eq_ignore_ascii_case(b"auth_type") {
            self.out[i].auth_type = value(&line[split + 1..]).unwrap_or_default();
        }
    }

    fn finish(mut self) -> Vec<ConfigProfile> {
        self.out.retain(|p| !p.host.is_empty());
        self.out
    }
}

/// The longest config line T7 reads. A longer line is skipped (a `host` or `auth_type` line is
/// short). The line buffer has this fixed capacity, so it never grows and leaves no copy behind.
pub const MAX_LINE: usize = 4096;

/// Overwrite `buf` with zeros.
fn wipe(buf: &mut [u8]) {
    buf.fill(0);
    compiler_fence(Ordering::SeqCst);
    std::hint::black_box(buf);
}

/// Read `reader` one byte at a time into one reused line buffer, parse each line, and overwrite
/// the buffer with zeros after each line. The whole text is never in memory: at most one line.
/// Use an unbuffered reader (a `std::fs::File`): a `BufReader` keeps many lines that it never
/// wipes.
pub fn parse_config_lines(reader: &mut dyn std::io::Read) -> std::io::Result<Vec<ConfigProfile>> {
    let mut parser = ConfigParser::new();
    let mut line = [0u8; MAX_LINE];
    let mut len = 0usize;
    let mut too_long = false;
    let mut byte = [0u8; 1];
    let result = loop {
        match reader.read(&mut byte) {
            Ok(0) => break Ok(()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => break Err(e),
        }
        if byte[0] == b'\n' {
            if !too_long {
                parser.feed_line(&line[..len]);
            }
            wipe(&mut line[..len]);
            len = 0;
            too_long = false;
        } else if len < MAX_LINE {
            line[len] = byte[0];
            len += 1;
        } else {
            too_long = true;
        }
    };
    if result.is_ok() && !too_long {
        parser.feed_line(&line[..len]);
    }
    wipe(&mut line);
    wipe(&mut byte);
    result.map(|()| parser.finish())
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
///
/// It reads one line at a time into one reused buffer that it zeroes after each line
/// ([`parse_config_lines`]); the whole file is never in memory. A secret line passes through that
/// buffer only, and no secret value is parsed or kept. TODO(#17): R5 runs the CLI now (#16), so list
/// through `databricks auth profiles --skip-validate --output json` instead.
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
        let unreadable = CoreError::Unavailable {
            component: ID,
            reason: "profile_config_unreadable",
        };
        let mut file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(_) => return Err(unreadable),
        };
        parse_config_lines(&mut file).map_err(|_| unreadable)
    }
}

/// The token storage verdict of one `databricks-cli` profile (W-10a).
enum Storage {
    /// `token_storage.mode` is `secure` (the OS key store).
    Secure,
    /// Plaintext, or no mode (an older CLI keeps tokens in a plaintext file). It blocks the
    /// profile, and `cli_status` reports `plaintextStorage: true`.
    Plaintext,
    /// The check failed (for example a refused CLI or a failed exit). It blocks the profile, but
    /// it is no evidence of plaintext storage.
    Failed,
    /// The R5 is a component-test double (`StubCliRunner`) that answers `NotImplemented`. The
    /// shipped R5 (`DatabricksCliRunner`, #16) never does, so the app never takes this arm.
    /// Through the stub no token can flow either (R6 uses the same R5).
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
                Storage::Plaintext
            }
        }
        Err(CoreError::NotImplemented { .. }) => Storage::Unchecked,
        Err(_) => Storage::Failed,
    }
}

/// T7 host side: the profile list over the CLI config file and R5.
pub struct ProfileService {
    config: Arc<dyn ConfigSource>,
    cli: Arc<dyn CliRunner>,
    cache: Mutex<Option<Vec<Profile>>>,
    /// W-10a: the last profile check found plaintext token storage (`cli_status`).
    plaintext: AtomicBool,
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
            plaintext: AtomicBool::new(false),
        }
    }

    /// Every profile with its `supported` verdict. The first call and `refresh` read the config
    /// file and check the storage again; other calls return the last list.
    pub fn list(&self, refresh: bool) -> CoreResult<Vec<Profile>> {
        if !refresh {
            if let Some(list) = self
                .cache
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .as_ref()
            {
                return Ok(list.clone());
            }
        }
        // The cache lock is not held while R5 runs `auth describe`, so R7 and R8 calls that read
        // the last list do not wait behind a refresh.
        let mut plaintext = false;
        let list: Vec<Profile> = self
            .config
            .profiles()?
            .into_iter()
            .map(|p| {
                let supported = p.auth_type == U2M_AUTH_TYPE
                    && match storage_of(self.cli.as_ref(), &p.name) {
                        Storage::Secure | Storage::Unchecked => true,
                        Storage::Plaintext => {
                            plaintext = true;
                            false
                        }
                        Storage::Failed => false,
                    };
                Profile {
                    name: p.name,
                    host: p.host,
                    auth_type: p.auth_type,
                    supported,
                    workspace_id: None,
                }
            })
            .collect();
        self.plaintext.store(plaintext, Ordering::SeqCst);
        *self.cache.lock().unwrap_or_else(|p| p.into_inner()) = Some(list.clone());
        Ok(list)
    }
}

impl crate::cli_status::PlaintextStorage for ProfileService {
    /// True when the last profile check found a `databricks-cli` profile with plaintext token
    /// storage (W-10a). False before the first check. It runs no CLI child.
    fn plaintext_storage(&self) -> bool {
        self.plaintext.load(Ordering::SeqCst)
    }
}

impl ProfileHosts for ProfileService {
    /// The host of a supported profile only (W-03b, W-10a).
    fn workspace_url(&self, profile: &str) -> CoreResult<String> {
        let mut list = self.list(false)?;
        if !list.iter().any(|p| p.name == profile) {
            // A profile added after the last list (for example by a sign-in): read again once.
            list = self.list(true)?;
        }
        list.into_iter()
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
