//! The shared harness runtime process (RTR-21a, U11) and its launcher.
//!
//! One runtime process of a harness kind hosts the sessions of every MOG that uses that kind.
//! The host talks to it with JSON lines: frames on stdin (`open_session`, `turn`,
//! `close_session`), events on stdout (`session_failed`). End of stdout means the process exited.
//! TODO(#111): SP-19 picks the sidecar program (Agent SDK or headless CLI) and fixes the
//! protocol; [`StdRuntimeLauncher`] already gives it a clean environment.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::{AgentError, Harness};

/// The program and arguments of the shared runtime of one harness kind. No token, secret or
/// user content goes in the arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    /// The runtime program.
    pub program: PathBuf,
    /// The fixed arguments.
    pub args: Vec<String>,
    /// A Bottle-owned scratch home folder for the runtime (`HOME`, `USERPROFILE`). The user's
    /// home is never passed, so the harness cannot find `~/.databrickscfg` or the CLI token cache
    /// through it (ARC-7). `None` gives the runtime no home variable at all.
    pub home: Option<PathBuf>,
}

impl RuntimeConfig {
    /// A config for `program` with `args` and no home folder.
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            home: None,
        }
    }

    /// The same config with the scratch home folder `home`.
    pub fn with_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.home = Some(home.into());
        self
    }
}

/// Everything needed to start one runtime process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    /// The harness kind the runtime serves.
    pub harness: Harness,
    /// The program.
    pub program: PathBuf,
    /// The arguments.
    pub args: Vec<String>,
    /// The complete environment. The process inherits nothing else.
    pub env: Vec<(String, String)>,
}

impl LaunchSpec {
    /// The value of `name` in [`LaunchSpec::env`].
    pub fn env_value(&self, name: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
}

/// An event from a runtime process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeEvent {
    /// One session in the runtime stopped or stopped answering; the process still runs.
    SessionFailed {
        /// The agent session ID.
        session_id: String,
    },
    /// The runtime process exited.
    Exited,
}

/// The callback that gets the events of one runtime process.
pub type RuntimeEvents = Arc<dyn Fn(RuntimeEvent) + Send + Sync>;

/// A running runtime process.
pub trait RuntimeProcess: Send + Sync {
    /// Queue one frame for the runtime. It must not block: the host may call it while it holds
    /// its state lock. A runtime that does not take its frames fails with
    /// [`AgentError::Unavailable`] and must end in [`RuntimeEvent::Exited`].
    /// TODO(#111): a runtime that takes frames but never answers is caught only by an answer or
    /// heartbeat deadline, which SP-19 defines with the sidecar protocol.
    fn send_frame(&self, frame: &Value) -> Result<(), AgentError>;
    /// Stop the process.
    fn stop(&self);
}

/// Starts runtime processes.
pub trait RuntimeLauncher: Send + Sync {
    /// Start the runtime of `spec`. `events` gets its events until it exits. `launch` must not
    /// call `events` before it returns. It runs under the host lock, so it must return quickly:
    /// it spawns the process and never waits for the runtime to answer.
    fn launch(
        &self,
        spec: &LaunchSpec,
        events: RuntimeEvents,
    ) -> Result<Box<dyn RuntimeProcess>, AgentError>;
}

/// The command for `spec`: the program, the arguments and ONLY the spec environment
/// (`env_clear`), so no variable of the Bottle process leaks into the harness (ARC-7, RTR-28).
pub fn harness_command(spec: &LaunchSpec) -> Command {
    let mut c = Command::new(&spec.program);
    c.args(&spec.args)
        .env_clear()
        .envs(spec.env.iter().map(|(k, v)| (k, v)));
    c
}

/// Parse one stdout line of the runtime. Unknown lines are ignored.
fn parse_event(line: &str) -> Option<RuntimeEvent> {
    let v: Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "session_failed" => Some(RuntimeEvent::SessionFailed {
            session_id: v.get("sessionId")?.as_str()?.to_owned(),
        }),
        _ => None,
    }
}

/// The default write deadline of [`StdRuntimeLauncher`].
pub const WRITE_DEADLINE: Duration = Duration::from_secs(10);
/// The number of frames that can wait for the runtime.
pub const FRAME_QUEUE: usize = 64;

/// Starts the runtime as a real child process with piped stdin and stdout. Its stderr goes to
/// the null device: harness output can hold user content and is never logged (SEC rules).
///
/// Frames go through a bounded queue to a writer thread, so [`RuntimeProcess::send_frame`] never
/// blocks. If one write takes longer than `write_deadline`, the runtime does not answer
/// (ARC-F14): a watchdog kills it, its stdout ends and the host gets [`RuntimeEvent::Exited`].
#[derive(Debug, Clone, Copy)]
pub struct StdRuntimeLauncher {
    /// The longest time one frame write may take.
    pub write_deadline: Duration,
}

impl Default for StdRuntimeLauncher {
    fn default() -> Self {
        Self {
            write_deadline: WRITE_DEADLINE,
        }
    }
}

struct StdRuntimeProcess {
    child: Arc<Mutex<Child>>,
    frames: Mutex<Option<SyncSender<Vec<u8>>>>,
}

/// Stop the runtime and every process it started. On unix the runtime leads its own process
/// group, so a wrapper (`npx`, `uv run`, a shell shim) and its children stop too.
/// TODO(#111): on Windows put the runtime in a job object; today only the direct child stops.
fn kill(child: &Mutex<Child>) {
    let mut c = child.lock().unwrap_or_else(|p| p.into_inner());
    #[cfg(unix)]
    {
        if let Ok(pid) = i32::try_from(c.id()) {
            // SAFETY: `kill` with a negative pid signals the process group that the child leads
            // (`process_group(0)` at spawn). It touches no memory.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
    let _ = c.kill();
    let _ = c.wait();
}

impl RuntimeProcess for StdRuntimeProcess {
    fn send_frame(&self, frame: &Value) -> Result<(), AgentError> {
        let guard = self.frames.lock().unwrap_or_else(|p| p.into_inner());
        let tx = guard.as_ref().ok_or(AgentError::Unavailable)?;
        let mut line = frame.to_string().into_bytes();
        line.push(b'\n');
        tx.try_send(line).map_err(|e| match e {
            TrySendError::Full(_) | TrySendError::Disconnected(_) => AgentError::Unavailable,
        })
    }

    fn stop(&self) {
        self.frames.lock().unwrap_or_else(|p| p.into_inner()).take();
        kill(&self.child);
    }
}

impl Drop for StdRuntimeProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

fn spawn_thread(name: &str, f: impl FnOnce() + Send + 'static) -> Result<(), AgentError> {
    std::thread::Builder::new()
        .name(name.into())
        .spawn(f)
        .map(|_| ())
        .map_err(|_| AgentError::LaunchFailed)
}

impl RuntimeLauncher for StdRuntimeLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        events: RuntimeEvents,
    ) -> Result<Box<dyn RuntimeProcess>, AgentError> {
        let mut cmd = harness_command(spec);
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| AgentError::LaunchFailed)?;
        let stdin = child.stdin.take().ok_or(AgentError::LaunchFailed)?;
        let stdout = child.stdout.take().ok_or(AgentError::LaunchFailed)?;
        let child = Arc::new(Mutex::new(child));
        let (tx, rx) = sync_channel::<Vec<u8>>(FRAME_QUEUE);
        let busy_since: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
        let done = Arc::new(AtomicBool::new(false));
        // `Exited` goes out exactly once: from the reader at end of output, or from the watchdog
        // after it stops a runtime whose pipes a grandchild still holds open.
        let exited = Arc::new(AtomicBool::new(false));
        let exit_once = {
            let (exited, events) = (exited.clone(), events.clone());
            move || {
                if !exited.swap(true, Ordering::SeqCst) {
                    events(RuntimeEvent::Exited);
                }
            }
        };
        let started = self.spawn_threads(
            stdin,
            stdout,
            rx,
            &child,
            &busy_since,
            &done,
            events,
            exit_once,
        );
        if started.is_err() {
            kill(&child);
            return Err(AgentError::LaunchFailed);
        }
        Ok(Box::new(StdRuntimeProcess {
            child,
            frames: Mutex::new(Some(tx)),
        }))
    }
}

impl StdRuntimeLauncher {
    #[allow(clippy::too_many_arguments)]
    fn spawn_threads(
        &self,
        mut stdin: std::process::ChildStdin,
        stdout: std::process::ChildStdout,
        rx: std::sync::mpsc::Receiver<Vec<u8>>,
        child: &Arc<Mutex<Child>>,
        busy_since: &Arc<Mutex<Option<Instant>>>,
        done: &Arc<AtomicBool>,
        events: RuntimeEvents,
        exit_once: impl Fn() + Clone + Send + 'static,
    ) -> Result<(), AgentError> {
        let (busy, d) = (busy_since.clone(), done.clone());
        spawn_thread("bottle-agent-runtime-writer", move || {
            for line in rx {
                *busy.lock().unwrap_or_else(|p| p.into_inner()) = Some(Instant::now());
                let ok = stdin.write_all(&line).and_then(|_| stdin.flush()).is_ok();
                *busy.lock().unwrap_or_else(|p| p.into_inner()) = None;
                if !ok {
                    break;
                }
            }
            d.store(true, Ordering::SeqCst);
        })?;

        let (c, d, deadline) = (child.clone(), done.clone(), self.write_deadline);
        let (busy_since, watchdog_exit) = (busy_since.clone(), exit_once.clone());
        let tick = (deadline / 4).clamp(Duration::from_millis(10), Duration::from_millis(500));
        spawn_thread("bottle-agent-runtime-watchdog", move || {
            while !d.load(Ordering::SeqCst) {
                std::thread::sleep(tick);
                let stuck = busy_since
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .is_some_and(|t| t.elapsed() > deadline);
                if stuck {
                    // The runtime does not take its frames (ARC-F14): stop its process group
                    // and report the exit, even if a grandchild keeps the pipes open.
                    kill(&c);
                    watchdog_exit();
                    break;
                }
            }
        })?;

        let d = done.clone();
        spawn_thread("bottle-agent-runtime-reader", move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(e) = parse_event(&line) {
                    events(e);
                }
            }
            d.store(true, Ordering::SeqCst);
            exit_once();
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_f14_parses_session_failed_and_ignores_other_lines() {
        assert_eq!(
            parse_event(r#"{"type":"session_failed","sessionId":"as-1"}"#),
            Some(RuntimeEvent::SessionFailed {
                session_id: "as-1".into()
            })
        );
        assert_eq!(parse_event("not json"), None);
        assert_eq!(parse_event(r#"{"type":"log","text":"x"}"#), None);
        assert_eq!(parse_event(r#"{"type":"session_failed"}"#), None);
    }
}
