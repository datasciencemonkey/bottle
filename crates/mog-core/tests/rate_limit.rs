//! The shared per-profile rate bucket (#138): G-09b priority by call class, the G-BACKOFF ask
//! schedule (G-10b), the `Retry-After` hold of a 503 and the busy card text (G-09b, ARC-F3).
//! Every test runs on the #229 sim clock, never on wall time.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::http_transport::backoff::{
    ask_backoff, backoff_cap, Jitter, OsJitter, BACKOFF_MAX, BACKOFF_START, MAX_ASK_TRIES,
};
use mog_core::http_transport::rate_bucket::{AimdBucket, CallClass, RateBucket};
use mog_core::http_transport::rate_limit::{acquire, BusyState};
use mog_core::http_transport::Pause;

const P: &str = "bottle-test";

fn full_bucket() -> (SimClock, AimdBucket) {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    for _ in 0..40 {
        assert_eq!(b.reserve(P), Duration::ZERO);
    }
    (clock, b)
}

#[test]
fn g_09b_call_class_of_each_genie_tool_and_route() {
    assert_eq!(CallClass::of_tool("genie_poll_response"), CallClass::Poll);
    assert_eq!(
        CallClass::of_tool("genie_get_query_result"),
        CallClass::Poll
    );
    assert_eq!(
        CallClass::of_tool("genie_cancel_response"),
        CallClass::Cancel
    );
    assert_eq!(CallClass::of_tool("genie_ask"), CallClass::Ask);
    assert_eq!(CallClass::of_tool("anything_else"), CallClass::Ask);
    assert_eq!(CallClass::of_route("genie.get_message"), CallClass::Poll);
    assert_eq!(CallClass::of_route("genie.query_result"), CallClass::Poll);
    assert_eq!(CallClass::of_route("genie.message"), CallClass::Ask);
    assert_eq!(CallClass::of_route("scim.me"), CallClass::Ask);
    assert!(CallClass::Poll < CallClass::Cancel && CallClass::Cancel < CallClass::Ask);
}

#[test]
fn g_09b_a_full_bucket_lets_queued_calls_go_as_poll_then_cancel_then_ask() {
    let (clock, b) = full_bucket();
    // Queued in the worst order: the ask first, the poll last.
    let ask = b.enqueue(P, CallClass::Ask);
    let cancel = b.enqueue(P, CallClass::Cancel);
    let poll = b.enqueue(P, CallClass::Poll);
    let mut waiting = vec![("ask", ask), ("cancel", cancel), ("poll", poll)];
    let mut order = Vec::new();
    while !waiting.is_empty() {
        let mut next_wait = Duration::MAX;
        let mut granted = None;
        // Every waiter asks in queue order, so the ask always asks first.
        for (i, (name, t)) in waiting.iter().enumerate() {
            let w = b.take(P, t);
            if w.is_zero() {
                granted = Some((i, *name));
                break;
            }
            next_wait = next_wait.min(w);
        }
        match granted {
            Some((i, name)) => {
                order.push(name);
                waiting.remove(i);
            }
            None => clock.advance(next_wait),
        }
    }
    assert_eq!(order, ["poll", "cancel", "ask"]);
}

#[test]
fn g_09b_same_class_calls_go_in_fifo_order() {
    let (clock, b) = full_bucket();
    let first = b.enqueue(P, CallClass::Ask);
    let second = b.enqueue(P, CallClass::Ask);
    clock.advance(Duration::from_secs(60));
    assert!(
        b.take(P, &second) > Duration::ZERO,
        "the older ask goes first"
    );
    assert_eq!(b.take(P, &first), Duration::ZERO);
    assert_eq!(b.take(P, &second), Duration::ZERO);
}

#[test]
fn g_09b_an_unqueued_reserve_does_not_jump_a_queued_poll() {
    let (clock, b) = full_bucket();
    let poll = b.enqueue(P, CallClass::Poll);
    clock.advance(Duration::from_secs(60));
    assert!(b.reserve(P) > Duration::ZERO);
    assert_eq!(b.take(P, &poll), Duration::ZERO);
    assert_eq!(b.reserve(P), Duration::ZERO);
}

#[test]
fn g_09b_a_left_ticket_no_longer_blocks_lower_classes() {
    let (clock, b) = full_bucket();
    let poll = b.enqueue(P, CallClass::Poll);
    let ask = b.enqueue(P, CallClass::Ask);
    clock.advance(Duration::from_secs(60));
    assert!(b.take(P, &ask) > Duration::ZERO);
    b.leave(P, poll);
    assert_eq!(b.take(P, &ask), Duration::ZERO);
}

#[test]
fn g_09b_queues_are_per_profile() {
    let (_clock, b) = full_bucket();
    let _poll = b.enqueue(P, CallClass::Poll);
    let other = b.enqueue("other-profile", CallClass::Ask);
    assert_eq!(b.take("other-profile", &other), Duration::ZERO);
}

#[derive(Default)]
struct SimPause {
    clock: SimClock,
    pauses: Mutex<Vec<Duration>>,
}

impl Pause for SimPause {
    fn pause(&self, d: Duration) {
        self.pauses.lock().unwrap().push(d);
        self.clock.advance(d);
    }
}

#[test]
fn g_09b_acquire_reports_the_first_wait_once_and_counts_the_call() {
    let pause = SimPause::default();
    let b = AimdBucket::new(Arc::new(pause.clock.clone()));
    let mut waits = Vec::new();
    for _ in 0..40 {
        acquire(&b, &pause, P, CallClass::Ask, &mut |w| waits.push(w));
    }
    assert!(waits.is_empty(), "no wait under the limit");
    acquire(&b, &pause, P, CallClass::Ask, &mut |w| waits.push(w));
    assert_eq!(waits.len(), 1, "the queued call reports its wait once");
    assert!(pause.clock.now() >= Duration::from_secs(60));
    // The ticket is spent: a later poll is not blocked by it.
    let poll = b.enqueue(P, CallClass::Poll);
    pause.clock.advance(Duration::from_secs(60));
    assert_eq!(b.take(P, &poll), Duration::ZERO);
}

#[test]
fn g_10a_a_503_retry_after_holds_the_profile_without_an_aimd_cut() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    b.hold(P, Duration::from_secs(9));
    assert_eq!(b.rate_per_min(P), 40, "a 503 is not a 429: no AIMD cut");
    assert_eq!(b.reserve(P), Duration::from_secs(9));
    clock.advance(Duration::from_secs(9));
    assert_eq!(b.reserve(P), Duration::ZERO);
    // The hold is clamped to 1-300 s like a 429 wait.
    b.hold(P, Duration::from_secs(5000));
    assert_eq!(b.reserve(P), Duration::from_secs(300));
}

struct Fixed(f64);

impl Jitter for Fixed {
    fn unit(&self) -> f64 {
        self.0
    }
}

#[test]
fn g_10b_g_backoff_caps_start_2s_factor_2_max_60s() {
    assert_eq!(BACKOFF_START, Duration::from_secs(2));
    assert_eq!(BACKOFF_MAX, Duration::from_secs(60));
    assert_eq!(MAX_ASK_TRIES, 5);
    let caps: Vec<u64> = (1..=7).map(|r| backoff_cap(r).as_secs()).collect();
    assert_eq!(caps, [2, 4, 8, 16, 32, 60, 60]);
    assert_eq!(backoff_cap(u32::MAX), BACKOFF_MAX);
}

#[test]
fn g_10b_full_jitter_is_a_uniform_part_of_the_cap() {
    assert_eq!(ask_backoff(1, &Fixed(0.0)), Duration::ZERO);
    assert_eq!(ask_backoff(1, &Fixed(0.5)), Duration::from_secs(1));
    assert_eq!(ask_backoff(3, &Fixed(0.25)), Duration::from_secs(2));
    // A jitter source out of range is held inside [0, cap].
    assert_eq!(ask_backoff(2, &Fixed(7.0)), Duration::from_secs(4));
    assert_eq!(ask_backoff(2, &Fixed(-1.0)), Duration::ZERO);
    assert_eq!(ask_backoff(2, &Fixed(f64::NAN)), Duration::ZERO);
    for _ in 0..100 {
        let u = OsJitter.unit();
        assert!((0.0..1.0).contains(&u), "{u}");
        assert!(ask_backoff(4, &OsJitter) <= Duration::from_secs(16));
    }
}

#[test]
fn g_09b_arc_f3_busy_card_text() {
    assert_eq!(
        BusyState::Queued.card_text().as_deref(),
        Some("Genie busy · queued")
    );
    assert_eq!(
        BusyState::Retrying {
            wait: Duration::from_millis(6200)
        }
        .card_text()
        .as_deref(),
        Some("Genie busy, retrying in 7 s")
    );
    assert_eq!(
        BusyState::Retrying {
            wait: Duration::ZERO
        }
        .card_text()
        .as_deref(),
        Some("Genie busy, retrying in 1 s"),
        "a jitter of 0 s still shows a whole second"
    );
    assert_eq!(BusyState::Clear.card_text(), None);
}
