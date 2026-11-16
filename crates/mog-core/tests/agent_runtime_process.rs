//! The std runtime launcher of R16 AgentHost with a REAL child process (#112, RTR-21a, ARC-F14).
//!
//! A shell script stands in for the shared harness runtime: it reads one session frame from
//! stdin, reports that session as failed and exits. The launcher must deliver the frame on
//! stdin, turn the stdout line into `SessionFailed` and report `Exited` at end of output.
//! TODO(#111): the real sidecar protocol comes from SP-19.
#![cfg(unix)]

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use mog_core::agent_host::adapter::launch_spec;
use mog_core::agent_host::runtime::{
    RuntimeConfig, RuntimeEvent, RuntimeLauncher, StdRuntimeLauncher,
};
use mog_core::agent_host::Harness;
use serde_json::json;

/// A busy machine (several agents building at once) can take tens of seconds to start
/// `/bin/sh`. Every wait in this file ends on an event; `BOUND` only keeps a broken run from
/// hanging. Where a test must tell the watchdog from a natural exit, the runtime lives for
/// 600 s, far longer than `BOUND`.
const BOUND: Duration = Duration::from_secs(120);

#[test]
fn rtr_21a_arc_f14_std_launcher_delivers_frames_and_reports_session_failure_and_exit() {
    let script = r#"read line; case "$line" in *open_session*) printf '%s\n' '{"type":"session_failed","sessionId":"as-1"}';; esac"#;
    let cfg = RuntimeConfig::new("/bin/sh", vec!["-c".into(), script.into()]);
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let events = Arc::new(move |e: RuntimeEvent| {
        let _ = tx.lock().unwrap().send(e);
    });
    let proc = StdRuntimeLauncher::default()
        .launch(&spec, events)
        .expect("spawns");
    proc.send_frame(&json!({"type": "open_session", "sessionId": "as-1"}))
        .unwrap();
    let first = rx.recv_timeout(BOUND).expect("event");
    assert_eq!(
        first,
        RuntimeEvent::SessionFailed {
            session_id: "as-1".into()
        }
    );
    let second = rx.recv_timeout(BOUND).expect("exit");
    assert_eq!(second, RuntimeEvent::Exited);
    proc.stop();
}

#[test]
fn arc_f14_std_launcher_reports_exit_when_the_runtime_dies() {
    let cfg = RuntimeConfig::new("/bin/sh", vec!["-c".into(), "exit 3".into()]);
    let spec = launch_spec(Harness::Codex, &cfg, "http://127.0.0.1:1");
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let proc = StdRuntimeLauncher::default()
        .launch(
            &spec,
            Arc::new(move |e| {
                let _ = tx.lock().unwrap().send(e);
            }),
        )
        .expect("spawns");
    assert_eq!(rx.recv_timeout(BOUND).unwrap(), RuntimeEvent::Exited);
    proc.stop();
}

#[test]
fn arc_f14_std_launcher_fails_cleanly_for_a_missing_program() {
    let cfg = RuntimeConfig::new("/nonexistent/bottle-harness-runtime", Vec::new());
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let r = StdRuntimeLauncher::default().launch(&spec, Arc::new(|_| {}));
    assert_eq!(r.err().map(|e| e.code()), Some("agent_launch_failed"));
}

/// Calls `send_frame` with `frame` `times` times on a helper thread and returns how many calls
/// were refused, or `None` if the calls did not all return within [`BOUND`].
fn send_on_thread(
    proc: &Arc<Box<dyn mog_core::agent_host::runtime::RuntimeProcess>>,
    frame: serde_json::Value,
    times: usize,
) -> Option<usize> {
    let (done_tx, done_rx) = mpsc::channel();
    let p = proc.clone();
    std::thread::spawn(move || {
        let refused = (0..times).filter(|_| p.send_frame(&frame).is_err()).count();
        let _ = done_tx.send(refused);
    });
    done_rx.recv_timeout(BOUND).ok()
}

#[test]
fn arc_f14_send_frame_returns_while_the_runtime_does_not_read() {
    // The runtime stays alive and never reads stdin, and the write deadline is 600 s, so the
    // watchdog does not act within BOUND. A `send_frame` that wrote to the pipe itself would
    // block on the full pipe (2 MiB of frames, the pipe holds 64 KiB) for 600 s.
    let cfg = RuntimeConfig::new("/bin/sleep", vec!["600".into()]);
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let launcher = StdRuntimeLauncher {
        write_deadline: Duration::from_secs(600),
    };
    let proc = Arc::new(launcher.launch(&spec, Arc::new(|_| {})).expect("spawns"));
    let big = json!({ "type": "turn", "sessionId": "as-1", "turn": "x".repeat(256 * 1024) });
    let refused = send_on_thread(&proc, big, 8);
    proc.stop();
    assert_eq!(
        refused,
        Some(0),
        "send_frame never blocks and queues all 8 frames"
    );
}

#[test]
fn arc_f14_std_launcher_stops_a_runtime_that_does_not_take_its_frames() {
    // The runtime stays alive for 600 s and never reads stdin: "does not answer" (ARC-F14).
    // An `Exited` within BOUND comes from the watchdog (300 ms deadline), not a natural exit.
    let cfg = RuntimeConfig::new("/bin/sleep", vec!["600".into()]);
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let launcher = StdRuntimeLauncher {
        write_deadline: Duration::from_millis(300),
    };
    let proc = launcher
        .launch(
            &spec,
            Arc::new(move |e| {
                let _ = tx.lock().unwrap().send(e);
            }),
        )
        .expect("spawns");
    let big = json!({ "type": "turn", "sessionId": "as-1", "turn": "x".repeat(256 * 1024) });
    for _ in 0..8 {
        let _ = proc.send_frame(&big);
    }
    assert_eq!(
        rx.recv_timeout(BOUND).expect("killed"),
        RuntimeEvent::Exited
    );
    proc.stop();
}

/// Makes a named pipe. Opening one end blocks until the other end opens, so the test waits on
/// an event from the script instead of polling a file on a timer (a busy machine can take tens of
/// seconds to start `/bin/sh`).
fn fifo(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("bottle-agent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_file(&path);
    let made = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo makes {}", path.display());
    path
}

/// Reads a named pipe to its end on a thread and sends what it read. End of file comes only
/// when every writer has closed its end.
fn read_to_eof(path: std::path::PathBuf) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut out = String::new();
        if let Ok(mut f) = std::fs::File::open(&path) {
            let _ = f.read_to_string(&mut out);
        }
        let _ = tx.send(out);
    });
    rx
}

#[test]
fn arc_f14_watchdog_reports_exit_and_stops_a_grandchild_that_holds_the_pipes() {
    // A wrapper that leaves a grandchild behind; the grandchild keeps stdin and stdout open for
    // 600 s, so an `Exited` within the bound below cannot come from the end of stdout.
    // The grandchild alone also holds the write end of `alive` (fd 3): that pipe reaches end of
    // file exactly when the grandchild has exited, which needs no polling with `kill -0` (a
    // zombie still answers it).
    let pid_fifo = fifo("gc-pid");
    let alive_fifo = fifo("gc-alive");
    let script = format!(
        "sleep 600 3>'{}' & echo $! > '{}'; exec sleep 600",
        alive_fifo.display(),
        pid_fifo.display()
    );
    let pid_rx = read_to_eof(pid_fifo.clone());
    let gone_rx = read_to_eof(alive_fifo.clone());
    let cfg = RuntimeConfig::new("/bin/sh", vec!["-c".into(), script]);
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let launcher = StdRuntimeLauncher {
        write_deadline: Duration::from_millis(300),
    };
    let proc = launcher
        .launch(
            &spec,
            Arc::new(move |e| {
                let _ = tx.lock().unwrap().send(e);
            }),
        )
        .expect("spawns");
    let pid = pid_rx
        .recv_timeout(BOUND)
        .expect("the wrapper wrote the grandchild pid")
        .trim()
        .to_owned();
    assert!(!pid.is_empty(), "the grandchild started");
    let big = json!({ "type": "turn", "sessionId": "as-1", "turn": "x".repeat(256 * 1024) });
    for _ in 0..4 {
        let _ = proc.send_frame(&big);
    }
    assert_eq!(
        rx.recv_timeout(BOUND)
            .expect("Exited although the pipes stay open"),
        RuntimeEvent::Exited
    );
    let gone = gone_rx.recv_timeout(BOUND).is_ok();
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "Exited arrives once"
    );
    if !gone {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-9", &pid])
            .status();
    }
    let _ = std::fs::remove_file(&pid_fifo);
    let _ = std::fs::remove_file(&alive_fifo);
    assert!(
        gone,
        "the grandchild in the runtime's process group is stopped"
    );
    proc.stop();
}

#[test]
fn arc_f14_a_full_frame_queue_fails_fast_instead_of_blocking() {
    // The runtime never reads and the write deadline is 600 s, so nothing drains the queue
    // within BOUND: a `send_frame` that waited for room would block for 600 s.
    let cfg = RuntimeConfig::new("/bin/sleep", vec!["600".into()]);
    let spec = launch_spec(Harness::Codex, &cfg, "http://127.0.0.1:1");
    let launcher = StdRuntimeLauncher {
        write_deadline: Duration::from_secs(600),
    };
    let proc = Arc::new(launcher.launch(&spec, Arc::new(|_| {})).expect("spawns"));
    let frame = json!({ "type": "turn", "sessionId": "as-1", "turn": "y".repeat(8 * 1024) });
    let refused = send_on_thread(
        &proc,
        frame,
        mog_core::agent_host::runtime::FRAME_QUEUE + 72,
    );
    proc.stop();
    let refused = refused.expect("send_frame never blocks");
    assert!(refused > 0, "a full queue gives agent_unavailable");
}

#[test]
fn arc_f14_watchdog_reports_exit_when_an_escaped_process_holds_the_pipes() {
    // A grandchild that leaves the process group (setsid) keeps the pipes open, so the group
    // kill cannot close them. Only the watchdog's own `Exited` lets the host recover.
    if std::process::Command::new("python3")
        .arg("-V")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        panic!("python3 is required for this test (it starts the setsid grandchild)");
    }
    // Both processes live for 600 s, so an `Exited` within BOUND comes from the watchdog.
    let pid_fifo = fifo("esc-pid");
    let script = format!(
        "python3 -c 'import os,time; os.setsid(); time.sleep(600)' & echo $! > '{}'; exec sleep 600",
        pid_fifo.display()
    );
    let pid_rx = read_to_eof(pid_fifo.clone());
    let cfg = RuntimeConfig::new("/bin/sh", vec!["-c".into(), script]);
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let (tx, rx) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let launcher = StdRuntimeLauncher {
        write_deadline: Duration::from_millis(300),
    };
    let proc = launcher
        .launch(
            &spec,
            Arc::new(move |e| {
                let _ = tx.lock().unwrap().send(e);
            }),
        )
        .expect("spawns");
    let pid = pid_rx
        .recv_timeout(BOUND)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let big = json!({ "type": "turn", "sessionId": "as-1", "turn": "x".repeat(256 * 1024) });
    for _ in 0..4 {
        let _ = proc.send_frame(&big);
    }
    let got = rx.recv_timeout(BOUND);
    // Clean up the escaped process before any assert.
    if !pid.is_empty() {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-9", &pid])
            .status();
    }
    let _ = std::fs::remove_file(&pid_fifo);
    assert!(!pid.is_empty(), "the escaped grandchild started");
    assert_eq!(got.expect("Exited from the watchdog"), RuntimeEvent::Exited);
    proc.stop();
}
