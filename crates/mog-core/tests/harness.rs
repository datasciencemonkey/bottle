//! Self-tests of the test harness (#229, AT-22): the monotonic clock, the SessionGuard clock seam
//! and the gate fixture skip helpers.

use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mog_core::clock::{MonotonicClock, SimClock, SystemClock};
use mog_core::session_guard::{SessionGuard, StubSessionGuard};
use mog_core::testkit::{self, Fixture, ReferenceDevice};

fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |k| map.get(k).cloned()
}

#[test]
fn at_22_sim_clock_moves_only_on_advance() {
    let c = SimClock::starting_at(Duration::from_secs(1));
    assert_eq!(c.now(), Duration::from_secs(1));
    assert_eq!(c.now(), Duration::from_secs(1));
    c.advance(Duration::from_secs(55 * 60));
    assert_eq!(c.now(), Duration::from_secs(1 + 55 * 60));
    assert_eq!(SimClock::new().now(), Duration::ZERO);
}

#[test]
fn at_22_sim_clock_clones_share_one_time() {
    let a = SimClock::new();
    let b = a.clone();
    b.advance(Duration::from_millis(250));
    assert_eq!(a.now(), Duration::from_millis(250));
}

#[test]
fn at_22_sim_clock_is_thread_safe() {
    let c = SimClock::new();
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let c = c.clone();
            thread::spawn(move || {
                for _ in 0..1_000 {
                    c.advance(Duration::from_millis(1));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread");
    }
    assert_eq!(c.now(), Duration::from_millis(8_000));
}

#[test]
fn at_22_sim_clock_saturates_instead_of_overflowing() {
    let c = SimClock::starting_at(Duration::MAX);
    c.advance(Duration::from_secs(1));
    assert_eq!(c.now(), Duration::MAX);
}

#[test]
fn at_22_system_clock_never_goes_back() {
    let c = SystemClock::new();
    let a = c.now();
    let b = c.now();
    assert!(b >= a);
}

#[test]
fn at_22_session_guard_takes_an_injected_clock() {
    let sim = SimClock::new();
    let guard = StubSessionGuard::new(Arc::new(sim.clone()));
    assert_eq!(guard.clock().now(), Duration::ZERO);
    sim.advance(Duration::from_secs(240 * 60));
    assert_eq!(guard.clock().now(), Duration::from_secs(240 * 60));
    // The seam does not change the R3 stub behaviour.
    assert!(guard.arm("s1").is_err());
}

#[test]
fn at_22_session_guard_default_uses_the_system_clock() {
    let guard = StubSessionGuard::default();
    let a = guard.clock().now();
    assert!(guard.clock().now() >= a);
}

#[test]
fn at_22_live_fixture_skips_with_a_clear_reason_when_unset() {
    match testkit::live_from(env(&[])).expect("no error") {
        Fixture::Skip(reason) => {
            assert!(reason.contains("BOTTLE_TEST_PROFILE is not set"), "{reason}");
            assert!(reason.contains("docs/tests/README.md"), "{reason}");
        }
        Fixture::Ready(_) => panic!("expected a skip"),
    }
    assert!(matches!(
        testkit::live_from(env(&[("BOTTLE_TEST_PROFILE", " ")])).expect("no error"),
        Fixture::Skip(_)
    ));
}

#[test]
fn at_22_live_fixture_reads_profile_and_warehouse() {
    let f = testkit::live_from(env(&[
        ("BOTTLE_TEST_PROFILE", "bottle-test"),
        ("BOTTLE_TEST_WAREHOUSE_ID", "abc123"),
    ]))
    .expect("no error");
    match f {
        Fixture::Ready(w) => {
            assert_eq!(w.profile, "bottle-test");
            assert_eq!(w.warehouse_id.as_deref(), Some("abc123"));
        }
        Fixture::Skip(r) => panic!("unexpected skip: {r}"),
    }
}

#[test]
fn at_22_malformed_profile_is_an_error_not_a_skip() {
    let err = testkit::live_from(env(&[("BOTTLE_TEST_PROFILE", "a b; rm -rf")])).unwrap_err();
    assert!(err.to_string().contains("BOTTLE_TEST_PROFILE"));
}

#[test]
fn at_22_device_fixture_accepts_only_reference_devices() {
    assert!(matches!(
        testkit::device_from(env(&[])).expect("no error"),
        Fixture::Skip(_)
    ));
    match testkit::device_from(env(&[("BOTTLE_TEST_DEVICE", "win11-i5-16gb")])).expect("no error") {
        Fixture::Ready(d) => assert_eq!(d, ReferenceDevice::Win11I5_16Gb),
        Fixture::Skip(r) => panic!("unexpected skip: {r}"),
    }
    assert!(testkit::device_from(env(&[("BOTTLE_TEST_DEVICE", "raspberry")])).is_err());
}

#[test]
fn at_22_hid_rig_fixture_needs_a_port() {
    let skip = testkit::hid_rig_from(env(&[("BOTTLE_TEST_DEVICE", "mac-m1-current")])).expect("ok");
    match skip {
        Fixture::Skip(r) => assert!(r.contains("BOTTLE_TEST_HID_PORT"), "{r}"),
        Fixture::Ready(_) => panic!("expected a skip"),
    }
    let ready = testkit::hid_rig_from(env(&[
        ("BOTTLE_TEST_DEVICE", "mac-m1-current"),
        ("BOTTLE_TEST_HID_PORT", "/dev/cu.usbmodem1"),
    ]))
    .expect("ok");
    match ready {
        Fixture::Ready(rig) => {
            assert_eq!(rig.device, ReferenceDevice::MacM1Current);
            assert_eq!(rig.port, "/dev/cu.usbmodem1");
        }
        Fixture::Skip(r) => panic!("unexpected skip: {r}"),
    }
}

#[test]
fn at_22_live_or_skip_macro_returns_early_without_a_profile() {
    use std::cell::Cell;
    fn body(lookup: impl Fn(&str) -> Option<String>, ran: &Cell<bool>) {
        let w = mog_core::live_or_skip!(lookup);
        assert_eq!(w.profile, "bottle-test");
        ran.set(true);
    }
    let ran = Cell::new(false);
    body(env(&[]), &ran);
    assert!(!ran.get());
    body(env(&[("BOTTLE_TEST_PROFILE", "bottle-test")]), &ran);
    assert!(ran.get());
}
