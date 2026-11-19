//! The child timeout of CliRunner (W-04a: a hung `databricks auth token` must not hang R6).
//!
//! The fake seams check that every child gets the runner timeout and that a timeout maps to a
//! content-free error. One real-process test checks that `RealProcess` kills a hung child.

mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use cli_runner::{CliError, CliPolicy, CliRunner, Env, ProcOutput, Process, RealFs, RealProcess};
use common::{abs, cli_in, FakeEnv, FakeFs};

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

fn install_fake(name: &str) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
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
