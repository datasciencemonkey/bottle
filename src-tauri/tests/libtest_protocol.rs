//! #382 NFR-23a: the `harness = false` AppKit test (`tests/macos_panel_appkit.rs`) follows the
//! libtest command-line protocol, so cargo-nextest can list it, run each test alone and write
//! each result to the JUnit report that the requirement coverage check reads.
//! These tests drive the shared protocol module with the argument lists that nextest and
//! `cargo test` pass.

#[allow(dead_code)] // `main` is used only by the custom harness.
#[path = "support/libtest_protocol.rs"]
mod libtest_protocol;

use libtest_protocol::{list, parse, run, Args, Test};

fn args(a: &[&str]) -> Args {
    parse(a.iter().map(|s| s.to_string())).expect("valid arguments")
}

fn pass() -> Result<(), String> {
    Ok(())
}

fn fail() -> Result<(), String> {
    Err("detail".into())
}

fn boom() -> Result<(), String> {
    panic!("boom")
}

const TESTS: &[Test] = &[
    Test {
        name: "mog_02_a",
        run: pass,
    },
    Test {
        name: "mog_02_ab",
        run: pass,
    },
    Test {
        name: "mog_03a_c",
        run: fail,
    },
];

fn listed(a: &[&str]) -> String {
    let mut out = Vec::new();
    list(TESTS, &args(a), &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

fn ran(tests: &[Test], a: &[&str]) -> (bool, String) {
    let mut out = Vec::new();
    let ok = run(tests, &args(a), &mut out).unwrap();
    (ok, String::from_utf8(out).unwrap())
}

#[test]
fn nfr_23a_list_terse_prints_name_colon_test_for_each_test() {
    let a = args(&["--list", "--format", "terse"]);
    assert!(a.list);
    assert_eq!(
        listed(&["--list", "--format", "terse"]),
        "mog_02_a: test\nmog_02_ab: test\nmog_03a_c: test\n"
    );
    assert_eq!(
        listed(&["--list", "--format=terse"]),
        "mog_02_a: test\nmog_02_ab: test\nmog_03a_c: test\n"
    );
}

#[test]
fn nfr_23a_list_ignored_prints_nothing_because_no_test_is_ignored() {
    assert_eq!(listed(&["--list", "--format", "terse", "--ignored"]), "");
    assert_eq!(
        listed(&["--list", "--format", "terse", "--include-ignored"]),
        "mog_02_a: test\nmog_02_ab: test\nmog_03a_c: test\n"
    );
}

#[test]
fn nfr_23a_list_honours_filters() {
    assert_eq!(
        listed(&["--list", "--format", "terse", "mog_02"]),
        "mog_02_a: test\nmog_02_ab: test\n"
    );
    assert_eq!(
        listed(&["--list", "--format", "terse", "--exact", "mog_02_a"]),
        "mog_02_a: test\n"
    );
}

#[test]
fn nfr_23a_exact_runs_only_the_named_test() {
    let (ok, out) = ran(TESTS, &["--exact", "mog_02_a", "--nocapture"]);
    assert!(ok, "{out}");
    assert!(out.contains("test mog_02_a ... ok"), "{out}");
    assert!(!out.contains("mog_02_ab"), "{out}");
    assert!(!out.contains("mog_03a_c"), "{out}");
    assert!(out.contains("1 passed; 0 failed"), "{out}");
}

#[test]
fn nfr_23a_a_failing_test_fails_the_run_with_its_detail() {
    let (ok, out) = ran(TESTS, &["--exact", "mog_03a_c"]);
    assert!(!ok, "{out}");
    assert!(out.contains("test mog_03a_c ... FAILED"), "{out}");
    assert!(out.contains("detail"), "{out}");
}

#[test]
fn nfr_23a_no_arguments_runs_every_test_as_cargo_test_does() {
    let (ok, out) = ran(TESTS, &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("test mog_02_a ... ok"), "{out}");
    assert!(out.contains("test mog_02_ab ... ok"), "{out}");
    assert!(out.contains("test mog_03a_c ... FAILED"), "{out}");
    assert!(out.contains("2 passed; 1 failed"), "{out}");
}

#[test]
fn nfr_23a_ignored_runs_nothing_and_passes() {
    let (ok, out) = ran(TESTS, &["--ignored"]);
    assert!(ok, "{out}");
    assert!(out.contains("0 passed; 0 failed"), "{out}");
    assert!(out.contains("3 filtered out"), "{out}");
}

#[test]
fn nfr_23a_skip_and_libtest_flags_with_values_are_understood() {
    let a = args(&[
        "--test-threads",
        "1",
        "--color",
        "never",
        "-q",
        "--skip",
        "mog_03a",
        "mog_0",
    ]);
    assert_eq!(a.filters, vec!["mog_0".to_string()]);
    let (ok, out) = ran(
        TESTS,
        &["--test-threads=1", "--skip", "mog_03a", "-q", "mog_0"],
    );
    assert!(ok, "{out}");
    assert!(out.contains("2 passed; 0 failed"), "{out}");
}

#[test]
fn nfr_23a_a_panicking_test_fails_and_the_next_test_still_runs() {
    let tests = [
        Test {
            name: "mog_02_panics",
            run: boom,
        },
        Test {
            name: "mog_02_after",
            run: pass,
        },
    ];
    let (ok, out) = ran(&tests, &[]);
    assert!(!ok, "{out}");
    assert!(out.contains("test mog_02_panics ... FAILED"), "{out}");
    assert!(out.contains("test mog_02_after ... ok"), "{out}");
}

#[test]
fn nfr_23a_an_unknown_flag_value_is_an_error() {
    assert!(parse(["--format".to_string()]).is_err());
    assert!(parse(["--format".to_string(), "pretty-ish".to_string()]).is_err());
}
