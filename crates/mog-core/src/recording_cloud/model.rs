//! The two model calls of a recording (ADR 0103, UPDATE 4), on the existing Unity Gateway route
//! `mlflow.chat_completions` (`POST /ai-gateway/mlflow/v1/chat/completions`, NFR-21a; never
//! `/serving-endpoints`, NFR-21b):
//!
//! - transcribe: one request per audio part, the audio inline as a base64 `audio_url` data URL
//!   (a reference is not possible: the model fetches URLs itself and cannot sign in);
//! - notes: one chat call with the instructions and the joined transcript.
//!
//! ADR 0123: both models are `system.ai.*` ids (the dotted id passes the endpoint rule), and
//! [`audio_chat_body`] is the one audio request that cloud dictation shares.
//!
//! Live facts (DEFAULT, 2026-11-19, synthetic clips): `audio/ogg` (Opus), `audio/flac` and
//! `audio/wav` data URLs work; `input_audio` is refused; the request cap is about 30 MB (30 MB
//! passed, 32 MB dropped the connection), so Bottle keeps one request under [`REQUEST_BUDGET`].
//! 16 kbps Opus is about 9.6 MB of base64 per hour. About 25 audio tokens per second. A
//! non-audio payload got a made-up transcript with a 200, so Bottle sends only bytes whose magic
//! matches the fixed MIME type.

use serde_json::{json, Value};

use super::errors::ErrorKind;
use super::settings_store::{current_endpoint, is_endpoint_name};
use super::transcript::RawSegment;

/// The largest model request Bottle sends (bytes).
pub const REQUEST_BUDGET: usize = 24 * 1024 * 1024;
/// `max_tokens` of the transcribe call (a 2 h call is about 55k output tokens).
pub const TRANSCRIBE_MAX_TOKENS: u32 = 64_000;
/// `max_tokens` of the notes call.
pub const NOTES_MAX_TOKENS: u32 = 8_000;

/// The transcribe prompt: a strict line format that [`parse_transcript`] reads tolerantly.
pub const TRANSCRIBE_PROMPT: &str = "Transcribe this audio verbatim with speaker labels and mm:ss timestamps. \
Write one line per turn, exactly in this format: [mm:ss] Speaker N: text. \
Use [h:mm:ss] after one hour. Number the speakers in the order they first speak. \
The person closest to the microphone is the user who recorded the call; label them You if you can tell. \
Write nothing else.";

/// The audio container of a part, from its magic bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    /// Ogg (Opus).
    Ogg,
    /// FLAC.
    Flac,
    /// RIFF WAVE.
    Wav,
}

impl AudioFormat {
    /// The fixed MIME type.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Ogg => "audio/ogg",
            Self::Flac => "audio/flac",
            Self::Wav => "audio/wav",
        }
    }
}

/// The format of `bytes`, or `None` when it is not audio Bottle encodes.
pub fn sniff(bytes: &[u8]) -> Option<AudioFormat> {
    match bytes {
        [b'O', b'g', b'g', b'S', ..] => Some(AudioFormat::Ogg),
        [b'f', b'L', b'a', b'C', ..] => Some(AudioFormat::Flac),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'A', b'V', b'E', ..] => Some(AudioFormat::Wav),
        _ => None,
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Standard base64 with padding.
pub fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            B64[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// The transcribe request of one part. `context` is the last lines of the previous part, so
/// the model keeps the speaker labels (only when there is more than one part).
pub fn transcribe_request(
    endpoint: &str,
    audio: &[u8],
    context: Option<&str>,
) -> Result<Value, ErrorKind> {
    let mut prompt = TRANSCRIBE_PROMPT.to_owned();
    if let Some(c) = context.filter(|c| !c.trim().is_empty()) {
        let tail: String = c
            .chars()
            .rev()
            .take(4000)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        prompt.push_str("\nThis audio continues a call. The last lines before it were:\n");
        prompt.push_str(&tail);
        prompt.push_str(
            "\nKeep the same speaker labels. Timestamps start at [00:00] for this audio.",
        );
    }
    audio_chat_body(endpoint, &prompt, audio, TRANSCRIBE_MAX_TOKENS)
}

/// The one audio-to-model request body, shared by call recording and cloud dictation (ADR 0123):
/// the prompt and the audio inline as an `audio_url` data URL on the chat route. The audio must
/// be bytes that Bottle encoded (its magic is checked) and must fit [`REQUEST_BUDGET`].
pub fn audio_chat_body(
    endpoint: &str,
    prompt: &str,
    audio: &[u8],
    max_tokens: u32,
) -> Result<Value, ErrorKind> {
    let endpoint = current_endpoint(endpoint);
    if !is_endpoint_name(endpoint) {
        return Err(ErrorKind::TranscribeEndpointMissing);
    }
    let format = sniff(audio).ok_or(ErrorKind::AudioUnreadable)?;
    // base64 is 4/3 of the bytes; 64 KiB covers the prompt, the context and the JSON.
    if audio.len().div_ceil(3) * 4 + 64 * 1024 > REQUEST_BUDGET {
        return Err(ErrorKind::TooLong);
    }
    let url = format!("data:{};base64,{}", format.mime(), base64(audio));
    Ok(json!({
        "model": endpoint,
        "messages": [{"role": "user", "content": [
            {"type": "text", "text": prompt},
            {"type": "audio_url", "audio_url": {"url": url}},
        ]}],
        "max_tokens": max_tokens,
    }))
}

/// The notes request: the instructions, then the transcript.
pub fn notes_request(
    endpoint: &str,
    instructions: &str,
    transcript: &str,
) -> Result<Value, ErrorKind> {
    let endpoint = current_endpoint(endpoint);
    if !is_endpoint_name(endpoint) {
        return Err(ErrorKind::NotesEndpointMissing);
    }
    Ok(json!({
        "model": endpoint,
        "messages": [{"role": "user", "content": format!("{instructions}\n\nTranscript:\n{transcript}")}],
        "max_tokens": NOTES_MAX_TOKENS,
    }))
}

/// The text of a chat reply: `choices[0].message.content` as a string, or the joined `text`
/// parts of a content list (Gemini adds `thoughtSignature` parts, which are dropped).
pub fn reply_text(v: &Value) -> Option<String> {
    let c = v.pointer("/choices/0/message/content")?;
    match c {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let t: String = parts
                .iter()
                .filter(|p| {
                    p.get("type")
                        .and_then(Value::as_str)
                        .is_none_or(|t| t == "text")
                })
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            Some(t)
        }
        _ => None,
    }
    .filter(|t| !t.trim().is_empty())
}

/// `mm:ss`, `h:mm:ss` or `mm:ss.f` to seconds.
fn parse_ts(ts: &str) -> Option<f64> {
    let parts: Vec<&str> = ts.trim().split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let mut total = 0.0;
    for (i, p) in parts.iter().enumerate() {
        let v: f64 = p.trim().parse().ok()?;
        if v < 0.0 || (i > 0 && v >= 60.0) {
            return None;
        }
        total = total * 60.0 + v;
    }
    Some(total)
}

fn clean_label(raw: &str) -> String {
    let l = raw.trim().trim_matches('*').trim();
    match l
        .strip_prefix("Speaker")
        .or_else(|| l.strip_prefix("speaker"))
    {
        Some(n) if !n.trim().is_empty() => n.trim().to_owned(),
        _ => l.to_owned(),
    }
}

/// Read model lines `[mm:ss] Speaker N: text` into segments, tolerantly: list marks and bold
/// marks are dropped, `[h:mm:ss]` works, a line with no timestamp continues the turn before,
/// fences and blank lines are skipped. `end` is the next start (the last one: `duration_s`).
pub fn parse_transcript(text: &str, duration_s: Option<f64>) -> Vec<RawSegment> {
    let mut out: Vec<RawSegment> = Vec::new();
    for line in text.lines() {
        let l = line
            .trim()
            .trim_start_matches(['-', '*', '•'])
            .trim()
            .trim_start_matches("**")
            .trim();
        if l.is_empty() || l.starts_with("```") {
            continue;
        }
        let parsed = l.strip_prefix('[').and_then(|r| {
            let close = r.find(']')?;
            let start = parse_ts(&r[..close])?;
            let rest = r[close + 1..].trim().trim_start_matches("**").trim();
            let (label, said) = rest.split_once(':')?;
            if label.len() > 40 {
                return None;
            }
            Some((
                start,
                clean_label(label),
                said.trim().trim_start_matches("**").trim().to_owned(),
            ))
        });
        match parsed {
            Some((start, speaker_id, text)) => out.push(RawSegment {
                start,
                end: start,
                speaker_id,
                text,
            }),
            None => {
                if let Some(last) = out.last_mut() {
                    if !last.text.is_empty() {
                        last.text.push(' ');
                    }
                    last.text.push_str(l);
                }
            }
        }
    }
    for i in 0..out.len() {
        let next = out.get(i + 1).map(|n| n.start);
        out[i].end = next
            .or(duration_s)
            .unwrap_or(out[i].start)
            .max(out[i].start);
    }
    out
}
