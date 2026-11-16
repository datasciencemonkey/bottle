//! Minimal `text/event-stream` parser for MCP Streamable HTTP replies
//! (WHATWG event-stream rules: `data:` lines join with `\n`, `:` lines are comments,
//! a blank line ends an event, CRLF, LF and a bare CR end a line).

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub id: Option<String>,
    pub data: String,
}

pub fn parse_events(body: &str) -> Vec<SseEvent> {
    let mut out = Vec::new();
    let mut cur = SseEvent::default();
    let mut data_lines: Vec<&str> = Vec::new();
    let flush = |cur: &mut SseEvent, data_lines: &mut Vec<&str>, out: &mut Vec<SseEvent>| {
        if !data_lines.is_empty() {
            cur.data = data_lines.join("\n");
            out.push(std::mem::take(cur));
        } else {
            *cur = SseEvent::default();
        }
        data_lines.clear();
    };
    // CRLF, LF and a bare CR all end a line.
    let normalised = body.replace("\r\n", "\n").replace('\r', "\n");
    for line in normalised.split('\n') {
        if line.is_empty() {
            flush(&mut cur, &mut data_lines, &mut out);
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => data_lines.push(value),
            "event" => cur.event = Some(value.to_string()),
            "id" => cur.id = Some(value.to_string()),
            _ => {}
        }
    }
    flush(&mut cur, &mut data_lines, &mut out);
    out
}
