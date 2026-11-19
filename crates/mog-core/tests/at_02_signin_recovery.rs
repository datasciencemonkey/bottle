//! AT-02 "Recover expired sign-in" (#233), section 15. Gate: live.
//!
//! The AT script: expire a U2M CLI profile, start sign-in from the chip and complete it. Repeat
//! with a PAT and a service-principal profile, an old CLI and plaintext token storage. Pass: the
//! MOG config is unchanged, paused polls resume with the same IDs, PAT and SP profiles are
//! rejected, and the old CLI and plaintext storage show blocking messages.
//!
//! The Rust side of AT-02 is R5 CliRunner (#16) and R6 TokenBroker (#18). R5 ships, so W-02a..d
//! run here on fakes of its seams. Only the token buffer of R6 ships, so W-18c's zero overwrite
//! runs here. R6 refresh and 401 (W-05a..c) run here against the real `CliTokenBroker` with a
//! fake CLI (#18). The other tests are written against `mog_core::token_broker::TokenBroker` and stay
//! `#[ignore]` with the feature issue, so they never count as a pass. The live parts read the
//! #229 fixture (`BOTTLE_TEST_PROFILE`) and need a person: docs/tests/AT-SIGNIN-WORKSPACE-HOP.md.
//! The TS side (ProfileService, the chip, held work) is `src/acceptance/at-02-signin-recovery.test.ts`.
//!
//! No test here reads, prints or stores a token. Test tokens are made-up bytes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mog_core::cli_runner::{CliRunner, SecretStdout};
use mog_core::clock::{MonotonicClock, SimClock, SimWallClock};
use mog_core::error::CoreResult;
use mog_core::http_transport::ProfileHosts;
use mog_core::token_broker::{
    CliTokenBroker, SecretToken, StubTokenBroker, TokenBroker, TokenLease,
};

/// A made-up token shape. It is not a real token and it never leaves the test.
fn fake_token() -> SecretToken {
    SecretToken::new("fake-test-token-0123456789abcdef".to_owned())
}

// ---------- Runs today: the R6 token buffer (W-18c) ----------

#[test]
fn at_02_w_18c_a_token_buffer_wipe_writes_zeros_buffer_part_only() {
    let mut t = fake_token();
    let len = t.len();
    assert!(len > 0 && !t.is_zeroed());
    // This is the buffer part of W-18c only: the sign-out that calls wipe is TODO(#163)
    // in the pending test below. Drop calls wipe too. Every byte is zero and the length stays.
    t.wipe();
    assert!(t.is_zeroed());
    assert_eq!(t.len(), len);
    // The buffer never shows in Debug output, before or after the wipe.
    let shown = format!("{t:?}");
    assert_eq!(shown, "SecretToken(<redacted>)");
    assert!(!shown.contains("fake-test-token"));
}

// ---------- R5 CliRunner (W-02a..d), on fakes of the #16 seams ----------
//
// The fakes stand in for the filesystem, the environment and the child process, so these run on
// every OS without a real CLI. They are `Send` because `CliRunner` is `Send + Sync`.

mod r5_fakes {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use cli_runner::{binary_name, Env, Fs, ProcOutput, Process};

    /// An absolute path on this OS: `/a/b` on unix, `C:\a\b` on Windows.
    pub fn abs(rel: &str) -> PathBuf {
        let mut p = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
        for part in rel.split('/').filter(|s| !s.is_empty()) {
            p.push(part);
        }
        p
    }

    pub fn cli_in(dir: &Path) -> PathBuf {
        dir.join(binary_name(cfg!(windows)))
    }

    /// Files with unix modes; every path is its own canonical path.
    #[derive(Clone, Default)]
    pub struct FakeFs(pub HashMap<PathBuf, u32>);

    impl Fs for FakeFs {
        fn is_file(&self, path: &Path) -> bool {
            self.0.contains_key(path)
        }
        fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
            Ok(path.to_path_buf())
        }
        fn unix_mode(&self, path: &Path) -> Option<u32> {
            if cfg!(windows) {
                return None;
            }
            self.0.get(path).copied().or(Some(0o755))
        }
    }

    #[derive(Clone)]
    pub struct FakeEnv {
        pub path: OsString,
        pub temp: PathBuf,
    }

    impl Env for FakeEnv {
        fn var(&self, name: &str) -> Option<OsString> {
            (name == "PATH").then(|| self.path.clone())
        }
        fn vars(&self) -> Vec<(OsString, OsString)> {
            vec![("PATH".into(), self.path.clone())]
        }
        fn temp_dir(&self) -> PathBuf {
            self.temp.clone()
        }
    }

    /// A stand-in `databricks` that prints `version`. Every call is recorded.
    #[derive(Clone)]
    pub struct FakeCli {
        pub version: String,
        pub calls: Arc<Mutex<Vec<Vec<OsString>>>>,
    }

    impl FakeCli {
        pub fn new(version: &str) -> Self {
            Self {
                version: version.into(),
                calls: Arc::default(),
            }
        }
        pub fn call_count(&self) -> usize {
            self.calls.lock().expect("calls").len()
        }
    }

    impl Process for FakeCli {
        fn run(
            &self,
            _program: &Path,
            args: &[OsString],
            _env: &[(OsString, OsString)],
        ) -> io::Result<ProcOutput> {
            self.calls.lock().expect("calls").push(args.to_vec());
            let stdout = match args.first().and_then(|a| a.to_str()) {
                Some("--version") => format!("Databricks CLI v{}\n", self.version),
                Some("version") => format!("{{\"Version\":\"{}\"}}", self.version),
                _ => String::new(),
            };
            Ok(ProcOutput {
                code: Some(0),
                stdout: stdout.into_bytes(),
                stderr: Vec::new(),
            })
        }
    }

    /// A machine with one CLI in `dir` (mode 0755) and `dir` first on PATH.
    pub fn machine(dir: &Path, version: &str) -> (FakeFs, FakeEnv, FakeCli) {
        let mut files = HashMap::new();
        files.insert(cli_in(dir), 0o755);
        files.insert(dir.to_path_buf(), 0o755);
        let env = FakeEnv {
            path: std::env::join_paths([dir]).expect("PATH"),
            temp: abs("var/fake-temp"),
        };
        (FakeFs(files), env, FakeCli::new(version))
    }
}

use mog_core::cli_runner::{CliPolicy, DatabricksCliRunner};
use r5_fakes::{abs, cli_in, machine};

#[test]
fn at_02_w_02a_w_02c_the_cli_resolves_to_an_absolute_path_with_version_1_17_or_later() {
    let dir = abs("opt/databricks/bin");
    let (fs, env, cli) = machine(&dir, "1.25.0");
    let r5 = DatabricksCliRunner::new(fs, env, cli, CliPolicy::default());
    let path = r5.resolve().expect("W-02a: resolve the CLI");
    assert!(path.is_absolute(), "W-02a: {path:?} is not absolute");
    assert_eq!(path, cli_in(&dir));
    let v = r5
        .run_json(&["version", "--output", "json"])
        .expect("run the CLI");
    assert_eq!(v["Version"], "1.25.0", "W-02c: {v}");

    // W-02c: 1.16 is refused with a stable, content-free code. The "Install the Databricks
    // CLI" step that the card shows for it is W-17c (TS side).
    let (fs, env, cli) = machine(&dir, "1.16.9");
    let old = DatabricksCliRunner::new(fs, env, cli, CliPolicy::default());
    let err = old.resolve().expect_err("W-02c: 1.16 is refused");
    assert_eq!(err.code(), "cli_unsupported", "W-02c: {err}");
    let err = old
        .run_json(&["version", "--output", "json"])
        .expect_err("W-02c: no command runs on an old CLI");
    assert_eq!(err.code(), "cli_unsupported");
}

#[test]
fn at_02_w_02b_a_cli_in_a_temporary_or_world_writable_folder_is_refused() {
    // A fake `databricks` in the temporary folder, first on PATH.
    let temp = abs("var/fake-temp/evil");
    let (fs, mut env, cli) = machine(&temp, "1.25.0");
    env.temp = abs("var/fake-temp");
    let r5 = DatabricksCliRunner::new(fs, env, cli.clone(), CliPolicy::default());
    let err = r5
        .resolve()
        .expect_err("W-02b: a CLI in the temp folder is refused");
    assert_eq!(err.code(), "cli_unsafe_path", "W-02b: {err}");
    assert!(
        !err.to_string().contains("evil"),
        "the error names no path: {err}"
    );
    assert_eq!(cli.call_count(), 0, "W-02b: no child process starts");

    if cfg!(unix) {
        let dir = abs("opt/shared/bin");
        let (mut fs, env, cli) = machine(&dir, "1.25.0");
        fs.0.insert(dir.clone(), 0o777);
        let r5 = DatabricksCliRunner::new(fs, env, cli.clone(), CliPolicy::default());
        let err = r5
            .resolve()
            .expect_err("W-02b: a world-writable folder is refused");
        assert_eq!(err.code(), "cli_unsafe_path", "W-02b: {err}");
        assert_eq!(cli.call_count(), 0, "W-02b: no child process starts");
    }
}

#[test]
fn at_02_w_02d_a_changed_cli_shows_a_warning_at_the_next_launch() {
    let dir = abs("opt/databricks/bin");
    let (fs, env, cli) = machine(&dir, "1.25.0");
    let launch_1 = DatabricksCliRunner::new(fs, env, cli, CliPolicy::default());
    let first = launch_1.launch_check(None).expect("launch 1");
    assert!(first.warnings.is_empty(), "{:?}", first.warnings);

    // The CLI is replaced by another version; launch 2 compares with the stored record.
    let (fs, env, cli) = machine(&dir, "1.26.1");
    let launch_2 = DatabricksCliRunner::new(fs, env, cli, CliPolicy::default());
    let second = launch_2
        .launch_check(Some(&first.record))
        .expect("launch 2");
    let codes: Vec<_> = second.warnings.iter().map(|w| w.code()).collect();
    assert_eq!(codes, ["CLI_VERSION_CHANGED"], "W-02d: {codes:?}");
}

#[test]
fn at_02_w_02a_w_02c_status_reports_the_verified_path_and_version_for_cli_status() {
    // The `{path, version, found}` part of the `cli_status` IPC reply (Settings, #16).
    let dir = abs("opt/databricks/bin");
    let (fs, env, cli) = machine(&dir, "1.25.0");
    let r5 = DatabricksCliRunner::new(fs, env, cli, CliPolicy::default());
    assert!(
        !r5.status().found,
        "nothing is verified before the launch check"
    );
    r5.launch_check(None).expect("launch");
    let st = r5.status();
    assert!(st.found);
    assert_eq!(st.path.as_deref(), Some(cli_in(&dir).as_path()));
    assert_eq!(st.version.map(|v| v.to_string()).as_deref(), Some("1.25.0"));
}

#[test]
fn arc_6_the_real_cli_runner_refuses_auth_token_before_any_child_starts() {
    let dir = abs("opt/databricks/bin");
    let (fs, env, cli) = machine(&dir, "1.25.0");
    let r5 = DatabricksCliRunner::new(fs, env, cli.clone(), CliPolicy::default());
    let err = r5
        .run_json(&["auth", "token", "--profile", "p"])
        .expect_err("ARC-6: refused");
    assert_eq!(err.code(), "token_command_refused");
    assert_eq!(cli.call_count(), 0, "no child process starts");
}

// ---------- R6 TokenBroker refresh and 401 (W-05a..d, #18) ----------

/// 2026-11-17T12:00:00Z in Unix milliseconds.
const T0_MS: i64 = 1_794_916_800_000;

/// A fake R5: counts `auth token` runs and answers a made-up token that expires at 13:00Z.
#[derive(Default)]
struct CountingCli {
    runs: AtomicUsize,
}

impl CliRunner for CountingCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/usr/local/bin/databricks"))
    }
    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value> {
        mog_core::cli_runner::refuse_token_command(args)?;
        Ok(serde_json::json!({}))
    }
    fn run_auth_token(&self, _profile: &str) -> CoreResult<SecretStdout> {
        let n = self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(SecretStdout::new(
            format!(
                r#"{{"access_token":"test-token-not-real-{n}","expiry":"2026-11-17T13:00:00Z"}}"#
            )
            .into_bytes(),
        ))
    }
}

struct OneHost;

impl ProfileHosts for OneHost {
    fn workspace_url(&self, _profile: &str) -> CoreResult<String> {
        Ok("https://h.cloud.databricks.com".to_owned())
    }
}

const HOST: &str = "h.cloud.databricks.com";

fn r6(clock: &SimClock) -> (CliTokenBroker, Arc<CountingCli>) {
    let cli = Arc::new(CountingCli::default());
    let b = CliTokenBroker::new(
        cli.clone(),
        Arc::new(OneHost),
        Arc::new(clock.clone()),
        Arc::new(SimWallClock::at(T0_MS)),
    );
    (b, cli)
}

#[test]
fn at_02_w_05a_n_mogs_on_one_profile_cause_one_cli_call_60_s_before_expiry() {
    let clock = SimClock::new();
    let (r6, cli) = r6(&clock);
    for _mog in 0..8 {
        r6.with_token("bottle-test", HOST, &mut |b| assert!(!b.is_empty()))
            .unwrap();
    }
    assert_eq!(cli.runs.load(Ordering::SeqCst), 1);
    clock.advance(Duration::from_secs(3_539));
    r6.with_token("bottle-test", HOST, &mut |_| ()).unwrap();
    assert_eq!(cli.runs.load(Ordering::SeqCst), 1, "none before 3,540 s");
    clock.advance(Duration::from_secs(1));
    assert_eq!(clock.now(), Duration::from_secs(3_540));
    for _mog in 0..8 {
        r6.with_token("bottle-test", HOST, &mut |_| ()).unwrap();
    }
    assert_eq!(
        cli.runs.load(Ordering::SeqCst),
        2,
        "one CLI call for 8 MOGs at 3,540 s"
    );
}

#[test]
fn at_02_w_05b_w_05c_a_401_drops_the_token_and_the_next_call_gets_a_new_one() {
    let clock = SimClock::new();
    let (r6, cli) = r6(&clock);
    let mut first = Vec::new();
    let lease = r6
        .with_token("bottle-test", HOST, &mut |b| first = b.to_vec())
        .unwrap();
    // The bridge sees HTTP 401 and tells R6 (R7 does this, W-05d is in http_transport_reqwest.rs).
    r6.invalidate("bottle-test", lease).unwrap();
    let mut second = Vec::new();
    r6.with_token("bottle-test", HOST, &mut |b| second = b.to_vec())
        .unwrap();
    assert_ne!(first, second, "W-05b: a new token");
    assert_eq!(cli.runs.load(Ordering::SeqCst), 2, "W-05c: one CLI run");
}

// ---------- Pending: sign-out (W-18a..c) ----------

#[test]
#[ignore = "pending TODO(#163): sign-out of bottle-* profiles is not built (R6 drop_all is built in #18)"]
fn at_02_w_18a_w_18c_after_sign_out_no_call_succeeds_and_no_token_is_left() {
    let r6 = StubTokenBroker;
    r6.with_token("bottle-test", HOST, &mut |_| ())
        .expect("TODO(#163): a token before sign-out");
    // TODO(#163): "Sign out of bottle-test" runs `databricks auth logout --profile bottle-test`
    // through CliRunner, and R6 wipes the profile's buffers (W-18c) through
    // `token_events::sign_out` (#465, tested in tests/token_events.rs).
    r6.invalidate("bottle-test", TokenLease::default())
        .expect("TODO(#163): sign-out calls token_events::sign_out");
    assert!(
        r6.with_token("bottle-test", HOST, &mut |_| ()).is_err(),
        "W-18a: a call after sign-out must fail"
    );
    todo!("TODO(#163): assert `auth logout` ran once for a bottle-* profile only, and a memory scan finds no token bytes");
}

// ---------- Pending, live: the AT-02 run on a real workspace (needs:human, needs:u2m) ----------

#[test]
#[ignore = "pending TODO(#42): live run needs R6 on the shipped R5 (DatabricksCliRunner, #16) and a person with a U2M profile (needs:u2m)"]
fn at_02_w_05c_w_16b_live_an_expired_u2m_profile_recovers_after_sign_in() {
    // TODO(#42): read workspace_id through R6 on DatabricksCliRunner (#16) for the fixture profile
    // (never a token). The stub check comes first, so a forced run of this pending test fails
    // and never reads as a skip.
    let r6 = StubTokenBroker;
    r6.invalidate("bottle-test", TokenLease::default())
        .expect("TODO(#42): R6 runs on DatabricksCliRunner");
    let w = mog_core::live_or_skip!();
    let wsid = r6
        .workspace_id(&w.profile)
        .expect("TODO(#42): workspace_id from x-databricks-org-id");
    assert!(!wsid.is_empty());
    todo!("TODO(#42): the person runs `databricks auth logout --profile <your-profile>`, asks, signs in from the chip; assert the paused poll resumes with the same conversation and response IDs");
}
