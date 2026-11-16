//! AT-02 "Recover expired sign-in" (#233), section 15. Gate: live.
//!
//! The AT script: expire a U2M CLI profile, start sign-in from the chip and complete it. Repeat
//! with a PAT and a service-principal profile, an old CLI and plaintext token storage. Pass: the
//! MOG config is unchanged, paused polls resume with the same IDs, PAT and SP profiles are
//! rejected, and the old CLI and plaintext storage show blocking messages.
//!
//! The Rust side of AT-02 is R5 CliRunner (#16, PR #255 is open) and R6 TokenBroker (#18). Only
//! the token buffer of R6 ships, so W-18c's zero overwrite runs here. The other tests are written
//! against `mog_core::cli_runner::CliRunner` and `mog_core::token_broker::TokenBroker` and stay
//! `#[ignore]` with the feature issue, so they never count as a pass. The live parts read the
//! #229 fixture (`BOTTLE_TEST_PROFILE`) and need a person: docs/tests/AT-SIGNIN-WORKSPACE-HOP.md.
//! The TS side (ProfileService, the chip, held work) is `src/acceptance/at-02-signin-recovery.test.ts`.
//!
//! No test here reads, prints or stores a token. Test tokens are made-up bytes.

use std::sync::Arc;
use std::time::Duration;

use mog_core::cli_runner::{CliRunner, StubCliRunner};
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::token_broker::{SecretToken, StubTokenBroker, TokenBroker};

/// A made-up token shape. It is not a real token and it never leaves the test.
fn fake_token() -> SecretToken {
    SecretToken::new("fake-test-token-0123456789abcdef".to_owned())
}

// ---------- Runs today: the R6 token buffer (W-18c) ----------

#[test]
fn at_02_w_18c_sign_out_overwrites_the_token_buffer_with_zeros() {
    let mut t = fake_token();
    let len = t.len();
    assert!(len > 0 && !t.is_zeroed());
    // Sign-out calls wipe (Drop calls it too). Every byte is zero, and the length stays, so no
    // reallocation left a copy behind.
    t.wipe();
    assert!(t.is_zeroed());
    assert_eq!(t.len(), len);
    // The buffer never shows in Debug output, before or after the wipe.
    let shown = format!("{t:?}");
    assert_eq!(shown, "SecretToken(<redacted>)");
    assert!(!shown.contains("fake-test-token"));
}

// ---------- Pending: R5 CliRunner (W-02a..d) ----------

#[test]
#[ignore = "pending TODO(#16): CliRunner resolve and vet are not built (PR #255 is open)"]
fn at_02_w_02a_w_02c_the_cli_resolves_to_an_absolute_path_with_version_1_17_or_later() {
    let r5 = StubCliRunner;
    let path = r5.resolve().expect("TODO(#16): resolve the CLI");
    assert!(path.is_absolute(), "W-02a: {path:?} is not absolute");
    let v = r5
        .run_json(&["version", "--output", "json"])
        .expect("TODO(#16): run the CLI");
    assert!(v.get("Version").is_some(), "W-02c: no version in {v}");
    todo!("TODO(#16): assert the version is 1.17 or later and that 1.16 is refused with the 'Install the Databricks CLI' step (W-17c)");
}

#[test]
#[ignore = "pending TODO(#16): CliRunner path checks are not built (PR #255 is open)"]
fn at_02_w_02b_a_cli_in_a_temporary_or_world_writable_folder_is_refused() {
    // TODO(#16): put a fake `databricks` in a temporary folder first on PATH, then resolve.
    let r5 = StubCliRunner;
    let err = r5
        .resolve()
        .expect_err("TODO(#16): a CLI in the temporary folder is refused");
    assert_ne!(err.code(), "not_implemented", "W-02b: {err}");
    todo!("TODO(#16): assert the refusal names the folder rule and no child process starts");
}

#[test]
#[ignore = "pending TODO(#16): the launch re-check of the CLI path and version is not built"]
fn at_02_w_02d_a_changed_cli_shows_a_warning_at_the_next_launch() {
    let r5 = StubCliRunner;
    let first = r5.resolve().expect("TODO(#16): resolve at launch 1");
    // TODO(#16): replace the CLI with a different version, then start a new CliRunner (launch 2).
    let second = StubCliRunner
        .resolve()
        .expect("TODO(#16): resolve at launch 2");
    assert_eq!(first, second);
    todo!("TODO(#16): assert launch 2 reports the changed path or version as a warning");
}

// ---------- Pending: R6 TokenBroker refresh and 401 (W-05a..d) ----------

#[test]
#[ignore = "pending TODO(#18): TokenBroker refresh at expiry - 60 s and single-flight are not built"]
fn at_02_w_05a_n_mogs_on_one_profile_cause_one_cli_call_60_s_before_expiry() {
    let clock = SimClock::new();
    let r6: Arc<dyn TokenBroker> = Arc::new(StubTokenBroker);
    // TODO(#18): a TokenBroker with `clock` and a counting CLI double; the token expires at 3,600 s.
    for _mog in 0..8 {
        r6.with_token("bottle-test", &mut |b| assert!(!b.is_empty()))
            .expect("TODO(#18): a token for each MOG");
    }
    clock.advance(Duration::from_secs(3_600 - 60));
    assert_eq!(clock.now(), Duration::from_secs(3_540));
    todo!("TODO(#18): assert exactly one CLI call for 8 MOGs at 3,540 s and none before it");
}

#[test]
#[ignore = "pending TODO(#18): drop on 401 and the one retry are not built"]
fn at_02_w_05b_w_05c_w_05d_a_401_drops_the_token_and_one_retry_runs_with_a_new_one() {
    let r6 = StubTokenBroker;
    r6.with_token("bottle-test", &mut |_| ())
        .expect("TODO(#18): first token");
    // The bridge sees HTTP 401 and tells R6.
    r6.invalidate("bottle-test")
        .expect("TODO(#18): drop the token on 401");
    r6.with_token("bottle-test", &mut |_| ())
        .expect("TODO(#18): a new token from one CLI run");
    todo!("TODO(#18): assert the trace shows one CLI run, one retry, and that a second 401 gives 'Sign in to <host>' (W-05d)");
}

// ---------- Pending: sign-out (W-18a..c) ----------

#[test]
#[ignore = "pending TODO(#163): sign-out of bottle-* profiles is not built (the token wipe is TODO(#18))"]
fn at_02_w_18a_w_18c_after_sign_out_no_call_succeeds_and_no_token_is_left() {
    let r6 = StubTokenBroker;
    r6.with_token("bottle-test", &mut |_| ())
        .expect("TODO(#18): a token before sign-out");
    // TODO(#163): "Sign out of bottle-test" runs `databricks auth logout --profile bottle-test`
    // through CliRunner, and R6 wipes the profile's buffers (W-18c).
    r6.invalidate("bottle-test").expect("TODO(#18): wipe");
    assert!(
        r6.with_token("bottle-test", &mut |_| ()).is_err(),
        "W-18a: a call after sign-out must fail"
    );
    todo!("TODO(#163): assert `auth logout` ran once for a bottle-* profile only, and a memory scan finds no token bytes");
}

// ---------- Pending, live: the AT-02 run on a real workspace (needs:human, needs:u2m) ----------

#[test]
#[ignore = "pending TODO(#18): live run needs TokenBroker and a person with a U2M profile (needs:u2m)"]
fn at_02_w_05c_w_16b_live_an_expired_u2m_profile_recovers_after_sign_in() {
    // TODO(#18): read workspace_id through R6 for the fixture profile (never a token). The stub
    // check comes first, so a forced run of this pending test fails and never reads as a skip.
    let r6 = StubTokenBroker;
    r6.invalidate("bottle-test")
        .expect("TODO(#18): TokenBroker is built");
    let w = mog_core::live_or_skip!();
    let wsid = r6
        .workspace_id(&w.profile)
        .expect("TODO(#18): workspace_id from x-databricks-org-id");
    assert!(!wsid.is_empty());
    todo!("TODO(#42): the person runs `databricks auth logout --profile <your-profile>`, asks, signs in from the chip; assert the paused poll resumes with the same conversation and response IDs");
}
