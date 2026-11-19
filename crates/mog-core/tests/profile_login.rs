//! "Connect a workspace" host side (#162): `profile_login` and `profile_login_cancel`.
//!
//! W-07a, W-07b, W-07c, W-07g, W-07h, W-07i, W-08, W-15. Fake seams only: no real
//! `databricks auth login` runs, the real `~/.databrickscfg` is never read, and every host is
//! a fake host. No token exists here.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cli_runner::{binary_name, Env, Fs, ProcOutput, Process};
use mog_core::cli_runner::{CliPolicy, CliRunner, DatabricksCliRunner};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc_gateway::{GatewayComponents, ERROR_CODES};
use mog_core::profile_login::{
    login_args, proposed_profile_name, workspace_host, LoginVerifier, ProfileLogin, LOGIN_SCOPES,
    LOGIN_TIMEOUT,
};
use mog_core::profile_service::{ConfigFile, ConfigProfile, ConfigSource};
use serde_json::{json, Value};

const HOST: &str = "https://acme-prod.cloud.databricks.com";

// ---------- URL step (W-07a, W-07b, W-07g) ----------

/// [W-07g] a pasted browser URL keeps only `https://<host>`
#[test]
fn w_07g_a_pasted_browser_url_keeps_only_the_host() {
    for pasted in [
        "https://acme-prod.cloud.databricks.com/browse?o=123",
        "  https://acme-prod.cloud.databricks.com/  ",
        "https://ACME-prod.cloud.databricks.com/sql/editor#tab",
        "acme-prod.cloud.databricks.com/browse?o=123",
        "https://acme-prod.cloud.databricks.com:443/?o=1",
    ] {
        assert_eq!(workspace_host(pasted).as_deref(), Some(HOST), "{pasted}");
    }
    assert_eq!(
        workspace_host("https://adb-1234567890123456.7.azuredatabricks.net/?o=1234567890123456")
            .as_deref(),
        Some("https://adb-1234567890123456.7.azuredatabricks.net")
    );
}

/// [W-07b] a URL outside the Databricks domain list is refused
#[test]
fn w_07b_a_non_databricks_domain_is_refused() {
    for bad in [
        "https://example.com/browse?o=1",
        "https://cloud.databricks.com",
        "https://acme.cloud.databricks.com.evil.example",
        "https://accounts.cloud.databricks.com",
        "http://acme-prod.cloud.databricks.com",
        "https://user@acme-prod.cloud.databricks.com",
        "https://acme-prod.cloud.databricks.com:8443",
        "https://10.0.0.1",
        "",
        "not a url",
    ] {
        assert_eq!(workspace_host(bad), None, "{bad}");
    }
}

/// [W-07a] [W-07g] the proposed name is `bottle-<workspace>`; the app never guesses a URL
#[test]
fn w_07a_w_07g_the_proposed_name_is_bottle_workspace() {
    assert_eq!(proposed_profile_name(HOST), "bottle-acme-prod");
    assert_eq!(
        proposed_profile_name("https://adb-1234567890123456.7.azuredatabricks.net"),
        "bottle-adb-1234567890123456"
    );
}

/// [W-07c] [W-15] the CLI arguments, with and without a custom OAuth client
#[test]
fn w_07c_w_15_login_arguments() {
    assert_eq!(
        login_args(HOST, "bottle-acme-prod", None),
        [
            "auth",
            "login",
            "--host",
            HOST,
            "--profile",
            "bottle-acme-prod"
        ]
    );
    assert_eq!(LOGIN_SCOPES, "all-apis,offline_access");
    assert_eq!(
        login_args(HOST, "bottle-custom", Some("custom-app-id")),
        [
            "auth",
            "login",
            "--host",
            HOST,
            "--profile",
            "bottle-custom",
            "--client-id",
            "custom-app-id",
            "--scopes",
            "all-apis,offline_access"
        ]
    );
    assert_eq!(LOGIN_TIMEOUT, Duration::from_secs(300));
}

// ---------- the component over fakes ----------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cli {
    Ok,
    Fail,
    TimeOut,
    /// Wait until the cancel flag is set.
    WaitForCancel,
}

struct FakeCli {
    mode: Cli,
    calls: Mutex<Vec<(Vec<String>, Duration)>>,
    started: AtomicBool,
}

impl FakeCli {
    fn new(mode: Cli) -> Arc<Self> {
        Arc::new(Self {
            mode,
            calls: Mutex::new(Vec::new()),
            started: AtomicBool::new(false),
        })
    }
    fn calls(&self) -> Vec<(Vec<String>, Duration)> {
        self.calls.lock().unwrap().clone()
    }
}

impl CliRunner for FakeCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/usr/local/bin/databricks"))
    }
    fn run_json(&self, _args: &[&str]) -> CoreResult<Value> {
        Err(CoreError::NotImplemented { component: "R5" })
    }
    fn run_login(&self, args: &[&str], timeout: Duration, cancel: &AtomicBool) -> CoreResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push((args.iter().map(|a| a.to_string()).collect(), timeout));
        self.started.store(true, Ordering::SeqCst);
        let fail = |reason| {
            Err(CoreError::Unavailable {
                component: "R5",
                reason,
            })
        };
        match self.mode {
            Cli::Ok => Ok(()),
            Cli::Fail => fail("cli_failed"),
            Cli::TimeOut => fail("cli_timed_out"),
            Cli::WaitForCancel => {
                for _ in 0..2000 {
                    if cancel.load(Ordering::SeqCst) {
                        return fail("cli_cancelled");
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                panic!("the cancel never came");
            }
        }
    }
}

struct FakeConfig(Vec<ConfigProfile>);

impl ConfigSource for FakeConfig {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        Ok(self.0.clone())
    }
}

fn profile(name: &str, host: &str, auth_type: &str) -> ConfigProfile {
    ConfigProfile {
        name: name.into(),
        host: host.into(),
        auth_type: auth_type.into(),
    }
}

/// The W-06a check: answers with a fixed result and records the profile.
struct FakeVerifier {
    result: Result<(), &'static str>,
    checked: Mutex<Vec<String>>,
}

impl FakeVerifier {
    fn new(result: Result<(), &'static str>) -> Arc<Self> {
        Arc::new(Self {
            result,
            checked: Mutex::new(Vec::new()),
        })
    }
}

impl LoginVerifier for FakeVerifier {
    fn verify(&self, profile: &str) -> CoreResult<()> {
        self.checked.lock().unwrap().push(profile.to_owned());
        self.result.map_err(|reason| CoreError::Unavailable {
            component: "R6",
            reason,
        })
    }
}

fn login(
    config: Vec<ConfigProfile>,
    cli: Arc<FakeCli>,
    verifier: Arc<FakeVerifier>,
) -> Arc<ProfileLogin> {
    Arc::new(ProfileLogin::new(
        Arc::new(FakeConfig(config)),
        cli,
        verifier,
    ))
}

fn code(r: CoreResult<Value>) -> &'static str {
    match r {
        Err(CoreError::IpcRefused { code }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn args(url: &str, profile: &str) -> Value {
    json!({ "workspaceUrl": url, "profile": profile })
}

/// [W-07c] [W-07g] a sign-in runs `auth login` for the host only, with the 5 min limit, then the W-06a check
#[test]
fn w_07c_w_07g_a_sign_in_runs_auth_login_and_the_w_06a_check() {
    let cli = FakeCli::new(Cli::Ok);
    let verifier = FakeVerifier::new(Ok(()));
    let t7 = login(vec![], cli.clone(), verifier.clone());
    let reply = t7
        .call(
            "profile_login",
            args(
                "https://acme-prod.cloud.databricks.com/browse?o=123",
                "bottle-acme-prod",
            ),
        )
        .unwrap();
    assert!(reply["loginId"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(reply.as_object().unwrap().len(), 1, "only the loginId");
    let calls = cli.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].0,
        login_args(HOST, "bottle-acme-prod", None)
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(calls[0].1, Duration::from_secs(300));
    assert_eq!(*verifier.checked.lock().unwrap(), vec!["bottle-acme-prod"]);
}

/// [W-15] a custom public OAuth app signs in with `--client-id` and the two scopes
#[test]
fn w_15_a_custom_client_signs_in_with_client_id_and_scopes() {
    let cli = FakeCli::new(Cli::Ok);
    let t7 = login(vec![], cli.clone(), FakeVerifier::new(Ok(())));
    t7.call(
        "profile_login",
        json!({ "workspaceUrl": HOST, "profile": "bottle-custom", "clientId": "custom-app-id" }),
    )
    .unwrap();
    let args = &cli.calls()[0].0;
    assert_eq!(
        &args[6..],
        [
            "--client-id",
            "custom-app-id",
            "--scopes",
            "all-apis,offline_access"
        ]
    );
}

/// [W-15] a client ID that is not a plain ID never reaches the CLI
#[test]
fn w_15_a_bad_client_id_is_refused() {
    for bad in ["", "--profile", "a b", "id;rm"] {
        let cli = FakeCli::new(Cli::Ok);
        let t7 = login(vec![], cli.clone(), FakeVerifier::new(Ok(())));
        let r = t7.call(
            "profile_login",
            json!({ "workspaceUrl": HOST, "profile": "bottle-x", "clientId": bad }),
        );
        assert_eq!(code(r), "invalid_args", "{bad}");
        assert!(cli.calls().is_empty());
    }
}

/// [W-07b] a non-Databricks domain is refused before any CLI child starts
#[test]
fn w_07b_a_non_databricks_domain_starts_no_cli() {
    let cli = FakeCli::new(Cli::Ok);
    let t7 = login(vec![], cli.clone(), FakeVerifier::new(Ok(())));
    let r = t7.call(
        "profile_login",
        args("https://example.com/?o=1", "bottle-x"),
    );
    assert_eq!(code(r), "host_refused");
    assert!(cli.calls().is_empty());
}

/// [W-07g] an existing profile name is refused (any case), and a bad name is refused
#[test]
fn w_07g_an_existing_name_is_refused() {
    let cli = FakeCli::new(Cli::Ok);
    let t7 = login(
        vec![profile("bottle-acme-prod", HOST, "databricks-cli")],
        cli.clone(),
        FakeVerifier::new(Ok(())),
    );
    for name in ["bottle-acme-prod", "BOTTLE-acme-prod"] {
        assert_eq!(
            code(t7.call("profile_login", args(HOST, name))),
            "profile_exists"
        );
    }
    for bad in ["", "-x", "a b", "a]b", "--host", &"x".repeat(65)] {
        assert_eq!(
            code(t7.call("profile_login", args(HOST, bad))),
            "invalid_args",
            "{bad}"
        );
    }
    assert!(cli.calls().is_empty());
}

/// [W-07i] [W-08] with a PAT profile for the host, a new bottle-* profile signs in; the PAT profile is never used or changed
#[test]
fn w_07i_w_08_a_pat_profile_for_the_host_stays_unchanged() {
    let dir = std::env::temp_dir().join(format!("bottle-login-w07i-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("databrickscfg");
    let text = format!(
        "[acme]\nhost = {HOST}\ntoken = test-token-not-real\n\n[acme-sp]\nhost = {HOST}\nauth_type = oauth-m2m\n"
    );
    std::fs::write(&cfg, &text).unwrap();
    let cli = FakeCli::new(Cli::Ok);
    let t7 = ProfileLogin::new(
        Arc::new(ConfigFile::new(Some(cfg.clone()))),
        cli.clone(),
        FakeVerifier::new(Ok(())),
    );
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "acme"))),
        "profile_exists"
    );
    t7.call("profile_login", args(HOST, "bottle-acme-prod"))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&cfg).unwrap(), text);
    let calls = cli.calls();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].0.iter().any(|a| a == "acme" || a == "acme-sp"));
    let _ = std::fs::remove_dir_all(&dir);
}

/// [W-07c] a sign-in that runs over 5 min answers `timeout`
#[test]
fn w_07c_a_sign_in_over_the_limit_answers_timeout() {
    let verifier = FakeVerifier::new(Ok(()));
    let t7 = login(vec![], FakeCli::new(Cli::TimeOut), verifier.clone());
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "timeout"
    );
    assert!(verifier.checked.lock().unwrap().is_empty());
}

/// [W-07h] a failed CLI answers `cli_failed`, with no CLI text
#[test]
fn w_07h_a_failed_cli_answers_cli_failed() {
    let t7 = login(vec![], FakeCli::new(Cli::Fail), FakeVerifier::new(Ok(())));
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "cli_failed"
    );
}

/// [W-07h] Cancel stops the CLI child; the waiting sign-in answers `login_cancelled`
#[test]
fn w_07h_cancel_stops_the_cli_child() {
    let cli = FakeCli::new(Cli::WaitForCancel);
    let t7 = login(vec![], cli.clone(), FakeVerifier::new(Ok(())));
    let t = t7.clone();
    let waiting = std::thread::spawn(move || t.call("profile_login", args(HOST, "bottle-x")));
    while !cli.started.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(5));
    }
    // A second sign-in for the same name while one runs is refused.
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "profile_exists"
    );
    assert_eq!(
        t7.call("profile_login_cancel", json!({ "profile": "bottle-x" }))
            .unwrap(),
        json!({})
    );
    assert_eq!(code(waiting.join().unwrap()), "login_cancelled");
    // "Open the browser again": after the cancel, the same name signs in again.
    assert_eq!(
        t7.call("profile_login_cancel", json!({ "profile": "bottle-x" }))
            .unwrap(),
        json!({}),
        "an unknown or finished profile gives {{}}"
    );
}

/// [W-07h] a failed W-06a check shows a Table W-ERR code, never the upstream reason
#[test]
fn w_07h_a_failed_check_answers_a_w_err_code() {
    for (reason, expected) in [
        ("auth_expired", "auth_expired"),
        ("login_required", "auth_expired"),
        ("upstream_rejected", "no_grant"),
        ("non_json_reply", "non_json_reply"),
        ("offline", "offline"),
        ("transport_failed", "offline"),
        ("timeout", "timeout"),
        ("profile_not_supported", "host_failed"),
    ] {
        let t7 = login(
            vec![],
            FakeCli::new(Cli::Ok),
            FakeVerifier::new(Err(reason)),
        );
        let got = code(t7.call("profile_login", args(HOST, "bottle-x")));
        assert_eq!(got, expected, "{reason}");
        assert!(
            ERROR_CODES.contains(&got),
            "{got} crosses the IPC as itself"
        );
    }
}

/// [W-07h] every new code of the two rows is a gateway code
#[test]
fn w_07h_the_new_codes_are_gateway_codes() {
    for c in [
        "profile_exists",
        "login_cancelled",
        "cli_failed",
        "no_grant",
    ] {
        assert!(ERROR_CODES.contains(&c), "{c}");
    }
}

// ---------- R5 `run_login` on the shipped runner ----------

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

#[derive(Clone)]
struct OneCliFs;

impl Fs for OneCliFs {
    fn is_file(&self, path: &Path) -> bool {
        path == cli_path()
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
        (name == "PATH").then(|| abs("usr/local/bin").into_os_string())
    }
    fn vars(&self) -> Vec<(OsString, OsString)> {
        vec![("PATH".into(), abs("usr/local/bin").into_os_string())]
    }
    fn temp_dir(&self) -> PathBuf {
        abs("var/fake-temp")
    }
}

/// One login child: its args and timeout.
type Seen = (Vec<OsString>, Option<Duration>);

/// Answers `--version` and records the login child (its args and timeout).
#[derive(Clone, Default)]
struct LoginProcess {
    seen: Arc<Mutex<Vec<Seen>>>,
    code: Option<i32>,
}

impl Process for LoginProcess {
    fn run(
        &self,
        program: &Path,
        args: &[OsString],
        env: &[(OsString, OsString)],
    ) -> io::Result<ProcOutput> {
        self.run_with_timeout(program, args, env, None)
    }
    fn run_with_timeout(
        &self,
        _program: &Path,
        args: &[OsString],
        _env: &[(OsString, OsString)],
        timeout: Option<Duration>,
    ) -> io::Result<ProcOutput> {
        if args.first().is_some_and(|a| a == "--version") {
            return Ok(ProcOutput {
                code: Some(0),
                stdout: b"Databricks CLI v1.25.0\n".to_vec(),
                stderr: Vec::new(),
            });
        }
        self.seen.lock().unwrap().push((args.to_vec(), timeout));
        Ok(ProcOutput {
            code: self.code,
            stdout: b"Profile bottle-x was successfully saved".to_vec(),
            stderr: b"debug test-token-not-real".to_vec(),
        })
    }
}

/// [W-07c] R5 runs `auth login` with the caller's limit and refuses any other command
#[test]
fn w_07c_r5_run_login_runs_only_auth_login() {
    let proc = LoginProcess {
        code: Some(0),
        ..LoginProcess::default()
    };
    let r5 = DatabricksCliRunner::new(OneCliFs, PathEnv, proc.clone(), CliPolicy::default());
    let cancel = AtomicBool::new(false);
    let a = login_args(HOST, "bottle-x", None);
    r5.run_login(&a, LOGIN_TIMEOUT, &cancel).unwrap();
    let seen = proc.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].1, Some(LOGIN_TIMEOUT));
    for refused in [
        &["auth", "token", "--profile", "bottle-x"][..],
        &["auth", "logout", "--profile", "bottle-x"][..],
        &["auth", "login", "--profile", "token"][..],
        &["api", "get", "/x"][..],
    ] {
        assert!(
            r5.run_login(refused, LOGIN_TIMEOUT, &cancel).is_err(),
            "{refused:?}"
        );
    }
    assert_eq!(proc.seen.lock().unwrap().len(), 1, "no refused command ran");
}

/// [W-07h] a failed `auth login` is a content-free error
#[test]
fn w_07h_r5_a_failed_login_is_content_free() {
    let proc = LoginProcess {
        code: Some(1),
        ..LoginProcess::default()
    };
    let r5 = DatabricksCliRunner::new(OneCliFs, PathEnv, proc, CliPolicy::default());
    let err = r5
        .run_login(
            &login_args(HOST, "bottle-x", None),
            LOGIN_TIMEOUT,
            &AtomicBool::new(false),
        )
        .unwrap_err();
    let text = format!("{err:?} {err}");
    assert!(!text.contains("test-token-not-real"), "{text}");
    assert!(!text.contains("bottle-x"), "{text}");
    assert!(matches!(
        err,
        CoreError::Unavailable {
            reason: "cli_failed",
            ..
        }
    ));
}

// ---------- review fixes (#583): retry after a failed check, early cancel, pasted URLs ----------

/// A config that the fake CLI writes to, as the real `auth login` does.
#[derive(Default)]
struct SharedConfig(Mutex<Vec<ConfigProfile>>);

impl ConfigSource for SharedConfig {
    fn profiles(&self) -> CoreResult<Vec<ConfigProfile>> {
        Ok(self.0.lock().unwrap().clone())
    }
}

/// `auth login` that writes `[profile]` with its host on success.
struct WritingCli {
    config: Arc<SharedConfig>,
    runs: Mutex<usize>,
}

impl CliRunner for WritingCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/usr/local/bin/databricks"))
    }
    fn run_json(&self, _args: &[&str]) -> CoreResult<Value> {
        Err(CoreError::NotImplemented { component: "R5" })
    }
    fn run_login(&self, args: &[&str], _t: Duration, _c: &AtomicBool) -> CoreResult<()> {
        *self.runs.lock().unwrap() += 1;
        let host = args[3].to_owned();
        let name = args[5].to_owned();
        self.config
            .0
            .lock()
            .unwrap()
            .push(profile(&name, &host, "databricks-cli"));
        Ok(())
    }
}

/// Fails the first `fails` checks, then passes.
struct FlakyVerifier(Mutex<u32>);

impl LoginVerifier for FlakyVerifier {
    fn verify(&self, _profile: &str) -> CoreResult<()> {
        let mut left = self.0.lock().unwrap();
        if *left > 0 {
            *left -= 1;
            return Err(CoreError::Unavailable {
                component: "R6",
                reason: "offline",
            });
        }
        Ok(())
    }
}

/// [W-07h] "Try again" after a failed W-06a check runs only the check: `auth login` already wrote the profile
#[test]
fn w_07h_try_again_after_a_failed_check_runs_only_the_check() {
    let config = Arc::new(SharedConfig::default());
    let cli = Arc::new(WritingCli {
        config: config.clone(),
        runs: Mutex::new(0),
    });
    let t7 = ProfileLogin::new(
        config.clone(),
        cli.clone(),
        Arc::new(FlakyVerifier(Mutex::new(1))),
    );
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "offline"
    );
    assert_eq!(*cli.runs.lock().unwrap(), 1);
    // The CLI wrote [bottle-x]; the retry checks it again instead of answering profile_exists.
    let reply = t7.call("profile_login", args(HOST, "bottle-x")).unwrap();
    assert!(reply["loginId"].is_string());
    assert_eq!(*cli.runs.lock().unwrap(), 1, "no second auth login");
    // Once connected, the name is a normal existing profile again.
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "profile_exists"
    );
}

/// [W-07h] [W-08] the retry path never adopts a profile for another host
#[test]
fn w_07h_w_08_a_retry_for_another_host_is_refused() {
    let config = Arc::new(SharedConfig::default());
    let cli = Arc::new(WritingCli {
        config: config.clone(),
        runs: Mutex::new(0),
    });
    let t7 = ProfileLogin::new(config, cli, Arc::new(FlakyVerifier(Mutex::new(1))));
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "offline"
    );
    let other = "https://other-ws.cloud.databricks.com";
    assert_eq!(
        code(t7.call("profile_login", args(other, "bottle-x"))),
        "profile_exists"
    );
}

/// [W-07h] a cancel that arrives just before the sign-in registers still stops it
#[test]
fn w_07h_a_cancel_just_before_the_start_still_stops_the_sign_in() {
    let cli = FakeCli::new(Cli::Ok);
    let t7 = login(vec![], cli.clone(), FakeVerifier::new(Ok(())));
    t7.call("profile_login_cancel", json!({ "profile": "bottle-x" }))
        .unwrap();
    assert_eq!(
        code(t7.call("profile_login", args(HOST, "bottle-x"))),
        "login_cancelled"
    );
    assert!(cli.calls().is_empty(), "no auth login ran");
    // The marker is used once: the next sign-in runs.
    t7.call("profile_login", args(HOST, "bottle-x")).unwrap();
}

/// [W-07g] a pasted URL with no scheme keeps its host even if its query holds another URL
#[test]
fn w_07g_a_scheme_less_paste_with_a_url_in_its_query_keeps_its_host() {
    assert_eq!(
        workspace_host("acme-prod.cloud.databricks.com/?next=https://x.example").as_deref(),
        Some(HOST)
    );
}
