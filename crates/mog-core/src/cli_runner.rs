//! R5 CliRunner (section 4.3).
//!
//! Responsibilities: Resolves `databricks` to an absolute path, checks version ≥ 1.17, runs it as a child process outside the boundary with a clean env, parses JSON. Installs the official CLI only after consent.
//!
//! The output of `databricks auth token` goes only to R6 TokenBroker (ARC-6). So
//! [`CliRunner::run_json`] refuses `auth token` for every caller ([`refuse_token_command`]);
//! R6 gets its tokens through its own path, [`CliRunner::run_auth_token`] (#18, W-04a).

use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{compiler_fence, AtomicBool, Ordering};
use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use ::cli_runner::{CliError, Env, Fs, ProcOutput, Process, RealEnv, RealFs, RealProcess};
pub use ::cli_runner::{CliPolicy, CliRecord, CliStatus, CliWarning, LaunchReport};

use crate::error::{CoreError, CoreResult};

const ID: &str = "R5";

/// R5 kills a CLI child that runs longer than this (the reference bridge uses 30 s too), so a
/// hung `databricks auth token` cannot hang R6 or a profile check.
pub const CLI_TIMEOUT: Duration = Duration::from_secs(30);

/// Refuse `databricks auth token` (in any case, with any flags before, between or after).
///
/// The rule is wide on purpose: it refuses when the word `auth` comes before a later word
/// `token` anywhere in the arguments (a flag value such as `-p token` is refused too).
/// Every [`CliRunner`] implementation must call this first in `run_json`, so a token never
/// comes back to a caller as plain JSON (ARC-6).
pub fn refuse_token_command(args: &[&str]) -> CoreResult<()> {
    let words: Vec<String> = args.iter().map(|a| a.to_ascii_lowercase()).collect();
    let refused = words
        .iter()
        .position(|w| w == "auth")
        .is_some_and(|i| words[i + 1..].iter().any(|w| w == "token"));
    if refused {
        Err(CoreError::TokenCommandRefused { component: ID })
    } else {
        Ok(())
    }
}

/// The one command that gives a token (W-04a): `auth token --profile <p> --output json`.
/// Every [`CliRunner::run_auth_token`] implementation runs exactly these arguments.
pub fn auth_token_args(profile: &str) -> [&str; 6] {
    ["auth", "token", "--profile", profile, "--output", "json"]
}

/// Runs the Databricks CLI as a child process with a clean environment.
pub trait CliRunner: Send + Sync {
    /// Resolve `databricks` to an absolute path and check its version.
    fn resolve(&self) -> CoreResult<PathBuf>;
    /// Run the CLI with these arguments and parse its JSON output.
    /// It refuses `auth token` with [`CoreError::TokenCommandRefused`].
    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value>;
    /// Run [`auth_token_args`] for `profile` and return its raw stdout, unparsed and unlogged.
    ///
    /// ARC-6: only R6 TokenBroker calls this (a test scans the sources). R6 parses the bytes,
    /// keeps the token in memory only, and wipes the [`SecretStdout`]. The caller passes the
    /// profile: R5 never picks one.
    /// The default answers [`CoreError::NotImplemented`] (component-test doubles); the shipped
    /// [`DatabricksCliRunner`] runs the CLI.
    fn run_auth_token(&self, _profile: &str) -> CoreResult<SecretStdout> {
        Err(CoreError::NotImplemented { component: ID })
    }
    /// Run `databricks auth login ...` (#162, W-07c) with the caller's `timeout`, and kill it as
    /// soon as `cancel` is set (W-07h). It refuses any other command ([`login_command_ok`]).
    /// The CLI keeps the tokens in its own cache: the child output is wiped, never parsed,
    /// returned or logged. Errors are content-free: `cli_timed_out`, `cli_cancelled`,
    /// `cli_failed` or a launch-check reason.
    /// The default answers [`CoreError::NotImplemented`] (component-test doubles).
    fn run_login(
        &self,
        _args: &[&str],
        _timeout: Duration,
        _cancel: &AtomicBool,
    ) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// True for `auth login` with flags only (`--name value` pairs): no `token` word anywhere, so
/// the ARC-6 refusal holds too. Every [`CliRunner::run_login`] implementation checks this first.
pub fn login_command_ok(args: &[&str]) -> bool {
    args.len() >= 2
        && args[0] == "auth"
        && args[1] == "login"
        && refuse_token_command(args).is_ok()
        && args[2..].chunks(2).all(|pair| pair[0].starts_with("--"))
}

/// The raw stdout of `databricks auth token`: it holds a token in plain text (W-04b).
///
/// Not `Clone`, `Debug` is redacted, and the buffer is overwritten with zeros on
/// [`SecretStdout::wipe`] and on drop.
pub struct SecretStdout {
    bytes: Vec<u8>,
}

impl SecretStdout {
    /// Take ownership of the stdout bytes (no copy).
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }

    /// The stdout bytes, for R6 to parse.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Overwrite the buffer with zeros now. `Drop` calls this too.
    pub fn wipe(&mut self) {
        self.bytes.fill(0);
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut self.bytes);
    }

    /// True if every byte is zero (after [`SecretStdout::wipe`]).
    pub fn is_zeroed(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }
}

impl Drop for SecretStdout {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl fmt::Debug for SecretStdout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretStdout(<redacted>)")
    }
}

/// The R5 stub for component tests. Every method returns [`CoreError::NotImplemented`].
/// The shipped R5 is [`DatabricksCliRunner`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubCliRunner;

impl CliRunner for StubCliRunner {
    fn resolve(&self) -> CoreResult<PathBuf> {
        // The stub stays for component tests. The shipped R5 is `DatabricksCliRunner`.
        Err(CoreError::NotImplemented { component: ID })
    }

    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value> {
        refuse_token_command(args)?;
        // The stub stays for component tests. The shipped R5 is `DatabricksCliRunner`.
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The shipped R5: `cli_runner::CliRunner` (#16) behind the [`CliRunner`] trait.
///
/// The `cli-runner` crate resolves the CLI to an absolute path (W-02a), refuses a temporary or
/// world-writable location (W-02b), needs version 1.17 or later (W-02c), reports a changed CLI
/// at the next launch (W-02d) and gives every child a clean environment (W-09). This adapter
/// adds the ARC-6 `auth token` refusal and maps each refusal to a content-free
/// [`CoreError::Unavailable`] reason. A path or version never goes into an error.
pub struct DatabricksCliRunner<F = RealFs, E = RealEnv, P = RealProcess> {
    /// Children run under the read lock, so they run at the same time (one slow `auth token`
    /// does not hold up the others). The launch check takes the write lock.
    inner: RwLock<::cli_runner::CliRunner<F, E, P>>,
}

impl DatabricksCliRunner {
    /// R5 on the real filesystem, environment and processes.
    pub fn system(policy: CliPolicy) -> Self {
        Self {
            inner: RwLock::new(::cli_runner::CliRunner::system(policy).with_timeout(CLI_TIMEOUT)),
        }
    }
}

impl<F: Fs + Send + Sync, E: Env + Send + Sync, P: Process + Send + Sync>
    DatabricksCliRunner<F, E, P>
{
    /// R5 on the given seams (tests use fakes).
    pub fn new(fs: F, env: E, process: P, policy: CliPolicy) -> Self {
        Self {
            inner: RwLock::new(
                ::cli_runner::CliRunner::new(fs, env, process, policy).with_timeout(CLI_TIMEOUT),
            ),
        }
    }

    /// The launch check (W-02a..d): resolve, vet, read the version, and compare with the record
    /// of the previous launch. Changes come back as warnings, not errors.
    pub fn launch_check(&self, previous: Option<&CliRecord>) -> CoreResult<LaunchReport> {
        self.write()
            .launch_check(previous)
            .map_err(|e| unavailable(&e))
    }

    /// The `{path, version, found}` part of the `cli_status` reply: the CLI that the last launch
    /// check verified. `plaintextStorage` comes from the profile check (W-10a), not from R5.
    pub fn status(&self) -> CliStatus {
        self.read().status()
    }

    // A panic in another caller leaves no half-written state worth refusing over: the next call
    // runs the launch check again.
    fn read(&self) -> RwLockReadGuard<'_, ::cli_runner::CliRunner<F, E, P>> {
        self.inner.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, ::cli_runner::CliRunner<F, E, P>> {
        self.inner.write().unwrap_or_else(|p| p.into_inner())
    }
}

impl<F: Fs + Send + Sync, E: Env + Send + Sync, P: Process + Send + Sync> CliRunner
    for DatabricksCliRunner<F, E, P>
{
    fn resolve(&self) -> CoreResult<PathBuf> {
        self.launch_check(None).map(|report| report.record.path)
    }

    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value> {
        refuse_token_command(args)?;
        let out = self.run_verified(args)?;
        if !out.success() {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "cli_failed",
            });
        }
        serde_json::from_slice(&out.stdout).map_err(|_| CoreError::Unavailable {
            component: ID,
            reason: "cli_output_not_json",
        })
    }

    fn run_auth_token(&self, profile: &str) -> CoreResult<SecretStdout> {
        self.auth_token(profile)
    }

    fn run_login(&self, args: &[&str], timeout: Duration, cancel: &AtomicBool) -> CoreResult<()> {
        if !login_command_ok(args) {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "cli_command_refused",
            });
        }
        self.ensure_verified()?;
        let ProcOutput {
            code,
            mut stdout,
            mut stderr,
        } = self
            .read()
            .run_cancellable(args, timeout, cancel)
            .map_err(|e| unavailable(&e))?;
        stdout.fill(0);
        stderr.fill(0);
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box((&mut stdout, &mut stderr));
        if code != Some(0) {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "cli_failed",
            });
        }
        Ok(())
    }
}

impl<F: Fs + Send + Sync, E: Env + Send + Sync, P: Process + Send + Sync>
    DatabricksCliRunner<F, E, P>
{
    /// Run the verified CLI (the launch check runs first if no CLI is verified yet), with the
    /// clean env (W-09) and [`CLI_TIMEOUT`].
    fn run_verified(&self, args: &[&str]) -> CoreResult<ProcOutput> {
        self.ensure_verified()?;
        // `run` re-checks that the verified CLI is still in a safe place (W-02b).
        self.read().run(args).map_err(|e| unavailable(&e))
    }

    /// Run the launch check if no CLI is verified yet.
    fn ensure_verified(&self) -> CoreResult<()> {
        if self.read().verified().is_none() {
            let mut inner = self.write();
            // Another caller may have run the check while this one waited.
            if inner.verified().is_none() {
                inner.launch_check(None).map_err(|e| unavailable(&e))?;
            }
        }
        Ok(())
    }

    /// W-04a: run exactly [`auth_token_args`] for the caller's `profile` (R5 never picks one).
    ///
    /// The stdout `Vec` moves into the [`SecretStdout`] with no copy. The stderr is overwritten
    /// with zeros before this returns. A failed exit, a timeout or a refused CLI is a
    /// content-free [`CoreError::Unavailable`]: no stdout, stderr or profile goes into an error
    /// or a log.
    fn auth_token(&self, profile: &str) -> CoreResult<SecretStdout> {
        let ProcOutput {
            code,
            stdout,
            mut stderr,
        } = self.run_verified(&auth_token_args(profile))?;
        // Owned at once, so it is wiped on every path below (drop).
        let stdout = SecretStdout::new(stdout);
        stderr.fill(0);
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut stderr);
        if code != Some(0) {
            return Err(CoreError::Unavailable {
                component: ID,
                reason: "cli_failed",
            });
        }
        Ok(stdout)
    }
}

/// The content-free reason code for a `cli-runner` refusal.
fn unavailable(e: &CliError) -> CoreError {
    let reason = match e {
        CliError::Missing | CliError::NotAbsolute { .. } => "cli_missing",
        CliError::UnsafeLocation { .. } => "cli_unsafe_path",
        CliError::Unsupported { .. } => "cli_unsupported",
        CliError::VersionUnreadable => "cli_version_unreadable",
        CliError::NotVerified => "cli_not_verified",
        CliError::Spawn { .. } => "cli_spawn_failed",
        CliError::TimedOut => "cli_timed_out",
        CliError::Cancelled => "cli_cancelled",
    };
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}
