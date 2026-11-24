//! L1 (review of slice 2a): the MCP shim that Genie Code starts runs in its OWN process group, so
//! the group kill of the session host (`ServerProc::terminate`) does not reach it. The shim must
//! end on its own when the app-server dies. This test starts the real `bottle --agent-mcp-shim`
//! under a shell that puts it in its own group (`set -m`), connects it to a fake guard socket,
//! ends the "app-server" through the real spawner, and checks that no shim process is left.
#![cfg(unix)]

use std::os::unix::net::UnixListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mog_core::agent_run::guard::channel::{read_frame, write_frame};
use mog_core::agent_run::guard::shim::{ENV_SECRET, ENV_SESSION, ENV_SOCKET};
use mog_core::agent_run::seam::SHIM_ARG;
use mog_core::agent_run::session::spawn::ProcessSpawner;
use mog_core::agent_run::session::{SpawnSpec, Spawner};

fn left(marker: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", marker])
        .output()
        .map(|o| !o.stdout.is_empty())
        .unwrap_or(false)
}

#[test]
fn no_shim_process_is_left_after_the_app_server_is_terminated() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let id = format!("{}-{nanos:x}", std::process::id());
    let dir = std::env::temp_dir().join(format!("shim-{id}"));
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("a.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    let connected = Arc::new(AtomicBool::new(false));
    let c = Arc::clone(&connected);
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let hello = read_frame(&mut s).unwrap().unwrap();
        assert!(hello.get("hello").is_some());
        write_frame(&mut s, &serde_json::json!({"ok": true, "tool": {}})).unwrap();
        c.store(true, Ordering::SeqCst);
        // Keep the channel open: only the parent change may end the shim.
        std::thread::sleep(Duration::from_secs(8));
    });

    let marker = format!("orphan-marker-{id}");
    let script = format!("set -m; sleep 20 | \"$BIN\" {SHIM_ARG} {marker} & wait");
    let spec = SpawnSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script],
        env: vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("BIN".into(), env!("CARGO_BIN_EXE_bottle").into()),
            (ENV_SOCKET.into(), sock.to_string_lossy().into_owned()),
            (ENV_SESSION.into(), "s".repeat(32)),
            (ENV_SECRET.into(), "00".repeat(32)),
        ],
        cwd: None,
    };
    let mut proc = ProcessSpawner.spawn(&spec).unwrap();
    let _io = proc.take_io();
    let start = Instant::now();
    while !connected.load(Ordering::SeqCst) {
        assert!(start.elapsed() < Duration::from_secs(10), "the shim did not connect");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(left(&marker), "the shim runs before the end");

    proc.terminate();

    let start = Instant::now();
    while left(&marker) {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "a shim process is left after terminate()"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
}
