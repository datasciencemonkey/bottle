//! The job manifest `<name>.job.json` (ADR 0103): the status of one recording in the volume,
//! read back for the Recordings list. All work runs on the device (UPDATE 4): no job, no
//! warehouse, no statement.

use serde::{Deserialize, Serialize};

use super::errors::{ErrorKind, Stage};

/// The status of a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Capture runs.
    Recording,
    /// Parts and sidecars go to the volume.
    Uploading,
    /// The transcribe model runs.
    Transcribing,
    /// ai_query writes the notes.
    WritingNotes,
    /// All files are in the volume.
    Done,
    /// A stage failed (`error_kind`, `error_stage`).
    Failed,
}

/// One audio part in the manifest.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartEntry {
    /// The part file name.
    pub file: String,
    /// Its duration in seconds.
    pub duration_s: f64,
    /// It is in the volume.
    #[serde(default)]
    pub uploaded: bool,
}

impl PartEntry {
    /// A part not uploaded yet.
    pub fn new(file: &str, duration_s: f64) -> Self {
        Self {
            file: file.to_owned(),
            duration_s,
            uploaded: false,
        }
    }
}

/// `<name>.job.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobManifest {
    /// Always 1.
    pub version: u32,
    /// The recording ID.
    pub id: String,
    /// The recording name (the folder name).
    pub name: String,
    /// The start time (UTC, RFC 3339).
    pub started_at: String,
    /// The total duration.
    pub duration_s: f64,
    /// The audio parts.
    #[serde(default)]
    pub parts: Vec<PartEntry>,
    /// The status.
    pub status: Status,
    /// The error, when failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<ErrorKind>,
    /// The failed stage, when failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_stage: Option<Stage>,
    /// The transcribe model endpoint.
    pub transcribe_endpoint: String,
    /// The notes model endpoint.
    pub notes_endpoint: String,
    /// The last change (UTC, RFC 3339).
    pub updated_at: String,
}

impl JobManifest {
    /// A new manifest in `uploading`.
    pub fn new(
        id: &str,
        name: &str,
        started_at: &str,
        parts: Vec<PartEntry>,
        transcribe_endpoint: &str,
        notes_endpoint: &str,
        now: &str,
    ) -> Self {
        let duration_s = parts.iter().map(|p| p.duration_s).sum();
        Self {
            version: 1,
            id: id.to_owned(),
            name: name.to_owned(),
            started_at: started_at.to_owned(),
            duration_s,
            parts,
            status: Status::Uploading,
            error_kind: None,
            error_stage: None,
            transcribe_endpoint: transcribe_endpoint.to_owned(),
            notes_endpoint: notes_endpoint.to_owned(),
            updated_at: now.to_owned(),
        }
    }
    /// Move to `status` and clear any error.
    pub fn set_status(&mut self, status: Status, now: &str) {
        self.status = status;
        self.error_kind = None;
        self.error_stage = None;
        self.updated_at = now.to_owned();
    }
    /// Fail at `stage` with `kind`.
    pub fn fail(&mut self, kind: ErrorKind, stage: Stage, now: &str) {
        self.status = Status::Failed;
        self.error_kind = Some(kind);
        self.error_stage = Some(stage);
        self.updated_at = now.to_owned();
    }
}

/// The Unix time in ms of an RFC 3339 UTC stamp `YYYY-MM-DDTHH:MM:SSZ` (as [`iso_utc`] writes).
pub fn parse_iso_utc_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    // Days from civil (H. Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400) + hh * 3600 + mm * 60 + ss) * 1000)
}

/// RFC 3339 UTC (`YYYY-MM-DDTHH:MM:SSZ`) of a Unix time in ms.
pub fn iso_utc(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil from days (H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}
