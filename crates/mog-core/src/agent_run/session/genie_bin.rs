//! Find the Genie Code binary and check it against the pin (plan Amendment 2, A7; v1 2.2).
//!
//! Bottle looks in its own bin folder first, then on `PATH`, then at `~/.local/bin/genie`. It
//! resolves symlinks, checks the SHA-256 against the pin FIRST, and runs `genie --version` (5 s
//! cap) only on a file with the pinned digest, so Bottle never runs a binary that it did not
//! pin. The helper binary of the release is required when its digest is pinned. A relative
//! `PATH` entry is skipped. The answer is cached by `(realpath, size, mtime)` of `genie` and of
//! the helper, so a changed file is checked again.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

use crate::agent_run::pin::{pinned_digest, GENIE_PINNED};
use crate::types::GenieCodeBinary;

/// The cap on `genie --version`.
pub const VERSION_CAP: Duration = Duration::from_secs(5);

/// The target triple of this build, or "" for a target that Bottle does not pin.
pub fn current_target() -> &'static str {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "aarch64-apple-darwin"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "x86_64-apple-darwin"
    } else {
        ""
    }
}

/// What the check needs from the system.
pub trait BinProbe: Send + Sync {
    /// The version that `bin --version` prints, if it ran within the cap.
    fn version(&self, bin: &Path) -> Option<String>;
    /// The SHA-256 of the file, in lower-case hex.
    fn sha256_hex(&self, bin: &Path) -> Option<String>;
}

/// The real probe.
#[derive(Debug, Default)]
pub struct SystemProbe;

impl BinProbe for SystemProbe {
    fn version(&self, bin: &Path) -> Option<String> {
        let mut child = std::process::Command::new(bin)
            .arg("--version")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        let mut out = child.stdout.take()?;
        let reader = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = (&mut out).take(4096).read_to_string(&mut s);
            s
        });
        let start = std::time::Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(st)) if st.success() => break,
                Ok(Some(_)) | Err(_) => return None,
                Ok(None) if start.elapsed() >= VERSION_CAP => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        let text = reader.join().ok()?;
        text.split_whitespace().last().map(str::to_owned)
    }

    fn sha256_hex(&self, bin: &Path) -> Option<String> {
        let mut f = fs::File::open(bin).ok()?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf).ok()?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
        Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
}

/// The result of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinInfo {
    pub state: GenieCodeBinary,
    pub version: Option<String>,
    /// The resolved path, for a binary that is ready.
    pub path: Option<PathBuf>,
}

impl BinInfo {
    fn of(state: GenieCodeBinary) -> Self {
        Self {
            state,
            version: None,
            path: None,
        }
    }
}

#[derive(Clone)]
struct Cached {
    real: PathBuf,
    size: u64,
    mtime: Option<SystemTime>,
    /// Size and mtime of the helper binary next to it, if any.
    helper: Option<(u64, Option<SystemTime>)>,
    info: BinInfo,
}

/// The size and mtime of the helper binary next to `real`.
fn helper_stamp(real: &Path) -> Option<(u64, Option<SystemTime>)> {
    let m = fs::metadata(real.parent()?.join(HELPER)).ok()?;
    Some((m.len(), m.modified().ok()))
}

const HELPER: &str = "genie-code-mode-host";

/// The finder and checker.
pub struct GenieBin {
    candidates: Vec<PathBuf>,
    target: String,
    probe: Arc<dyn BinProbe>,
    cache: Mutex<Vec<Cached>>,
}

impl std::fmt::Debug for GenieBin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenieBin").finish_non_exhaustive()
    }
}

impl GenieBin {
    pub fn new(candidates: Vec<PathBuf>, target: &str, probe: Arc<dyn BinProbe>) -> Self {
        Self {
            candidates,
            target: target.to_owned(),
            probe,
            cache: Mutex::new(Vec::new()),
        }
    }

    /// The real finder: Bottle's own bin folder, `PATH`, then `~/.local/bin/genie`.
    pub fn system(own_bin: Option<PathBuf>, home: Option<PathBuf>, path_var: &str) -> Self {
        let mut c = Vec::new();
        if let Some(d) = own_bin {
            c.push(d.join("genie"));
        }
        // A relative entry would make the result depend on the working folder: skip it.
        for d in path_var.split(':').filter(|d| d.starts_with('/')) {
            c.push(Path::new(d).join("genie"));
        }
        if let Some(h) = home {
            c.push(h.join(".local/bin/genie"));
        }
        Self::new(c, current_target(), Arc::new(SystemProbe))
    }

    /// Check the binary. `recheck` ignores the cache.
    pub fn check(&self, recheck: bool) -> BinInfo {
        if pinned_digest(&self.target, "genie").is_none() {
            return BinInfo::of(GenieCodeBinary::UnsupportedOs);
        }
        let mut first_bad: Option<BinInfo> = None;
        for cand in &self.candidates {
            let Ok(real) = fs::canonicalize(cand) else {
                continue;
            };
            let Ok(meta) = fs::metadata(&real) else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            let info = self.verify_cached(&real, &meta, recheck);
            if info.state == GenieCodeBinary::Ready {
                return info;
            }
            first_bad.get_or_insert(info);
        }
        first_bad.unwrap_or_else(|| BinInfo::of(GenieCodeBinary::NotInstalled))
    }

    fn verify_cached(&self, real: &Path, meta: &fs::Metadata, recheck: bool) -> BinInfo {
        let (size, mtime) = (meta.len(), meta.modified().ok());
        let helper = helper_stamp(real);
        if !recheck {
            let cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(c) = cache.iter().find(|c| {
                c.real == real && c.size == size && c.mtime == mtime && c.helper == helper
            }) {
                return c.info.clone();
            }
        }
        let info = self.verify(real);
        let mut cache = self.cache.lock().unwrap_or_else(|p| p.into_inner());
        cache.retain(|c| c.real != real);
        cache.push(Cached {
            real: real.to_path_buf(),
            size,
            mtime,
            helper,
            info: info.clone(),
        });
        info
    }

    fn verify(&self, real: &Path) -> BinInfo {
        let wrong = |v: Option<String>| BinInfo {
            state: GenieCodeBinary::WrongVersion,
            version: v,
            path: None,
        };
        // The digest first: a file that is not the pinned one is never run.
        let want = pinned_digest(&self.target, "genie");
        if want.is_none() || self.probe.sha256_hex(real).as_deref() != want {
            return wrong(None);
        }
        // The helper binary of the same release must be there and match, if its digest is pinned.
        if let Some(want_host) = pinned_digest(&self.target, HELPER) {
            let host = real.parent().map(|d| d.join(HELPER));
            let ok = host
                .filter(|h| h.exists())
                .is_some_and(|h| self.probe.sha256_hex(&h).as_deref() == Some(want_host));
            if !ok {
                return wrong(None);
            }
        }
        let version = self.probe.version(real);
        if version.as_deref() != Some(GENIE_PINNED) {
            return wrong(version);
        }
        BinInfo {
            state: GenieCodeBinary::Ready,
            version,
            path: Some(real.to_path_buf()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeProbe {
        version: Mutex<Option<String>>,
        sha: Mutex<String>,
        runs: AtomicUsize,
    }

    impl BinProbe for FakeProbe {
        fn version(&self, _: &Path) -> Option<String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            self.version.lock().unwrap().clone()
        }
        fn sha256_hex(&self, p: &Path) -> Option<String> {
            if p.ends_with("genie-code-mode-host") {
                return Some(pinned_digest("aarch64-apple-darwin", "genie-code-mode-host")?.into());
            }
            Some(self.sha.lock().unwrap().clone())
        }
    }

    fn probe(v: &str) -> Arc<FakeProbe> {
        Arc::new(FakeProbe {
            version: Mutex::new(Some(v.to_owned())),
            sha: Mutex::new(
                pinned_digest("aarch64-apple-darwin", "genie")
                    .unwrap()
                    .into(),
            ),
            runs: AtomicUsize::new(0),
        })
    }

    fn dir() -> PathBuf {
        let mut n = [0u8; 6];
        getrandom::fill(&mut n).unwrap();
        let p = std::env::temp_dir().join(format!(
            "r1-bin-{}",
            n.iter().map(|b| format!("{b:02x}")).collect::<String>()
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }

    /// A file, and the helper binary next to it (it is required while its digest is pinned).
    fn touch(d: &Path, name: &str) -> PathBuf {
        let p = d.join(name);
        fs::write(&p, b"bin").unwrap();
        let h = d.join(HELPER);
        if !h.exists() {
            fs::write(h, b"helper").unwrap();
        }
        p
    }

    #[test]
    fn the_pinned_version_with_the_pinned_digest_is_ready() {
        let d = dir();
        let g = touch(&d, "genie");
        let p = probe(GENIE_PINNED);
        let b = GenieBin::new(vec![g.clone()], "aarch64-apple-darwin", p);
        let i = b.check(false);
        assert_eq!(i.state, GenieCodeBinary::Ready);
        assert_eq!(i.version.as_deref(), Some("0.5.0"));
        assert_eq!(i.path, Some(fs::canonicalize(g).unwrap()));
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn another_version_or_digest_is_wrong_version() {
        let d = dir();
        let g = touch(&d, "genie");
        let b = GenieBin::new(vec![g.clone()], "aarch64-apple-darwin", probe("0.6.1"));
        let i = b.check(false);
        assert_eq!(i.state, GenieCodeBinary::WrongVersion);
        assert_eq!(i.version.as_deref(), Some("0.6.1"));
        let p = probe(GENIE_PINNED);
        *p.sha.lock().unwrap() = "0".repeat(64);
        let b = GenieBin::new(vec![g], "aarch64-apple-darwin", p);
        assert_eq!(b.check(false).state, GenieCodeBinary::WrongVersion);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_helper_binary_with_another_digest_is_wrong_version() {
        struct BadHelper(Arc<FakeProbe>);
        impl BinProbe for BadHelper {
            fn version(&self, p: &Path) -> Option<String> {
                self.0.version(p)
            }
            fn sha256_hex(&self, p: &Path) -> Option<String> {
                if p.ends_with("genie-code-mode-host") {
                    return Some("1".repeat(64));
                }
                self.0.sha256_hex(p)
            }
        }
        let d = dir();
        let g = touch(&d, "genie");
        let b = GenieBin::new(
            vec![g],
            "aarch64-apple-darwin",
            Arc::new(BadHelper(probe(GENIE_PINNED))),
        );
        assert_eq!(b.check(false).state, GenieCodeBinary::WrongVersion);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_missing_helper_is_wrong_version_and_a_wrong_digest_is_never_run() {
        let d = dir();
        let g = d.join("genie");
        fs::write(&g, b"bin").unwrap();
        let p = probe(GENIE_PINNED);
        let b = GenieBin::new(vec![g.clone()], "aarch64-apple-darwin", p.clone());
        assert_eq!(b.check(false).state, GenieCodeBinary::WrongVersion);
        assert_eq!(p.runs.load(Ordering::SeqCst), 0, "no helper: never run");
        fs::write(d.join(HELPER), b"h").unwrap();
        let p = probe(GENIE_PINNED);
        *p.sha.lock().unwrap() = "0".repeat(64);
        let b = GenieBin::new(vec![g], "aarch64-apple-darwin", p.clone());
        let i = b.check(false);
        assert_eq!((i.state, i.version), (GenieCodeBinary::WrongVersion, None));
        assert_eq!(
            p.runs.load(Ordering::SeqCst),
            0,
            "a wrong digest is never run"
        );
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_relative_path_entry_is_skipped() {
        let b = GenieBin::system(None, None, "bin:/usr/bin::.:/opt/x");
        assert_eq!(
            b.candidates,
            vec![
                PathBuf::from("/usr/bin/genie"),
                PathBuf::from("/opt/x/genie")
            ]
        );
    }

    #[test]
    fn a_changed_helper_is_checked_again() {
        let d = dir();
        let g = touch(&d, "genie");
        let p = probe(GENIE_PINNED);
        let b = GenieBin::new(vec![g], "aarch64-apple-darwin", p.clone());
        b.check(false);
        fs::write(d.join(HELPER), b"a different helper").unwrap();
        b.check(false);
        assert_eq!(p.runs.load(Ordering::SeqCst), 2);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn no_file_is_not_installed_and_an_unpinned_target_is_unsupported() {
        let d = dir();
        let b = GenieBin::new(
            vec![d.join("genie")],
            "aarch64-apple-darwin",
            probe("0.5.0"),
        );
        assert_eq!(b.check(false).state, GenieCodeBinary::NotInstalled);
        let g = touch(&d, "genie");
        let b = GenieBin::new(vec![g], "x86_64-apple-darwin", probe("0.5.0"));
        assert_eq!(b.check(false).state, GenieCodeBinary::UnsupportedOs);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn a_later_candidate_that_is_ready_wins_over_an_earlier_bad_one() {
        let d1 = dir();
        let d2 = dir();
        let bad = touch(&d1, "genie");
        let good = touch(&d2, "genie");
        struct ByPath(PathBuf, Arc<FakeProbe>);
        impl BinProbe for ByPath {
            fn version(&self, p: &Path) -> Option<String> {
                if p.starts_with(fs::canonicalize(&self.0).unwrap()) {
                    return Some("0.1.0".into());
                }
                self.1.version(p)
            }
            fn sha256_hex(&self, p: &Path) -> Option<String> {
                self.1.sha256_hex(p)
            }
        }
        let b = GenieBin::new(
            vec![bad, good.clone()],
            "aarch64-apple-darwin",
            Arc::new(ByPath(d1.clone(), probe(GENIE_PINNED))),
        );
        let i = b.check(false);
        assert_eq!(i.state, GenieCodeBinary::Ready);
        assert_eq!(i.path, Some(fs::canonicalize(good).unwrap()));
        let _ = fs::remove_dir_all(d1);
        let _ = fs::remove_dir_all(d2);
    }

    #[test]
    fn the_answer_is_cached_until_the_file_changes_or_recheck() {
        let d = dir();
        let g = touch(&d, "genie");
        let p = probe(GENIE_PINNED);
        let b = GenieBin::new(vec![g.clone()], "aarch64-apple-darwin", p.clone());
        b.check(false);
        b.check(false);
        assert_eq!(p.runs.load(Ordering::SeqCst), 1);
        b.check(true);
        assert_eq!(p.runs.load(Ordering::SeqCst), 2);
        fs::write(&g, b"a longer binary").unwrap();
        b.check(false);
        assert_eq!(p.runs.load(Ordering::SeqCst), 3);
        let _ = fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_is_resolved_before_the_check() {
        let d = dir();
        let real = touch(&d, "genie-real");
        let link = d.join("genie");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let b = GenieBin::new(vec![link], "aarch64-apple-darwin", probe(GENIE_PINNED));
        assert_eq!(b.check(false).path, Some(fs::canonicalize(real).unwrap()));
        let _ = fs::remove_dir_all(d);
    }
}
