//! AT-34 "Poll budget and 429" (#234), section 15: the Rust part. Pass: calls stay at 40/min or
//! less; polls keep priority; AIMD halves and recovers; Retry-After is honoured.
//!
//! The shared per-profile bucket lives in R7 HttpTransport (G-09a..G-09d) and the 429 rules in
//! R7 and R8 (G-10a, G-10b). Both are stubs today: the bucket is #138, the R7 client is #364 and
//! R8 is #30. So every test here is `#[ignore = "pending TODO(#n)"]`. Each one runs on the #229 sim
//! clock (`mog_core::clock::SimClock`) with a scripted server, never on wall time. The poll
//! delay floor (G-08) and the in-flight limits (G-11a..c) are TS and stay pending in
//! `src/acceptance/at-34-poll-budget.test.ts`. The admin QPM limit (ADM-07) and the live 429
//! are a person's steps: docs/tests/AT-GENIE-MCP.md, part AT-34.

use std::time::Duration;

use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::http_transport::{HttpTransport, StubHttpTransport};
use mog_core::ipc::JsonObject;
use mog_core::types::{TagApp, TagOp, Tags};

/// The default bucket of G-09a.
const LIMIT_PER_MIN: u32 = 40;

fn bucket_exists() {
    // The stub answers NotImplemented; #138 replaces it with the bucket under test.
    let t = StubHttpTransport;
    let tags = Tags {
        app: TagApp::Bottle,
        mog: "00000000-0000-4000-8000-0000000000e4".into(),
        op: TagOp::Genie,
    };
    assert!(
        t.request("bottle-test", "scim.me", &JsonObject::new(), None, &tags)
            .is_ok(),
        "R7 is still a stub"
    );
}

#[test]
#[ignore = "pending TODO(#138): the per-profile rate bucket is not built"]
fn at_34_g_09a_one_profile_stays_at_40_calls_per_minute() {
    bucket_exists();
    let clock = SimClock::new();
    clock.advance(Duration::from_secs(60));
    assert_eq!(clock.now(), Duration::from_secs(60));
    // When #138 lands: offer 100 calls in one sim minute on one profile; at most LIMIT_PER_MIN
    // leave. Two profiles have two buckets.
    let _ = LIMIT_PER_MIN;
    todo!("#138: count the calls that leave in each 60 s window")
}

#[test]
#[ignore = "pending TODO(#138): the per-profile rate bucket is not built"]
fn at_34_g_09b_polls_then_cancels_then_asks() {
    bucket_exists();
    // When #138 lands: with the bucket full, queue an ask, a cancel and a poll in that order.
    // They leave as poll, cancel, ask. The queued ask reports "Genie busy · queued".
    todo!("#138: assert the leave order and the queued state")
}

#[test]
#[ignore = "pending TODO(#138): AIMD is not built"]
fn at_34_g_09c_g_09d_a_429_halves_the_rate_then_it_recovers() {
    bucket_exists();
    // When #138 lands: after one 429 the rate is 20 calls/min for 5 min (G-09c), then it adds
    // 5 calls/min each minute: 25, 30, 35, 40, so 40 again 4 min later (G-09d).
    todo!("#138: drive a 429 and read the rate each sim minute")
}

#[test]
#[ignore = "pending TODO(#138): Retry-After handling is not built"]
fn at_34_g_10a_retry_after_seconds_date_and_clamp() {
    bucket_exists();
    // When #138 lands: `Retry-After: 7` makes the next call wait 7 s; an HTTP date waits until
    // that time; 0 waits 1 s and 900 waits 300 s (clamp 1-300 s); an FMAPI body with
    // error.retry_after works the same.
    todo!("#138: assert the wait before the next call for each form")
}

#[test]
#[ignore = "pending TODO(#138): the G-BACKOFF schedule is not built"]
fn at_34_g_10b_a_429_without_retry_after_backs_off_an_ask_only() {
    bucket_exists();
    // When #138 lands: an ask that gets 429 with no retry header tries again with full jitter,
    // start 2 s, factor 2, maximum 60 s, 5 tries or less (Table G-BACKOFF). A poll that gets
    // the same 429 is not sent again early: only its interval gets longer.
    todo!("#138: record the ask retry delays and the poll interval")
}

#[test]
#[ignore = "pending TODO(#138): live AT-34 needs the bucket and a person with a U2M profile and an admin QPM limit (needs:u2m, needs:admin)"]
fn at_34_adm_07_live_four_asks_under_an_admin_qpm_limit() {
    let w = mog_core::live_or_skip!();
    // Steps in docs/tests/AT-GENIE-MCP.md, part AT-34.
    let _ = w.profile;
    todo!("#138: run 4 in-flight asks on one profile and count calls per minute")
}
