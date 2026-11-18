//! AT-34 "Poll budget and 429" (#234), section 15: the Rust part. Pass: calls stay at 40/min or
//! less; polls keep priority; AIMD halves and recovers; Retry-After is honoured.
//!
//! The shared per-profile bucket is R7 `AimdBucket` (G-09a..G-09d, #364 and #138). R8
//! McpTransport, the UC2 Genie One path, waits in it and owns the 429 rules (G-10a, G-10b, #138).
//! Each test runs on the #229 sim clock (`mog_core::clock::SimClock`) with a scripted server,
//! never on wall time. The poll delay floor (G-08) and the in-flight limits (G-11a..c) are TS
//! and stay pending in `src/acceptance/at-34-poll-budget.test.ts`. The admin QPM limit (ADM-07)
//! and the live 429 are a person's steps: docs/tests/AT-GENIE-MCP.md, part AT-34.

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fake_mcp::*;
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::http_transport::backoff::Jitter;
use mog_core::http_transport::rate_bucket::{
    parse_retry_after, retry_after_from_body, AimdBucket, CallClass, RateBucket,
};
use mog_core::http_transport::rate_limit::{BusyObserver, BusyState, RateLimit};
use mog_core::http_transport::Pause;
use mog_core::mcp_transport::{McpCall, McpLimits, McpSeams, McpTransport, ReqwestMcpTransport};
use mog_core::request_identity::BottleIdentity;
use serde_json::json;

/// The default bucket of G-09a.
const LIMIT_PER_MIN: u32 = 40;
const P: &str = "bottle-test";

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

#[test]
fn at_34_g_09a_one_profile_stays_at_40_calls_per_minute() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    let mut sent = Vec::new();
    // Offer 100 calls as fast as the bucket lets them go, on one sim clock.
    while sent.len() < 100 {
        let w = b.reserve(P);
        if w.is_zero() {
            sent.push(clock.now());
        } else {
            clock.advance(w);
        }
    }
    let first_minute = sent.iter().filter(|t| **t < secs(60)).count();
    assert_eq!(first_minute, LIMIT_PER_MIN as usize);
    for (i, t) in sent.iter().enumerate() {
        let in_window = sent[i..].iter().take_while(|u| **u < *t + secs(60)).count();
        assert!(
            in_window <= LIMIT_PER_MIN as usize,
            "{in_window} in a minute"
        );
    }
    // Two profiles have two buckets.
    assert_eq!(b.reserve("another-profile"), Duration::ZERO);
}

#[test]
fn at_34_g_09b_polls_then_cancels_then_asks() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    for _ in 0..LIMIT_PER_MIN {
        b.reserve(P);
    }
    // With the bucket full, queue an ask, a cancel and a poll in that order.
    let mut queue = vec![
        ("ask", b.enqueue(P, CallClass::Ask)),
        ("cancel", b.enqueue(P, CallClass::Cancel)),
        ("poll", b.enqueue(P, CallClass::Poll)),
    ];
    let mut left = Vec::new();
    while !queue.is_empty() {
        let granted = queue.iter().position(|(_, t)| b.take(P, t).is_zero());
        match granted {
            Some(i) => left.push(queue.remove(i).0),
            None => clock.advance(Duration::from_millis(250)),
        }
    }
    assert_eq!(left, ["poll", "cancel", "ask"]);
    // The queued ask reports "Genie busy · queued" (R8: mcp_rate_limit.rs).
    assert_eq!(
        BusyState::Queued.card_text().as_deref(),
        Some("Genie busy · queued")
    );
}

#[test]
fn at_34_g_09c_g_09d_a_429_halves_the_rate_then_it_recovers() {
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    b.on_rate_limited(P, None);
    // G-09c: 20 calls/min for the 5 min after a 429.
    for _ in 0..5 {
        assert_eq!(b.rate_per_min(P), 20);
        clock.advance(secs(59));
        assert_eq!(b.rate_per_min(P), 20);
        clock.advance(secs(1));
    }
    // G-09d: then 5 calls/min more each minute: 40 again 4 min later.
    let mut seen = vec![b.rate_per_min(P)];
    for _ in 0..5 {
        clock.advance(secs(60));
        seen.push(b.rate_per_min(P));
    }
    assert_eq!(seen, [20, 25, 30, 35, 40, 40]);
}

#[test]
fn at_34_g_10a_retry_after_seconds_date_and_clamp() {
    let now = UNIX_EPOCH + secs(1_445_412_480); // Wed, 21 Oct 2015 07:28:00 GMT
    for (value, want) in [
        ("7", 7),
        ("Wed, 21 Oct 2015 07:28:30 GMT", 30),
        ("0", 1),
        ("900", 300),
    ] {
        let clock = SimClock::new();
        let b = AimdBucket::new(Arc::new(clock.clone()));
        b.on_rate_limited(P, parse_retry_after(value, now));
        assert_eq!(b.reserve(P), secs(want), "Retry-After: {value}");
        clock.advance(secs(want));
        assert_eq!(b.reserve(P), Duration::ZERO, "Retry-After: {value}");
    }
    // An FMAPI body with error.retry_after works the same.
    let clock = SimClock::new();
    let b = AimdBucket::new(Arc::new(clock.clone()));
    b.on_rate_limited(
        P,
        retry_after_from_body(&json!({"error": {"retry_after": 12}})),
    );
    assert_eq!(b.reserve(P), secs(12));
    let _ = SystemTime::now();
}

#[derive(Default)]
struct SimPause {
    clock: SimClock,
}

impl Pause for SimPause {
    fn pause(&self, d: Duration) {
        self.clock.advance(d);
    }
}

struct Half;

impl Jitter for Half {
    fn unit(&self) -> f64 {
        0.5
    }
}

#[derive(Default)]
struct Busy(Mutex<Vec<BusyState>>);

impl BusyObserver for Busy {
    fn busy(&self, _mog: &str, state: BusyState) {
        self.0.lock().unwrap().push(state);
    }
}

#[test]
fn at_34_g_10b_a_429_without_retry_after_backs_off_an_ask_only() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| json_reply(429, json!({}))));
    let pause = Arc::new(SimPause::default());
    let busy = Arc::new(Busy::default());
    let t = ReqwestMcpTransport::with_seams(McpSeams {
        hosts: Hosts::one(&s.origin()),
        endpoints: Endpoints::one(&s.url()),
        tokens: Arc::new(FakeBroker::default()),
        policy: Arc::new(LoopbackPolicy),
        decorator: Arc::new(BottleIdentity::new()),
        log: Arc::new(Lines::default()),
        limits: McpLimits::default(),
    })
    .unwrap()
    .with_rate_limit(RateLimit {
        bucket: Arc::new(AimdBucket::new(Arc::new(pause.clock.clone()))),
        pause,
        jitter: Arc::new(Half),
        observer: busy.clone(),
    });
    let tg = tags(MOG);
    // An ask tries again with full jitter: start 2 s, factor 2, 5 tries or less (G-BACKOFF).
    let asked = t.call_tool(McpCall::new(
        PROFILE,
        "genie_ask",
        obj(json!({"question": QUESTION})),
        &tg,
    ));
    assert_eq!(asked.unwrap_err().code(), "rate_limited");
    assert_eq!(s.count("tools/call"), 5);
    let waits: Vec<BusyState> = busy.0.lock().unwrap().clone();
    assert_eq!(
        waits,
        [
            BusyState::Retrying { wait: secs(1) },
            BusyState::Retrying { wait: secs(2) },
            BusyState::Retrying { wait: secs(4) },
            BusyState::Retrying { wait: secs(8) },
            BusyState::Clear,
        ]
    );
    // A poll that gets the same 429 is not sent again early.
    let polled = t.call_tool(McpCall::new(
        PROFILE,
        "genie_poll_response",
        obj(json!({"conversation_id": "c-1", "response_id": "r-1"})),
        &tg,
    ));
    assert_eq!(polled.unwrap_err().code(), "rate_limited");
    assert_eq!(s.count("tools/call"), 6);
}

#[test]
#[ignore = "pending TODO(#183): live AT-34 needs a person with a U2M profile and an admin QPM limit (needs:u2m, needs:admin)"]
fn at_34_adm_07_live_four_asks_under_an_admin_qpm_limit() {
    let w = mog_core::live_or_skip!();
    // Steps in docs/tests/AT-GENIE-MCP.md, part AT-34.
    let _ = w.profile;
    todo!("#183: run 4 in-flight asks on one profile and count calls per minute")
}
