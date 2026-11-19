//! R5 at app start and the `cli_status` IPC (#16): W-02a, W-02c, W-02d, W-10a and the MDM keys
//! `CliPath` and `CliMinVersion` (Appendix C).
//!
//! Fake seams only: no real CLI runs. The record file is written to a fresh folder under the OS
//! temp dir and holds only a path and a version (no secret).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use cli_runner::install::InstallError;
use cli_runner::{binary_name, Env, Fs, ProcOutput, Process, Version};
use mog_core::cli_runner::{CliPolicy, CliRecord, CliWarning, DatabricksCliRunner};
use mog_core::cli_status::{
    cli_policy, CliInstaller, CliRecordFile, CliStatusComponent, PlaintextStorage,
    CLI_MIN_VERSION_KEY, CLI_PATH_KEY, RECORD_FILE,
};
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::policy_store::{MemoryPolicyStore, StubPolicyStore};
use serde_json::{json, Value};

fn abs(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

fn cli_path() -> PathBuf {
    abs("usr/local/bin").join(binary_name(cfg!(windows)))
}

/// One CLI at [`cli_path`] when `present`.
#[derive(Clone)]
struct OneCli {
    present: Arc<AtomicBool>,
}

impl Fs for OneCli {
    fn is_file(&self, path: &Path) -> bool {
        self.present.load(Ordering::SeqCst) && path == cli_path()
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
    fn unix_mode(&self, _path: &Path) -> Option<u32> {
        (!cfg!(windows)).then_some(0o755)
    }
}

#[derive(Clone)]
struct PathEnv;

impl Env for PathEnv {
    fn var(&self, name: &str) -> Option<OsString> {
        (name == "PATH").then(|| std::env::join_paths([abs("usr/local/bin")]).unwrap())
    }
    fn vars(&self) -> Vec<(OsString, OsString)> {
        vec![("PATH".into(), self.var("PATH").unwrap())]
    }
    fn temp_dir(&self) -> PathBuf {
        abs("var/fake-temp")
    }
}

/// `--version` prints the version in `version`.
#[derive(Clone)]
struct VersionCli {
    version: Arc<Mutex<String>>,
}

impl Process for VersionCli {
    fn run(
        &self,
        _program: &Path,
        _args: &[OsString],
        _env: &[(OsString, OsString)],
    ) -> io::Result<ProcOutput> {
        Ok(ProcOutput {
            code: Some(0),
            stdout: format!("Databricks CLI v{}\n", self.version.lock().unwrap()).into_bytes(),
            stderr: Vec::new(),
        })
    }
}

struct Machine {
    present: Arc<AtomicBool>,
    version: Arc<Mutex<String>>,
}

impl Machine {
    fn new(version: &str) -> Self {
        Self {
            present: Arc::new(AtomicBool::new(true)),
            version: Arc::new(Mutex::new(version.into())),
        }
    }
    /// A new R5, as at each app start.
    fn r5(&self) -> Arc<DatabricksCliRunner<OneCli, PathEnv, VersionCli>> {
        Arc::new(DatabricksCliRunner::new(
            OneCli {
                present: self.present.clone(),
            },
            PathEnv,
            VersionCli {
                version: self.version.clone(),
            },
            CliPolicy::default(),
        ))
    }
}

struct Plaintext(AtomicBool);

impl PlaintextStorage for Plaintext {
    fn plaintext_storage(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A fresh folder for the record file.
fn record_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bottle-cli-status-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn component(
    m: &Machine,
    dir: &Path,
    plaintext: Option<Arc<dyn PlaintextStorage>>,
) -> CliStatusComponent {
    CliStatusComponent::new(m.r5(), CliRecordFile::in_dir(dir), plaintext)
}

fn status(c: &CliStatusComponent) -> Value {
    c.call("cli_status", json!({})).unwrap()
}

/// [W-02a] [W-02c] `cli_status` gives the verified absolute path and version after the launch check
#[test]
fn w_02a_w_02c_cli_status_gives_the_verified_path_and_version() {
    let m = Machine::new("1.25.0");
    let dir = record_dir("status");
    let c = component(&m, &dir, None);
    assert_eq!(
        status(&c),
        json!({"path": "", "version": "", "found": false, "plaintextStorage": false}),
        "before the launch check nothing is verified"
    );
    assert!(c.launch().is_empty());
    assert_eq!(
        status(&c),
        json!({
            "path": cli_path().to_str().unwrap(),
            "version": "1.25.0",
            "found": true,
            "plaintextStorage": false
        })
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-02d] the launch stores the verified record; a changed CLI at the next launch gives warnings
#[test]
fn w_02d_a_changed_cli_at_the_next_launch_gives_a_warning() {
    let m = Machine::new("1.25.0");
    let dir = record_dir("changed");
    assert!(
        component(&m, &dir, None).launch().is_empty(),
        "first launch"
    );
    let file = CliRecordFile::in_dir(&dir);
    assert_eq!(
        file.load(),
        Some(CliRecord {
            path: cli_path(),
            version: Version::new(1, 25, 0)
        })
    );
    // Same CLI: no warning.
    assert!(component(&m, &dir, None).launch().is_empty());
    // A new version: one warning at launch, kept for the UI, and the record moves on.
    *m.version.lock().unwrap() = "1.26.0".into();
    let c = component(&m, &dir, None);
    let warnings = c.launch();
    assert_eq!(
        warnings,
        vec![CliWarning::VersionChanged {
            previous: Version::new(1, 25, 0),
            current: Version::new(1, 26, 0)
        }]
    );
    assert_eq!(c.warnings(), warnings);
    assert_eq!(file.load().unwrap().version, Version::new(1, 26, 0));
    assert!(component(&m, &dir, None).launch().is_empty(), "warned once");
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-02c] a CLI below 1.25 gives the "1.25+ recommended" warning
#[test]
fn w_02c_a_cli_below_the_recommended_version_gives_a_warning() {
    let m = Machine::new("1.20.0");
    let dir = record_dir("old");
    let warnings = component(&m, &dir, None).launch();
    assert!(
        warnings
            .iter()
            .any(|w| matches!(w, CliWarning::BelowRecommended { .. })),
        "{warnings:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-02d] a missing CLI does not fail the launch, keeps the old record, and shows not found
#[test]
fn w_02d_a_missing_cli_does_not_fail_the_launch() {
    let m = Machine::new("1.25.0");
    let dir = record_dir("missing");
    component(&m, &dir, None).launch();
    m.present.store(false, Ordering::SeqCst);
    let c = component(&m, &dir, None);
    assert!(c.launch().is_empty());
    assert_eq!(status(&c)["found"], json!(false));
    assert_eq!(status(&c)["path"], json!(""));
    assert!(
        CliRecordFile::in_dir(&dir).load().is_some(),
        "the last verified record stays for the next compare"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-02d] the record file holds only the path and the version; a bad file is no record
#[test]
fn w_02d_the_record_file_holds_only_path_and_version() {
    let m = Machine::new("1.25.0");
    let dir = record_dir("file");
    component(&m, &dir, None).launch();
    let text = std::fs::read_to_string(dir.join(RECORD_FILE)).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["path", "version"]);
    std::fs::write(dir.join(RECORD_FILE), "not json").unwrap();
    assert_eq!(CliRecordFile::in_dir(&dir).load(), None);
    // A bad file gives no warning and is replaced by the verified record.
    assert!(component(&m, &dir, None).launch().is_empty());
    assert!(CliRecordFile::in_dir(&dir).load().is_some());
    assert_eq!(CliRecordFile::new(None).load(), None, "no app config dir");
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-10a] `plaintextStorage` comes from the T7 profile check, not from R5
#[test]
fn w_10a_plaintext_storage_comes_from_the_profile_check() {
    let m = Machine::new("1.25.0");
    let dir = record_dir("plain");
    let check = Arc::new(Plaintext(AtomicBool::new(true)));
    let c = component(&m, &dir, Some(check.clone()));
    c.launch();
    assert_eq!(status(&c)["plaintextStorage"], json!(true));
    check.0.store(false, Ordering::SeqCst);
    assert_eq!(status(&c)["plaintextStorage"], json!(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-02a] other commands are not this component's
#[test]
fn w_02a_other_commands_are_not_the_cli_status_component() {
    let m = Machine::new("1.25.0");
    let c = component(&m, &record_dir("other"), None);
    assert_eq!(
        c.call("profiles_list", json!({})).unwrap_err().code(),
        "not_implemented"
    );
}

/// [W-02a] [W-02c] MDM `CliPath` and `CliMinVersion` set the R5 policy; a bad or missing key keeps the default
#[test]
fn w_02a_w_02c_mdm_cli_path_and_min_version_set_the_policy() {
    assert_eq!(CLI_PATH_KEY, "CliPath");
    assert_eq!(CLI_MIN_VERSION_KEY, "CliMinVersion");
    let set = MemoryPolicyStore::default()
        .with(CLI_PATH_KEY, json!(cli_path().to_str().unwrap()))
        .with(CLI_MIN_VERSION_KEY, json!("1.30.0"));
    assert_eq!(
        cli_policy(&set),
        CliPolicy {
            cli_path: Some(cli_path()),
            min_version: Version::new(1, 30, 0)
        }
    );
    // The stub store (no MDM source yet) and unset keys give the default.
    assert_eq!(cli_policy(&StubPolicyStore), CliPolicy::default());
    assert_eq!(
        cli_policy(&MemoryPolicyStore::default()),
        CliPolicy::default()
    );
    // A minimum below 1.17 or a bad value never lowers the floor.
    let low = MemoryPolicyStore::default().with(CLI_MIN_VERSION_KEY, json!("1.2.0"));
    assert_eq!(cli_policy(&low).min_version, Version::new(1, 17, 0));
    let bad = MemoryPolicyStore::default()
        .with(CLI_MIN_VERSION_KEY, json!(7))
        .with(CLI_PATH_KEY, json!(""));
    assert_eq!(cli_policy(&bad), CliPolicy::default());
}

// #161: `cli_install {consent:true}` (W-17b, W-17d, W-17e, W-17f, U16). The installer is a fake:
// it "installs" by making the one CLI of the fake machine appear. No network.

/// A fake installer: counts its calls, then makes the CLI appear or fails.
struct FakeInstaller {
    present: Arc<AtomicBool>,
    calls: Mutex<u32>,
    fail: bool,
}

impl CliInstaller for FakeInstaller {
    fn install(&self) -> Result<PathBuf, InstallError> {
        *self.calls.lock().unwrap() += 1;
        if self.fail {
            return Err(InstallError::ChecksumMismatch);
        }
        self.present.store(true, Ordering::SeqCst);
        Ok(cli_path())
    }
}

fn no_cli_with_installer(fail: bool) -> (Machine, Arc<FakeInstaller>) {
    let m = Machine::new("1.26.0");
    m.present.store(false, Ordering::SeqCst);
    let installer = Arc::new(FakeInstaller {
        present: m.present.clone(),
        calls: Mutex::new(0),
        fail,
    });
    (m, installer)
}

/// [W-17b] [U16] `cli_install` without `consent: true` installs nothing
#[test]
fn w_17b_cli_install_needs_consent_true() {
    let (m, installer) = no_cli_with_installer(false);
    let dir = record_dir("install-consent");
    let c = component(&m, &dir, None).with_installer(installer.clone());
    for args in [json!({}), json!({"consent": false})] {
        assert!(c.call("cli_install", args).is_err());
    }
    assert_eq!(*installer.calls.lock().unwrap(), 0);
    assert_eq!(status(&c)["found"], json!(false));
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-17e] [W-17f] after the install R5 checks the new CLI again and `cli_install` answers its path
#[test]
fn w_17e_w_17f_cli_install_runs_the_launch_check_on_the_new_cli() {
    let (m, installer) = no_cli_with_installer(false);
    let dir = record_dir("install-ok");
    let c = component(&m, &dir, None).with_installer(installer.clone());
    c.launch();
    assert_eq!(status(&c)["found"], json!(false));
    let reply = c.call("cli_install", json!({"consent": true})).unwrap();
    assert_eq!(
        reply,
        json!({
            "path": cli_path().to_str().unwrap(),
            "version": "1.26.0",
            "found": true,
            "plaintextStorage": false
        })
    );
    assert_eq!(*installer.calls.lock().unwrap(), 1);
    assert_eq!(status(&c), reply);
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-17d] [SEC-20b] a refused install is a content-free error and the CLI stays not found
#[test]
fn w_17d_a_failed_install_is_refused_without_detail() {
    let (m, installer) = no_cli_with_installer(true);
    let dir = record_dir("install-fail");
    let c = component(&m, &dir, None).with_installer(installer);
    let err = c.call("cli_install", json!({"consent": true})).unwrap_err();
    // The host sends it as `host_failed` (not in ERROR_CODES): no path, URL or hash crosses.
    assert_eq!(err.code(), "cli_install_checksum_mismatch");
    assert_eq!(status(&c)["found"], json!(false));
    // No installer (no user folder on this machine): refused too.
    let c = component(&m, &dir, None);
    assert!(c.call("cli_install", json!({"consent": true})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-17d] the real download asks only HTTPS GitHub release hosts on port 443
#[test]
fn w_17d_release_downloads_go_only_to_github_release_hosts() {
    use mog_core::cli_status::is_release_url;
    let ok = |u: &str| is_release_url(&reqwest::Url::parse(u).unwrap());
    assert!(ok(
        "https://github.com/databricks/cli/releases/download/v1.26.0/databricks_cli_1.26.0_SHA256SUMS"
    ));
    assert!(ok(
        "https://release-assets.githubusercontent.com/github-production-release-asset/x"
    ));
    assert!(ok("https://objects.githubusercontent.com/x"));
    for bad in [
        "http://github.com/databricks/cli/releases/download/v1.26.0/x",
        "https://github.com:8443/x",
        "https://evil.example/x",
        "https://github.com.evil.example/x",
        "https://user@github.com/x",
    ] {
        assert!(!ok(bad), "{bad}");
    }
}
