//! The R15 event callback: one thread-safe callback, `{ event, payload }` JSON, and a core
//! side that never waits for the host (NFR-18b, NFR-18a, ARC-6). The wall-clock 1 ms latency
//! checks were removed as flaky on shared runners (#499, #508); the device run is the evidence.
//!
//! The callback slot is process-wide, so the tests in this file take `SERIAL` first.

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use mog_core_ffi::{
    dropped_events, emit_event, mog_set_event_callback, MOG_EVENT_QUEUE_CAPACITY, MOG_STATUS_OK,
};
use serde_json::json;

static SERIAL: Mutex<()> = Mutex::new(());

fn serial() -> MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What a callback saw. The tests pass a `&Recorder` as `user_data`.
#[derive(Default)]
struct Recorder {
    events: Mutex<Vec<(String, Instant)>>,
    count: AtomicUsize,
    inside: AtomicBool,
    overlapped: AtomicBool,
    sleep_ms: u64,
    clear_from_inside: bool,
    /// While true, the callback does not return (a host that blocks).
    hold: AtomicBool,
}

impl Recorder {
    fn slow(sleep_ms: u64) -> Self {
        Recorder {
            sleep_ms,
            ..Recorder::default()
        }
    }

    fn as_user_data(&self) -> *mut c_void {
        self as *const Recorder as *mut c_void
    }

    fn wait_for(&self, n: usize, timeout: Duration) -> bool {
        let end = Instant::now() + timeout;
        while Instant::now() < end {
            if self.count.load(Ordering::SeqCst) >= n {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        self.count.load(Ordering::SeqCst) >= n
    }
}

extern "C" fn record(event_json: *const c_char, user_data: *mut c_void) {
    let at = Instant::now();
    // SAFETY: the tests pass a `&Recorder` that outlives the callback registration.
    let rec = unsafe { &*(user_data as *const Recorder) };
    if rec.inside.swap(true, Ordering::SeqCst) {
        rec.overlapped.store(true, Ordering::SeqCst);
    }
    // SAFETY: the core passes a valid NUL-terminated string for the length of the call.
    let text = unsafe { CStr::from_ptr(event_json) }
        .to_str()
        .expect("UTF-8 event")
        .to_owned();
    rec.events
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push((text, at));
    while rec.hold.load(Ordering::SeqCst) {
        thread::sleep(Duration::from_millis(1));
    }
    if rec.sleep_ms > 0 {
        thread::sleep(Duration::from_millis(rec.sleep_ms));
    }
    if rec.clear_from_inside {
        assert_eq!(mog_set_event_callback(None, ptr::null_mut()), MOG_STATUS_OK);
    }
    rec.inside.store(false, Ordering::SeqCst);
    rec.count.fetch_add(1, Ordering::SeqCst);
}

fn set(rec: &Recorder) {
    assert_eq!(
        mog_set_event_callback(Some(record), rec.as_user_data()),
        MOG_STATUS_OK
    );
}

fn clear() {
    assert_eq!(mog_set_event_callback(None, ptr::null_mut()), MOG_STATUS_OK);
}

#[test]
fn nfr_18a_event_json_is_event_and_payload() {
    let _s = serial();
    let rec = Recorder::default();
    set(&rec);
    assert!(emit_event(
        "auth:status",
        json!({"profile": "bottle-dev", "ok": true})
    ));
    assert!(rec.wait_for(1, Duration::from_secs(5)));
    clear();
    let events = rec.events.lock().unwrap_or_else(PoisonError::into_inner);
    let got: serde_json::Value = serde_json::from_str(&events[0].0).expect("event is JSON");
    assert_eq!(
        got,
        json!({"event": "auth:status", "payload": {"profile": "bottle-dev", "ok": true}})
    );
}

#[test]
fn nfr_18b_callback_is_thread_safe_and_never_runs_twice_at_once() {
    let _s = serial();
    let rec = Recorder::default();
    set(&rec);
    let threads = 8;
    let per_thread = 500;
    thread::scope(|s| {
        for t in 0..threads {
            s.spawn(move || {
                for i in 0..per_thread {
                    // 4,000 events can outrun the bounded queue on a loaded machine; a full
                    // queue drops the event, so try again until it fits.
                    while !emit_event("capture:state", json!({ "t": t, "i": i })) {
                        thread::sleep(Duration::from_micros(100));
                    }
                }
            });
        }
        // Set the same callback again from another thread while events flow.
        s.spawn(|| {
            for _ in 0..100 {
                set(&rec);
            }
        });
    });
    assert!(rec.wait_for(threads * per_thread, Duration::from_secs(10)));
    clear();
    assert_eq!(rec.count.load(Ordering::SeqCst), threads * per_thread);
    assert!(
        !rec.overlapped.load(Ordering::SeqCst),
        "two callbacks ran at once"
    );
}

#[test]
fn nfr_18b_cleared_callback_is_not_called_after_set_returns() {
    let _s = serial();
    let rec = Recorder::slow(5);
    set(&rec);
    for i in 0..10 {
        assert!(emit_event("auth:status", json!({ "seq": i })));
    }
    assert!(rec.wait_for(1, Duration::from_secs(5)));
    // `clear` waits for a call in flight, so after it returns the count is final.
    clear();
    let after_clear = rec.count.load(Ordering::SeqCst);
    assert!(!rec.inside.load(Ordering::SeqCst));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(rec.count.load(Ordering::SeqCst), after_clear);
}

#[test]
fn nfr_18b_callback_may_clear_itself_without_a_deadlock() {
    let _s = serial();
    let rec = Recorder {
        clear_from_inside: true,
        ..Recorder::default()
    };
    set(&rec);
    assert!(emit_event("hotkey:panic", json!({})));
    assert!(
        rec.wait_for(1, Duration::from_secs(5)),
        "the callback deadlocked"
    );
    assert!(emit_event("hotkey:panic", json!({})));
    thread::sleep(Duration::from_millis(50));
    assert_eq!(rec.count.load(Ordering::SeqCst), 1);
}

#[test]
fn arc_6_secret_shaped_event_never_reaches_the_callback() {
    let _s = serial();
    let rec = Recorder::default();
    set(&rec);
    let fake = format!("dapi{}", "0123456789abcdef0123456789abcdef");
    assert!(!emit_event("auth:status", json!({ "token": fake })));
    let jwt = ["ey", "JhbGciOiJIUzI1NiJ9", ".e30.sig"].concat();
    assert!(!emit_event(&jwt, json!({})));
    assert!(emit_event("auth:status", json!({ "ok": true })));
    assert!(rec.wait_for(1, Duration::from_secs(5)));
    thread::sleep(Duration::from_millis(50));
    clear();
    assert_eq!(rec.count.load(Ordering::SeqCst), 1);
}

#[test]
fn nfr_18b_full_event_queue_drops_and_counts_without_blocking() {
    let _s = serial();
    let rec = Recorder::default();
    rec.hold.store(true, Ordering::SeqCst);
    set(&rec);
    let before = dropped_events();
    let extra = 50;
    let mut accepted = 0;
    let start = Instant::now();
    for i in 0..MOG_EVENT_QUEUE_CAPACITY + extra {
        if emit_event("net:status", json!({ "seq": i })) {
            accepted += 1;
        }
    }
    let took = start.elapsed();
    // The delivery thread may hold one event in the blocked callback; the queue holds the rest.
    assert!(
        (MOG_EVENT_QUEUE_CAPACITY..=MOG_EVENT_QUEUE_CAPACITY + 1).contains(&accepted),
        "accepted {accepted}"
    );
    let dropped = dropped_events() - before;
    assert_eq!(
        dropped as usize,
        MOG_EVENT_QUEUE_CAPACITY + extra - accepted
    );
    assert!(took <= Duration::from_secs(1), "emit blocked: {took:?}");
    rec.hold.store(false, Ordering::SeqCst);
    assert!(rec.wait_for(accepted, Duration::from_secs(10)));
    clear();
}
