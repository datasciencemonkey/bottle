mod common;

use cli_runner::{binary_name, resolve, CliError, CliPolicy};
use common::{abs, cli_in, FakeEnv, FakeFs};

/// [W-02a] resolves `databricks` on PATH to an absolute path
#[test]
fn w_02a_resolves_from_path() {
    let cli = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/bin"), abs("usr/local/bin")]);
    let r = resolve(&fs, &env, &CliPolicy::default()).unwrap();
    assert_eq!(r.found, cli);
    assert_eq!(r.canonical, cli);
    assert!(r.canonical.is_absolute());
}

/// [W-02a] the first PATH match wins and symlinks resolve to the canonical target
#[test]
fn w_02a_first_match_and_symlink() {
    let first = cli_in(&abs("opt/homebrew/bin"));
    let target = cli_in(&abs("opt/homebrew/Cellar/databricks/1.25.0/bin"));
    let second = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new()
        .file(&target, 0o755)
        .file(&second, 0o755)
        .link(&first, &target);
    let env = FakeEnv::new().with_path(&[abs("opt/homebrew/bin"), abs("usr/local/bin")]);
    let r = resolve(&fs, &env, &CliPolicy::default()).unwrap();
    assert_eq!(r.found, first);
    assert_eq!(r.canonical, target);
}

/// [W-02a] relative and empty PATH entries are skipped, never resolved against the cwd
#[test]
fn w_02a_skips_relative_path_entries() {
    let rel = std::path::PathBuf::from("bin");
    let fs = FakeFs::new().file(&rel.join(binary_name(cfg!(windows))), 0o755);
    let env = FakeEnv::new().with_path(&[rel.clone(), std::path::PathBuf::from(".")]);
    assert_eq!(
        resolve(&fs, &env, &CliPolicy::default()),
        Err(CliError::Missing)
    );
}

/// [W-02a] no CLI on PATH (or no PATH) gives CLI_MISSING
#[test]
fn w_02a_missing_cli() {
    let fs = FakeFs::new();
    let env = FakeEnv::new().with_path(&[abs("usr/bin")]);
    let err = resolve(&fs, &env, &CliPolicy::default()).unwrap_err();
    assert_eq!(err, CliError::Missing);
    assert_eq!(err.code(), "CLI_MISSING");
    assert_eq!(
        resolve(&fs, &FakeEnv::new(), &CliPolicy::default()),
        Err(CliError::Missing)
    );
}

/// [W-02a] a file without any execute bit is skipped on PATH (unix mode bits)
#[test]
fn w_02a_skips_non_executable() {
    let a = cli_in(&abs("a"));
    let b = cli_in(&abs("b"));
    let fs = FakeFs::new().file(&a, 0o644).file(&b, 0o755);
    let env = FakeEnv::new().with_path(&[abs("a"), abs("b")]);
    assert_eq!(resolve(&fs, &env, &CliPolicy::default()).unwrap().found, b);
}

/// [W-02a] the MDM CliPath wins over PATH, must be absolute and must exist
#[test]
fn w_02a_honours_policy_cli_path() {
    let managed = cli_in(&abs("opt/managed"));
    let on_path = cli_in(&abs("usr/local/bin"));
    let fs = FakeFs::new().file(&managed, 0o755).file(&on_path, 0o755);
    let env = FakeEnv::new().with_path(&[abs("usr/local/bin")]);

    let policy = CliPolicy {
        cli_path: Some(managed.clone()),
        ..CliPolicy::default()
    };
    assert_eq!(resolve(&fs, &env, &policy).unwrap().canonical, managed);

    let policy = CliPolicy {
        cli_path: Some("databricks".into()),
        ..CliPolicy::default()
    };
    assert_eq!(
        resolve(&fs, &env, &policy),
        Err(CliError::NotAbsolute {
            path: "databricks".into()
        })
    );

    let policy = CliPolicy {
        cli_path: Some(cli_in(&abs("nowhere"))),
        ..CliPolicy::default()
    };
    assert_eq!(resolve(&fs, &env, &policy), Err(CliError::Missing));
}
