//! An incremental `text/event-stream` parser for MCP streamable HTTP replies (G-04a).
//!
//! WHATWG event-stream rules: CRLF, LF or a bare CR ends a line (also across chunk ends), a line
//! that starts with `:` is a comment, `data:` lines join with `\n`, and a blank line ends an
//! event. The rules follow the SP-09 spike parser (`spikes/sp-09/src/sse.rs`), made incremental
//! so that R8 stops reading as soon as the response to its request arrives.

/// One server-sent event.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    /// The `event:` field, if any.
    pub event: Option<String>,
    /// The `id:` field, if any.
    pub id: Option<String>,
    /// The `data:` lines, joined with `\n`.
    pub data: String,
}

/// Feeds bytes in, gives whole events out.
#[derive(Debug, Default)]
pub struct SseParser {
    line: Vec<u8>,
    last_cr: bool,
    data: Vec<String>,
    event: Option<String>,
    id: Option<String>,
}

impl SseParser {
    /// A parser with no state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `bytes` and add every event that ends in them to `out`.
    pub fn push(&mut self, bytes: &[u8], out: &mut Vec<SseEvent>) {
        for &b in bytes {
            if self.last_cr {
                self.last_cr = false;
                if b == b'\n' {
                    continue;
                }
            }
            match b {
                b'\r' => {
                    self.last_cr = true;
                    self.end_line(out);
                }
                b'\n' => self.end_line(out),
                _ => self.line.push(b),
            }
        }
    }

    /// End of stream: a last line and a last event with no blank line still count.
    pub fn finish(&mut self, out: &mut Vec<SseEvent>) {
        if !self.line.is_empty() {
            self.end_line(out);
        }
        self.dispatch(out);
    }

    fn end_line(&mut self, out: &mut Vec<SseEvent>) {
        let raw = std::mem::take(&mut self.line);
        if raw.is_empty() {
            self.dispatch(out);
            return;
        }
        if raw[0] == b':' {
            return;
        }
        let line = String::from_utf8_lossy(&raw);
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line.as_ref(), ""),
        };
        match field {
            "data" => self.data.push(value.to_owned()),
            "event" => self.event = Some(value.to_owned()),
            "id" => self.id = Some(value.to_owned()),
            _ => {}
        }
    }

    fn dispatch(&mut self, out: &mut Vec<SseEvent>) {
        let event = self.event.take();
        let id = self.id.take();
        if self.data.is_empty() {
            return;
        }
        out.push(SseEvent {
            event,
            id,
            data: std::mem::take(&mut self.data).join("\n"),
        });
    }
}

/// Every event in a whole body.
pub fn parse_events(body: &[u8]) -> Vec<SseEvent> {
    let mut p = SseParser::new();
    let mut out = Vec::new();
    p.push(body, &mut out);
    p.finish(&mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sp_09_sse_parser_handles_crlf_comments_and_multiline_data() {
        let body = b": hi\r\nevent: message\r\ndata: {\"a\":\r\ndata: 1}\r\n\r\ndata: x\n\n";
        let ev = parse_events(body);
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].event.as_deref(), Some("message"));
        assert_eq!(ev[0].data, "{\"a\":\n1}");
        assert_eq!(ev[1].data, "x");
        assert_eq!(ev[1].event, None);
    }

    #[test]
    fn sp_09_sse_parser_accepts_bare_cr_and_no_space_after_colon() {
        let ev = parse_events(b"data:a\rdata:b\r\rid: 7\rdata: c\r\r");
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].data, "a\nb");
        assert_eq!(ev[1].id.as_deref(), Some("7"));
    }

    #[test]
    fn sp_09_sse_parser_is_the_same_for_any_chunk_split() {
        let body = b"event: message\r\ndata: one\r\n\r\n: c\r\ndata: two\r\ndata: 2\r\n\r\n";
        let whole = parse_events(body);
        for cut in 0..body.len() {
            let mut p = SseParser::new();
            let mut out = Vec::new();
            p.push(&body[..cut], &mut out);
            p.push(&body[cut..], &mut out);
            p.finish(&mut out);
            assert_eq!(out, whole, "cut at {cut}");
        }
        assert_eq!(whole.len(), 2);
    }

    #[test]
    fn sp_09_sse_parser_drops_events_with_no_data_and_flushes_at_end() {
        let ev = parse_events(b"event: ping\n\ndata: last");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].data, "last");
        assert_eq!(ev[0].event, None);
    }
}
