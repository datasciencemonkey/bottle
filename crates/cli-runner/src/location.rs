//! Refuse a CLI in a world-writable or temporary directory (W-02b).

use std::path::{Path, PathBuf};

use crate::error::{CliError, UnsafeReason};
use crate::sys::{Env, Fs};

/// Temporary directories a CLI must not live in.
pub fn temp_roots<F: Fs + ?Sized, E: Env + ?Sized>(fs: &F, env: &E) -> Vec<PathBuf> {
    let mut raw: Vec<PathBuf> = vec![env.temp_dir()];
    for name in ["TMPDIR", "TEMP", "TMP"] {
        if let Some(v) = env.var(name) {
            raw.push(PathBuf::from(v));
        }
    }
    // Only absolute env values count: an empty or relative one would match
    // everything or depend on the cwd.
    raw.retain(|p| p.is_absolute());
    raw.extend(FIXED_TEMP_ROOTS.iter().map(PathBuf::from));
    let mut roots = Vec::new();
    for p in raw {
        if let Ok(c) = fs.canonicalize(&p) {
            push_unique(&mut roots, c);
        }
        push_unique(&mut roots, p);
    }
    roots
}

/// Temp roots checked on every OS (harmless where they do not exist).
pub const FIXED_TEMP_ROOTS: &[&str] = &[
    "/tmp",
    "/var/tmp",
    "/private/tmp",
    "/private/var/tmp",
    "C:\\Windows\\Temp",
];

fn push_unique(v: &mut Vec<PathBuf>, p: PathBuf) {
    if !p.as_os_str().is_empty() && !v.contains(&p) {
        v.push(p);
    }
}

/// True if `child` is `root` or inside it (case-insensitive on Windows).
pub fn path_within(child: &Path, root: &Path) -> bool {
    fn norm(c: std::path::Component<'_>) -> String {
        let s = c.as_os_str().to_string_lossy();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s.into_owned()
        }
    }
    let mut root_parts = root.components().peekable();
    if root_parts.peek().is_none() {
        return false;
    }
    let mut child_parts = child.components();
    root_parts.all(|r| child_parts.next().is_some_and(|c| norm(c) == norm(r)))
}

/// Refuses `found` (as found on PATH or in policy) or `canonical` (symlinks
/// resolved) if either is in a temp root, or if the binary or its folder is
/// world-writable (unix mode bits).
pub fn check_location<F: Fs + ?Sized, E: Env + ?Sized>(
    fs: &F,
    env: &E,
    found: &Path,
    canonical: &Path,
) -> Result<(), CliError> {
    let refuse = |reason| {
        Err(CliError::UnsafeLocation {
            path: canonical.to_path_buf(),
            reason,
        })
    };
    let roots = temp_roots(fs, env);
    for p in [found, canonical] {
        if let Some(root) = roots.iter().find(|r| path_within(p, r)) {
            return refuse(UnsafeReason::TempDir { root: root.clone() });
        }
    }
    let mut checked: Vec<&Path> = Vec::new();
    for p in [
        Some(found),
        found.parent(),
        Some(canonical),
        canonical.parent(),
    ]
    .into_iter()
    .flatten()
    {
        if checked.contains(&p) {
            continue;
        }
        checked.push(p);
        if fs.unix_mode(p).is_some_and(|m| m & 0o002 != 0) {
            return refuse(UnsafeReason::WorldWritable {
                path: p.to_path_buf(),
            });
        }
    }
    Ok(())
}
