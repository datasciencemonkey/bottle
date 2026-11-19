mod common;

use std::ffi::OsString;

use cli_runner::{CliError, CliPolicy, CliRecord, CliRunner, CliWarning, UnsafeReason, Version};
use common::{abs, cli_in, FakeEnv, FakeFs, FakeProcess};

fn setup(version_output: &str) -> (CliRunner<FakeFs, FakeEnv, FakeProcess>, FakeProcess) {
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new()
        .with_path(&[abs("usr/local/bin")])
        .set("HOME", abs("home/someone"))
        .set("DATABRICKS_TOKEN", "test-token-not-real")
        .set("DATABRICKS_HOST", "https://fake.example.invalid")
        .set("DATABRICKS_CLIENT_SECRET", "fake")
        .set("DATABRICKS_AUTH_TYPE", "pat");
    let proc = FakeProcess::new(version_output);
    (
        CliRunner::new(fs, env, proc.clone(), CliPolicy::default()),
        proc,
    )
}

fn env_names(env: &[(OsString, OsString)]) -> Vec<String> {
    env.iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect()
}

/// [W-02a] [W-02c] launch check records the absolute path and version for Settings
#[test]
fn w_02a_w_02c_launch_check_records_path_and_version() {
    let (mut runner, proc) = setup("Databricks CLI v1.25.0\n");
    let report = runner.launch_check(None).unwrap();
    let cli = cli_in(&abs("usr/local/bin"));
    assert_eq!(
        report.record,
        CliRecord {
            path: cli.clone(),
            version: Version::new(1, 25, 0)
        }
    );
    assert!(report.warnings.is_empty());
    let calls = proc.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, cli);
    assert_eq!(calls[0].args, vec![OsString::from("--version")]);

    let status = runner.status();
    assert!(status.found);
    assert_eq!(status.path, Some(cli));
    assert_eq!(status.version, Some(Version::new(1, 25, 0)));
}

/// [W-02c] a CLI older than 1.17 is blocked with CLI_UNSUPPORTED
#[test]
fn w_02c_blocks_old_cli() {
    let (mut runner, _) = setup("Databricks CLI v1.16.9");
    let err = runner.launch_check(None).unwrap_err();
    assert_eq!(
        err,
        CliError::Unsupported {
            found: Version::new(1, 16, 9),
            min: Version::new(1, 17, 0)
        }
    );
    assert_eq!(err.code(), "CLI_UNSUPPORTED");
    assert_eq!(err.backend_code(), "CLI_MISSING");
    assert!(!runner.status().found);
    assert!(runner.verified().is_none());
}

/// [W-02c] MDM CliMinVersion raises the minimum
#[test]
fn w_02c_policy_min_version() {
    let proc = FakeProcess::new("Databricks CLI v1.20.0");
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/local/bin")]);
    let policy = CliPolicy {
        cli_path: None,
        min_version: Version::new(1, 22, 0),
    };
    let mut runner = CliRunner::new(fs, env, proc, policy);
    assert!(matches!(
        runner.launch_check(None),
        Err(CliError::Unsupported { .. })
    ));
}

/// [W-02c] 1.17 to 1.24 is accepted with a "1.25+ recommended" warning
#[test]
fn w_02c_recommends_1_25() {
    let (mut runner, _) = setup("Databricks CLI v1.17.0");
    let report = runner.launch_check(None).unwrap();
    assert_eq!(
        report.warnings,
        vec![CliWarning::BelowRecommended {
            found: Version::new(1, 17, 0),
            recommended: Version::new(1, 25, 0)
        }]
    );
    assert_eq!(report.warnings[0].code(), "CLI_BELOW_RECOMMENDED");
}

/// [W-02c] unreadable version output fails closed
#[test]
fn w_02c_unreadable_version() {
    let (mut runner, _) = setup("something else");
    let err = runner.launch_check(None).unwrap_err();
    assert_eq!(err, CliError::VersionUnreadable);
    assert_eq!(err.code(), "CLI_VERSION_UNREADABLE");
}

/// [W-02d] a changed version shows a warning at launch
#[test]
fn w_02d_warns_on_version_change() {
    let (mut runner, proc) = setup("Databricks CLI v1.25.0");
    let first = runner.launch_check(None).unwrap().record;
    proc.set_version_output("Databricks CLI v1.26.1");
    let report = runner.launch_check(Some(&first)).unwrap();
    assert_eq!(
        report.warnings,
        vec![CliWarning::VersionChanged {
            previous: Version::new(1, 25, 0),
            current: Version::new(1, 26, 1)
        }]
    );
    assert_eq!(report.warnings[0].code(), "CLI_VERSION_CHANGED");
    assert_eq!(
        proc.calls.borrow().len(),
        2,
        "version is read again at each launch"
    );
}

/// [W-02d] a changed path shows a warning at launch; an unchanged CLI shows none
#[test]
fn w_02d_warns_on_path_change() {
    let (mut runner, _) = setup("Databricks CLI v1.25.0");
    let previous = CliRecord {
        path: cli_in(&abs("old/place")),
        version: Version::new(1, 25, 0),
    };
    let report = runner.launch_check(Some(&previous)).unwrap();
    assert_eq!(
        report.warnings,
        vec![CliWarning::PathChanged {
            previous: previous.path.clone(),
            current: cli_in(&abs("usr/local/bin"))
        }]
    );
    assert_eq!(report.warnings[0].code(), "CLI_PATH_CHANGED");
    let same = report.record.clone();
    assert!(runner
        .launch_check(Some(&same))
        .unwrap()
        .warnings
        .is_empty());
}

/// [W-02b] [W-02d] launch check refuses a CLI that moved into a temp dir
#[test]
fn w_02b_w_02d_launch_check_refuses_temp() {
    let cli = cli_in(&abs("fake-os-temp/dl"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("fake-os-temp/dl")]);
    let proc = FakeProcess::new("Databricks CLI v1.25.0");
    let mut runner = CliRunner::new(fs, env, proc.clone(), CliPolicy::default());
    let err = runner.launch_check(None).unwrap_err();
    assert!(matches!(
        err,
        CliError::UnsafeLocation {
            reason: UnsafeReason::TempDir { .. },
            ..
        }
    ));
    assert_eq!(err.code(), "CLI_UNSAFE_PATH");
    assert!(proc.calls.borrow().is_empty(), "an unsafe CLI is never run");
}

/// [W-02a] no CLI: launch check fails with CLI_MISSING and status shows not found
#[test]
fn w_02a_launch_check_missing() {
    let mut runner = CliRunner::new(
        FakeFs::new(),
        FakeEnv::new().with_path(&[abs("usr/bin")]),
        FakeProcess::new(""),
        CliPolicy::default(),
    );
    assert_eq!(runner.launch_check(None), Err(CliError::Missing));
    assert_eq!(
        runner.status(),
        cli_runner::CliStatus {
            found: false,
            path: None,
            version: None
        }
    );
}

/// [W-09] the `--version` child and every run get only the clean env
#[test]
fn w_09_every_child_gets_clean_env() {
    let (mut runner, proc) = setup("Databricks CLI v1.25.0");
    runner.launch_check(None).unwrap();
    let out = runner
        .run(["auth", "profiles", "--output", "json"])
        .unwrap();
    assert!(out.success());
    let calls = proc.calls.borrow();
    assert_eq!(calls.len(), 2);
    for call in calls.iter() {
        assert_eq!(env_names(&call.env), vec!["PATH", "HOME"]);
        assert!(!format!("{:?}", call.env).contains("test-token-not-real"));
    }
    assert_eq!(calls[1].args[0], OsString::from("auth"));
    assert_eq!(env_names(&runner.child_env()), vec!["PATH", "HOME"]);
}

/// [W-02d] run refuses before a launch check
#[test]
fn w_02d_run_requires_verified_cli() {
    let (runner, proc) = setup("Databricks CLI v1.25.0");
    let err = runner.run(["version"]).unwrap_err();
    assert_eq!(err, CliError::NotVerified);
    assert_eq!(err.code(), "CLI_NOT_VERIFIED");
    assert!(proc.calls.borrow().is_empty());
}

/// [W-02a] a spawn failure is a typed error
#[test]
fn w_02a_spawn_failure() {
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/local/bin")]);
    let mut proc = FakeProcess::new("");
    proc.fail_spawn = true;
    let mut runner = CliRunner::new(fs, env, proc, CliPolicy::default());
    let err = runner.launch_check(None).unwrap_err();
    assert!(matches!(err, CliError::Spawn { .. }));
    assert_eq!(err.code(), "CLI_SPAWN_FAILED");
}
