//! Real-process tests with a tiny fake `databricks` (src/bin/fake-databricks.rs).
//!
//! The fake is copied into CARGO_TARGET_TMPDIR (under target/, not a temp
//! root), so it passes the W-02b location check without any override.

mod common;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cli_runner::{
    is_allowed, CliError, CliPolicy, CliRecord, CliRunner, CliWarning, RealFs, RealProcess,
    UnsafeReason, Version,
};
use common::{cli_in, FakeEnv};

const FAKE: &str = env!("CARGO_BIN_EXE_fake-databricks");

/// Real-process tests run one at a time. Otherwise one test can fork while
/// another still holds a write fd on its freshly copied binary, and exec fails
/// with ETXTBSY ("executable file busy") on Linux.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
}

/// Copies the fake CLI into `parent/<name>/` and writes its version file.
fn install_fake(parent: &Path, name: &str, version: &str) -> PathBuf {
    let dir = parent.join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let cli = cli_in(&dir);
    std::fs::copy(FAKE, &cli).unwrap();
    std::fs::write(dir.join("fake-version.txt"), version).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

fn safe_parent() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
}

fn env_for(dir: &Path) -> FakeEnv {
    FakeEnv::new()
        .with_path(&[dir.to_path_buf()])
        .temp(std::env::temp_dir())
        .set("HOME", dir.as_os_str())
        .set("HTTPS_PROXY", "http://proxy.example.invalid:3128")
        // Fake values only: they must never reach the child.
        .set("DATABRICKS_TOKEN", "test-token-not-real")
        .set("DATABRICKS_HOST", "https://fake.example.invalid")
        .set("DATABRICKS_CLIENT_ID", "fake-client-id")
        .set("DATABRICKS_CLIENT_SECRET", "fake-client-secret")
        .set("DATABRICKS_AUTH_TYPE", "pat")
}

fn runner(dir: &Path) -> CliRunner<RealFs, FakeEnv, RealProcess> {
    CliRunner::new(RealFs, env_for(dir), RealProcess, CliPolicy::default())
}

/// [W-02a] [W-02c] a real CLI on PATH resolves to its canonical absolute path and version
#[test]
fn w_02a_w_02c_real_cli_resolves_and_reports_version() {
    let _serial = serial();
    let dir = install_fake(&safe_parent(), "ok", "1.25.0");
    let mut r = runner(&dir);
    let report = r.launch_check(None).unwrap();
    assert!(report.record.path.is_absolute());
    let expected = std::fs::canonicalize(cli_in(&dir)).unwrap();
    assert_eq!(
        std::fs::canonicalize(&report.record.path).unwrap(),
        expected
    );
    assert_eq!(report.record.version, Version::new(1, 25, 0));
    assert!(report.warnings.is_empty());
}

/// [W-02c] a real CLI older than 1.17 is blocked
#[test]
fn w_02c_real_old_cli_is_blocked() {
    let _serial = serial();
    let dir = install_fake(&safe_parent(), "old", "1.16.0");
    let err = runner(&dir).launch_check(None).unwrap_err();
    assert!(matches!(err, CliError::Unsupported { .. }));
}

/// [W-02b] a real CLI in the OS temp dir is refused and never run
#[test]
fn w_02b_real_cli_in_temp_dir_is_refused() {
    let _serial = serial();
    let dir = install_fake(&std::env::temp_dir(), "bottle-cli-runner-test", "1.25.0");
    let err = runner(&dir).launch_check(None).unwrap_err();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        matches!(
            err,
            CliError::UnsafeLocation {
                reason: UnsafeReason::TempDir { .. },
                ..
            }
        ),
        "{err:?}"
    );
}

/// [W-02b] a real CLI in a world-writable folder is refused (unix)
#[cfg(unix)]
#[test]
fn w_02b_real_world_writable_dir_is_refused() {
    let _serial = serial();
    use std::os::unix::fs::PermissionsExt;
    let dir = install_fake(&safe_parent(), "ww", "1.25.0");
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
    let err = runner(&dir).launch_check(None).unwrap_err();
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(
            err,
            CliError::UnsafeLocation {
                reason: UnsafeReason::WorldWritable { .. },
                ..
            }
        ),
        "{err:?}"
    );
}

/// [W-02d] a real CLI that was upgraded shows a version-changed warning at the next launch
#[test]
fn w_02d_real_changed_cli_warns() {
    let _serial = serial();
    let dir = install_fake(&safe_parent(), "changed", "1.25.0");
    let first = runner(&dir).launch_check(None).unwrap().record;
    std::fs::write(dir.join("fake-version.txt"), "1.26.0").unwrap();
    let report = runner(&dir).launch_check(Some(&first)).unwrap();
    assert_eq!(
        report.warnings,
        vec![CliWarning::VersionChanged {
            previous: Version::new(1, 25, 0),
            current: Version::new(1, 26, 0)
        }]
    );
    let moved = CliRecord {
        path: dir.join("elsewhere"),
        ..report.record.clone()
    };
    let report = runner(&dir).launch_check(Some(&moved)).unwrap();
    assert!(matches!(report.warnings[0], CliWarning::PathChanged { .. }));
}

/// [W-09] the real child process sees only PATH, HOME, config, proxy and CA variables
#[test]
fn w_09_real_child_env_is_clean() {
    let _serial = serial();
    let dir = install_fake(&safe_parent(), "env", "1.25.0");
    let mut r = runner(&dir);
    r.launch_check(None).unwrap();
    let out = r.run([OsString::from("env-names")]).unwrap();
    assert!(out.success(), "{:?}", String::from_utf8_lossy(&out.stderr));
    let names: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    assert!(names.iter().any(|n| n.eq_ignore_ascii_case("PATH")));
    assert!(names.iter().any(|n| n == "HTTPS_PROXY"));
    for n in &names {
        assert!(is_allowed(n, cfg!(windows)), "unexpected child var {n}");
        assert!(!n.to_ascii_uppercase().starts_with("DATABRICKS_TOKEN"));
        assert!(!n.to_ascii_uppercase().starts_with("DATABRICKS_HOST"));
        assert!(!n.to_ascii_uppercase().starts_with("DATABRICKS_CLIENT_"));
        assert!(!n.to_ascii_uppercase().starts_with("DATABRICKS_AUTH_TYPE"));
    }
}

/// [W-02d] run re-checks the verified CLI: a CLI removed after the launch check is CLI_MISSING
#[test]
fn w_02d_real_run_rechecks_removed_cli() {
    let _serial = serial();
    let dir = install_fake(&safe_parent(), "removed", "1.25.0");
    let mut r = runner(&dir);
    r.launch_check(None).unwrap();
    std::fs::remove_file(cli_in(&dir)).unwrap();
    assert_eq!(r.run(["env-names"]).unwrap_err(), CliError::Missing);
}
