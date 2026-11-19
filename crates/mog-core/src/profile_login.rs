//! "Connect a workspace", host side (#162): `profile_login` and `profile_login_cancel`.
//!
//! - W-07a, W-07b, W-07g: the caller passes the URL that the user pasted. [`workspace_host`]
//!   keeps only `https://<host>`, and refuses a host outside the Databricks domain list of 4.6
//!   rule 2 (`host_refused`). The app never guesses a URL.
//! - W-07g: the profile name must be a plain name that does not exist in the CLI config file
//!   and has no sign-in running (`profile_exists`). [`proposed_profile_name`] is the
//!   `bottle-<workspace>` proposal; the brain shows it and the user can change it.
//! - W-07c, W-15: R5 runs `databricks auth login --host <h> --profile <p>` (with an optional
//!   `--client-id <id> --scopes all-apis,offline_access`) for at most [`LOGIN_TIMEOUT`]. The
//!   call blocks until the end (the contract owner's choice for #162).
//! - W-07h: `profile_login_cancel {profile}` kills the running child (`login_cancelled`). After
//!   the sign-in, the [`LoginVerifier`] runs the W-06a check (the SCIM `workspace_id`) before
//!   the profile connects. A failure answers one fixed Table W-ERR code, never upstream text.
//! - W-07i, W-08: the CLI writes only the new `bottle-*` section. Bottle never writes the config
//!   file, and an existing PAT or service-principal profile is never used or changed.
//!
//! No token, host, profile or CLI text goes into a reply, an error or a log: the CLI keeps its
//! tokens in its own cache, and R6 gets them later with `databricks auth token` (ARC-6).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use crate::cli_runner::CliRunner;
use crate::error::{CoreError, CoreResult};
use crate::http_transport::host_policy::is_workspace_host;
use crate::ipc::{ProfileArgs, ProfileLoginArgs};
use crate::ipc_gateway::GatewayComponents;
use crate::profile_service::ConfigSource;

/// W-07c: `databricks auth login` times out after 5 min.
pub const LOGIN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// W-15: the scopes of a custom public OAuth app.
pub const LOGIN_SCOPES: &str = "all-apis,offline_access";
/// The prefix of every profile that Bottle creates (B0, U16).
pub const PROFILE_PREFIX: &str = "bottle-";
/// The longest profile name or client ID that Bottle passes to the CLI.
const MAX_NAME: usize = 64;

/// The `https://<host>` of a pasted workspace URL, or `None` if the host is not a Databricks
/// workspace host (W-07b). A URL with no scheme is read as `https://`; any other scheme, a user
/// name, a port other than 443 or an IP address is refused.
pub fn workspace_host(pasted: &str) -> Option<String> {
    let text = pasted.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let with_scheme = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let url = reqwest::Url::parse(&with_scheme).ok()?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    if url.port().is_some_and(|p| p != 443) {
        return None;
    }
    // `domain()` is `None` for an IP address. `Url` gives the host in lower case.
    let host = url.domain()?.to_owned();
    is_workspace_host(&host).then(|| format!("https://{host}"))
}

/// W-07g: the proposed name `bottle-<workspace>`, from the first label of `host`.
pub fn proposed_profile_name(host: &str) -> String {
    let bare = host.strip_prefix("https://").unwrap_or(host);
    let label = bare.split('.').next().unwrap_or_default();
    format!("{PROFILE_PREFIX}{label}")
}

/// True for a name or ID the CLI gets as a flag value: `[A-Za-z0-9._-]`, at most 64 characters,
/// not empty and not starting with `-` (so it is never read as a flag).
fn plain_value(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= MAX_NAME
        && !v.starts_with('-')
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// The `databricks auth login` arguments (W-07c), with the custom OAuth client of W-15.
pub fn login_args<'a>(host: &'a str, profile: &'a str, client_id: Option<&'a str>) -> Vec<&'a str> {
    let mut args = vec!["auth", "login", "--host", host, "--profile", profile];
    if let Some(id) = client_id {
        args.extend(["--client-id", id, "--scopes", LOGIN_SCOPES]);
    }
    args
}

/// The W-06a check after the sign-in: the new profile answers SCIM `Me` with a workspace ID.
/// The app's implementation asks R6 (`TokenBroker::workspace_id`). The Genie One probe has no
/// host seam yet, so it is not part of this check (a known MVP gap, stated on PR for #162).
pub trait LoginVerifier: Send + Sync {
    fn verify(&self, profile: &str) -> CoreResult<()>;
}

/// The `profile_login` and `profile_login_cancel` component.
pub struct ProfileLogin {
    config: Arc<dyn ConfigSource>,
    cli: Arc<dyn CliRunner>,
    verifier: Arc<dyn LoginVerifier>,
    /// The running sign-ins, by profile name, with their cancel flags.
    running: Mutex<HashMap<String, Arc<AtomicBool>>>,
    next_id: AtomicU64,
}

impl std::fmt::Debug for ProfileLogin {
    /// Shows nothing: no profile or host.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileLogin").finish_non_exhaustive()
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

/// The fixed IPC code for an R5 failure of `auth login`.
fn cli_code(e: &CoreError) -> &'static str {
    match e {
        CoreError::Unavailable {
            reason: "cli_timed_out",
            ..
        } => "timeout",
        CoreError::Unavailable {
            reason: "cli_cancelled",
            ..
        } => "login_cancelled",
        _ => "cli_failed",
    }
}

/// The Table W-ERR code for a failed W-06a check. Upstream text is never read here.
fn verify_code(e: &CoreError) -> &'static str {
    let CoreError::Unavailable { reason, .. } = e else {
        return "host_failed";
    };
    match *reason {
        "auth_expired" | "login_required" | "token_reply_invalid" | "token_unusable" => {
            "auth_expired"
        }
        // A 403 with a JSON body: the account has no grant on the workspace.
        "upstream_rejected" => "no_grant",
        // A page (an IP access list or proxy page) instead of JSON.
        "non_json_reply" | "redirect_refused" => "non_json_reply",
        "offline" | "transport_failed" => "offline",
        "timeout" => "timeout",
        _ => "host_failed",
    }
}

/// Removes the running entry when the sign-in ends, on every path.
struct Running<'a> {
    map: &'a Mutex<HashMap<String, Arc<AtomicBool>>>,
    key: String,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        self.map
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.key);
    }
}

fn args_of<T: for<'de> Deserialize<'de>>(args: Value) -> CoreResult<T> {
    serde_json::from_value(args).map_err(|_| refused("invalid_args"))
}

impl ProfileLogin {
    pub fn new(
        config: Arc<dyn ConfigSource>,
        cli: Arc<dyn CliRunner>,
        verifier: Arc<dyn LoginVerifier>,
    ) -> Self {
        Self {
            config,
            cli,
            verifier,
            running: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>> {
        self.running.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `profile_login`: blocks until the sign-in and the W-06a check end.
    fn login(&self, a: ProfileLoginArgs) -> CoreResult<Value> {
        let host = workspace_host(&a.workspace_url).ok_or(refused("host_refused"))?;
        let profile = a.profile.as_str();
        let client_id = a.client_id.as_deref();
        if !plain_value(profile) || client_id.is_some_and(|id| !plain_value(id)) {
            return Err(refused("invalid_args"));
        }
        // W-07g, W-08: never reuse a name. INI section names are compared without case, so a
        // name that differs only in case cannot shadow a profile either.
        let key = profile.to_ascii_lowercase();
        let exists = self
            .config
            .profiles()
            .map_err(|_| refused("host_failed"))?
            .iter()
            .any(|p| p.name.eq_ignore_ascii_case(profile));
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut running = self.running();
            if exists || running.contains_key(&key) {
                return Err(refused("profile_exists"));
            }
            running.insert(key.clone(), cancel.clone());
        }
        let _running = Running {
            map: &self.running,
            key,
        };
        self.cli
            .run_login(
                &login_args(&host, profile, client_id),
                LOGIN_TIMEOUT,
                &cancel,
            )
            .map_err(|e| refused(cli_code(&e)))?;
        // W-07h: a cancel after the CLI ended still wins.
        if cancel.load(Ordering::SeqCst) {
            return Err(refused("login_cancelled"));
        }
        self.verifier
            .verify(profile)
            .map_err(|e| refused(verify_code(&e)))?;
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        Ok(json!({ "loginId": format!("login-{id}") }))
    }

    /// `profile_login_cancel`: set the cancel flag of the running sign-in, if any.
    fn cancel(&self, a: ProfileArgs) -> CoreResult<Value> {
        if let Some(flag) = self.running().get(&a.profile.as_str().to_ascii_lowercase()) {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(json!({}))
    }
}

impl GatewayComponents for ProfileLogin {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        match command {
            "profile_login" => self.login(args_of(args)?),
            "profile_login_cancel" => self.cancel(args_of(args)?),
            _ => Err(refused("not_implemented")),
        }
    }
}
