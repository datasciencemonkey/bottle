//! A cancellable CLI child for `databricks auth login` (#162): W-07c (the long sign-in limit)
//! and W-07h (Cancel stops the CLI child process).

mod common;

use std::cell::RefCell;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cli_runner::{CliError, CliPolicy, CliRunner, Env, ProcOutput, Process, RealFs, RealProcess};
use common::{abs, cli_in, FakeEnv, FakeFs};

/// Records the timeout of each child and answers `--version`.
#[derive(Clone, Default)]
struct Recorder {
    timeouts: Rc<RefCell<Vec<Option<Duration>>>>,
}

impl Process for Recorder {
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
        let stdout = if args.first().is_some_and(|a| a == "--version") {
            b"Databricks CLI v1.25.0\n".to_vec()
        } else {
            Vec::new()
        };
        Ok(ProcOutput {
            code: Some(0),
            stdout,
            stderr: Vec::new(),
        })
    }
}

/// [W-07c] a cancellable child gets its own timeout, not the short runner timeout
#[test]
fn w_07c_a_cancellable_child_gets_its_own_timeout() {
    let proc = Recorder::default();
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/local/bin")]);
    let mut r = CliRunner::new(fs, env, proc.clone(), CliPolicy::default())
        .with_timeout(Duration::from_secs(30));
    r.launch_check(None).unwrap();
    let cancel = AtomicBool::new(false);
    r.run_cancellable(["auth", "login"], Duration::from_secs(300), &cancel)
        .unwrap();
    assert_eq!(
        *proc.timeouts.borrow(),
        vec![
            Some(Duration::from_secs(30)),
            Some(Duration::from_secs(300))
        ]
    );
}

/// [W-07h] a cancel before the start gives `Cancelled` and starts no child
#[test]
fn w_07h_a_cancel_before_the_start_starts_no_child() {
    let proc = Recorder::default();
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/local/bin")]);
    let mut r = CliRunner::new(fs, env, proc.clone(), CliPolicy::default());
    r.launch_check(None).unwrap();
    let cancel = AtomicBool::new(true);
    let err = r
        .run_cancellable(["auth", "login"], Duration::from_secs(300), &cancel)
        .unwrap_err();
    assert_eq!(err, CliError::Cancelled);
    assert_eq!(err.code(), "CLI_CANCELLED");
    assert_eq!(
        proc.timeouts.borrow().len(),
        1,
        "only the version check ran"
    );
}

/// Real-process tests run one at a time (ETXTBSY on Linux, see `real_process.rs`).
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
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

/// [W-07h] Cancel kills a real running CLI child at once (the fixture sleeps 60 s)
#[test]
fn w_07h_cancel_kills_a_real_running_cli_child() {
    let _serial = serial();
    let dir = install_fake("cancel");
    let env = FakeEnv::new()
        .with_path(&[dir.clone()])
        .temp(cli_runner::RealEnv.temp_dir())
        .set("HOME", dir.as_os_str());
    let mut r = CliRunner::new(RealFs, env, RealProcess, CliPolicy::default())
        .with_timeout(Duration::from_secs(10));
    r.launch_check(None).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        flag.store(true, Ordering::SeqCst);
    });
    let start = Instant::now();
    let err = r
        .run_cancellable(["sleep"], Duration::from_secs(120), &cancel)
        .unwrap_err();
    canceller.join().unwrap();
    assert_eq!(err, CliError::Cancelled);
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "the child was not killed: {:?}",
        start.elapsed()
    );
}

/// [W-07c] a real cancellable child is killed at its own timeout
#[test]
fn w_07c_a_real_cancellable_child_is_killed_at_its_timeout() {
    let _serial = serial();
    let dir = install_fake("cancel-timeout");
    let env = FakeEnv::new()
        .with_path(&[dir.clone()])
        .temp(cli_runner::RealEnv.temp_dir())
        .set("HOME", dir.as_os_str());
    let mut r = CliRunner::new(RealFs, env, RealProcess, CliPolicy::default())
        .with_timeout(Duration::from_secs(10));
    r.launch_check(None).unwrap();
    let cancel = AtomicBool::new(false);
    let start = Instant::now();
    let err = r
        .run_cancellable(["sleep"], Duration::from_secs(2), &cancel)
        .unwrap_err();
    assert_eq!(err, CliError::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(30));
}
