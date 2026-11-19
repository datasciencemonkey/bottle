//! The child timeout of CliRunner (W-04a: a hung `databricks auth token` must not hang R6).
//!
//! The fake seams check that every child gets the runner timeout and that a timeout maps to a
//! content-free error. One real-process test checks that `RealProcess` kills a hung child.

mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cli_runner::{CliError, CliPolicy, CliRunner, Env, ProcOutput, Process, RealFs, RealProcess};
use common::{abs, cli_in, fixture_root, FakeEnv, FakeFs, FixtureDir};

/// Answers `--version`, records the timeout of each child, and can time out the other calls.
#[derive(Clone, Default)]
struct TimedProcess {
    timeouts: Rc<RefCell<Vec<Option<Duration>>>>,
    hang: bool,
}

impl Process for TimedProcess {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
    ) -> io::Result<ProcOutput> {
        self.run_with_timeout(program, args, env, None)
    }

    fn run_with_timeout(
        &self,
        _program: &Path,
        args: &[OsString],
        _env: &[(OsString, OsString)],
        timeout: Option<Duration>,
    ) -> io::Result<ProcOutput> {
        self.timeouts.borrow_mut().push(timeout);
        if args.first().is_some_and(|a| a == "--version") {
            return Ok(ProcOutput {
                code: Some(0),
                stdout: b"Databricks CLI v1.25.0\n".to_vec(),
                stderr: Vec::new(),
            });
        }
        if self.hang {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "fake: timed out"));
        }
        Ok(ProcOutput {
            code: Some(0),
            stdout: b"{}".to_vec(),
            stderr: Vec::new(),
        })
    }
}

fn runner(proc: TimedProcess) -> CliRunner<FakeFs, FakeEnv, TimedProcess> {
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/local/bin")]);
    CliRunner::new(fs, env, proc, CliPolicy::default())
}

/// [W-04a] every child (the version check and each run) gets the runner timeout
#[test]
fn w_04a_every_child_gets_the_runner_timeout() {
    let proc = TimedProcess::default();
    let mut r = runner(proc.clone()).with_timeout(Duration::from_secs(30));
    assert_eq!(r.timeout(), Some(Duration::from_secs(30)));
    r.launch_check(None).unwrap();
    r.run(["auth", "describe"]).unwrap();
    assert_eq!(
        *proc.timeouts.borrow(),
        vec![Some(Duration::from_secs(30)), Some(Duration::from_secs(30))]
    );
}

/// [W-04a] without `with_timeout` a runner sets no timeout
#[test]
fn w_04a_a_runner_has_no_timeout_by_default() {
    let proc = TimedProcess::default();
    let mut r = runner(proc.clone());
    assert_eq!(r.timeout(), None);
    r.launch_check(None).unwrap();
    assert_eq!(*proc.timeouts.borrow(), vec![None]);
}

/// [W-04a] a child that times out is a content-free `TimedOut` error
#[test]
fn w_04a_a_timed_out_child_is_a_content_free_error() {
    let proc = TimedProcess {
        hang: true,
        ..TimedProcess::default()
    };
    let mut r = runner(proc).with_timeout(Duration::from_millis(50));
    r.launch_check(None).unwrap();
    let err = r.run(["auth", "token"]).unwrap_err();
    assert_eq!(err, CliError::TimedOut);
    assert_eq!(err.code(), "CLI_TIMED_OUT");
    assert!(!err.to_string().contains("auth"));
}

/// The real environment, so the hung fake can start (it needs no variable).
fn real_env_for(dir: &Path) -> FakeEnv {
    FakeEnv::new()
        .with_path(&[dir.to_path_buf()])
        .temp(cli_runner::RealEnv.temp_dir())
        .set("HOME", dir.as_os_str())
}

/// Copies the fake CLI into a fresh folder under `common::fixture_root` (not a temp root).
fn install_fake(name: &str) -> FixtureDir {
    let dir = FixtureDir::new(&fixture_root(), name);
    let cli = cli_in(&dir);
    std::fs::copy(env!("CARGO_BIN_EXE_fake-databricks"), &cli).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

/// [W-04a] a real hung CLI child is killed at the timeout; a quick child still returns its output
#[test]
fn w_04a_a_real_hung_cli_child_is_killed_at_the_timeout() {
    let dir = install_fake("timeout");
    let mut r = CliRunner::new(
        RealFs,
        real_env_for(&dir),
        RealProcess,
        CliPolicy::default(),
    )
    // 5 s, not less: the first start of a freshly copied binary can take over 500 ms on a
    // loaded machine (pre-push runs). The fixture sleeps 60 s, so a pass still proves the kill.
    .with_timeout(Duration::from_secs(5));
    // The version check runs under the same timeout and still returns its stdout.
    let report = r.launch_check(None).unwrap();
    assert_eq!(report.record.version.to_string(), "1.25.0");
    let start = Instant::now();
    let err = r.run(["sleep"]).unwrap_err();
    assert_eq!(err, CliError::TimedOut);
    assert!(
        start.elapsed() < Duration::from_secs(30),
        "the hung child was not killed: {:?}",
        start.elapsed()
    );
}

/// [W-01b] [W-04b] the Debug of a child output shows only the exit code and byte counts
#[test]
fn w_01b_w_04b_debug_of_a_child_output_never_shows_the_output() {
    let out = ProcOutput {
        code: Some(0),
        stdout: b"{\"access_token\":\"test-token-not-real\"}".to_vec(),
        stderr: b"debug test-token-not-real".to_vec(),
    };
    let text = format!("{out:?}");
    assert!(!text.contains("test-token-not-real"), "{text}");
    assert!(!text.contains("116"), "no byte values: {text}");
    assert_eq!(
        text,
        "ProcOutput { code: Some(0), stdout: \"<38 bytes>\", stderr: \"<25 bytes>\" }"
    );
    let res: Result<ProcOutput, CliError> = Ok(out);
    assert!(!format!("{res:?}").contains("test-token-not-real"));
}

/// [W-04a] a grandchild that keeps the pipes open cannot hold the call past the timeout
#[cfg(unix)]
#[test]
fn w_04a_a_grandchild_on_the_pipes_cannot_outlast_the_timeout() {
    let dir = install_fake("orphan");
    let mut r = CliRunner::new(
        RealFs,
        real_env_for(&dir),
        RealProcess,
        CliPolicy::default(),
    )
    // The version check gets 10 s: the first start of a freshly copied binary can take over
    // 1.5 s on a loaded machine (pre-push runs). The orphan run below gets the short timeout.
    .with_timeout(Duration::from_secs(10));
    r.launch_check(None).unwrap();
    let r = r.with_timeout(Duration::from_millis(1500));
    let start = Instant::now();
    // The child starts a grandchild that naps 5 s on the inherited pipes, then exits at once.
    let err = r.run(["orphan"]).unwrap_err();
    assert_eq!(err, CliError::TimedOut);
    assert!(
        start.elapsed() < Duration::from_millis(4000),
        "the call waited for the grandchild: {:?}",
        start.elapsed()
    );
}
