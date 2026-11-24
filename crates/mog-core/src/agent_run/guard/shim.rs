//! The stdio MCP shim (Section 2.3, A4). Genie Code starts it once per server process, as
//! `bottle --agent-mcp-shim`. It holds no rule. It speaks MCP over stdio (JSON-RPC, one message
//! per line), answers `initialize` and `ping` itself, and forwards `tools/call` of `ask_genie` and
//! `notifications/cancelled` to the guard over the app socket. It caps each frame at 64 KB in
//! each direction, and runs several calls at once, matched by request id.
//!
//! The tool schema comes from the guard in the `hello` reply. It has no enum of aliases.
//! `main.rs` calls [`run`] when its first argument is [`super::super::seam::SHIM_ARG`].
//!
//! The shim uses std only (and `libc` for the parent signal). It prints nothing to stderr: the
//! exit code tells why it ended (2 not configured, 1 no channel or `hello` refused, 0 stdin ended).

use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::channel::{hex_encode, read_frame, write_frame, MAX_FRAME};
use super::TOOL_NAME;
use crate::agent_run::seam::RunSecret;

/// The environment variable with the socket path.
pub const ENV_SOCKET: &str = "BOTTLE_AGENT_SOCK";
/// The environment variable with the session id.
pub const ENV_SESSION: &str = "BOTTLE_AGENT_SESSION";
/// The environment variable with the secret (64 hex digits).
pub const ENV_SECRET: &str = "BOTTLE_AGENT_SECRET";

/// The MCP protocol version that the shim answers with, if the client names none.
const PROTOCOL_VERSION: &str = "2025-06-18";

/// The environment that the session host sets for the shim in `config.toml`. The secret is
/// returned as text for the one config write and must not be kept.
pub fn env_pairs(
    socket: &Path,
    session_id: &str,
    secret: &RunSecret,
) -> Vec<(&'static str, String)> {
    vec![
        (ENV_SOCKET, socket.to_string_lossy().into_owned()),
        (ENV_SESSION, session_id.to_owned()),
        (ENV_SECRET, hex_encode(&secret.0)),
    ]
}

/// How [`run_with`] behaves at the end of the channel.
#[derive(Debug, Clone, Copy)]
pub struct ShimOpts {
    /// When the socket ends, send SIGTERM to the parent (the app-server) and exit, so a Bottle
    /// crash leaves no orphan.
    pub exit_on_socket_eof: bool,
    /// When stdin ends, wait this long for the calls that are still open.
    pub drain: Duration,
}

/// The shim entry for `main.rs`. It returns the exit code.
pub fn run() -> i32 {
    let (Ok(sock), Ok(sid), Ok(secret)) = (
        std::env::var(ENV_SOCKET),
        std::env::var(ENV_SESSION),
        std::env::var(ENV_SECRET),
    ) else {
        // No print: the exit code says it (2: not configured).
        return 2;
    };
    let Ok(stream) = UnixStream::connect(&sock) else {
        // 1: no channel.
        return 1;
    };
    let (Some(input), Some(output)) = (std_file(0), std_file(1)) else {
        return 1;
    };
    run_with(
        stream,
        &sid,
        &secret,
        io::BufReader::new(input),
        output,
        ShimOpts {
            exit_on_socket_eof: true,
            drain: Duration::ZERO,
        },
    )
}

/// A private handle on the process stdin (0) or stdout (1). The shim speaks the MCP protocol on
/// them: it is not a log, and the log gate bans the print and `stdout()` names.
fn std_file(fd: libc::c_int) -> Option<std::fs::File> {
    // SAFETY: `dup` takes a plain integer. A negative result is an error.
    let copy = unsafe { libc::dup(fd) };
    if copy < 0 {
        return None;
    }
    // SAFETY: `copy` is a new open descriptor that nothing else owns.
    Some(std::fs::File::from(unsafe { OwnedFd::from_raw_fd(copy) }))
}

/// One message per line, 64 KB at most. An oversize line is read to its end and dropped.
enum Line {
    Msg(Vec<u8>),
    TooLong,
    Eof,
}

fn read_line(r: &mut impl BufRead) -> io::Result<Line> {
    let mut buf = Vec::new();
    let mut too_long = false;
    loop {
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            return Ok(if buf.is_empty() && !too_long {
                Line::Eof
            } else if too_long {
                Line::TooLong
            } else {
                Line::Msg(buf)
            });
        }
        let (take, done) = match chunk.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (chunk.len(), false),
        };
        if !too_long {
            buf.extend_from_slice(&chunk[..take - usize::from(done)]);
            if buf.len() > MAX_FRAME {
                too_long = true;
                buf = Vec::new();
            }
        }
        r.consume(take);
        if done {
            return Ok(if too_long {
                Line::TooLong
            } else {
                Line::Msg(buf)
            });
        }
    }
}

struct Pending {
    /// Frame id to the JSON-RPC id of the request.
    by_frame: HashMap<u64, Value>,
}

/// The shim over a connected `stream`, `input` (stdin) and `output` (stdout).
pub fn run_with<R, W>(
    mut stream: UnixStream,
    session_id: &str,
    secret_hex: &str,
    mut input: R,
    output: W,
    opts: ShimOpts,
) -> i32
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let hello = json!({"hello": {"sessionId": session_id, "secret": secret_hex}});
    if write_frame(&mut stream, &hello).is_err() {
        return 1;
    }
    let tool = match read_frame(&mut stream) {
        Ok(Some(v)) if v.get("ok") == Some(&Value::Bool(true)) => {
            v.get("tool").cloned().unwrap_or(Value::Null)
        }
        _ => return 1,
    };
    let Ok(read_half) = stream.try_clone() else {
        return 1;
    };
    let writer = Arc::new(Mutex::new(stream));
    let out = Arc::new(Mutex::new(output));
    let pending = Arc::new(Mutex::new(Pending {
        by_frame: HashMap::new(),
    }));
    let closed = Arc::new(AtomicBool::new(false));
    // True once this side closes the channel, so the reader does not take it for a crash.
    let ending = Arc::new(AtomicBool::new(false));
    let next = AtomicU64::new(1);

    // Results from the guard go to stdout as JSON-RPC responses.
    let reader = {
        let (out, pending, closed, ending) = (
            Arc::clone(&out),
            Arc::clone(&pending),
            Arc::clone(&closed),
            Arc::clone(&ending),
        );
        let mut read_half = read_half;
        std::thread::spawn(move || {
            while let Ok(Some(f)) = read_frame(&mut read_half) {
                let Some(id) = f.get("id").and_then(Value::as_u64) else {
                    continue;
                };
                // A cancelled request has no entry: no response (MCP).
                let jid = pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .by_frame
                    .remove(&id);
                let Some(jid) = jid else { continue };
                let text = f.get("text").and_then(Value::as_str).unwrap_or("");
                let is_error = f.get("isError").and_then(Value::as_bool).unwrap_or(true);
                respond(
                    &out,
                    &json!({"jsonrpc": "2.0", "id": jid, "result": {
                        "content": [{"type": "text", "text": text}], "isError": is_error}}),
                );
            }
            closed.store(true, Ordering::SeqCst);
            if opts.exit_on_socket_eof && !ending.load(Ordering::SeqCst) {
                // SAFETY: `getppid` and `kill` take plain integers. The parent is the app-server.
                unsafe {
                    libc::kill(libc::getppid(), libc::SIGTERM);
                }
                std::process::exit(1);
            }
        })
    };

    let send = |v: &Value| -> bool {
        write_frame(&mut *writer.lock().unwrap_or_else(|p| p.into_inner()), v).is_ok()
    };
    loop {
        if closed.load(Ordering::SeqCst) {
            break;
        }
        let line = match read_line(&mut input) {
            Ok(Line::Msg(l)) => l,
            Ok(Line::TooLong) => {
                respond(
                    &out,
                    &json!({"jsonrpc": "2.0", "id": Value::Null,
                        "error": {"code": -32600, "message": "message too large"}}),
                );
                continue;
            }
            Ok(Line::Eof) | Err(_) => break,
        };
        let Ok(msg) = serde_json::from_slice::<Value>(&line) else {
            respond(
                &out,
                &json!({"jsonrpc": "2.0", "id": Value::Null,
                    "error": {"code": -32700, "message": "parse error"}}),
            );
            continue;
        };
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            ("notifications/cancelled", None) => {
                let rid = params.get("requestId");
                let frame = {
                    let mut p = pending.lock().unwrap_or_else(|p| p.into_inner());
                    let f = p
                        .by_frame
                        .iter()
                        .find(|(_, j)| Some(*j) == rid)
                        .map(|(f, _)| *f);
                    if let Some(f) = f {
                        p.by_frame.remove(&f);
                    }
                    f
                };
                if let Some(f) = frame {
                    let _ = send(&json!({"id": f, "op": "cancel"}));
                }
            }
            (_, None) => {}
            ("initialize", Some(id)) => {
                let v = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION);
                respond(
                    &out,
                    &json!({"jsonrpc": "2.0", "id": id, "result": {
                        "protocolVersion": v,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "bottle-agent-shim", "version": env!("CARGO_PKG_VERSION")},
                    }}),
                );
            }
            ("ping", Some(id)) => {
                respond(&out, &json!({"jsonrpc": "2.0", "id": id, "result": {}}));
            }
            ("tools/list", Some(id)) => {
                respond(
                    &out,
                    &json!({"jsonrpc": "2.0", "id": id, "result": {"tools": [tool.clone()]}}),
                );
            }
            ("tools/call", Some(id)) => {
                let name = params.get("name").and_then(Value::as_str);
                if name != Some(TOOL_NAME) {
                    respond(
                        &out,
                        &json!({"jsonrpc": "2.0", "id": id,
                            "error": {"code": -32602, "message": "unknown tool"}}),
                    );
                    continue;
                }
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                let fid = next.fetch_add(1, Ordering::SeqCst);
                pending
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .by_frame
                    .insert(fid, id.clone());
                if !send(&json!({"id": fid, "op": "call", "args": args})) {
                    pending
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .by_frame
                        .remove(&fid);
                    respond(
                        &out,
                        &json!({"jsonrpc": "2.0", "id": id,
                            "error": {"code": -32602, "message": "request too large"}}),
                    );
                }
            }
            (_, Some(id)) => respond(
                &out,
                &json!({"jsonrpc": "2.0", "id": id,
                    "error": {"code": -32601, "message": "method not found"}}),
            ),
        }
    }
    // stdin ended: wait for open calls, up to `drain`, then close the channel.
    let end = Instant::now() + opts.drain;
    while Instant::now() < end
        && !closed.load(Ordering::SeqCst)
        && !pending
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .by_frame
            .is_empty()
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    ending.store(true, Ordering::SeqCst);
    let _ = writer
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .shutdown(std::net::Shutdown::Both);
    let _ = reader.join();
    0
}

fn respond<W: Write>(out: &Arc<Mutex<W>>, v: &Value) {
    let mut w = out.lock().unwrap_or_else(|p| p.into_inner());
    let line = v.to_string();
    // The shim answers at most 64 KB per frame (the guard caps the text).
    if line.len() <= MAX_FRAME {
        let _ = writeln!(w, "{line}");
        let _ = w.flush();
    }
}
