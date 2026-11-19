//! Seams for filesystem, environment and process access.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Filesystem access used by CliRunner.
pub trait Fs {
    /// True if `path` is an existing regular file (symlinks followed).
    fn is_file(&self, path: &Path) -> bool;
    /// Absolute path with all symlinks resolved.
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf>;
    /// Unix permission bits of `path` (symlinks followed); `None` on other OSes
    /// or when the path cannot be read.
    fn unix_mode(&self, path: &Path) -> Option<u32>;
}

/// Environment access used by CliRunner.
pub trait Env {
    fn var(&self, name: &str) -> Option<OsString>;
    fn vars(&self) -> Vec<(OsString, OsString)>;
    /// The OS temporary directory (`std::env::temp_dir`).
    fn temp_dir(&self) -> PathBuf;
}

/// Result of a finished child process. The stdout of `auth token` holds a token, so `Debug`
/// shows only the exit code and the byte counts (W-01b, W-04b).
#[derive(Clone, PartialEq, Eq)]
pub struct ProcOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

impl std::fmt::Debug for ProcOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcOutput")
            .field("code", &self.code)
            .field("stdout", &format!("<{} bytes>", self.stdout.len()))
            .field("stderr", &format!("<{} bytes>", self.stderr.len()))
            .finish()
    }
}

impl ProcOutput {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Process access used by CliRunner. `env` is the complete child env: the
/// implementation must not inherit anything from the parent.
pub trait Process {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
    ) -> io::Result<ProcOutput>;

    /// [`Process::run`], but a child that runs longer than `timeout` is killed and the call
    /// fails with [`io::ErrorKind::TimedOut`]. `None` waits for the child to end.
    ///
    /// The default ignores `timeout` (for test doubles). [`RealProcess`] honours it.
    fn run_with_timeout(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
        _timeout: Option<Duration>,
    ) -> io::Result<ProcOutput> {
        self.run(program, args, env)
    }

    /// [`Process::run_with_timeout`], but the child is also killed as soon as `cancel` is set,
    /// and the call fails with [`io::ErrorKind::Interrupted`] (#162, W-07h: Cancel stops the
    /// `databricks auth login` child). Nothing the child wrote leaves the call on a cancel.
    ///
    /// The default checks `cancel` once before the start, then runs [`Process::run_with_timeout`]
    /// (for test doubles). [`RealProcess`] checks it while the child runs.
    fn run_cancellable(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<ProcOutput> {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        self.run_with_timeout(program, args, env, Some(timeout))
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "CLI child cancelled")
}

/// The real filesystem.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealFs;

impl Fs for RealFs {
    fn is_file(&self, path: &Path) -> bool {
        std::fs::metadata(path)
            .map(|m| m.is_file())
            .unwrap_or(false)
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        std::fs::canonicalize(path).map(simplify)
    }
    #[cfg(unix)]
    fn unix_mode(&self, path: &Path) -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).ok().map(|m| m.permissions().mode())
    }
    #[cfg(not(unix))]
    fn unix_mode(&self, _path: &Path) -> Option<u32> {
        None
    }
}

/// On Windows, turns `\\?\C:\x` into `C:\x` so Settings shows a normal path.
#[cfg(windows)]
fn simplify(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => p,
    }
}

#[cfg(not(windows))]
fn simplify(p: PathBuf) -> PathBuf {
    p
}

/// The real process environment.
#[derive(Debug, Default, Clone, Copy)]
pub struct RealEnv;

impl Env for RealEnv {
    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
    fn vars(&self) -> Vec<(OsString, OsString)> {
        std::env::vars_os().collect()
    }
    fn temp_dir(&self) -> PathBuf {
        std::env::temp_dir()
    }
}

/// Real child processes (`std::process::Command` with `env_clear`).
#[derive(Debug, Default, Clone, Copy)]
pub struct RealProcess;

impl Process for RealProcess {
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
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
        timeout: Option<Duration>,
    ) -> io::Result<ProcOutput> {
        let mut command = clean_command(program, args, env);
        let Some(timeout) = timeout else {
            let out = command.output()?;
            return Ok(ProcOutput {
                code: out.status.code(),
                stdout: out.stdout,
                stderr: out.stderr,
            });
        };
        run_until(command, timeout, None)
    }

    fn run_cancellable(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> io::Result<ProcOutput> {
        if cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        run_until(clean_command(program, args, env), timeout, Some(cancel))
    }
}

/// `program args` with exactly `env` (nothing inherited) and no stdin.
fn clean_command(
    program: &Path,
    args: &[OsString],
    env: &[(OsString, OsString)],
) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command
        .args(args)
        .env_clear()
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(std::process::Stdio::null());
    command
}

/// Run `command` with piped output; kill the child at `timeout` ([`io::ErrorKind::TimedOut`])
/// or when `cancel` is set ([`io::ErrorKind::Interrupted`]).
fn run_until(
    mut command: std::process::Command,
    timeout: Duration,
    cancel: Option<&AtomicBool>,
) -> io::Result<ProcOutput> {
    use std::process::Stdio;
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Read both pipes on threads, so a full pipe cannot block the child before the deadline.
    let stdout = child.stdout.take().map(read_all);
    let stderr = child.stderr.take().map(read_all);
    let deadline = Instant::now() + timeout;
    let mut was_cancelled = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if cancel.is_some_and(|c| c.load(Ordering::SeqCst)) {
            let _ = child.kill();
            let _ = child.wait();
            was_cancelled = true;
            break None;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    // A grandchild can keep the pipes open after the child ends, so the reads get only a
    // short grace after the deadline. A cancel does not wait for the deadline.
    let pipe_deadline = if was_cancelled {
        Instant::now()
    } else {
        deadline.max(Instant::now())
    } + PIPE_GRACE;
    let stdout = join(stdout, pipe_deadline);
    let stderr = join(stderr, pipe_deadline);
    match (status, stdout, stderr) {
        (Some(status), Some(stdout), Some(stderr)) => Ok(ProcOutput {
            code: status.code(),
            stdout,
            stderr,
        }),
        (_, stdout, stderr) => {
            // Nothing the child wrote leaves this function on a timeout or a cancel.
            for mut buf in [stdout, stderr].into_iter().flatten() {
                buf.fill(0);
            }
            if was_cancelled {
                return Err(cancelled());
            }
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "CLI child timed out",
            ))
        }
    }
}

/// How long the pipe reads may run past the deadline (a grandchild may hold the pipes).
const PIPE_GRACE: Duration = Duration::from_millis(250);

/// The first size of a pipe buffer. A token reply is far smaller, so the buffer does not grow,
/// and no partial copy of the token is freed unzeroed.
const PIPE_BUFFER: usize = 64 * 1024;

/// Read a child pipe to its end on a thread, and send the bytes.
fn read_all<R: io::Read + Send + 'static>(mut pipe: R) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = Vec::with_capacity(PIPE_BUFFER);
        let _ = pipe.read_to_end(&mut buf);
        if let Err(mpsc::SendError(mut late)) = tx.send(buf) {
            // The caller gave up (a grandchild held the pipe): wipe what came late.
            late.fill(0);
        }
    });
    rx
}

/// The bytes of one pipe, or `None` if the read did not end by `deadline`.
fn join(rx: Option<mpsc::Receiver<Vec<u8>>>, deadline: Instant) -> Option<Vec<u8>> {
    let Some(rx) = rx else {
        return Some(Vec::new());
    };
    rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .ok()
}
