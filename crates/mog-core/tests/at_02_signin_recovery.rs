//! AT-02 "Recover expired sign-in" (#233), section 15. Gate: live.
//!
//! The AT script: expire a U2M CLI profile, start sign-in from the chip and complete it. Repeat
//! with a PAT and a service-principal profile, an old CLI and plaintext token storage. Pass: the
//! MOG config is unchanged, paused polls resume with the same IDs, PAT and SP profiles are
//! rejected, and the old CLI and plaintext storage show blocking messages.
//!
//! The Rust side of AT-02 is R5 CliRunner (#16, PR #255 is open) and R6 TokenBroker (#18). Only
//! the token buffer of R6 ships, so W-18c's zero overwrite runs here. R6 refresh and 401 (W-05a..c)
//! run here against the real `CliTokenBroker` with a fake CLI (#18). The other tests are written
//! against `mog_core::cli_runner::CliRunner` and `mog_core::token_broker::TokenBroker` and stay
//! `#[ignore]` with the feature issue, so they never count as a pass. The live parts read the
//! #229 fixture (`BOTTLE_TEST_PROFILE`) and need a person: docs/tests/AT-SIGNIN-WORKSPACE-HOP.md.
//! The TS side (ProfileService, the chip, held work) is `src/acceptance/at-02-signin-recovery.test.ts`.
//!
//! No test here reads, prints or stores a token. Test tokens are made-up bytes.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use mog_core::cli_runner::{CliRunner, SecretStdout, StubCliRunner};
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
    // TODO(#16): replace the CLI with a different version, then start a new CliRunner (launch 2)
    // and read its launch report.
    assert!(first.is_absolute());
    StubCliRunner
        .resolve()
        .expect("TODO(#16): resolve at launch 2");
    todo!("TODO(#16): assert launch 2 reports the changed path or version as a warning");
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
    // through CliRunner, and R6 wipes the profile's buffers (W-18c).
    r6.invalidate("bottle-test", TokenLease::default())
        .expect("TODO(#163): sign-out calls CliTokenBroker::drop_all");
    assert!(
        r6.with_token("bottle-test", HOST, &mut |_| ()).is_err(),
        "W-18a: a call after sign-out must fail"
    );
    todo!("TODO(#163): assert `auth logout` ran once for a bottle-* profile only, and a memory scan finds no token bytes");
}

// ---------- Pending, live: the AT-02 run on a real workspace (needs:human, needs:u2m) ----------

#[test]
#[ignore = "pending TODO(#16): live run needs the real CliRunner auth token run and a person with a U2M profile (needs:u2m)"]
fn at_02_w_05c_w_16b_live_an_expired_u2m_profile_recovers_after_sign_in() {
    // TODO(#16): read workspace_id through R6 on the real CliRunner for the fixture profile
    // (never a token). The stub check comes first, so a forced run of this pending test fails
    // and never reads as a skip.
    let r6 = StubTokenBroker;
    r6.invalidate("bottle-test", TokenLease::default())
        .expect("TODO(#16): R6 runs on the real CliRunner");
    let w = mog_core::live_or_skip!();
    let wsid = r6
        .workspace_id(&w.profile)
        .expect("TODO(#16): workspace_id from x-databricks-org-id");
    assert!(!wsid.is_empty());
    todo!("TODO(#42): the person runs `databricks auth logout --profile <your-profile>`, asks, signs in from the chip; assert the paused poll resumes with the same conversation and response IDs");
}
