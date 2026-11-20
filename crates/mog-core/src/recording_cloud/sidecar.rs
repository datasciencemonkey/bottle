//! The local sidecar `job.local.json` of one recording (ADR 0103 follow-up): it sits in the
//! temp dir of the recording, next to the audio parts. After a crash it tells the launch scan
//! which recording the parts belong to and which profile to upload with, so the upload can
//! resume with no click. A part with no sidecar (a build before parts) is only "Not uploaded".
//!
//! It holds no token and no audio: the recording ID and name, the start time, the profile name
//! (a CLI profile name, not a secret) and the instructions of this recording. The file and the
//! dir are user-only (0600 and 0700 on unix).

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file name in the temp dir.
pub const SIDECAR_FILE: &str = "job.local.json";

/// The content of `job.local.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    /// Always 1.
    pub version: u32,
    /// The recording ID.
    pub id: String,
    /// The recording name (the volume folder name).
    pub name: String,
    /// The start, epoch ms.
    pub started_at_ms: i64,
    /// The home profile of `recording_start`.
    pub profile: String,
    /// The instructions of this recording, or `None` to use the preset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

impl Sidecar {
    /// A sidecar of version 1.
    pub fn new(
        id: &str,
        name: &str,
        started_at_ms: i64,
        profile: &str,
        instructions: Option<&str>,
    ) -> Self {
        Self {
            version: 1,
            id: id.to_owned(),
            name: name.to_owned(),
            started_at_ms,
            profile: profile.to_owned(),
            instructions: instructions.map(str::to_owned),
        }
    }
}

/// Write the sidecar into `dir` (replaces an older one).
pub fn write(dir: &Path, s: &Sidecar) -> io::Result<()> {
    use std::io::Write;
    let path = dir.join(SIDECAR_FILE);
    let tmp = dir.join(format!("{SIDECAR_FILE}.tmp"));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    let mut f = o.open(&tmp)?;
    f.write_all(&serde_json::to_vec(s).map_err(io::Error::other)?)?;
    f.sync_all()?;
    std::fs::rename(&tmp, &path)
}

/// Read the sidecar of `dir`, or `None` when there is none or it is not valid.
pub fn read(dir: &Path) -> Option<Sidecar> {
    let b = std::fs::read(dir.join(SIDECAR_FILE)).ok()?;
    serde_json::from_slice::<Sidecar>(&b)
        .ok()
        .filter(|s| s.version == 1)
}

/// Delete the sidecar of `dir`, then the dir itself if nothing else is in it.
pub fn remove(dir: &Path) {
    let _ = std::fs::remove_file(dir.join(SIDECAR_FILE));
    let _ = std::fs::remove_file(dir.join(format!("{SIDECAR_FILE}.tmp")));
    let _ = std::fs::remove_dir(dir);
}
