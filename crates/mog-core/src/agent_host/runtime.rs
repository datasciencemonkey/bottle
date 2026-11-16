//! The shared harness runtime process (RTR-21a, U11) and its launcher.
//!
//! One runtime process of a harness kind hosts the sessions of every MOG that uses that kind.
//! The host talks to it with JSON lines: frames on stdin (`open_session`, `turn`,
//! `close_session`), events on stdout (`session_failed`). End of stdout means the process exited.
//! TODO(#111): SP-19 picks the sidecar program (Agent SDK or headless CLI) and fixes the
//! protocol; [`StdRuntimeLauncher`] already gives it a clean environment.

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};

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
}

impl RuntimeConfig {
    /// A config for `program` with `args`.
    pub fn new(program: impl Into<PathBuf>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
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
    /// Write one frame to the runtime.
    fn send_frame(&self, frame: &Value) -> Result<(), AgentError>;
    /// Stop the process.
    fn stop(&self);
}

/// Starts runtime processes.
pub trait RuntimeLauncher: Send + Sync {
    /// Start the runtime of `spec`. `events` gets its events until it exits.
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

/// Starts the runtime as a real child process with piped stdin and stdout. Its stderr goes to
/// the null device: harness output can hold user content and is never logged (SEC rules).
#[derive(Debug, Default, Clone, Copy)]
pub struct StdRuntimeLauncher;

struct StdRuntimeProcess {
    child: Mutex<Child>,
    stdin: Mutex<Option<ChildStdin>>,
}

impl RuntimeProcess for StdRuntimeProcess {
    fn send_frame(&self, frame: &Value) -> Result<(), AgentError> {
        let mut guard = self.stdin.lock().unwrap_or_else(|p| p.into_inner());
        let stdin = guard.as_mut().ok_or(AgentError::Unavailable)?;
        let mut line = frame.to_string();
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|_| AgentError::Unavailable)
    }

    fn stop(&self) {
        self.stdin.lock().unwrap_or_else(|p| p.into_inner()).take();
        let mut child = self.child.lock().unwrap_or_else(|p| p.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

impl Drop for StdRuntimeProcess {
    fn drop(&mut self) {
        self.stop();
    }
}

impl RuntimeLauncher for StdRuntimeLauncher {
    fn launch(
        &self,
        spec: &LaunchSpec,
        events: RuntimeEvents,
    ) -> Result<Box<dyn RuntimeProcess>, AgentError> {
        let mut child = harness_command(spec)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| AgentError::LaunchFailed)?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or(AgentError::LaunchFailed)?;
        std::thread::Builder::new()
            .name("bottle-agent-runtime".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    if let Some(e) = parse_event(&line) {
                        events(e);
                    }
                }
                events(RuntimeEvent::Exited);
            })
            .map_err(|_| AgentError::LaunchFailed)?;
        Ok(Box::new(StdRuntimeProcess {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
        }))
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
