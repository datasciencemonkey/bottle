//! The app socket between the shim and the guard (Section 2.3, A4).
//!
//! One socket at `<app data>/agent/a.sock` in a 0700 folder, mode 0600, unlinked at start and at
//! stop. If the path is longer than 100 bytes (the `sun_path` limit), it goes under
//! `$TMPDIR/bottle-<uid>/a.sock`. Frames are a u32 big-endian length plus JSON, 64 KB at most in
//! each direction.
//!
//! Authentication runs in this order. Each failure closes the connection with no reply:
//! 1. The peer uid equals our uid.
//! 2. The parent PID of the peer is the `genie app-server` PID of an open session.
//! 3. The first frame is `hello{sessionId, secret}`. The 256-bit secret is compared in constant
//!    time, and the session must be the one whose app-server PID is that parent.
//! 4. One connection per session.
//! 5. The secret is valid only while the session is open.
//!
//! After `hello`, each `call` frame runs `ask_genie` on its own thread. A `cancel` frame is
//! accepted and ignored: a stop of the turn comes from the session host through `stop_turn`,
//! because a timeout keeps the Genie calls polling.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use serde_json::{json, Value};

use super::{peer, tool_schema, AgentGuard};

/// The largest frame, in each direction.
pub const MAX_FRAME: usize = 64 * 1024;
/// The longest socket path (the `sun_path` limit is 104 on macOS and 108 on Linux).
pub const MAX_SOCKET_PATH: usize = 100;
/// The most `ask_genie` calls at once on one connection.
pub const MAX_CALLS_PER_CONNECTION: usize = 32;
/// The time that a new connection has to send `hello`.
pub const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Write one frame: a u32 big-endian length plus JSON.
pub fn write_frame(w: &mut impl Write, v: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(v).map_err(|_| io::ErrorKind::InvalidData)?;
    if body.len() > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    w.write_all(&out)?;
    w.flush()
}

/// Read one frame. `Ok(None)` is a clean end at a frame boundary. A frame over [`MAX_FRAME`] or a
/// body that is not JSON is `InvalidData`, and the caller closes the connection.
pub fn read_frame(r: &mut impl Read) -> io::Result<Option<Value>> {
    let mut len = [0u8; 4];
    match r.read(&mut len[..1])? {
        0 => return Ok(None),
        _ => r.read_exact(&mut len[1..])?,
    }
    let n = u32::from_be_bytes(len) as usize;
    if n > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut body = vec![0u8; n];
    r.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|_| io::ErrorKind::InvalidData.into())
}

/// Lower-case hex of `b`.
pub fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The bytes of a hex string, or `None` if it is not an even run of hex digits.
pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || !s.is_ascii() {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// The socket path for the app data folder `dir`.
pub fn socket_path(dir: &Path) -> PathBuf {
    let p = dir.join("a.sock");
    if p.as_os_str().len() <= MAX_SOCKET_PATH {
        return p;
    }
    std::env::temp_dir()
        .join(format!("bottle-{}", peer::own_uid()))
        .join("a.sock")
}

fn private_dir(dir: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
}

type Conns = Arc<Mutex<HashMap<String, UnixStream>>>;

/// The listener of one guard. [`Server::stop`] ends it and unlinks the socket.
pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    accept: Option<JoinHandle<()>>,
    conns: Conns,
}

impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Server").finish_non_exhaustive()
    }
}

impl Server {
    /// Stop accepting, close every connection and unlink the socket.
    pub fn stop(mut self) {
        self.shutdown();
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.accept.take() {
            let _ = h.join();
        }
        for (_, s) in self.conns.lock().unwrap_or_else(|p| p.into_inner()).drain() {
            let _ = s.shutdown(std::net::Shutdown::Both);
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

impl AgentGuard {
    /// Start the app socket in `dir` (unlinking a stale one) and return its path. The host gives
    /// the path to the shim through [`super::shim::env_pairs`].
    pub fn listen(self: &Arc<Self>, dir: &Path) -> io::Result<PathBuf> {
        self.listen_as(dir, peer::own_uid())
    }

    /// [`AgentGuard::listen`] that accepts only peers with the uid `uid`. The production uid is
    /// our own; tests use another one to prove the refusal.
    pub fn listen_as(self: &Arc<Self>, dir: &Path, uid: u32) -> io::Result<PathBuf> {
        let path = socket_path(dir);
        if let Some(parent) = path.parent() {
            private_dir(parent)?;
        }
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let conns: Conns = Arc::default();
        let weak = Arc::downgrade(self);
        let accept = {
            let stop = Arc::clone(&stop);
            let conns = Arc::clone(&conns);
            std::thread::Builder::new()
                .name("bottle-agent-accept".into())
                .spawn(move || accept_loop(&listener, &weak, uid, &stop, &conns))?
        };
        let old = self
            .server
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .replace(Server {
                path: path.clone(),
                stop,
                accept: Some(accept),
                conns,
            });
        if let Some(old) = old {
            old.stop();
        }
        Ok(path)
    }

    /// Stop the app socket. The sessions stay.
    pub fn shutdown(&self) {
        let s = self.server.lock().unwrap_or_else(|p| p.into_inner()).take();
        if let Some(s) = s {
            s.stop();
        }
    }

    /// Close the shim connection of a session.
    pub(crate) fn close_connection(&self, session_id: &str) {
        let conns = self
            .server
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map(|s| Arc::clone(&s.conns));
        if let Some(conns) = conns {
            if let Some(s) = conns
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .remove(session_id)
            {
                let _ = s.shutdown(std::net::Shutdown::Both);
            }
        }
    }
}

fn accept_loop(
    listener: &UnixListener,
    guard: &Weak<AgentGuard>,
    uid: u32,
    stop: &AtomicBool,
    conns: &Conns,
) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let Some(g) = guard.upgrade() else { return };
                let conns = Arc::clone(conns);
                let _ = std::thread::Builder::new()
                    .name("bottle-agent-conn".into())
                    .spawn(move || serve(&g, stream, uid, &conns));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

/// The `hello` frame: `{"hello": {"sessionId": .., "secret": <64 hex>}}`.
fn parse_hello(v: &Value) -> Option<(String, Vec<u8>)> {
    let h = v.as_object()?.get("hello")?.as_object()?;
    let sid = h.get("sessionId")?.as_str()?.to_owned();
    let secret = hex_decode(h.get("secret")?.as_str()?)?;
    Some((sid, secret))
}

fn serve(guard: &Arc<AgentGuard>, stream: UnixStream, uid: u32, conns: &Conns) {
    let _ = stream.set_nonblocking(false);
    // 1. The uid of the peer.
    let Ok(p) = peer::peer(&stream) else { return };
    if p.uid != uid {
        return;
    }
    // 2. The parent of the peer is the app-server of an open session.
    let Some(ppid) = peer::parent_pid(p.pid) else {
        return;
    };
    if !guard.pid_is_bound(ppid) {
        return;
    }
    // 3. The hello, with the secret. 4. One connection. 5. The session is open.
    let Ok(mut reader) = stream.try_clone() else {
        return;
    };
    let _ = reader.set_read_timeout(Some(HELLO_TIMEOUT));
    let Ok(Some(first)) = read_frame(&mut reader) else {
        return;
    };
    let Some((sid, secret)) = parse_hello(&first) else {
        return;
    };
    if !guard.accept_hello(&sid, &secret, ppid) {
        return;
    }
    let Ok(writer_stream) = stream.try_clone() else {
        guard.release_connection(&sid);
        return;
    };
    conns
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(sid.clone(), stream);
    let writer = Arc::new(Mutex::new(writer_stream));
    let _ = reader.set_read_timeout(None);
    let hello_ok = json!({"ok": true, "tool": tool_schema()});
    let sent = write_frame(
        &mut *writer.lock().unwrap_or_else(|p| p.into_inner()),
        &hello_ok,
    );
    if sent.is_ok() {
        frames(guard, &sid, &mut reader, &writer);
    }
    guard.release_connection(&sid);
    conns.lock().unwrap_or_else(|p| p.into_inner()).remove(&sid);
}

fn frames(
    guard: &Arc<AgentGuard>,
    sid: &str,
    reader: &mut UnixStream,
    writer: &Arc<Mutex<UnixStream>>,
) {
    let running = Arc::new(AtomicUsize::new(0));
    while let Ok(Some(frame)) = read_frame(reader) {
        let Some(o) = frame.as_object() else { return };
        let Some(id) = o.get("id").and_then(Value::as_u64) else {
            return;
        };
        match o.get("op").and_then(Value::as_str) {
            Some("call") => {
                let args = o.get("args").cloned().unwrap_or(Value::Null);
                if running.fetch_add(1, Ordering::SeqCst) >= MAX_CALLS_PER_CONNECTION {
                    running.fetch_sub(1, Ordering::SeqCst);
                    reply(
                        writer,
                        id,
                        &super::receipt::ToolOut::refusal("invalid_args"),
                    );
                    continue;
                }
                let (g, w, sid, running) = (
                    Arc::clone(guard),
                    Arc::clone(writer),
                    sid.to_owned(),
                    Arc::clone(&running),
                );
                let spawned = std::thread::Builder::new()
                    .name("bottle-agent-ask".into())
                    .spawn(move || {
                        let out = g.ask_genie(&sid, &args);
                        reply(&w, id, &out);
                        running.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    return;
                }
            }
            // Accepted and ignored (see the module doc).
            Some("cancel") => {}
            _ => return,
        }
    }
}

fn reply(w: &Arc<Mutex<UnixStream>>, id: u64, out: &super::receipt::ToolOut) {
    let v = json!({"id": id, "text": out.text, "isError": out.is_error});
    let mut w = w.lock().unwrap_or_else(|p| p.into_inner());
    if write_frame(&mut *w, &v).is_err() {
        // Escaping made the frame too big: the model gets a short fixed result instead.
        let short = json!({"id": id, "text": "{\"status\":\"too_large\"}", "isError": true});
        let _ = write_frame(&mut *w, &short);
    }
}
