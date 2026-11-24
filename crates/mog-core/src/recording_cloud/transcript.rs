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

/// A gap in the recording where the user spoke to Bottle (ADR 0118). The mic was gated, so
/// the audio holds silence for the mic there. Times only; no text.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BottleGap {
    /// Start in seconds from the recording start.
    pub start_s: f64,
    /// Length in seconds.
    pub dur_s: f64,
}

/// A hold of at least this many seconds gets a marker line in the markdown. The hold is the gap
/// without the gate tail ([`crate::recording_capture::tap::GATE_TAIL_MS`]).
pub const MARKER_MIN_S: f64 = 1.0;

/// The length of the hold in a gap: the gap without the gate tail.
fn hold_s(g: &BottleGap) -> f64 {
    (g.dur_s - crate::recording_capture::tap::GATE_TAIL_MS as f64 / 1_000.0).max(0.0)
}

/// The gaps of all parts, with absolute times. A part starts where the parts before it end.
pub fn bottle_gaps(parts: &[super::job::PartEntry]) -> Vec<BottleGap> {
    let rate = f64::from(crate::recording_capture::SAMPLE_RATE);
    let mut offset = 0.0;
    let mut gaps = Vec::new();
    for p in parts {
        for r in &p.held_ranges {
            gaps.push(BottleGap {
                start_s: offset + r.start_samples as f64 / rate,
                dur_s: r.len_samples as f64 / rate,
            });
        }
        offset += p.duration_s;
    }
    gaps
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
    /// Where the user spoke to Bottle (ADR 0118).
    #[serde(default, rename = "bottleGaps", skip_serializing_if = "Vec::is_empty")]
    pub bottle_gaps: Vec<BottleGap>,
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

/// The transcript text for the notes model: no marker line (ADR 0118 decision 12).
pub fn render_markdown_for_model(doc: &TranscriptDoc) -> String {
    let mut d = doc.clone();
    d.bottle_gaps.clear();
    render_markdown(&d)
}

/// `<name>.transcript.md`: one turn per run of one speaker, `**Speaker N** [hh:mm:ss]`, turns
/// separated by a blank line.
pub fn render_markdown(doc: &TranscriptDoc) -> String {
    let mut markers: Vec<&BottleGap> = doc
        .bottle_gaps
        .iter()
        .filter(|g| hold_s(g) >= MARKER_MIN_S - 1e-6)
        .collect();
    markers.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    let mut markers = markers.into_iter().peekable();
    let mut out = format!("# Call {}\n", doc.name);
    let mut turn: Option<(&str, f64, Vec<&str>)> = None;
    let flush = |out: &mut String, turn: &mut Option<(&str, f64, Vec<&str>)>| {
        if let Some((sp, start, texts)) = turn.take() {
            out.push_str(&format!(
                "\n**{}** [{}]\n{}\n",
                speaker_name(sp),
                fmt_hms(start),
                texts.join(" ")
            ));
        }
    };
    let marker = |out: &mut String, g: &BottleGap| {
        let d = hold_s(g) as u64;
        out.push_str(&format!(
            "\n*You spoke to Bottle ({}:{:02})* [{}]\n",
            d / 60,
            d % 60,
            fmt_hms(g.start_s)
        ));
    };
    for s in doc.segments.iter().filter(|s| !s.text.is_empty()) {
        // A marker ends the run of the turn before it, so it stands at its start time.
        while let Some(g) = markers.next_if(|g| g.start_s <= s.start) {
            flush(&mut out, &mut turn);
            marker(&mut out, g);
        }
        match turn.as_mut() {
            Some((sp, _, texts)) if *sp == s.speaker => texts.push(&s.text),
            _ => {
                flush(&mut out, &mut turn);
                turn = Some((&s.speaker, s.start, vec![&s.text]));
            }
        }
    }
    flush(&mut out, &mut turn);
    for g in markers {
        marker(&mut out, g);
    }
    out
}
