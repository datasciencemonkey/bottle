//! The backend of the wizard STT step (MB-07, #413, ADR 0049).
//!
//! - W-17e: the model goes in a folder that only the user can write, with no admin rights:
//!   `~/Library/Application Support/Bottle/models` on macOS and `%LOCALAPPDATA%\Bottle\models`
//!   on Windows ([`models_dir`]).
//! - CAP-49a: [`FileConsent`] keeps the user's answer to the 487 MB download question in the
//!   models folder, so it lasts across launches. It is the [`ConsentGate`] that
//!   [`ModelInstaller::install`] reads before any request or file write.
//! - CAP-49b: [`SttSetup`] runs one install at a time, can cancel it between chunks, and a
//!   retry resumes from the `.partial` file.
//! - CAP-49c: a hash mismatch stops the install ([`SetupError::code`] `stt_hash_mismatch`).
//!
//! Every error is a content-free code: no path, URL, header or file content (SEC-03).
//! TODO(#529): the `stt_model_*` hub commands call this service.

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::stt_model::{
    ConsentGate, InstallError, ModelConsent, ModelInstaller, ModelSource, ModelSpec, ModelStatus,
    Progress, RangeFetch,
};

/// The file in the models folder that holds the stored answer (one word).
pub const CONSENT_FILE: &str = "stt-download-consent";

/// The OS rule that [`models_dir`] applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetOs {
    /// `$HOME/Library/Application Support/Bottle/models`.
    MacOs,
    /// `%LOCALAPPDATA%\Bottle\models`.
    Windows,
    /// Dev builds only (Linux CI): `$XDG_DATA_HOME/Bottle/models`, else
    /// `$HOME/.local/share/Bottle/models`.
    Other,
}

impl TargetOs {
    /// The rule of the OS that this build runs on.
    pub fn this() -> Self {
        if cfg!(target_os = "macos") {
            TargetOs::MacOs
        } else if cfg!(windows) {
            TargetOs::Windows
        } else {
            TargetOs::Other
        }
    }

    /// True for an absolute path in this OS's syntax, on any host (so tests run everywhere).
    fn is_absolute(self, v: &OsString) -> bool {
        let s = v.to_string_lossy();
        match self {
            TargetOs::Windows => {
                let b = s.as_bytes();
                s.starts_with(r"\\")
                    || (b.len() >= 3
                        && b[0].is_ascii_alphabetic()
                        && b[1] == b':'
                        && (b[2] == b'\\' || b[2] == b'/'))
            }
            TargetOs::MacOs | TargetOs::Other => s.starts_with('/'),
        }
    }
}

/// The W-17e models folder for `os`, from the environment `env`. `None` when the base variable
/// is missing, empty or not absolute (the app then offers no download).
pub fn models_dir(os: TargetOs, env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let base = |key: &str| env(key).filter(|v| !v.is_empty() && os.is_absolute(v));
    let bottle = match os {
        TargetOs::MacOs => PathBuf::from(base("HOME")?)
            .join("Library")
            .join("Application Support")
            .join("Bottle"),
        TargetOs::Windows => PathBuf::from(base("LOCALAPPDATA")?).join("Bottle"),
        TargetOs::Other => match base("XDG_DATA_HOME") {
            Some(data) => PathBuf::from(data).join("Bottle"),
            None => PathBuf::from(base("HOME")?)
                .join(".local")
                .join("share")
                .join("Bottle"),
        },
    };
    Some(bottle.join("models"))
}

/// The W-17e models folder of this device.
pub fn models_dir_for_this_device() -> Option<PathBuf> {
    models_dir(TargetOs::this(), &|k| std::env::var_os(k))
}

/// The stored answer to the model download question (CAP-49a), in `<models>/stt-download-consent`.
#[derive(Debug, Clone)]
pub struct FileConsent {
    path: PathBuf,
}

impl FileConsent {
    /// The answer stored under the models folder `root`.
    pub fn new(root: impl AsRef<Path>) -> Self {
        FileConsent {
            path: root.as_ref().join(CONSENT_FILE),
        }
    }

    /// Store `answer`. `NotAsked` removes the stored answer.
    pub fn write(&self, answer: ModelConsent) -> Result<(), SetupError> {
        let word = match answer {
            ModelConsent::Granted => "granted",
            ModelConsent::Declined => "declined",
            ModelConsent::NotAsked => {
                return match fs::remove_file(&self.path) {
                    Ok(()) => Ok(()),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                    Err(_) => Err(SetupError::Install(InstallError::Io)),
                };
            }
        };
        let io_err = |_| SetupError::Install(InstallError::Io);
        let dir = self.path.parent().ok_or(SetupError::ModelsDirUnavailable)?;
        fs::create_dir_all(dir).map_err(io_err)?;
        // Write a new file, then rename it, so a crash never leaves half a word.
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, word).map_err(io_err)?;
        fs::rename(&tmp, &self.path).map_err(io_err)
    }
}

impl ConsentGate for FileConsent {
    fn model_download_consent(&self) -> ModelConsent {
        match fs::read_to_string(&self.path).as_deref().map(str::trim) {
            Ok("granted") => ModelConsent::Granted,
            Ok("declined") => ModelConsent::Declined,
            _ => ModelConsent::NotAsked,
        }
    }
}

/// Why a setup call failed. Each variant has a content-free [`SetupError::code`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetupError {
    /// The installer stopped (#45 codes).
    Install(InstallError),
    /// An install runs already.
    Busy,
    /// The OS gives no user folder for the models (W-17e).
    ModelsDirUnavailable,
}

impl SetupError {
    /// The stable code for IPC and the wizard text.
    pub fn code(&self) -> &'static str {
        match self {
            SetupError::Install(e) => e.code(),
            SetupError::Busy => "stt_install_busy",
            SetupError::ModelsDirUnavailable => "stt_models_dir_unavailable",
        }
    }
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

impl std::error::Error for SetupError {}

impl From<InstallError> for SetupError {
    fn from(e: InstallError) -> Self {
        SetupError::Install(e)
    }
}

/// What the wizard step shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStatus {
    /// Missing, Partial or Installed (sizes only; `install` checks the hashes).
    pub model: ModelStatus,
    /// The stored answer.
    pub consent: ModelConsent,
    /// An install runs now.
    pub installing: bool,
    /// The pinned archive size, for the "487 MB" text.
    pub total_bytes: u64,
}

/// The archive extractor (`stt_local::extract_model_archive` with feature `local-stt`).
pub type Extractor = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;

/// The network side, shared with the install worker.
pub type SharedFetch = Arc<dyn RangeFetch + Send + Sync>;

/// The wizard STT step service: status, consent, install, cancel.
pub struct SttSetup {
    installer: ModelInstaller,
    consent: FileConsent,
    net: SharedFetch,
    extract: Arc<Extractor>,
    cancel: AtomicBool,
    busy: AtomicBool,
    total_bytes: u64,
}

/// Clears the busy flag when an install ends, also on a panic.
struct BusyGuard<'a>(&'a AtomicBool);

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl SttSetup {
    /// A service for `spec` under the models folder `root`. Nothing is created yet.
    pub fn new(
        root: impl AsRef<Path>,
        spec: ModelSpec,
        net: SharedFetch,
        extract: Arc<Extractor>,
    ) -> Self {
        let root = root.as_ref();
        let total_bytes = spec.archive.size;
        SttSetup {
            total_bytes,
            installer: ModelInstaller::new(root, spec),
            consent: FileConsent::new(root),
            net,
            extract,
            cancel: AtomicBool::new(false),
            busy: AtomicBool::new(false),
        }
    }

    /// The Parakeet model (D78) in this device's W-17e folder.
    pub fn for_this_device(net: SharedFetch, extract: Arc<Extractor>) -> Result<Self, SetupError> {
        let root = models_dir_for_this_device().ok_or(SetupError::ModelsDirUnavailable)?;
        Ok(SttSetup::new(
            root,
            ModelSpec::parakeet_tdt_v3_int8(),
            net,
            extract,
        ))
    }

    /// The folder that the recognizer loads.
    pub fn model_dir(&self) -> PathBuf {
        self.installer.model_dir()
    }

    /// What the step shows now.
    pub fn status(&self) -> SetupStatus {
        SetupStatus {
            model: self.installer.status(),
            consent: self.consent.model_download_consent(),
            installing: self.busy.load(Ordering::Acquire),
            total_bytes: self.total_bytes,
        }
    }

    /// Store the user's answer and return the new status.
    pub fn set_consent(&self, answer: ModelConsent) -> Result<SetupStatus, SetupError> {
        self.consent.write(answer)?;
        Ok(self.status())
    }

    /// Download, check and install the model. The gate reads the stored answer now, so the
    /// install runs only after Granted (CAP-49a). The wizard calls this also when `status`
    /// shows Installed, because only `install` checks the hashes.
    pub fn install(
        &self,
        source: ModelSource,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<PathBuf, SetupError> {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(SetupError::Busy);
        }
        let _guard = BusyGuard(&self.busy);
        self.cancel.store(false, Ordering::Release);
        let extract = self.extract.clone();
        let extract = move |a: &Path, d: &Path| extract(a, d);
        Ok(self.installer.install(
            &self.consent,
            self.net.as_ref(),
            source,
            &extract,
            &self.cancel,
            progress,
        )?)
    }

    /// Ask a running install to stop at its next chunk. `false` when no install runs.
    pub fn cancel(&self) -> bool {
        if self.busy.load(Ordering::Acquire) {
            self.cancel.store(true, Ordering::Release);
            true
        } else {
            false
        }
    }
}
