//! The JSON-RPC client side of `genie app-server` over stdio (plan Amendment 2, A3).
//!
//! This file holds the wire shapes, the line reader with its size cap, the stderr watcher and the
//! [`Spawner`] seam. It holds no policy. It never keeps or logs a JSONL line, a question, an
//! answer or stderr text: a line is parsed and dropped, and stderr only gives fixed [`Hint`]s.

use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;

use serde_json::{json, Value};

/// The largest stdout line that Bottle accepts. A longer line ends the session.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// The size of the stderr buffer. It only feeds the classifier.
pub const STDERR_RING_BYTES: usize = 8 * 1024;

/// One message from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The answer to a request of Bottle. `ok` is false for a JSON-RPC error. The error text is
    /// dropped.
    Response { id: u64, ok: bool, result: Value },
    /// A notification of the server.
    Notification { method: String, params: Value },
    /// A request of the server to the client (an approval, an elicitation, ...). Always a
    /// lockdown violation (AC-2a.23).
    ServerRequest { id: Value, method: String },
}

/// Parse one stdout line. `None` for a line that is not JSON-RPC.
pub fn parse_line(line: &[u8]) -> Option<Incoming> {
    let v: Value = serde_json::from_slice(line).ok()?;
    let obj = v.as_object()?;
    let method = obj.get("method").and_then(Value::as_str);
    match (method, obj.get("id")) {
        (Some(m), Some(id)) => Some(Incoming::ServerRequest {
            id: id.clone(),
            method: m.to_owned(),
        }),
        (Some(m), None) => Some(Incoming::Notification {
            method: m.to_owned(),
            params: obj.get("params").cloned().unwrap_or(Value::Null),
        }),
        (None, Some(id)) => {
            let id = id.as_u64()?;
            let ok = obj.contains_key("result") && !obj.contains_key("error");
            Some(Incoming::Response {
                id,
                ok,
                result: obj.get("result").cloned().unwrap_or(Value::Null),
            })
        }
        (None, None) => None,
    }
}

/// A request line, with its newline.
pub fn request(id: u64, method: &str, params: Value) -> Vec<u8> {
    line(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
}

/// A notification line, with its newline.
pub fn notification(method: &str, params: Option<Value>) -> Vec<u8> {
    let mut m = json!({ "jsonrpc": "2.0", "method": method });
    if let Some(p) = params {
        m["params"] = p;
    }
    line(m)
}

/// The refusal of a request of the server. It holds no text of the request.
pub fn refusal(id: &Value) -> Vec<u8> {
    line(json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32601, "message": "refused by the client" }
    }))
}

fn line(v: Value) -> Vec<u8> {
    let mut b = v.to_string().into_bytes();
    b.push(b'\n');
    b
}

/// The result of [`read_capped_line`].
#[derive(Debug, PartialEq, Eq)]
pub enum LineRead {
    Eof,
    Line,
    /// The line was longer than the cap. It was dropped up to its newline.
    TooLong,
}

/// Read one line into `buf` (without the newline), up to `cap` bytes.
pub fn read_capped_line<R: BufRead>(
    r: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> io::Result<LineRead> {
    buf.clear();
    let mut too_long = false;
    loop {
        let (done, used) = {
            let chunk = match r.fill_buf() {
                Ok(c) => c,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if chunk.is_empty() {
                return Ok(if too_long {
                    LineRead::TooLong
                } else if buf.is_empty() {
                    LineRead::Eof
                } else {
                    LineRead::Line
                });
            }
            match chunk.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    if !too_long {
                        if buf.len() + i > cap {
                            too_long = true;
                        } else {
                            buf.extend_from_slice(&chunk[..i]);
                        }
                    }
                    (true, i + 1)
                }
                None => {
                    if !too_long {
                        if buf.len() + chunk.len() > cap {
                            too_long = true;
                            buf.clear();
                        } else {
                            buf.extend_from_slice(chunk);
                        }
                    }
                    (false, chunk.len())
                }
            }
        };
        r.consume(used);
        if done {
            return Ok(if too_long {
                LineRead::TooLong
            } else {
                LineRead::Line
            });
        }
    }
}

/// What the stderr of the server told. A fixed code: the text never leaves the watcher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    /// 401, "auth login" or "refresh token".
    SignedOut,
    /// 403 or 404 on `/responses`.
    NotEligible,
    /// "waiting for network".
    Offline,
    /// "required MCP servers failed".
    McpFailed,
}

/// Classify one line of text (stderr, or the message of an error item). It keeps nothing.
pub fn classify_line(line: &str) -> Option<Hint> {
    let l = line.to_ascii_lowercase();
    if l.contains("waiting for network") {
        Some(Hint::Offline)
    } else if l.contains("required mcp server") && l.contains("failed") {
        Some(Hint::McpFailed)
    } else if l.contains("/responses") && (l.contains("403") || l.contains("404")) {
        Some(Hint::NotEligible)
    } else if l.contains("401") || l.contains("auth login") || l.contains("refresh token") {
        Some(Hint::SignedOut)
    } else {
        None
    }
}

/// The bounded stderr buffer: complete lines are classified and dropped, and a partial line
/// keeps at most [`STDERR_RING_BYTES`].
#[derive(Debug, Default)]
pub struct StderrWatcher {
    buf: Vec<u8>,
}

impl StderrWatcher {
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<Hint> {
        self.buf.extend_from_slice(chunk);
        let mut hints = Vec::new();
        while let Some(i) = self.buf.iter().position(|b| *b == b'\n') {
            let rest = self.buf.split_off(i + 1);
            let text = String::from_utf8_lossy(&self.buf[..i]).into_owned();
            self.buf = rest;
            hints.extend(classify_line(&text));
        }
        if self.buf.len() > STDERR_RING_BYTES {
            let cut = self.buf.len() - STDERR_RING_BYTES;
            self.buf.drain(..cut);
        }
        hints
    }

    /// The bytes held now (tests).
    pub fn held(&self) -> usize {
        self.buf.len()
    }
}

// ---------- the process seam ----------

/// What to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// The whole environment. The child gets nothing else.
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

/// The pipes of the child.
pub struct ServerIo {
    pub stdin: Box<dyn Write + Send>,
    pub stdout: Box<dyn Read + Send>,
    pub stderr: Box<dyn Read + Send>,
}

/// A started server process.
pub trait ServerProc: Send {
    fn pid(&self) -> u32;
    /// The pipes. Only the first call gives them.
    fn take_io(&mut self) -> Option<ServerIo>;
    /// SIGTERM to the process group, wait 2 s, SIGKILL, reap. It blocks for up to 2 s.
    fn terminate(&mut self);
}

/// Starts the server. The real one is `spawn::ProcessSpawner`; the tests use an in-process fake.
pub trait Spawner: Send + Sync {
    fn spawn(&self, spec: &SpawnSpec) -> io::Result<Box<dyn ServerProc>>;
}

/// The arguments of the server. There is no other.
pub fn server_args() -> Vec<String> {
    vec!["app-server".to_owned()]
}

/// True if `args` hold a flag or word that Bottle must never pass to genie (plan A3.1).
pub fn argv_forbidden(args: &[String]) -> bool {
    args.iter().any(|a| {
        matches!(
            a.as_str(),
            "--search"
                | "--add-dir"
                | "--approve-for-me"
                | "--full-auto"
                | "--yolo"
                | "exec-server"
                | "approvals_reviewer"
        ) || a.starts_with("--dangerously")
            || a.contains("approvals_reviewer")
            || a == "-c"
            || a == "--config"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parse_tells_the_three_message_kinds_apart() {
        assert_eq!(
            parse_line(br#"{"id":3,"result":{"x":1}}"#),
            Some(Incoming::Response {
                id: 3,
                ok: true,
                result: json!({"x":1})
            })
        );
        assert!(matches!(
            parse_line(br#"{"id":4,"error":{"code":-1,"message":"boom"}}"#),
            Some(Incoming::Response {
                id: 4,
                ok: false,
                ..
            })
        ));
        assert!(matches!(
            parse_line(br#"{"method":"turn/started","params":{}}"#),
            Some(Incoming::Notification { .. })
        ));
        assert_eq!(
            parse_line(br#"{"method":"item/fileChange/requestApproval","id":0,"params":{}}"#),
            Some(Incoming::ServerRequest {
                id: json!(0),
                method: "item/fileChange/requestApproval".into()
            })
        );
        assert_eq!(parse_line(b"not json"), None);
        assert_eq!(parse_line(b"[1]"), None);
    }

    #[test]
    fn a_line_over_the_cap_is_dropped_and_the_next_line_still_reads() {
        let mut data = vec![b'a'; 50];
        data.push(b'\n');
        data.extend_from_slice(b"ok\n");
        let mut r = Cursor::new(data);
        let mut buf = Vec::new();
        assert_eq!(
            read_capped_line(&mut r, &mut buf, 10).unwrap(),
            LineRead::TooLong
        );
        assert_eq!(
            read_capped_line(&mut r, &mut buf, 10).unwrap(),
            LineRead::Line
        );
        assert_eq!(buf, b"ok");
        assert_eq!(
            read_capped_line(&mut r, &mut buf, 10).unwrap(),
            LineRead::Eof
        );
    }

    #[test]
    fn the_stderr_watcher_gives_codes_and_keeps_a_bounded_buffer() {
        let mut w = StderrWatcher::default();
        assert_eq!(w.feed(b"retry: waiting for network\n"), vec![Hint::Offline]);
        assert_eq!(w.feed(b"HTTP 401 Unauthorized\n"), vec![Hint::SignedOut]);
        assert_eq!(
            w.feed(b"POST /responses failed: 403\n"),
            vec![Hint::NotEligible]
        );
        assert_eq!(
            w.feed(b"Error: required MCP servers failed to start\n"),
            vec![Hint::McpFailed]
        );
        assert!(w.feed(b"nothing to see\n").is_empty());
        assert!(w.feed(&vec![b'x'; 100_000]).is_empty());
        assert!(w.held() <= STDERR_RING_BYTES);
    }

    #[test]
    fn the_forbidden_argv_words_are_refused_and_the_real_argv_is_clean() {
        assert!(!argv_forbidden(&server_args()));
        for bad in [
            "--search",
            "--add-dir",
            "--approve-for-me",
            "--full-auto",
            "--yolo",
            "--dangerously-bypass-approvals-and-sandbox",
            "exec-server",
            "-c",
        ] {
            assert!(argv_forbidden(&[bad.to_owned()]), "{bad}");
        }
    }

    #[test]
    fn the_refusal_holds_the_id_and_no_request_text() {
        let r = String::from_utf8(refusal(&json!(7))).unwrap();
        let v: Value = serde_json::from_str(r.trim()).unwrap();
        assert_eq!(v["id"], 7);
        assert!(v["error"].is_object());
    }
}
