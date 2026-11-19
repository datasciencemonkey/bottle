//! The volume layout of a recording (ADR 0103): `<root>/Bottle/Recordings/<name>/` with
//! `<name>.part01.flac`, `.instructions.md`, `.job.json`, `.transcript.json`, `.transcript.md`
//! and `.notes.md`. Also the My Files
//! probe paths and the Catalog Explorer link. Every path is checked against the strict volume
//! path charset before it reaches a route.

pub use crate::http_transport::routes::is_volume_path;

use super::errors::ErrorKind;

/// The name of the managed volume Bottle creates in a `catalog.schema` (after the user confirms).
pub const MANAGED_VOLUME: &str = "bottle_audio";

/// A volume root: `/Volumes/<catalog>/<schema>/<volume>[/<folder>...]`, with no trailing `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeRoot {
    path: String,
}

/// A Unity Catalog name segment Bottle accepts: `[A-Za-z0-9_-]{1,255}`.
pub fn is_uc_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl VolumeRoot {
    /// Parse a volume root path (a trailing `/` is dropped).
    pub fn parse(path: &str) -> Result<Self, ErrorKind> {
        if !is_volume_path(path) {
            return Err(ErrorKind::BadSettings);
        }
        Ok(Self {
            path: path.trim_end_matches('/').to_owned(),
        })
    }
    fn seg(&self, i: usize) -> &str {
        self.path.split('/').nth(2 + i).unwrap_or("")
    }
    /// The catalog.
    pub fn catalog(&self) -> &str {
        self.seg(0)
    }
    /// The schema.
    pub fn schema(&self) -> &str {
        self.seg(1)
    }
    /// The volume.
    pub fn volume(&self) -> &str {
        self.seg(2)
    }
    /// The root path, with no trailing `/`.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// `<root>/Bottle/Recordings`.
    pub fn recordings_dir(&self) -> String {
        format!("{}/Bottle/Recordings", self.path)
    }
    /// `<root>/Bottle/Recordings/<name>`.
    pub fn folder(&self, name: &str) -> String {
        format!("{}/{name}", self.recordings_dir())
    }
}

/// `<name>.partNN.<ext>` (1-based) for a recording of more than one part; `ext` is `opus`
/// (Ogg Opus), `flac` or `wav`. A one-part recording is `<name>.opus`.
pub fn part_file(name: &str, n: u32, ext: &str) -> String {
    format!("{name}.part{n:02}.{ext}")
}
/// `<name>.instructions.md`.
pub fn instructions_file(name: &str) -> String {
    format!("{name}.instructions.md")
}
/// `<name>.job.json`.
pub fn job_file(name: &str) -> String {
    format!("{name}.job.json")
}
/// `<name>.transcript.json`.
pub fn transcript_json_file(name: &str) -> String {
    format!("{name}.transcript.json")
}
/// `<name>.transcript.md`.
pub fn transcript_md_file(name: &str) -> String {
    format!("{name}.transcript.md")
}
/// `<name>.notes.md`.
pub fn notes_file(name: &str) -> String {
    format!("{name}.notes.md")
}

/// The recording name of a local start time: `YYYY-MM-DD_HHMM`, or `None` for a bad time.
pub fn recording_name(year: u32, month: u32, day: u32, hour: u32, minute: u32) -> Option<String> {
    let ok = (1970..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60;
    ok.then(|| format!("{year:04}-{month:02}-{day:02}_{hour:02}{minute:02}"))
}

/// `base`, or `base_2`, `base_3`, ... when the name is taken.
pub fn unique_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_owned();
    }
    (2u32..)
        .map(|n| format!("{base}_{n}"))
        .find(|c| !taken.iter().any(|t| t == c))
        .unwrap_or_else(|| base.to_owned())
}

/// A name of [`recording_name`] form, with an optional `_N` clash suffix.
pub fn is_recording_name(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 15 {
        return false;
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(u8::is_ascii_digit);
    let head = digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'_'
        && digits(11..15);
    if !head {
        return false;
    }
    match &b[15..] {
        [] => true,
        [b'_', rest @ ..] => {
            !rest.is_empty() && rest.len() <= 4 && rest.iter().all(u8::is_ascii_digit)
        }
        _ => false,
    }
}

/// The first DNS label of a workspace URL (the deployment name), if it is `[a-z0-9-]`.
pub fn deployment_name(host_url: &str) -> Option<String> {
    let rest = host_url.strip_prefix("https://")?;
    let host = rest.split(['/', ':', '?', '#']).next()?;
    let label = host.split('.').next()?;
    (!label.is_empty()
        && label
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'))
    .then(|| label.to_owned())
}

/// The My Files paths to probe, in order: workspace ID, then deployment name (ADR 0103). The
/// `<workspace>` segment is undocumented, so Bottle tries only these two.
pub fn my_files_candidates(workspace_id: &str, host_url: &str, user_name: &str) -> Vec<String> {
    let mut segs: Vec<String> = Vec::new();
    if !workspace_id.is_empty() && workspace_id.bytes().all(|b| b.is_ascii_digit()) {
        segs.push(workspace_id.to_owned());
    }
    if let Some(d) = deployment_name(host_url) {
        if !segs.contains(&d) {
            segs.push(d);
        }
    }
    segs.into_iter()
        .map(|w| format!("/Volumes/databricks/home/{w}/{user_name}/my_files/"))
        .filter(|p| is_volume_path(p))
        .collect()
}

/// `/Volumes/<catalog>/<schema>/bottle_audio`.
pub fn managed_volume_path(catalog: &str, schema: &str) -> Result<String, ErrorKind> {
    if !is_uc_name(catalog) || !is_uc_name(schema) {
        return Err(ErrorKind::BadSettings);
    }
    Ok(format!("/Volumes/{catalog}/{schema}/{MANAGED_VOLUME}"))
}

fn encode_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A bare `https://<host>` origin of the profile, or `None`.
pub fn bare_https_origin(host_url: &str) -> Option<String> {
    let t = host_url.trim().trim_end_matches('/');
    let host = t.strip_prefix("https://")?;
    (!host.is_empty()
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-'))
    .then(|| format!("https://{host}"))
}

/// The Catalog Explorer link of a recording folder:
/// `https://<host>/explore/data/volumes/<c>/<s>/<v>?o=<workspace_id>&volumePath=<folder/>`.
pub fn catalog_explorer_url(
    host_url: &str,
    workspace_id: &str,
    root: &VolumeRoot,
    name: &str,
) -> Result<String, ErrorKind> {
    let origin = bare_https_origin(host_url).ok_or(ErrorKind::BadSettings)?;
    if workspace_id.is_empty() || !workspace_id.bytes().all(|b| b.is_ascii_digit()) {
        return Err(ErrorKind::BadSettings);
    }
    if !is_recording_name(name) {
        return Err(ErrorKind::BadSettings);
    }
    let folder = format!("{}/", root.folder(name));
    Ok(format!(
        "{origin}/explore/data/volumes/{}/{}/{}?o={workspace_id}&volumePath={}",
        root.catalog(),
        root.schema(),
        root.volume(),
        encode_component(&folder)
    ))
}
