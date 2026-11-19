//! Transcripts (ADR 0103): merge the parts into absolute times, and render the readable
//! markdown. The model lines are read by [`super::model::parse_transcript`].

use serde::{Deserialize, Serialize};

/// One segment of one part, with part-relative times.
#[derive(Debug, Clone, PartialEq)]
pub struct RawSegment {
    /// Start in seconds from the part start.
    pub start: f64,
    /// End in seconds from the part start.
    pub end: f64,
    /// The speaker label of the model ("1" for "Speaker 1", or a name such as "You").
    pub speaker_id: String,
    /// The text.
    pub text: String,
}

/// The transcript of one part.
#[derive(Debug, Clone, PartialEq)]
pub struct PartTranscript {
    /// The part duration, when known (0 when not).
    pub duration_s: f64,
    /// The segments.
    pub segments: Vec<RawSegment>,
}

/// One part to merge.
#[derive(Debug, Clone, PartialEq)]
pub struct PartInput {
    /// The part file name.
    pub file: String,
    /// The duration Bottle recorded, if known (it wins over the reported one).
    pub duration_s: Option<f64>,
    /// The part transcript.
    pub transcript: PartTranscript,
}

/// One part in `transcript.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartInfo {
    /// The part file name.
    pub file: String,
    /// Its start in seconds from the recording start.
    pub offset_s: f64,
    /// Its duration.
    pub duration_s: f64,
}

/// One segment in `transcript.json` (absolute times).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    /// Start in seconds from the recording start.
    pub start: f64,
    /// End in seconds from the recording start.
    pub end: f64,
    /// The speaker label ("1" for "Speaker 1", or a name such as "You").
    pub speaker: String,
    /// The text.
    pub text: String,
    /// The 1-based part.
    pub part: u32,
}

/// The merged parts.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    /// The parts with offsets.
    pub parts: Vec<PartInfo>,
    /// The segments, in time order.
    pub segments: Vec<Segment>,
    /// The total duration.
    pub duration_s: f64,
}

/// Merge parts into absolute times. The labels stay as the model wrote them: a later part gets
/// the last lines of the part before in its prompt, so the model keeps the labels.
pub fn merge_parts(parts: &[PartInput]) -> Merged {
    let mut offset = 0.0;
    let mut infos = Vec::with_capacity(parts.len());
    let mut segments = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        let n = i as u32 + 1;
        let dur = p
            .duration_s
            .filter(|d| *d > 0.0)
            .unwrap_or(p.transcript.duration_s);
        for s in &p.transcript.segments {
            let speaker = s.speaker_id.clone();
            segments.push(Segment {
                start: offset + s.start,
                end: offset + s.end,
                speaker,
                text: s.text.clone(),
                part: n,
            });
        }
        infos.push(PartInfo {
            file: p.file.clone(),
            offset_s: offset,
            duration_s: dur,
        });
        offset += dur;
    }
    Merged {
        parts: infos,
        segments,
        duration_s: offset,
    }
}

/// `<name>.transcript.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscriptDoc {
    /// Always 1.
    pub version: u32,
    /// The recording ID.
    pub id: String,
    /// The recording name.
    pub name: String,
    /// The start time (UTC, RFC 3339).
    pub started_at: String,
    /// The total duration.
    pub duration_s: f64,
    /// The instructions used for the notes.
    pub instructions: String,
    /// The parts.
    pub parts: Vec<PartInfo>,
    /// The segments.
    pub segments: Vec<Segment>,
}

/// `hh:mm:ss` of a time in seconds (floor).
pub fn fmt_hms(s: f64) -> String {
    let t = if s.is_finite() && s > 0.0 {
        s as u64
    } else {
        0
    };
    format!("{:02}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60)
}

/// The shown name of a label: "Speaker 1" for "1", the label itself for a name ("You").
pub fn speaker_name(label: &str) -> String {
    if !label.is_empty() && label.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        format!("Speaker {label}")
    } else {
        label.to_owned()
    }
}

/// `<name>.transcript.md`: one turn per run of one speaker, `**Speaker N** [hh:mm:ss]`, turns
/// separated by a blank line.
pub fn render_markdown(doc: &TranscriptDoc) -> String {
    let mut turns: Vec<(&str, f64, Vec<&str>)> = Vec::new();
    for s in doc.segments.iter().filter(|s| !s.text.is_empty()) {
        match turns.last_mut() {
            Some((sp, _, texts)) if *sp == s.speaker => texts.push(&s.text),
            _ => turns.push((&s.speaker, s.start, vec![&s.text])),
        }
    }
    let mut out = format!("# Call {}\n", doc.name);
    for (sp, start, texts) in turns {
        out.push_str(&format!(
            "\n**{}** [{}]\n{}\n",
            speaker_name(sp),
            fmt_hms(start),
            texts.join(" ")
        ));
    }
    out
}
