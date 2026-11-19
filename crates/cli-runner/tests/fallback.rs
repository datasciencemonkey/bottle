//! The known install folders after PATH (W-02a, W-02b). A macOS app that starts from Finder,
//! the Dock or at login gets the launchd PATH `/usr/bin:/bin:/usr/sbin:/sbin`, which has no
//! Homebrew or install-script folder. Fake seams only.

mod common;

use cli_runner::{fallback_dirs, resolve, CliError, CliPolicy, CliRunner, UnsafeReason};
use common::{abs, cli_in, FakeEnv, FakeFs, FakeProcess};

fn launchd_env() -> FakeEnv {
    FakeEnv::new()
        .with_path(&[abs("usr/bin"), abs("bin"), abs("usr/sbin"), abs("sbin")])
        .set("HOME", abs("Users/someone"))
}

/// [W-02a] the known folders follow the reference list: Homebrew, /usr/local/bin, ~/.local/bin, ~/bin
#[cfg(not(windows))]
#[test]
fn w_02a_fallback_folders_are_homebrew_usr_local_and_the_home_bins() {
    assert_eq!(
        fallback_dirs(&launchd_env()),
        vec![
            abs("opt/homebrew/bin"),
            abs("usr/local/bin"),
            abs("Users/someone/.local/bin"),
            abs("Users/someone/bin"),
        ]
    );
    // A relative or missing HOME adds no home folder.
    let rel = FakeEnv::new().set("HOME", "someone");
    assert_eq!(
        fallback_dirs(&rel),
        vec![abs("opt/homebrew/bin"), abs("usr/local/bin")]
    );
}

/// [W-02a] Windows has no fallback folder: a Windows app gets the user PATH
#[cfg(windows)]
#[test]
fn w_02a_windows_has_no_fallback_folder() {
    assert!(fallback_dirs(&launchd_env()).is_empty());
}

/// [W-02a] with the launchd PATH, a Homebrew CLI in /opt/homebrew/bin is found
#[cfg(not(windows))]
#[test]
fn w_02a_a_launchd_path_finds_the_homebrew_cli() {
    let cli = cli_in(&abs("opt/homebrew/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let r = resolve(&fs, &launchd_env(), &CliPolicy::default()).unwrap();
    assert_eq!(r.found, cli);
}

/// [W-02a] an install-script CLI in ~/.local/bin is found, also with no PATH at all
#[cfg(not(windows))]
#[test]
fn w_02a_a_home_local_bin_cli_is_found_without_path() {
    let cli = cli_in(&abs("Users/someone/.local/bin"));
    let fs = FakeFs::new().file(&cli, 0o755);
    let env = FakeEnv::new().set("HOME", abs("Users/someone"));
    assert_eq!(
        resolve(&fs, &env, &CliPolicy::default()).unwrap().found,
        cli
    );
}

/// [W-02a] a CLI on PATH wins over one in a fallback folder
#[cfg(not(windows))]
#[test]
fn w_02a_a_path_cli_wins_over_a_fallback_cli() {
    let on_path = cli_in(&abs("custom/bin"));
    let fs = FakeFs::new()
        .file(&on_path, 0o755)
        .file(&cli_in(&abs("opt/homebrew/bin")), 0o755);
    let env = FakeEnv::new()
        .with_path(&[abs("usr/bin"), abs("custom/bin")])
        .set("HOME", abs("Users/someone"));
    assert_eq!(
        resolve(&fs, &env, &CliPolicy::default()).unwrap().found,
        on_path
    );
}

/// [W-02a] the MDM CliPath wins: no fallback folder is searched when it is set
#[cfg(not(windows))]
#[test]
fn w_02a_mdm_cli_path_wins_over_the_fallback_folders() {
    let fs = FakeFs::new().file(&cli_in(&abs("opt/homebrew/bin")), 0o755);
    let policy = CliPolicy {
        cli_path: Some(cli_in(&abs("opt/managed/bin"))),
        ..CliPolicy::default()
    };
    assert_eq!(
        resolve(&fs, &launchd_env(), &policy).unwrap_err(),
        CliError::Missing
    );
}

/// [W-02b] a CLI in a world-writable fallback folder is refused, never run
#[cfg(unix)]
#[test]
fn w_02b_a_world_writable_fallback_folder_is_refused() {
    let dir = abs("opt/homebrew/bin");
    let fs = FakeFs::new().dir(&dir, 0o777).file(&cli_in(&dir), 0o755);
    let proc = FakeProcess::new("Databricks CLI v1.25.0\n");
    let mut r = CliRunner::new(fs, launchd_env(), proc.clone(), CliPolicy::default());
    let err = r.launch_check(None).unwrap_err();
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
    assert!(proc.calls.borrow().is_empty(), "the refused CLI never ran");
}
