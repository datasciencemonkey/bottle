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
    let first = rx.recv_timeout(Duration::from_secs(10)).expect("event");
    assert_eq!(
        first,
        RuntimeEvent::SessionFailed {
            session_id: "as-1".into()
        }
    );
    let second = rx.recv_timeout(Duration::from_secs(10)).expect("exit");
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
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).unwrap(),
        RuntimeEvent::Exited
    );
    proc.stop();
}

#[test]
fn arc_f14_std_launcher_fails_cleanly_for_a_missing_program() {
    let cfg = RuntimeConfig::new("/nonexistent/bottle-harness-runtime", Vec::new());
    let spec = launch_spec(Harness::Claude, &cfg, "http://127.0.0.1:1");
    let r = StdRuntimeLauncher::default().launch(&spec, Arc::new(|_| {}));
    assert_eq!(r.err().map(|e| e.code()), Some("agent_launch_failed"));
}

#[test]
fn arc_f14_std_launcher_stops_a_runtime_that_does_not_take_its_frames() {
    // The runtime stays alive and never reads stdin: "does not answer" (ARC-F14).
    let cfg = RuntimeConfig::new("/bin/sleep", vec!["30".into()]);
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
    let started = std::time::Instant::now();
    for _ in 0..8 {
        let t = std::time::Instant::now();
        let _ = proc.send_frame(&big);
        assert!(
            t.elapsed() < Duration::from_millis(200),
            "send_frame never blocks"
        );
    }
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(10)).expect("killed"),
        RuntimeEvent::Exited
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    proc.stop();
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn arc_f14_watchdog_reports_exit_and_stops_a_grandchild_that_holds_the_pipes() {
    // A wrapper that leaves a grandchild behind; the grandchild keeps stdin and stdout open.
    let pid_file = std::env::temp_dir().join(format!("bottle-agent-gc-{}", std::process::id()));
    let _ = std::fs::remove_file(&pid_file);
    let script = format!(
        "sleep 30 & echo $! > '{}'; exec sleep 30",
        pid_file.display()
    );
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
    let mut pid = String::new();
    for _ in 0..50 {
        pid = std::fs::read_to_string(&pid_file)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !pid.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!pid.is_empty(), "the grandchild started");
    let big = json!({ "type": "turn", "sessionId": "as-1", "turn": "x".repeat(256 * 1024) });
    for _ in 0..4 {
        let _ = proc.send_frame(&big);
    }
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(5))
            .expect("Exited although the pipes stay open"),
        RuntimeEvent::Exited
    );
    assert!(
        rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "Exited arrives once"
    );
    let mut gone = false;
    for _ in 0..40 {
        if !alive(&pid) {
            gone = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&pid_file);
    assert!(
        gone,
        "the grandchild in the runtime's process group is stopped"
    );
    proc.stop();
}

#[test]
fn arc_f14_a_full_frame_queue_fails_fast_instead_of_blocking() {
    let cfg = RuntimeConfig::new("/bin/sleep", vec!["30".into()]);
    let spec = launch_spec(Harness::Codex, &cfg, "http://127.0.0.1:1");
    let proc = StdRuntimeLauncher::default()
        .launch(&spec, Arc::new(|_| {}))
        .expect("spawns");
    let frame = json!({ "type": "turn", "sessionId": "as-1", "turn": "y".repeat(8 * 1024) });
    let mut refused = 0;
    let t = std::time::Instant::now();
    for _ in 0..(mog_core::agent_host::runtime::FRAME_QUEUE + 72) {
        if proc.send_frame(&frame).is_err() {
            refused += 1;
        }
    }
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "send_frame never blocks"
    );
    assert!(refused > 0, "a full queue gives agent_unavailable");
    proc.stop();
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
    let pid_file = std::env::temp_dir().join(format!("bottle-agent-esc-{}", std::process::id()));
    let _ = std::fs::remove_file(&pid_file);
    let script = format!(
        "python3 -c 'import os,time; os.setsid(); time.sleep(30)' & echo $! > '{}'; exec sleep 30",
        pid_file.display()
    );
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
    let mut pid = String::new();
    for _ in 0..50 {
        pid = std::fs::read_to_string(&pid_file)
            .unwrap_or_default()
            .trim()
            .to_owned();
        if !pid.is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let big = json!({ "type": "turn", "sessionId": "as-1", "turn": "x".repeat(256 * 1024) });
    for _ in 0..4 {
        let _ = proc.send_frame(&big);
    }
    let got = rx.recv_timeout(Duration::from_secs(5));
    // Clean up the escaped process before any assert.
    if !pid.is_empty() {
        let _ = std::process::Command::new("/bin/kill")
            .args(["-9", &pid])
            .status();
    }
    let _ = std::fs::remove_file(&pid_file);
    assert_eq!(got.expect("Exited from the watchdog"), RuntimeEvent::Exited);
    proc.stop();
}
