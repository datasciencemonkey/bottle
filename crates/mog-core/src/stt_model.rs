// Adapted from cjpais/Handy src-tauri/src/managers/model/download.rs @ aca8985 (MIT, Copyright (c) 2025 CJ Pais). See THIRD_PARTY_NOTICES.
//! R4 local STT model install (#45): the Parakeet TDT 0.6B v3 int8 model of sherpa-onnx.
//!
//! - CAP-49a: [`ModelInstaller::install`] asks the [`ConsentGate`] first. Without
//!   [`ModelConsent::Granted`] it sends no request and writes no file. The onboarding wizard
//!   (MB-07, #43) owns the consent question and implements the gate.
//! - CAP-49b: requests go only to the pinned sherpa-onnx release URL or the pinned Hugging Face
//!   mirror revision ([`is_allowed_url`]). A broken download keeps its `.partial` file and the next
//!   install asks for the rest only (HTTP `Range`). A server that ignores `Range` restarts the file.
//! - CAP-49c: every downloaded file and every extracted model file is checked against a pinned
//!   size and SHA-256. A mismatch deletes the bad file and stops the install.
//!
//! From Handy: the `.partial` resume file, the `Content-Range` start parser and the
//! delete-on-mismatch hash check. Bottle adds the consent gate, the URL allowlist and the
//! per-file pins, and has no logs, history or telemetry (SEC rules). Errors are content-free codes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};

/// The pinned GitHub release folder of the sherpa-onnx ASR models.
pub const GITHUB_RELEASE_BASE: &str =
    "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/";
/// The pinned Hugging Face mirror (an exact commit, so the bytes cannot change under the pins).
pub const HF_MIRROR_BASE: &str = "https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8/resolve/ed5d745623472f97cfc2e6c2916ee248798fc5a8/";

/// The vendored sherpa-onnx native archives (CAP-50a): the same list as
/// `crates/mog-core/sherpa-onnx-archives.sha256`, which `build.rs` and
/// `scripts/vendor_sherpa_onnx.py` read. Digests are the GitHub release asset digests of v2.0.1.
pub const SHERPA_ONNX_ARCHIVE_PINS: &[(&str, &str)] = &[
    (
        "sherpa-onnx-v2.0.1-osx-arm64-static-lib.tar.bz2",
        "d7a9b14f337958e7373c8c650f046df33d210bbaff2f33cb8322dda663ee5e1c",
    ),
    (
        "sherpa-onnx-v2.0.1-osx-x64-static-lib.tar.bz2",
        "6a3cb3c14104d3a02e7b2a6cd5621f590b14892cce7d0bc0935f7655ef518938",
    ),
    (
        "sherpa-onnx-v2.0.1-win-x64-static-MT-Release-lib.tar.bz2",
        "23e7b986b197eda0bbc47ff3d7d5c87afd6ef2b9077214ba26b0f1ec52ef0e66",
    ),
    (
        "sherpa-onnx-v2.0.1-linux-x64-static-lib.tar.bz2",
        "55cfadf65ba50d0afc1a802b7a337942819e51c94a5b5a384def1978f7c70871",
    ),
];

/// One file with a pinned size and SHA-256 (lower-case hex).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedFile {
    /// The file name.
    pub name: &'static str,
    /// The size in bytes.
    pub size: u64,
    /// The SHA-256, lower-case hex.
    pub sha256: &'static str,
}

/// A model: its folder name, its release archive and the files it must hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSpec {
    /// The folder name under the models folder (also the archive's top folder).
    pub name: &'static str,
    /// The release archive (`.tar.bz2`).
    pub archive: PinnedFile,
    /// The files that the recognizer loads.
    pub files: Vec<PinnedFile>,
}

impl ModelSpec {
    /// Parakeet TDT 0.6B v3 int8 (D78). Hashes from SP-04 (`docs/spikes/SP-04.md`); the
    /// archive digest is the GitHub release asset digest and the file digests match the
    /// Hugging Face LFS digests of the pinned mirror revision.
    pub fn parakeet_tdt_v3_int8() -> Self {
        ModelSpec {
            name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8",
            archive: PinnedFile {
                name: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8.tar.bz2",
                size: 487_211_443,
                sha256: "1495e91b59d7fc53ddca91f3431758ec99f8fa253fdb0e1e851be0f72b14b0f9",
            },
            files: vec![
                PinnedFile {
                    name: "encoder.int8.onnx",
                    size: 652_282_294,
                    sha256: "1afbe69c1ffdabee94755f42e4daf1d93549131a3da11356bda84fcef0f2c479",
                },
                PinnedFile {
                    name: "decoder.int8.onnx",
                    size: 11_845_274,
                    sha256: "0222a86147299d4dee90edf95b6d13827edb42ad8b7c9cf0b2582f030937c0b5",
                },
                PinnedFile {
                    name: "joiner.int8.onnx",
                    size: 6_355_277,
                    sha256: "cbd02f3696e9944970111fb439d9c3fd365a80374565f13f2b9c7007d253fc98",
                },
                PinnedFile {
                    name: "tokens.txt",
                    size: 93_939,
                    sha256: "d58544679ea4bc6ac563d1f545eb7d474bd6cfa467f0a6e2c1dc1c7d37e3c35d",
                },
            ],
        }
    }
}

/// Where the model comes from (CAP-49b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    /// The sherpa-onnx GitHub release archive (one 487 MB `.tar.bz2`, then an extract).
    GithubRelease,
    /// The Hugging Face mirror: the model files one by one, at a pinned commit.
    HuggingFaceMirror,
}

impl ModelSource {
    /// The URLs that this source fetches for `spec`, in order.
    pub fn urls(self, spec: &ModelSpec) -> Vec<String> {
        match self {
            ModelSource::GithubRelease => {
                vec![format!("{GITHUB_RELEASE_BASE}{}", spec.archive.name)]
            }
            ModelSource::HuggingFaceMirror => spec
                .files
                .iter()
                .map(|f| format!("{HF_MIRROR_BASE}{}", f.name))
                .collect(),
        }
    }
}

/// True only for a URL under the pinned release folder or the pinned mirror commit, with a plain
/// file name (CAP-49b).
pub fn is_allowed_url(url: &str) -> bool {
    [GITHUB_RELEASE_BASE, HF_MIRROR_BASE].iter().any(|base| {
        url.strip_prefix(base).is_some_and(|name| {
            !name.is_empty()
                && name != "."
                && name != ".."
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                && !name.contains("..")
        })
    })
}

/// The user's answer to the model download question (MB-07 STT step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelConsent {
    /// The user was not asked yet.
    NotAsked,
    /// The user said no.
    Declined,
    /// The user agreed to download the model.
    Granted,
}

/// The consent gate (CAP-49a). The onboarding wizard (#43) implements it from the user's answer.
pub trait ConsentGate {
    /// The user's current answer.
    fn model_download_consent(&self) -> ModelConsent;
}

/// A network error. It holds no URL, header or body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The server replied with this HTTP status.
    Status(u16),
    /// The connection failed.
    Transport,
}

/// The network side of the installer. The real one is `stt_local::HttpRangeFetch`
/// (feature `local-stt`); tests use a double.
pub trait RangeFetch {
    /// Open `url` from byte `offset`. Returns the offset that the body starts at: `offset` when
    /// the server honoured the `Range` header, or 0 when it sends the whole file.
    fn open(&self, url: &str, offset: u64) -> Result<(u64, Box<dyn Read + '_>), FetchError>;
}

/// Why an install stopped. Every variant maps to a content-free [`InstallError::code`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// CAP-49a: the user has not agreed to the download.
    ConsentRequired,
    /// CAP-49b: a URL is not under a pinned source.
    UrlNotAllowed,
    /// The download broke. The partial file stays for a resume.
    Download,
    /// CAP-49c: a file does not match its pinned size or SHA-256. The file is deleted.
    HashMismatch {
        /// The file name.
        file: &'static str,
    },
    /// The archive did not extract.
    Extract,
    /// A local file operation failed.
    Io,
    /// The caller cancelled the download. The partial file stays for a resume.
    Cancelled,
}

impl InstallError {
    /// The stable, content-free code for IPC and the wizard.
    pub fn code(&self) -> &'static str {
        match self {
            InstallError::ConsentRequired => "stt_consent_required",
            InstallError::UrlNotAllowed => "stt_url_not_allowed",
            InstallError::Download => "stt_download_failed",
            InstallError::HashMismatch { .. } => "stt_hash_mismatch",
            InstallError::Extract => "stt_extract_failed",
            InstallError::Io => "stt_io_failed",
            InstallError::Cancelled => "stt_download_cancelled",
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for InstallError {}

/// What is on disk now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStatus {
    /// Every model file is present with its pinned size.
    Installed,
    /// A download stopped; this many bytes are kept for a resume.
    Partial {
        /// Bytes in the `.partial` files.
        bytes: u64,
    },
    /// Nothing is downloaded.
    Missing,
}

/// Download progress for the wizard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    /// The file that downloads now.
    pub file: &'static str,
    /// Bytes of this file on disk.
    pub done: u64,
    /// The pinned size of this file.
    pub total: u64,
}

/// Installs one [`ModelSpec`] under a models folder: `<root>/<spec.name>/<files>`.
#[derive(Debug, Clone)]
pub struct ModelInstaller {
    root: PathBuf,
    spec: ModelSpec,
}

impl ModelInstaller {
    /// An installer for `spec` under the models folder `root`. Nothing is created yet.
    pub fn new(root: impl Into<PathBuf>, spec: ModelSpec) -> Self {
        ModelInstaller {
            root: root.into(),
            spec,
        }
    }

    /// The folder that holds the model files.
    pub fn model_dir(&self) -> PathBuf {
        self.root.join(self.spec.name)
    }

    fn partial(&self, file: &PinnedFile) -> PathBuf {
        self.root.join(format!("{}.partial", file.name))
    }

    /// What is on disk (sizes only; [`ModelInstaller::install`] checks the hashes).
    pub fn status(&self) -> ModelStatus {
        let dir = self.model_dir();
        let installed = self.spec.files.iter().all(|f| {
            fs::metadata(dir.join(f.name)).is_ok_and(|m| m.is_file() && m.len() == f.size)
        });
        if installed {
            return ModelStatus::Installed;
        }
        let bytes: u64 = std::iter::once(&self.spec.archive)
            .chain(self.spec.files.iter())
            .filter_map(|f| fs::metadata(self.partial(f)).ok())
            .map(|m| m.len())
            .sum();
        if bytes > 0 {
            ModelStatus::Partial { bytes }
        } else {
            ModelStatus::Missing
        }
    }

    /// Download, check and install the model. Returns the model folder.
    ///
    /// `extract(archive, dest)` unpacks the files of the release archive into `dest` (used only
    /// for [`ModelSource::GithubRelease`]; the real one is `stt_local::extract_model_archive`).
    /// `cancel` stops the download between chunks. `progress` gets a call for each chunk.
    pub fn install(
        &self,
        gate: &dyn ConsentGate,
        net: &dyn RangeFetch,
        source: ModelSource,
        extract: &dyn Fn(&Path, &Path) -> io::Result<()>,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, InstallError> {
        // CAP-49a: the consent check comes before any request or file.
        if gate.model_download_consent() != ModelConsent::Granted {
            return Err(InstallError::ConsentRequired);
        }
        let urls = source.urls(&self.spec);
        if !urls.iter().all(|u| is_allowed_url(u)) {
            return Err(InstallError::UrlNotAllowed);
        }
        let dir = self.model_dir();
        if self.status() == ModelStatus::Installed && self.verify_installed().is_ok() {
            return Ok(dir);
        }
        fs::create_dir_all(&self.root).map_err(|_| InstallError::Io)?;
        match source {
            ModelSource::GithubRelease => {
                let archive = self.spec.archive.clone();
                let path = self.root.join(archive.name);
                if !matches_pin(&path, &archive) {
                    self.download(net, &urls[0], &archive, &path, cancel, progress)?;
                }
                let staging = self.root.join(format!("{}.extracting", self.spec.name));
                let _ = fs::remove_dir_all(&staging);
                fs::create_dir_all(&staging).map_err(|_| InstallError::Io)?;
                if extract(&path, &staging).is_err() {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(InstallError::Extract);
                }
                if let Err(e) = verify_files(&staging, &self.spec.files) {
                    let _ = fs::remove_dir_all(&staging);
                    return Err(e);
                }
                let _ = fs::remove_dir_all(&dir);
                fs::rename(&staging, &dir).map_err(|_| InstallError::Io)?;
                let _ = fs::remove_file(&path);
            }
            ModelSource::HuggingFaceMirror => {
                fs::create_dir_all(&dir).map_err(|_| InstallError::Io)?;
                for (file, url) in self.spec.files.iter().zip(&urls) {
                    let dest = dir.join(file.name);
                    if !matches_pin(&dest, file) {
                        self.download(net, url, file, &dest, cancel, progress)?;
                    }
                }
            }
        }
        self.verify_installed().map(|()| dir)
    }

    /// Check every model file against its pin; delete the model folder on a mismatch.
    fn verify_installed(&self) -> Result<(), InstallError> {
        let dir = self.model_dir();
        let res = verify_files(&dir, &self.spec.files);
        if res.is_err() {
            let _ = fs::remove_dir_all(&dir);
        }
        res
    }

    /// Download one file to `<root>/<name>.partial` with resume, check it, then move it to `dest`.
    fn download(
        &self,
        net: &dyn RangeFetch,
        url: &str,
        file: &PinnedFile,
        dest: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<(), InstallError> {
        if !is_allowed_url(url) {
            return Err(InstallError::UrlNotAllowed);
        }
        let part = self.partial(file);
        let mut have = fs::metadata(&part).map_or(0, |m| m.len());
        if have > file.size {
            let _ = fs::remove_file(&part);
            have = 0;
        }
        if have < file.size {
            let (start, mut body) = net.open(url, have).map_err(|_| InstallError::Download)?;
            if start != 0 && start != have {
                return Err(InstallError::Download);
            }
            let mut out = if start == have && have > 0 {
                OpenOptions::new().append(true).open(&part)
            } else {
                // A fresh file, or the server ignored `Range` and sends the whole file.
                have = 0;
                File::create(&part)
            }
            .map_err(|_| InstallError::Io)?;
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(InstallError::Cancelled);
                }
                let n = match body.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => n,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => {
                        let _ = out.flush();
                        return Err(InstallError::Download);
                    }
                };
                if have + n as u64 > file.size {
                    drop(out);
                    let _ = fs::remove_file(&part);
                    return Err(InstallError::HashMismatch { file: file.name });
                }
                out.write_all(&buf[..n]).map_err(|_| InstallError::Io)?;
                have += n as u64;
                progress(Progress {
                    file: file.name,
                    done: have,
                    total: file.size,
                });
            }
            out.flush().map_err(|_| InstallError::Io)?;
        }
        if have < file.size {
            // The body ended early: keep the partial file for a resume.
            return Err(InstallError::Download);
        }
        if !matches_pin(&part, file) {
            // CAP-49c: a bad file must not be resumed or used.
            let _ = fs::remove_file(&part);
            return Err(InstallError::HashMismatch { file: file.name });
        }
        fs::rename(&part, dest).map_err(|_| InstallError::Io)
    }
}

fn verify_files(dir: &Path, files: &[PinnedFile]) -> Result<(), InstallError> {
    for f in files {
        if !matches_pin(&dir.join(f.name), f) {
            return Err(InstallError::HashMismatch { file: f.name });
        }
    }
    Ok(())
}

/// True when `path` has the pinned size and SHA-256.
fn matches_pin(path: &Path, pin: &PinnedFile) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == pin.size)
        && file_sha256_hex(path).is_ok_and(|h| h == pin.sha256)
}

/// The SHA-256 of `bytes`, lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// The SHA-256 of a file, read in 64 KiB chunks, lower-case hex.
pub fn file_sha256_hex(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The start offset of a `Content-Range: bytes <start>-<end>/<total>` header.
pub fn content_range_start(value: &str) -> Option<u64> {
    let range = value.trim().strip_prefix("bytes")?.trim_start();
    range.split('-').next()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_49b_content_range_start_parses_the_resume_offset() {
        assert_eq!(content_range_start("bytes 700-999/1000"), Some(700));
        assert_eq!(content_range_start(" bytes 0-9/10"), Some(0));
        assert_eq!(content_range_start("items 1-2/3"), None);
    }

    #[test]
    fn cap_49c_sha256_hex_is_lower_case() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
