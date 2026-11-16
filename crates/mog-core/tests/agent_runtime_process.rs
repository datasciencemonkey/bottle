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
    let proc = StdRuntimeLauncher.launch(&spec, events).expect("spawns");
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
    let proc = StdRuntimeLauncher
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
    let r = StdRuntimeLauncher.launch(&spec, Arc::new(|_| {}));
    assert_eq!(
        r.err().map(|e| e.code()),
        Some("agent_launch_failed")
    );
}
