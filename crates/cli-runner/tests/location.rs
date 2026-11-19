mod common;

use cli_runner::{check_location, path_within, temp_roots, CliError, UnsafeReason};
use common::{abs, cli_in, FakeEnv, FakeFs};

fn unsafe_reason(r: Result<(), CliError>) -> UnsafeReason {
    match r {
        Err(CliError::UnsafeLocation { reason, .. }) => reason,
        other => panic!("expected UnsafeLocation, got {other:?}"),
    }
}

/// [W-02b] a CLI in /tmp is refused
#[cfg(unix)]
#[test]
fn w_02b_refuses_cli_in_tmp() {
    let cli = std::path::PathBuf::from("/tmp/x/databricks");
    let fs = FakeFs::new()
        .dir(&std::path::PathBuf::from("/tmp/x"), 0o755)
        .file(&cli, 0o755);
    let env = FakeEnv::new();
    let reason = unsafe_reason(check_location(&fs, &env, &cli, &cli));
    assert_eq!(
        reason,
        UnsafeReason::TempDir {
            root: "/tmp".into()
        }
    );
    for root in ["/var/tmp", "/private/tmp"] {
        let cli = std::path::Path::new(root).join("databricks");
        let fs = FakeFs::new().file(&cli, 0o755);
        assert!(matches!(
            unsafe_reason(check_location(&fs, &env, &cli, &cli)),
            UnsafeReason::TempDir { .. }
        ));
    }
}

/// [W-02b] a CLI in the OS temp dir or in TMPDIR/TEMP/TMP is refused
#[test]
fn w_02b_refuses_cli_in_os_and_env_temp_dirs() {
    let os_temp = abs("os/temp");
    let env = FakeEnv::new()
        .temp(os_temp.clone())
        .set("TMPDIR", abs("custom/tmpdir"))
        .set("TEMP", abs("users/me/AppData/Local/Temp"))
        .set("TMP", abs("users/me/tmp"));
    for dir in [
        os_temp.join("sub"),
        abs("custom/tmpdir"),
        abs("users/me/AppData/Local/Temp/x"),
        abs("users/me/tmp"),
    ] {
        let cli = cli_in(&dir);
        let fs = FakeFs::new().file(&cli, 0o755);
        assert!(
            matches!(
                unsafe_reason(check_location(&fs, &env, &cli, &cli)),
                UnsafeReason::TempDir { .. }
            ),
            "{} not refused",
            cli.display()
        );
    }
}

/// [W-02b] the fixed temp roots are on the list on every OS; empty or relative env values are ignored
#[test]
fn w_02b_temp_roots_list() {
    let env = FakeEnv::new()
        .set("TMPDIR", "")
        .set("TEMP", "relative/temp");
    let roots = temp_roots(&FakeFs::new(), &env);
    for fixed in ["/tmp", "/var/tmp", "/private/tmp"] {
        assert!(
            roots.iter().any(|r| r == std::path::Path::new(fixed)),
            "{fixed} missing"
        );
    }
    assert!(roots.contains(&env.temp));
    assert!(roots.iter().all(|r| !r.as_os_str().is_empty()));
    assert!(!roots
        .iter()
        .any(|r| r == std::path::Path::new("relative/temp")));
}

/// [W-02b] a symlink that points into a temp dir is refused (canonical path is checked)
#[test]
fn w_02b_refuses_symlink_into_temp() {
    let found = cli_in(&abs("usr/local/bin"));
    let target = cli_in(&abs("fake-os-temp/evil"));
    let fs = FakeFs::new().file(&target, 0o755).link(&found, &target);
    let env = FakeEnv::new();
    assert!(matches!(
        unsafe_reason(check_location(&fs, &env, &found, &target)),
        UnsafeReason::TempDir { .. }
    ));
}

/// [W-02b] a world-writable binary or folder is refused (unix mode bits)
#[test]
fn w_02b_refuses_world_writable() {
    let env = FakeEnv::new();
    let dir = abs("opt/shared/bin");
    let cli = cli_in(&dir);

    let fs = FakeFs::new().dir(&dir, 0o777).file(&cli, 0o755);
    assert_eq!(
        unsafe_reason(check_location(&fs, &env, &cli, &cli)),
        UnsafeReason::WorldWritable { path: dir.clone() }
    );

    let fs = FakeFs::new().dir(&dir, 0o1777).file(&cli, 0o755);
    assert!(matches!(
        unsafe_reason(check_location(&fs, &env, &cli, &cli)),
        UnsafeReason::WorldWritable { .. }
    ));

    let fs = FakeFs::new().dir(&dir, 0o755).file(&cli, 0o757);
    assert_eq!(
        unsafe_reason(check_location(&fs, &env, &cli, &cli)),
        UnsafeReason::WorldWritable { path: cli.clone() }
    );
}

/// [W-02b] a CLI in a normal, non-writable folder is accepted; group-writable is not world-writable
#[test]
fn w_02b_accepts_safe_location() {
    let env = FakeEnv::new();
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new()
        .dir(&abs("usr/local/bin"), 0o775)
        .file(&cli, 0o755);
    assert_eq!(check_location(&fs, &env, &cli, &cli), Ok(()));
    // No mode bits (as on Windows): only the temp-dir rule applies.
    let fs = FakeFs::new();
    assert_eq!(check_location(&fs, &env, &cli, &cli), Ok(()));
}

/// [W-02b] path_within compares whole components, never string prefixes
#[test]
fn w_02b_path_within_components() {
    assert!(path_within(&abs("tmp/a/b"), &abs("tmp")));
    assert!(path_within(&abs("tmp"), &abs("tmp")));
    assert!(!path_within(&abs("tmpfoo/a"), &abs("tmp")));
    assert!(!path_within(&abs("usr/bin"), &abs("tmp")));
    assert!(!path_within(&abs("usr/bin"), std::path::Path::new("")));
}

/// [W-02b] Windows branch: temp checks are case-insensitive and C:\Windows\Temp is refused
#[cfg(windows)]
#[test]
fn w_02b_windows_temp_rules() {
    use std::path::PathBuf;
    assert!(path_within(
        &PathBuf::from("C:\\USERS\\ME\\APPDATA\\LOCAL\\TEMP\\x\\databricks.exe"),
        &PathBuf::from("c:\\Users\\me\\AppData\\Local\\Temp")
    ));
    let cli = PathBuf::from("C:\\Windows\\Temp\\databricks.exe");
    let fs = FakeFs::new().file(&cli, 0o755);
    assert!(matches!(
        unsafe_reason(check_location(&fs, &FakeEnv::new(), &cli, &cli)),
        UnsafeReason::TempDir { .. }
    ));
}
