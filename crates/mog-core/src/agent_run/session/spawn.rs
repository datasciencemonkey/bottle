//! The real process side of the session host (unix): the spawner, the process-group kill and the
//! stale-process sweep.
//!
//! The server runs in its own process group, so one signal reaches it and every child of it. The
//! environment is cleared and set from the allow-list of the caller.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use super::appserver::ServerIo;
use super::appserver::{ServerProc, SpawnSpec, Spawner};

/// The grace between SIGTERM and SIGKILL.
pub const TERM_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// Starts real processes.
#[derive(Debug, Default)]
pub struct ProcessSpawner;

#[cfg(unix)]
mod imp {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};
    use std::time::Instant;

    pub struct Proc {
        child: Child,
        io: Option<ServerIo>,
        done: bool,
    }

    impl Spawner for ProcessSpawner {
        fn spawn(&self, spec: &SpawnSpec) -> io::Result<Box<dyn ServerProc>> {
            let mut cmd = Command::new(&spec.program);
            cmd.args(&spec.args)
                .env_clear()
                .envs(spec.env.iter().map(|(k, v)| (k, v)))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .process_group(0);
            if let Some(cwd) = &spec.cwd {
                cmd.current_dir(cwd);
            }
            let mut child = cmd.spawn()?;
            let io = match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
                (Some(i), Some(o), Some(e)) => ServerIo {
                    stdin: Box::new(i),
                    stdout: Box::new(o),
                    stderr: Box::new(e),
                },
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::other("no pipes"));
                }
            };
            Ok(Box::new(Proc {
                child,
                io: Some(io),
                done: false,
            }))
        }
    }

    fn group(pid: u32, sig: libc::c_int) {
        if let Ok(p) = i32::try_from(pid) {
            if p > 1 {
                // SAFETY: killpg on a plain pid number has no memory effect.
                unsafe { libc::killpg(p, sig) };
            }
        }
    }

    impl ServerProc for Proc {
        fn pid(&self) -> u32 {
            self.child.id()
        }

        fn take_io(&mut self) -> Option<ServerIo> {
            self.io.take()
        }

        fn terminate(&mut self) {
            if self.done {
                return;
            }
            self.done = true;
            let pid = self.child.id();
            drop(self.io.take());
            group(pid, libc::SIGTERM);
            let start = Instant::now();
            loop {
                match self.child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) if start.elapsed() >= TERM_GRACE => break,
                    Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
                }
            }
            // Always: a child of the leader can outlive it.
            group(pid, libc::SIGKILL);
            let _ = self.child.wait();
        }
    }

    impl Drop for Proc {
        fn drop(&mut self) {
            self.terminate();
        }
    }

    /// True if a process with this pid exists.
    pub fn alive(pid: u32) -> bool {
        match i32::try_from(pid) {
            // SAFETY: signal 0 only checks that the pid exists.
            Ok(p) if p > 1 => unsafe { libc::kill(p, 0) == 0 },
            _ => false,
        }
    }

    /// Signal the group of `pid`, SIGTERM first, SIGKILL after the grace.
    pub fn kill_group(pid: u32) {
        group(pid, libc::SIGTERM);
        let start = Instant::now();
        while alive(pid) && start.elapsed() < TERM_GRACE {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        group(pid, libc::SIGKILL);
    }

    /// The executable path of `pid`, from `ps`.
    pub fn exe_of(pid: u32) -> Option<PathBuf> {
        let out = Command::new("/bin/ps")
            .args(["-o", "comm=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let s = String::from_utf8(out.stdout).ok()?;
        let s = s.trim();
        (!s.is_empty()).then(|| PathBuf::from(s))
    }
}

#[cfg(not(unix))]
mod imp {
    use super::*;

    impl Spawner for ProcessSpawner {
        fn spawn(&self, _: &SpawnSpec) -> io::Result<Box<dyn ServerProc>> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Genie Code is not available on this OS",
            ))
        }
    }

    pub fn alive(_: u32) -> bool {
        false
    }
    pub fn kill_group(_: u32) {}
    pub fn exe_of(_: u32) -> Option<PathBuf> {
        None
    }
}

pub use imp::{alive, exe_of, kill_group};

/// Write the pid of the server, outside GENIE_HOME, so a crash of Bottle leaves a trace that the
/// next start can sweep.
pub fn write_pid(file: &Path, pid: u32) -> io::Result<()> {
    fs::write(file, pid.to_string())
}

pub fn remove_pid(file: &Path) {
    let _ = fs::remove_file(file);
}

/// At app start: kill the server that an earlier Bottle left, if its executable is `expected`.
/// A pid that now belongs to another program is left alone. The pid file always goes.
pub fn sweep_stale(
    pid_file: &Path,
    expected: &Path,
    exe_of: &dyn Fn(u32) -> Option<PathBuf>,
) -> bool {
    let pid = fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    remove_pid(pid_file);
    let Some(pid) = pid else { return false };
    let Some(exe) = exe_of(pid) else { return false };
    let same = exe == expected
        || (!exe.is_absolute() && expected.file_name().is_some_and(|n| n == exe.as_os_str()));
    if same {
        kill_group(pid);
    }
    same
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;

    fn spawn_sleep() -> Box<dyn ServerProc> {
        ProcessSpawner
            .spawn(&SpawnSpec {
                program: PathBuf::from("/bin/sh"),
                args: vec!["-c".into(), "sleep 600".into()],
                env: vec![("PATH".into(), "/usr/bin:/bin".into())],
                cwd: None,
            })
            .unwrap()
    }

    #[test]
    fn terminate_kills_the_leader_and_its_child_in_the_process_group() {
        let mut p = spawn_sleep();
        let pid = p.pid();
        assert!(alive(pid));
        // The shell may exec `sleep` or fork it. Either way the group must die.
        p.terminate();
        assert!(!alive(pid));
        // The group is gone: no member answers to the group id.
        // SAFETY: signal 0 only checks that the group exists.
        let rc = unsafe { libc::killpg(i32::try_from(pid).unwrap(), 0) };
        assert_eq!(rc, -1);
        p.terminate();
    }

    #[test]
    fn the_child_gets_only_the_given_environment_and_its_own_group() {
        let mut p = ProcessSpawner
            .spawn(&SpawnSpec {
                program: PathBuf::from("/usr/bin/env"),
                args: vec![],
                env: vec![("ONLY".into(), "mine".into())],
                cwd: None,
            })
            .unwrap();
        let mut io = p.take_io().unwrap();
        let mut out = String::new();
        io.stdout.read_to_string(&mut out).unwrap();
        assert_eq!(out.trim(), "ONLY=mine");
        assert!(p.take_io().is_none());
        // SAFETY: getpgid only reads.
        let pgid = unsafe { libc::getpgid(i32::try_from(p.pid()).unwrap()) };
        assert_ne!(pgid, unsafe { libc::getpgrp() });
        p.terminate();
    }

    fn pid_file() -> PathBuf {
        let mut n = [0u8; 6];
        getrandom::fill(&mut n).unwrap();
        std::env::temp_dir().join(format!(
            "r1-pid-{}",
            n.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ))
    }

    #[test]
    fn the_sweep_kills_only_a_process_whose_exe_is_the_pinned_genie() {
        let genie = Path::new("/pinned/genie");
        // Another program: left alone, pid file removed.
        let mut other = spawn_sleep();
        let f = pid_file();
        write_pid(&f, other.pid()).unwrap();
        assert!(!sweep_stale(&f, genie, &|_| Some(PathBuf::from(
            "/bin/sleep"
        ))));
        assert!(alive(other.pid()));
        assert!(!f.exists());
        // The pinned genie: killed.
        write_pid(&f, other.pid()).unwrap();
        assert!(sweep_stale(&f, genie, &|_| Some(genie.to_path_buf())));
        let start = std::time::Instant::now();
        while alive(other.pid()) && start.elapsed().as_secs() < 5 {
            // The leader is a zombie until it is reaped: wait through the handle.
            other.terminate();
        }
        assert!(!alive(other.pid()));
        // A missing or bad file does nothing.
        assert!(!sweep_stale(&f, genie, &|_| Some(genie.to_path_buf())));
    }
}
