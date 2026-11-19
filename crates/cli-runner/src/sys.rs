//! Seams for filesystem, environment and process access.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
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

/// Result of a finished child process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcOutput {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
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
        use std::process::{Command, Stdio};
        let mut command = Command::new(program);
        command
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null());
        let Some(timeout) = timeout else {
            let out = command.output()?;
            return Ok(ProcOutput {
                code: out.status.code(),
                stdout: out.stdout,
                stderr: out.stderr,
            });
        };
        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        // Read both pipes on threads, so a full pipe cannot block the child before the deadline.
        let stdout = child.stdout.take().map(read_all);
        let stderr = child.stderr.take().map(read_all);
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let stdout = join(stdout);
        let mut stderr = join(stderr);
        let Some(status) = status else {
            // Nothing the child wrote leaves this function on a timeout.
            let mut stdout = stdout;
            stdout.fill(0);
            stderr.fill(0);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "CLI child timed out",
            ));
        };
        Ok(ProcOutput {
            code: status.code(),
            stdout,
            stderr,
        })
    }
}

/// Read a child pipe to its end on a thread.
fn read_all<R: io::Read + Send + 'static>(mut pipe: R) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

fn join(handle: Option<std::thread::JoinHandle<Vec<u8>>>) -> Vec<u8> {
    handle.and_then(|h| h.join().ok()).unwrap_or_default()
}
