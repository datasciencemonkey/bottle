//! R5 `run_auth_token` on the shipped `DatabricksCliRunner` (#16, W-04a, W-09, W-02d, ARC-6).
//!
//! Fake seams only: no real CLI runs and no real token exists. Every token-shaped value here is
//! plainly fake (`test-token-not-real`).

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cli_runner::{binary_name, Env, Fs, ProcOutput, Process};
use mog_core::cli_runner::{
    auth_token_args, CliPolicy, CliRunner, DatabricksCliRunner, CLI_TIMEOUT,
};
use mog_core::error::CoreError;

const FAKE: &str = "test-token-not-real";

fn abs(rel: &str) -> PathBuf {
    let mut p = PathBuf::from(if cfg!(windows) { "C:\\" } else { "/" });
    for part in rel.split('/').filter(|s| !s.is_empty()) {
        p.push(part);
    }
    p
}

fn bin_dir() -> PathBuf {
    abs("usr/local/bin")
}

fn cli() -> PathBuf {
    bin_dir().join(binary_name(cfg!(windows)))
}

#[derive(Clone)]
struct OneCliFs {
    temp_cli: bool,
}

impl Fs for OneCliFs {
    fn is_file(&self, path: &Path) -> bool {
        path == cli() || (self.temp_cli && path == temp().join(binary_name(cfg!(windows))))
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(path.to_path_buf())
    }
    fn unix_mode(&self, _path: &Path) -> Option<u32> {
        if cfg!(windows) {
            None
        } else {
            Some(0o755)
        }
    }
}

fn temp() -> PathBuf {
    abs("var/fake-temp")
}

/// PATH plus the variables W-09 removes, with fake values.
#[derive(Clone)]
struct DirtyEnv {
    path: OsString,
}

impl Env for DirtyEnv {
    fn var(&self, name: &str) -> Option<OsString> {
        self.vars()
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }
    fn vars(&self) -> Vec<(OsString, OsString)> {
        vec![
            ("PATH".into(), self.path.clone()),
            ("HOME".into(), abs("home/someone").into()),
            ("DATABRICKS_TOKEN".into(), FAKE.into()),
            (
                "DATABRICKS_HOST".into(),
                "https://fake.example.invalid".into(),
            ),
            (
                "DATABRICKS_CLIENT_SECRET".into(),
                "fake-client-secret".into(),
            ),
            ("DATABRICKS_AUTH_TYPE".into(), "pat".into()),
        ]
    }
    fn temp_dir(&self) -> PathBuf {
        temp()
    }
}

#[derive(Debug, Clone)]
struct Call {
    args: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    timeout: Option<Duration>,
}

/// A stand-in `databricks`: `--version` prints 1.25.0, anything else gives `reply`.
#[derive(Clone)]
struct ScriptedCli {
    reply: Arc<dyn Fn() -> io::Result<ProcOutput> + Send + Sync>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl ScriptedCli {
    fn new(reply: impl Fn() -> io::Result<ProcOutput> + Send + Sync + 'static) -> Self {
        Self {
            reply: Arc::new(reply),
            calls: Arc::default(),
        }
    }
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }
}

impl Process for ScriptedCli {
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
        env: &[(OsString, OsString)],
        timeout: Option<Duration>,
    ) -> io::Result<ProcOutput> {
        self.calls.lock().unwrap().push(Call {
            args: args.to_vec(),
            env: env.to_vec(),
            timeout,
        });
        if args.first().is_some_and(|a| a == "--version") {
            return Ok(ProcOutput {
                code: Some(0),
                stdout: b"Databricks CLI v1.25.0\n".to_vec(),
                stderr: Vec::new(),
            });
        }
        (self.reply)()
    }
}

fn token_json() -> Vec<u8> {
    format!(
        "{{\"access_token\":\"{FAKE}\",\"token_type\":\"Bearer\",\"expiry\":\"2099-01-01T00:00:00Z\"}}"
    )
    .into_bytes()
}

fn ok_token() -> io::Result<ProcOutput> {
    Ok(ProcOutput {
        code: Some(0),
        stdout: token_json(),
        stderr: format!("debug: {FAKE}").into_bytes(),
    })
}

fn r5(cli: ScriptedCli) -> DatabricksCliRunner<OneCliFs, DirtyEnv, ScriptedCli> {
    let env = DirtyEnv {
        path: std::env::join_paths([bin_dir()]).unwrap(),
    };
    DatabricksCliRunner::new(OneCliFs { temp_cli: false }, env, cli, CliPolicy::default())
}

fn args_of(call: &Call) -> Vec<String> {
    call.args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect()
}

/// [W-04a] [W-02d] `run_auth_token` checks the CLI first, then runs exactly `auth_token_args`
#[test]
fn w_04a_w_02d_run_auth_token_runs_exactly_the_token_command_after_the_launch_check() {
    let cli = ScriptedCli::new(ok_token);
    let r5 = r5(cli.clone());
    let out = r5.run_auth_token("caller-profile").unwrap();
    assert_eq!(out.as_bytes(), token_json().as_slice());
    let calls = cli.calls();
    assert_eq!(calls.len(), 2, "the launch check, then the token command");
    assert_eq!(args_of(&calls[0]), ["--version"]);
    assert_eq!(args_of(&calls[1]), auth_token_args("caller-profile"));
    // A second call reuses the verified CLI: no second version check.
    drop(r5.run_auth_token("caller-profile").unwrap());
    assert_eq!(cli.calls().len(), 3);
}

/// [W-09] [W-04a] the token child gets the clean env: no DATABRICKS_TOKEN, HOST, CLIENT_* or AUTH_TYPE
#[test]
fn w_09_w_04a_the_token_child_gets_the_clean_env() {
    let cli = ScriptedCli::new(ok_token);
    drop(r5(cli.clone()).run_auth_token("caller-profile").unwrap());
    let token_call = cli.calls().pop().unwrap();
    let names: Vec<String> = token_call
        .env
        .iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert!(names.contains(&"PATH".to_string()));
    for gone in [
        "DATABRICKS_TOKEN",
        "DATABRICKS_HOST",
        "DATABRICKS_CLIENT_SECRET",
        "DATABRICKS_AUTH_TYPE",
    ] {
        assert!(
            !names.contains(&gone.to_string()),
            "{gone} reached the child"
        );
    }
    assert!(!format!("{:?}", token_call.env).contains(FAKE));
}

/// [W-04a] the token child has the R5 timeout (30 s, as the reference bridge)
#[test]
fn w_04a_the_token_child_has_the_r5_timeout() {
    assert_eq!(CLI_TIMEOUT, Duration::from_secs(30));
    let cli = ScriptedCli::new(ok_token);
    drop(r5(cli.clone()).run_auth_token("caller-profile").unwrap());
    assert!(cli.calls().iter().all(|c| c.timeout == Some(CLI_TIMEOUT)));
}

fn assert_content_free(err: &CoreError, reason: &str) {
    assert!(
        matches!(err, CoreError::Unavailable { component: "R5", reason: r } if *r == reason),
        "{err:?}"
    );
    for text in [format!("{err:?}"), err.to_string()] {
        assert!(!text.contains(FAKE), "a token reached the error: {text}");
        assert!(
            !text.contains("caller-profile"),
            "the profile reached the error: {text}"
        );
        assert!(!text.contains("debug"), "stderr reached the error: {text}");
    }
}

/// [W-04a] a failed `auth token` exit is a content-free `cli_failed`: no stdout, stderr or profile
#[test]
fn w_04a_a_failed_token_exit_is_a_content_free_error() {
    let cli = ScriptedCli::new(|| {
        Ok(ProcOutput {
            code: Some(1),
            stdout: token_json(),
            stderr: format!("debug: refresh failed for {FAKE}").into_bytes(),
        })
    });
    let err = r5(cli).run_auth_token("caller-profile").unwrap_err();
    assert_content_free(&err, "cli_failed");
}

/// [W-04a] a hung `auth token` child is a content-free `cli_timed_out`
#[test]
fn w_04a_a_timed_out_token_child_is_a_content_free_error() {
    let cli = ScriptedCli::new(|| Err(io::Error::new(io::ErrorKind::TimedOut, FAKE)));
    let err = r5(cli).run_auth_token("caller-profile").unwrap_err();
    assert_content_free(&err, "cli_timed_out");
}

/// [W-02b] [W-04a] a CLI in a temporary folder is refused before any token child starts
#[test]
fn w_02b_w_04a_a_cli_in_a_temp_folder_gets_no_token_child() {
    let cli = ScriptedCli::new(ok_token);
    let env = DirtyEnv {
        path: std::env::join_paths([temp()]).unwrap(),
    };
    let r5 = DatabricksCliRunner::new(
        OneCliFs { temp_cli: true },
        env,
        cli.clone(),
        CliPolicy::default(),
    );
    let err = r5.run_auth_token("caller-profile").unwrap_err();
    assert_content_free(&err, "cli_unsafe_path");
    assert!(
        cli.calls().is_empty(),
        "no child may start: {:?}",
        cli.calls().len()
    );
}

/// [W-04a] [ARC-6] the returned stdout is redacted in Debug and wiped on drop
#[test]
fn w_04a_arc_6_the_token_stdout_is_redacted_and_wipes() {
    let cli = ScriptedCli::new(ok_token);
    let mut out = r5(cli).run_auth_token("caller-profile").unwrap();
    assert_eq!(format!("{out:?}"), "SecretStdout(<redacted>)");
    out.wipe();
    assert!(out.is_zeroed());
}

/// [W-04a] two token children run at the same time: one slow child does not hold up the others
#[test]
fn w_04a_two_token_children_run_at_the_same_time() {
    use std::sync::Condvar;
    let inside = Arc::new((Mutex::new(0usize), Condvar::new()));
    let peak = Arc::new(Mutex::new(0usize));
    let (i2, p2) = (inside.clone(), peak.clone());
    let cli = ScriptedCli::new(move || {
        let (count, cv) = &*i2;
        let mut n = count.lock().unwrap();
        *n += 1;
        let mut top = p2.lock().unwrap();
        *top = (*top).max(*n);
        drop(top);
        cv.notify_all();
        // Wait (at most 2 s) until the other child is inside too.
        let (mut n, _) = cv
            .wait_timeout_while(n, Duration::from_secs(2), |n| *n < 2)
            .unwrap();
        *n -= 1;
        drop(n);
        ok_token()
    });
    let r5 = Arc::new(r5(cli));
    // The launch check runs first, so both threads find a verified CLI.
    r5.launch_check(None).unwrap();
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let r5 = r5.clone();
            std::thread::spawn(move || drop(r5.run_auth_token("caller-profile").unwrap()))
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    assert_eq!(
        *peak.lock().unwrap(),
        2,
        "the second child waited for the first"
    );
}
