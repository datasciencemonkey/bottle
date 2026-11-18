//! R8 McpTransport on the shared per-profile rate bucket (#138): G-09a, G-09b, G-09c, G-10a and
//! G-10b on the UC2 Genie One path, against the fake Genie One MCP server on loopback.
//!
//! The bucket runs on the #229 sim clock: [`SimPause`] moves the clock instead of sleeping, so
//! no test depends on wall time. No live workspace.
//!
//! TODO(#136): SP-18 confirms the live MCP 429 shape. These tests use an HTTP 429 with a
//! `Retry-After` header or an FMAPI `error.retry_after` body, the two shapes of G-10a.

#[path = "support/fake_mcp.rs"]
mod fake_mcp;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fake_mcp::*;
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::error::CoreResult;
use mog_core::http_transport::backoff::Jitter;
use mog_core::http_transport::rate_bucket::{AimdBucket, RateBucket, DEFAULT_RATE_PER_MIN};
use mog_core::http_transport::rate_limit::{BusyObserver, BusyState, RateLimit};
use mog_core::http_transport::Pause;
use mog_core::mcp_transport::{
    McpCall, McpLimits, McpReply, McpSeams, McpTransport, ReqwestMcpTransport,
};
use mog_core::request_identity::BottleIdentity;
use serde_json::json;

#[derive(Default)]
struct SimPause {
    clock: SimClock,
    pauses: Mutex<Vec<Duration>>,
}

impl SimPause {
    fn total(&self) -> Duration {
        self.pauses.lock().unwrap().iter().sum()
    }
}

impl Pause for SimPause {
    fn pause(&self, d: Duration) {
        self.pauses.lock().unwrap().push(d);
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
struct Seen(Mutex<Vec<(String, BusyState)>>);

impl Seen {
    fn all(&self) -> Vec<(String, BusyState)> {
        self.0.lock().unwrap().clone()
    }
    fn states(&self) -> Vec<BusyState> {
        self.all().into_iter().map(|(_, s)| s).collect()
    }
}

impl BusyObserver for Seen {
    fn busy(&self, mog: &str, state: BusyState) {
        self.0.lock().unwrap().push((mog.to_owned(), state));
    }
}

struct Rig {
    t: ReqwestMcpTransport,
    pause: Arc<SimPause>,
    bucket: Arc<AimdBucket>,
    seen: Arc<Seen>,
}

fn rig(s: &FakeMcp) -> Rig {
    rig_with(s, Arc::new(SimPause::default()))
}

fn rig_with(s: &FakeMcp, pause: Arc<SimPause>) -> Rig {
    let bucket = Arc::new(AimdBucket::new(Arc::new(pause.clock.clone())));
    let seen = Arc::new(Seen::default());
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
        bucket: bucket.clone(),
        pause: pause.clone(),
        jitter: Arc::new(Half),
        observer: seen.clone(),
    });
    Rig {
        t,
        pause,
        bucket,
        seen,
    }
}

fn ask(r: &Rig) -> CoreResult<McpReply> {
    let t = tags(MOG);
    r.t.call_tool(McpCall::new(
        PROFILE,
        "genie_ask",
        obj(json!({"question": QUESTION})),
        &t,
    ))
}

fn poll(r: &Rig) -> CoreResult<McpReply> {
    let t = tags(MOG);
    r.t.call_tool(McpCall::new(
        PROFILE,
        "genie_poll_response",
        obj(json!({"conversation_id": "c-1", "response_id": "r-1"})),
        &t,
    ))
}

fn code<T: std::fmt::Debug>(r: CoreResult<T>) -> &'static str {
    r.expect_err("the call must fail").code()
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

// ---------- G-09a: the shared bucket ----------

#[test]
fn g_09a_r8_requests_stay_at_40_per_minute_on_one_profile() {
    let pause = Arc::new(SimPause::default());
    let clock = pause.clock.clone();
    let times = Arc::new(Mutex::new(Vec::new()));
    let times_in = times.clone();
    let s = genie_one(move |_| {
        times_in.lock().unwrap().push(clock.now());
        None
    });
    let r = rig_with(&s, pause.clone());
    for _ in 0..100 {
        poll(&r).unwrap();
    }
    let sent = times.lock().unwrap().clone();
    assert_eq!(
        sent.len(),
        102,
        "initialize, the notification and 100 polls"
    );
    for (i, t) in sent.iter().enumerate() {
        let in_window = sent[i..].iter().take_while(|u| **u < *t + secs(60)).count();
        assert!(in_window <= 40, "{in_window} requests in one minute");
    }
    assert!(pause.clock.now() >= secs(120));
}

#[test]
fn g_09a_r7_calls_and_r8_calls_share_the_profile_bucket() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    // 40 calls of R7 on the same profile and bucket fill the minute.
    for _ in 0..DEFAULT_RATE_PER_MIN {
        assert_eq!(r.bucket.reserve(PROFILE), Duration::ZERO);
    }
    poll(&r).unwrap();
    assert!(r.pause.total() >= secs(60), "waited {:?}", r.pause.total());
}

// ---------- G-09b: priority and the queued state ----------

#[test]
fn g_09b_a_queued_ask_reports_genie_busy_queued_then_clear() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    for _ in 0..DEFAULT_RATE_PER_MIN {
        r.bucket.reserve(PROFILE);
    }
    ask(&r).unwrap();
    assert_eq!(
        r.seen.all(),
        [
            (MOG.to_owned(), BusyState::Queued),
            (MOG.to_owned(), BusyState::Clear)
        ]
    );
    assert_eq!(
        BusyState::Queued.card_text().as_deref(),
        Some("Genie busy · queued")
    );
}

#[test]
fn g_09b_an_ask_under_the_limit_shows_no_busy_state() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    ask(&r).unwrap();
    assert!(r.seen.all().is_empty());
}

#[test]
fn g_09b_a_waiting_poll_is_not_shown_as_a_queued_ask() {
    let s = genie_one(|_| None);
    let r = rig(&s);
    for _ in 0..DEFAULT_RATE_PER_MIN {
        r.bucket.reserve(PROFILE);
    }
    poll(&r).unwrap();
    assert!(r.pause.total() > Duration::ZERO);
    assert!(r.seen.all().is_empty());
}

// ---------- G-09c, G-10a: a 429 with a retry wait ----------

#[test]
fn g_10a_an_ask_429_with_retry_after_waits_then_is_sent_again() {
    let n = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && n.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| json_reply(429, json!({})).with_header("Retry-After", "7"))
    });
    let r = rig(&s);
    let reply = ask(&r).unwrap();
    assert_eq!(reply.http_status, 200);
    assert_eq!(s.count("tools/call"), 2);
    assert!(r.pause.total() >= secs(7), "waited {:?}", r.pause.total());
    assert_eq!(
        r.seen.states(),
        [BusyState::Retrying { wait: secs(7) }, BusyState::Clear]
    );
    assert_eq!(r.bucket.rate_per_min(PROFILE), DEFAULT_RATE_PER_MIN / 2);
}

#[test]
fn g_10a_an_fmapi_error_retry_after_body_is_obeyed() {
    let n = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && n.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| json_reply(429, json!({"error": {"retry_after": 3}})))
    });
    let r = rig(&s);
    ask(&r).unwrap();
    assert!(r.pause.total() >= secs(3), "waited {:?}", r.pause.total());
    assert_eq!(r.seen.states()[0], BusyState::Retrying { wait: secs(3) });
}

#[test]
fn g_10a_retry_after_is_clamped_to_300_s() {
    let n = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && n.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| json_reply(429, json!({})).with_header("Retry-After", "900"))
    });
    let r = rig(&s);
    ask(&r).unwrap();
    assert_eq!(r.seen.states()[0], BusyState::Retrying { wait: secs(300) });
    assert!(r.pause.total() >= secs(300) && r.pause.total() < secs(301));
}

#[test]
fn g_10a_a_poll_429_is_not_resent_and_the_next_poll_waits_for_retry_after() {
    let n = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && n.fetch_add(1, Ordering::SeqCst) == 0)
            .then(|| json_reply(429, json!({})).with_header("Retry-After", "5"))
    });
    let r = rig(&s);
    assert_eq!(code(poll(&r)), "rate_limited");
    assert_eq!(s.count("tools/call"), 1, "R8 never sends a poll again");
    assert_eq!(r.pause.total(), Duration::ZERO);
    poll(&r).unwrap();
    assert!(r.pause.total() >= secs(5), "waited {:?}", r.pause.total());
    assert!(r.seen.all().is_empty(), "a poll shows no retry countdown");
}

#[test]
fn g_10a_a_503_retry_after_holds_the_next_call() {
    let n = AtomicUsize::new(0);
    let s = genie_one(move |x| {
        (x.rpc_method() == "tools/call" && n.fetch_add(1, Ordering::SeqCst) == 0).then(|| {
            json_reply(503, json!({"error_code": "TEMPORARILY_UNAVAILABLE"}))
                .with_header("Retry-After", "4")
        })
    });
    let r = rig(&s);
    assert_eq!(poll(&r).unwrap().http_status, 503);
    assert_eq!(r.bucket.rate_per_min(PROFILE), DEFAULT_RATE_PER_MIN);
    poll(&r).unwrap();
    assert!(r.pause.total() >= secs(4), "waited {:?}", r.pause.total());
}

// ---------- G-10b: no retry header ----------

#[test]
fn g_10b_an_ask_429_without_a_header_backs_off_by_g_backoff_for_5_tries() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| json_reply(429, json!({}))));
    let r = rig(&s);
    assert_eq!(code(ask(&r)), "rate_limited");
    assert_eq!(s.count("tools/call"), 5, "5 tries or less");
    // Full jitter at 0.5 of the caps 2, 4, 8 and 16 s.
    let mut want: Vec<BusyState> = [1, 2, 4, 8]
        .into_iter()
        .map(|s| BusyState::Retrying { wait: secs(s) })
        .collect();
    want.push(BusyState::Clear);
    assert_eq!(r.seen.states(), want);
}

#[test]
fn g_10b_a_poll_429_without_a_header_only_lengthens_its_interval() {
    let s = genie_one(|x| (x.rpc_method() == "tools/call").then(|| json_reply(429, json!({}))));
    let r = rig(&s);
    assert_eq!(code(poll(&r)), "rate_limited");
    assert_eq!(s.count("tools/call"), 1);
    assert!(r.seen.all().is_empty());
    // G-09c: the profile now runs at half rate, so the next polls are spaced further apart.
    assert_eq!(r.bucket.rate_per_min(PROFILE), DEFAULT_RATE_PER_MIN / 2);
}
