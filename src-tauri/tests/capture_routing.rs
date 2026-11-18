//! #425 (ADR 0055): the Tauri host routes `capture_*` through the hardened gateway to one R2
//! `CaptureCore`, relays its events to the brain, and runs the cap timer. The tests use the
//! real `CaptureCore` over the fake device and the simulated clock, so the 120 s and 5 min caps
//! fire with no real sleep.

use bottle_lib::capture_host::{
    CapTimer, CaptureHost, CaptureTicket, TimerWait, UnwiredMic, TICK_RETRY_MS, UNWIRED_MIC,
};
use bottle_lib::ipc::{admit, route, BrainLink, HostComponents};
use mog_core::capture::{AudioDevice, CaptureService, HANDS_FREE_MAX_MS, HOLD_MAX_MS};
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::ipc::{events, ChannelRef};
use mog_core::testkit::capture::FakeAudioDevice;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;

// ---------- fixtures ----------

/// The calls' brain link. Capture never uses it: capture events go through the host relay.
struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

type Relayed = Arc<Mutex<Vec<(String, Value)>>>;

struct Rig {
    host: CaptureHost,
    device: FakeAudioDevice,
    clock: SimClock,
    relayed: Relayed,
}

fn rig_with(device: Box<dyn AudioDevice>, fake: FakeAudioDevice) -> Rig {
    rig_with_relay(device, fake, |_| {})
}

/// A rig whose relay first calls `before(event)` (for example to panic once).
fn rig_with_relay(
    device: Box<dyn AudioDevice>,
    fake: FakeAudioDevice,
    before: impl Fn(&str) + Send + Sync + 'static,
) -> Rig {
    let clock = SimClock::new();
    let relayed: Relayed = Arc::default();
    let sink = Arc::clone(&relayed);
    let host = CaptureHost::new(
        device,
        Arc::new(clock.clone()) as Arc<dyn MonotonicClock>,
        Arc::new(move |event: &str, payload: Value| {
            sink.lock().unwrap().push((event.to_owned(), payload));
            before(event);
            true
        }),
    )
    .expect("the cap timer thread starts");
    Rig {
        host,
        device: fake,
        clock,
        relayed,
    }
}

fn rig() -> Rig {
    let fake = FakeAudioDevice::new();
    rig_with(Box::new(fake.clone()), fake)
}

impl Rig {
    fn call_from(&self, from: &str, command: &str, args: Value) -> Result<Value, Value> {
        let components = HostComponents {
            capture: Some(&self.host),
            ..HostComponents::none(from, &NoBrain)
        };
        route(components, command, args)
    }

    fn call(&self, command: &str, args: Value) -> Result<Value, Value> {
        self.call_from("brain", command, args)
    }

    /// A call whose admission the IPC thread took earlier (`ticket`), as `handler` does for
    /// a call that runs on a worker.
    fn call_admitted(
        &self,
        ticket: CaptureTicket,
        command: &str,
        args: Value,
    ) -> Result<Value, Value> {
        let components = HostComponents {
            capture: Some(&self.host),
            capture_ticket: Some(ticket),
            ..HostComponents::none("brain", &NoBrain)
        };
        route(components, command, args)
    }

    fn start(&self, mog: &str, mode: &str) -> Result<Value, Value> {
        self.call("capture_start", json!({"mogId": mog, "mode": mode}))
    }

    fn lease_of(&self, mog: &str, mode: &str) -> String {
        let state = self.start(mog, mode).expect("the capture starts");
        state["leaseId"].as_str().expect("a lease ID").to_owned()
    }

    fn relayed(&self) -> Vec<(String, Value)> {
        self.relayed.lock().unwrap().clone()
    }

    fn stops(&self) -> Vec<Value> {
        self.relayed()
            .into_iter()
            .filter(|(e, _)| e == events::CAPTURE_STOPPED)
            .map(|(_, p)| p)
            .collect()
    }

    fn timer(&self) -> &CapTimer {
        self.host.timer()
    }

    /// Move the simulated clock and let the timer thread act on it.
    fn advance_ms(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
        self.timer().settle();
    }
}

fn err(code: &str) -> Result<Value, Value> {
    Err(json!({ "error": code }))
}

const CAPTURE_COMMANDS: [&str; 7] = [
    "capture_start",
    "capture_stop",
    "capture_pause",
    "capture_resume",
    "capture_status",
    "capture_stop_all",
    "capture_subscribe",
];

fn sample_args(command: &str) -> Value {
    match command {
        "capture_start" => json!({"mogId": "a1", "mode": "ptt"}),
        "capture_stop" | "capture_pause" | "capture_resume" => {
            json!({"mogId": "a1", "leaseId": "1"})
        }
        "capture_subscribe" => json!({"onLevel": "__CHANNEL__:1", "onPartial": "__CHANNEL__:2"}),
        _ => json!({}),
    }
}

// ---------- routing (CAP-01a, CAP-02a, CAP-03) ----------

#[test]
fn cap_02a_at_17_capture_start_returns_the_state_with_its_lease_id() {
    let r = rig();
    let state = r.start("a1", "ptt").unwrap();
    assert_eq!(state["owner"], "a1");
    assert_eq!(state["mode"], "ptt");
    assert_eq!(state["recording"], true);
    let lease = state["leaseId"].as_str().unwrap();
    assert!(!lease.is_empty());
    assert!(r.device.log().mic_open, "the lease owns an open mic");
}

#[test]
fn cap_01a_a_second_capture_start_gets_mic_busy() {
    let r = rig();
    r.start("a1", "ptt").unwrap();
    assert_eq!(r.start("b2", "ptt"), err("mic_busy"));
    assert_eq!(r.start("b2", "hands_free"), err("mic_busy"));
}

#[test]
fn cap_03_ptt_and_hands_free_start_and_stop_through_the_gateway() {
    let r = rig();
    for mode in ["ptt", "hands_free"] {
        let lease = r.lease_of("a1", mode);
        let state = r
            .call("capture_stop", json!({"mogId": "a1", "leaseId": lease}))
            .unwrap();
        assert_eq!(state["recording"], false, "{mode}");
        assert!(state.get("leaseId").is_none(), "{mode}");
        assert!(!r.device.log().mic_open, "{mode}: the mic closed");
    }
    let stops = r.stops();
    assert_eq!(stops.len(), 2);
    assert!(stops.iter().all(|s| s["reason"] == "manual"));
}

#[test]
fn cap_03_a_session_start_needs_consent() {
    // TODO(#65): a session starts once `session_attest` issues consent tokens that R2 checks.
    let r = rig();
    for args in [
        json!({"mogId": "a1", "mode": "session"}),
        json!({"mogId": "a1", "mode": "session", "consentToken": "test-consent-not-real"}),
    ] {
        assert_eq!(r.call("capture_start", args), err("consent_required"));
    }
    assert_eq!(r.device.log().mic_opens, 0, "no stream opened");
}

#[test]
fn cap_03_pause_applies_only_to_a_session() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    assert_eq!(
        r.call("capture_pause", json!({"mogId": "a1", "leaseId": lease})),
        err("invalid_mode")
    );
    assert!(r.device.log().mic_open, "the PTT still records");
}

#[test]
fn cap_03_others_track_outside_a_session_is_invalid_mode() {
    let r = rig();
    assert_eq!(
        r.call(
            "capture_start",
            json!({"mogId": "a1", "mode": "ptt", "othersTrack": true})
        ),
        err("invalid_mode")
    );
}

#[test]
fn cap_02a_stop_and_pause_need_the_lease_of_the_same_mog() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    for command in ["capture_stop", "capture_pause"] {
        // Another MOG cannot stop, pause or probe the lease (ADR 0007).
        assert_eq!(
            r.call(command, json!({"mogId": "b2", "leaseId": lease})),
            err("unknown_lease"),
            "{command}"
        );
        // R2 never issued these IDs.
        for unknown in ["999", "0", "007", "abc"] {
            assert_eq!(
                r.call(command, json!({"mogId": "a1", "leaseId": unknown})),
                err("unknown_lease"),
                "{command} {unknown}"
            );
        }
        // The gateway refuses a malformed lease ID before R2 sees it.
        for bad in [json!("a b"), json!("+1"), json!(""), json!(7)] {
            assert_eq!(
                r.call(command, json!({"mogId": "a1", "leaseId": bad})),
                err("invalid_args"),
                "{command} {bad}"
            );
        }
    }
    assert!(r.device.log().mic_open, "the lease of a1 still records");
    r.call("capture_stop", json!({"mogId": "a1", "leaseId": lease}))
        .unwrap();
    assert_eq!(
        r.call("capture_stop", json!({"mogId": "a1", "leaseId": lease})),
        err("unknown_lease"),
        "a released lease is unknown"
    );
}

#[test]
fn cap_02a_capture_resume_needs_the_lease_of_the_same_mog() {
    // #279 / ADR 0039: R2 resumes only a paused session. No session starts yet (TODO(#65)),
    // so a live PTT or hands-free lease answers `invalid_mode` from R2.
    let r = rig();
    for mode in ["ptt", "hands_free"] {
        let lease = r.lease_of("a1", mode);
        assert_eq!(
            r.call("capture_resume", json!({"mogId": "a1", "leaseId": lease})),
            err("invalid_mode"),
            "{mode}"
        );
        assert_eq!(
            r.call("capture_resume", json!({"mogId": "b2", "leaseId": lease})),
            err("unknown_lease"),
            "{mode}: another MOG cannot resume the lease"
        );
        r.call("capture_stop", json!({"mogId": "a1", "leaseId": lease}))
            .unwrap();
        assert_eq!(
            r.call("capture_resume", json!({"mogId": "a1", "leaseId": lease})),
            err("unknown_lease"),
            "{mode}: a released lease"
        );
    }
    assert_eq!(
        r.call("capture_resume", json!({"mogId": "a1", "leaseId": "999"})),
        err("unknown_lease")
    );
    assert_eq!(
        r.call("capture_resume", json!({"mogId": "a1", "leaseId": "a b"})),
        err("invalid_args")
    );
}

#[test]
fn cap_02a_a_ptt_over_hands_free_gets_its_own_lease_and_the_hands_free_lease_resumes() {
    let r = rig();
    let hf = r.lease_of("a1", "hands_free");
    let ptt = r.lease_of("b2", "ptt");
    assert_ne!(hf, ptt);
    let state = r
        .call("capture_stop", json!({"mogId": "b2", "leaseId": ptt}))
        .unwrap();
    assert_eq!(state["leaseId"], hf.as_str(), "ARC-F9: hands-free resumes");
    assert_eq!(state["owner"], "a1");
}

#[test]
fn cap_02a_capture_status_reports_the_lease_that_owns_the_mic() {
    let r = rig();
    let idle = r.call("capture_status", json!({})).unwrap();
    assert_eq!(idle["recording"], false);
    let lease = r.lease_of("a1", "hands_free");
    let state = r.call("capture_status", json!({})).unwrap();
    assert_eq!(state["leaseId"], lease.as_str());
    assert_eq!(state["mode"], "hands_free");
}

#[test]
fn mb_05_capture_stop_all_stops_every_lease_and_closes_the_mic() {
    let r = rig();
    r.lease_of("a1", "hands_free");
    r.lease_of("b2", "ptt");
    let state = r.call("capture_stop_all", json!({})).unwrap();
    assert_eq!(state["recording"], false);
    assert!(!r.device.log().mic_open);
    let reasons: Vec<Value> = r.stops().into_iter().map(|s| s["reason"].clone()).collect();
    assert_eq!(reasons, vec![json!("panic"), json!("panic")]);
    assert_eq!(
        r.start("c3", "ptt").map(|s| s["owner"].clone()),
        Ok(json!("c3"))
    );
}

#[test]
fn cap_02a_capture_subscribe_checks_its_channels() {
    // This host has no level stream (`CaptureHost::new`); `level_host.rs` tests the stream
    // (#476). TODO(#514): R4's partial text feeds `onPartial`.
    let r = rig();
    assert_eq!(
        r.call(
            "capture_subscribe",
            json!({"onLevel": "__CHANNEL__:1", "onPartial": "__CHANNEL__:2"})
        ),
        Ok(json!({}))
    );
    assert_eq!(
        r.call(
            "capture_subscribe",
            json!({"onLevel": "http://x", "onPartial": "__CHANNEL__:2"})
        ),
        err("invalid_args")
    );
}

#[test]
fn cap_02a_a_view_cannot_call_capture() {
    let r = rig();
    for from in ["mog-a1", "card-a1", "hub"] {
        for command in CAPTURE_COMMANDS {
            assert_eq!(
                r.call_from(from, command, sample_args(command)),
                err("window_not_allowed"),
                "{from} {command}"
            );
        }
    }
    assert_eq!(r.device.log().mic_opens, 0);
}

#[test]
fn cap_01a_no_managed_capture_host_is_host_failed() {
    for command in CAPTURE_COMMANDS {
        let components = HostComponents::none("brain", &NoBrain);
        assert_eq!(
            route(components, command, sample_args(command)),
            err("host_failed"),
            "{command}"
        );
    }
}

#[test]
fn cap_01a_the_unwired_mic_fails_with_a_typed_code_and_no_lease() {
    // The app uses `UnwiredMic` only where #388 has no cpal backend (not macOS or Windows).
    let fake = FakeAudioDevice::new();
    let r = rig_with(Box::new(UnwiredMic), fake);
    assert_eq!(
        UnwiredMic.open(mog_core::capture::AudioSource::Mic),
        Err(UNWIRED_MIC)
    );
    assert_eq!(r.start("a1", "ptt"), err("host_failed"));
    let state = r.call("capture_status", json!({})).unwrap();
    assert_eq!(state["recording"], false);
    assert!(r.stops().is_empty());
}

// ---------- events to the brain (CAP-01b, AT-17) ----------

#[test]
fn cap_02a_at_17_capture_state_goes_to_the_brain_with_the_lease_id() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    let states: Vec<Value> = r
        .relayed()
        .into_iter()
        .filter(|(e, _)| e == events::CAPTURE_STATE)
        .map(|(_, p)| p)
        .collect();
    let last = states.last().expect("a capture.state event");
    assert_eq!(last["leaseId"], lease.as_str());
    assert_eq!(last["owner"], "a1");
    assert_eq!(last["recording"], true);
    assert_eq!(events::CAPTURE_STATE, "capture:state");
}

#[test]
fn cap_02a_capture_stopped_names_the_lease_the_mog_the_mode_and_the_reason() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    r.call("capture_stop", json!({"mogId": "a1", "leaseId": lease}))
        .unwrap();
    assert_eq!(
        r.stops(),
        vec![json!({"leaseId": lease, "mogId": "a1", "mode": "ptt", "reason": "manual"})]
    );
    assert_eq!(events::CAPTURE_STOPPED, "capture:stopped");
}

#[test]
fn cap_01b_capture_events_and_replies_carry_no_audio_or_text() {
    let r = rig();
    let mut replies = vec![r.start("a1", "hands_free").unwrap()];
    replies.push(r.start("b2", "ptt").unwrap());
    replies.push(r.call("capture_status", json!({})).unwrap());
    replies.push(r.call("capture_stop_all", json!({})).unwrap());
    let state_keys = [
        "owner",
        "mode",
        "leaseId",
        "recording",
        "paused",
        "sessionId",
        "startedAt",
        "directedTo",
        "othersTrack",
        // #453 (CAP-17b): the mic health, a fixed code (checked below).
        "mic",
    ];
    let stop_keys = ["leaseId", "mogId", "mode", "reason"];
    let check = |v: &Value, allowed: &[&str]| {
        let obj = v.as_object().expect("an object");
        for (k, val) in obj {
            assert!(allowed.contains(&k.as_str()), "unexpected field {k}");
            assert!(!val.is_array() && !val.is_object(), "{k} holds a structure");
            if k == "mic" {
                let code = val.as_str().expect("mic is a code");
                assert!(
                    ["ok", "zero_buffers", "permission_denied", "no_device"].contains(&code),
                    "mic code {code}"
                );
            }
        }
    };
    for reply in &replies {
        check(reply, &state_keys);
    }
    let relayed = r.relayed();
    assert!(!relayed.is_empty());
    for (event, payload) in &relayed {
        match event.as_str() {
            events::CAPTURE_STATE => check(payload, &state_keys),
            events::CAPTURE_STOPPED => check(payload, &stop_keys),
            other => panic!("unexpected event {other}"),
        }
    }
}

// ---------- the cap timer (CAP-04a, CAP-04b) ----------

#[test]
fn cap_04a_the_timer_stops_a_ptt_hold_after_120_s() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    assert_eq!(r.timer().armed(), Some(HOLD_MAX_MS));
    r.advance_ms(HOLD_MAX_MS - 1);
    assert!(r.stops().is_empty(), "no stop 1 ms before the cap");
    assert!(r.device.log().mic_open);
    r.advance_ms(1);
    assert_eq!(
        r.stops(),
        vec![json!({"leaseId": lease, "mogId": "a1", "mode": "ptt", "reason": "hold_cap"})]
    );
    assert!(!r.device.log().mic_open, "the mic closed at the cap");
    assert_eq!(r.timer().armed(), None, "no deadline is left");
    let state = r.call("capture_status", json!({})).unwrap();
    assert_eq!(state["recording"], false);
}

#[test]
fn cap_04b_the_timer_stops_hands_free_after_5_min() {
    let r = rig();
    let lease = r.lease_of("a1", "hands_free");
    assert_eq!(r.timer().armed(), Some(HANDS_FREE_MAX_MS));
    r.advance_ms(HANDS_FREE_MAX_MS - 1);
    assert!(r.stops().is_empty(), "no stop 1 ms before the cap");
    r.advance_ms(1);
    assert_eq!(
        r.stops(),
        vec![
            json!({"leaseId": lease, "mogId": "a1", "mode": "hands_free", "reason": "hands_free_cap"})
        ]
    );
    assert!(!r.device.log().mic_open);
}

#[test]
fn cap_04a_an_earlier_ptt_deadline_wakes_the_timer_and_hands_free_keeps_its_cap() {
    let r = rig();
    let hf = r.lease_of("a1", "hands_free");
    assert_eq!(r.timer().armed(), Some(HANDS_FREE_MAX_MS));
    r.advance_ms(60_000);
    let ptt = r.lease_of("b2", "ptt");
    // The PTT cap (60 s + 120 s) is before the hands-free cap (300 s).
    assert_eq!(r.timer().armed(), Some(60_000 + HOLD_MAX_MS));
    r.advance_ms(HOLD_MAX_MS);
    assert_eq!(r.stops().len(), 1);
    assert_eq!(r.stops()[0]["leaseId"], ptt.as_str());
    assert_eq!(r.stops()[0]["reason"], "hold_cap");
    assert_eq!(r.timer().armed(), Some(HANDS_FREE_MAX_MS), "rescheduled");
    let state = r.call("capture_status", json!({})).unwrap();
    assert_eq!(state["leaseId"], hf.as_str(), "hands-free resumed");
    r.advance_ms(HANDS_FREE_MAX_MS - 60_000 - HOLD_MAX_MS);
    assert_eq!(r.stops().len(), 2);
    assert_eq!(r.stops()[1]["leaseId"], hf.as_str());
    assert_eq!(r.stops()[1]["reason"], "hands_free_cap");
    assert!(!r.device.log().mic_open);
}

#[test]
fn cap_04a_a_manual_stop_leaves_no_cap_to_fire() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    r.call("capture_stop", json!({"mogId": "a1", "leaseId": lease}))
        .unwrap();
    r.advance_ms(HOLD_MAX_MS);
    assert_eq!(r.stops().len(), 1, "only the manual stop");
    assert_eq!(r.timer().armed(), None);
    // A new PTT after the old cap time gets a fresh 120 s.
    r.lease_of("a1", "ptt");
    assert_eq!(r.timer().armed(), Some(2 * HOLD_MAX_MS));
}

#[test]
fn cap_04a_the_timer_waits_without_a_busy_loop() {
    // No real sleep: the timer reports what it waits for. A bounded wait is the exact time left
    // to the deadline on the clock, and with no deadline it waits with no timeout.
    let r = rig();
    r.lease_of("a1", "ptt");
    r.timer().settle();
    assert_eq!(
        r.timer().last_wait(),
        TimerWait::For(Duration::from_millis(HOLD_MAX_MS))
    );
    let ticks = r.timer().ticks();
    r.advance_ms(30_000);
    assert_eq!(
        r.timer().last_wait(),
        TimerWait::For(Duration::from_millis(HOLD_MAX_MS - 30_000))
    );
    assert_eq!(r.timer().ticks(), ticks, "no tick before the deadline");
    // With no lease there is no deadline, and the thread waits for an arm.
    r.call("capture_stop_all", json!({})).unwrap();
    r.advance_ms(HOLD_MAX_MS);
    assert_eq!(r.timer().last_wait(), TimerWait::Forever);
    assert_eq!(r.timer().armed(), None);
    let ticks = r.timer().ticks();
    r.timer().settle();
    r.timer().settle();
    assert_eq!(r.timer().ticks(), ticks, "no tick with no deadline");
    assert_eq!(r.timer().last_wait(), TimerWait::Forever);
}

#[test]
fn cap_04a_the_timer_ticks_a_bare_service_at_each_deadline() {
    // CapTimer over the core alone: an arm from `tick()` drives the stop.
    let fake = FakeAudioDevice::new();
    let clock = SimClock::new();
    let stops = Arc::new(mog_core::testkit::capture::RecordingCaptureEvents::default());
    let core = Arc::new(mog_core::capture::CaptureCore::new(
        Box::new(fake.clone()),
        Arc::new(clock.clone()),
        stops.clone(),
    ));
    let timer = CapTimer::spawn(core.clone(), Arc::new(clock.clone())).unwrap();
    core.start(mog_core::capture::CaptureRequest {
        mog_id: "a1".into(),
        mode: mog_core::types::CaptureMode::Ptt,
        others_track: false,
    })
    .unwrap();
    timer.arm(core.tick());
    clock.advance(Duration::from_millis(HOLD_MAX_MS));
    timer.settle();
    assert_eq!(
        stops.last_stop().map(|s| s.reason),
        Some(mog_core::capture::StopReason::HoldCap)
    );
    timer.shutdown();
    assert!(!fake.log().mic_open);
}

#[test]
fn cap_04a_shutdown_stops_every_lease_and_ends_the_timer() {
    let r = rig();
    r.lease_of("a1", "hands_free");
    r.host.shutdown();
    assert!(!r.device.log().mic_open, "the mic closes on quit");
    assert!(r.timer().is_stopped());
    r.host.shutdown();
}

// ---------- order of a queued start and a later stop (MB-05, ADR 0055) ----------

#[test]
fn mb_05_a_stop_all_after_a_queued_capture_start_wins() {
    // The IPC thread admits `capture_start` and queues it on a worker. `capture_stop_all`
    // arrives later and runs inline first. The start must not keep the mic.
    let r = rig();
    let ticket = r.host.ticket();
    r.call("capture_stop_all", json!({})).unwrap();
    assert_eq!(
        r.call_admitted(
            ticket,
            "capture_start",
            json!({"mogId": "a1", "mode": "hands_free"})
        ),
        err("unknown_lease")
    );
    assert!(
        !r.device.log().mic_open,
        "the mic is closed after the panic"
    );
    assert_eq!(
        r.device.log().mic_opens,
        0,
        "the mic never opened after the panic"
    );
    assert!(
        !r.relayed()
            .iter()
            .any(|(e, p)| e == events::CAPTURE_STATE && p["recording"] == true),
        "the brain never heard recording"
    );
    let state = r.call("capture_status", json!({})).unwrap();
    assert_eq!(state["recording"], false);
    assert_eq!(r.host.live_owner_count(), 0);
    // A start admitted after the stop is not affected.
    let ticket = r.host.ticket();
    let state = r
        .call_admitted(
            ticket,
            "capture_start",
            json!({"mogId": "a1", "mode": "ptt"}),
        )
        .unwrap();
    assert_eq!(state["recording"], true);
}

#[test]
fn cap_02a_a_stop_of_another_lease_does_not_cancel_a_queued_start() {
    let r = rig();
    let hf = r.lease_of("a1", "hands_free");
    let ticket = r.host.ticket();
    r.call("capture_stop", json!({"mogId": "a1", "leaseId": hf}))
        .unwrap();
    let state = r
        .call_admitted(
            ticket,
            "capture_start",
            json!({"mogId": "b2", "mode": "ptt"}),
        )
        .unwrap();
    assert_eq!(state["owner"], "b2");
    assert!(r.device.log().mic_open);
}

#[test]
fn cap_04a_shutdown_refuses_a_queued_capture_start() {
    let r = rig();
    let ticket = r.host.ticket();
    r.host.shutdown();
    assert_eq!(
        r.call_admitted(
            ticket,
            "capture_start",
            json!({"mogId": "a1", "mode": "ptt"})
        ),
        err("host_failed")
    );
    assert_eq!(r.start("a1", "ptt"), err("host_failed"));
    assert_eq!(r.device.log().mic_opens, 0, "no stream opened after quit");
}

// ---------- the owner map forgets every stopped lease (CAP-02a) ----------

#[test]
fn cap_02a_the_host_forgets_the_owner_of_every_stopped_lease() {
    let r = rig();
    let lease = r.lease_of("a1", "ptt");
    assert_eq!(r.host.live_owner_count(), 1);
    r.call("capture_stop", json!({"mogId": "a1", "leaseId": lease}))
        .unwrap();
    assert_eq!(r.host.live_owner_count(), 0, "manual stop");
    r.lease_of("a1", "ptt");
    r.advance_ms(HOLD_MAX_MS);
    assert_eq!(r.stops().last().unwrap()["reason"], "hold_cap");
    assert_eq!(r.host.live_owner_count(), 0, "cap stop");
    r.lease_of("a1", "hands_free");
    r.lease_of("b2", "ptt");
    assert_eq!(r.host.live_owner_count(), 2);
    r.call("capture_stop_all", json!({})).unwrap();
    assert_eq!(r.host.live_owner_count(), 0, "stop_all");
}

// ---------- a panic in tick() does not end the caps (CAP-04a, CAP-04b) ----------

#[test]
fn cap_04a_cap_04b_a_panicking_event_sink_does_not_end_the_caps() {
    let fake = FakeAudioDevice::new();
    let panicked = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let once = Arc::clone(&panicked);
    let r = rig_with_relay(Box::new(fake.clone()), fake, move |event| {
        if event == events::CAPTURE_STOPPED && !once.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            panic!("test sink panic");
        }
    });
    r.lease_of("a1", "ptt");
    // The hold cap fires; the sink panics inside tick() on the timer thread.
    r.advance_ms(HOLD_MAX_MS);
    assert!(panicked.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        !r.device.log().mic_open,
        "the stop was committed before the event"
    );
    assert!(!r.timer().is_stopped(), "the timer thread lives on");
    // The timer retries, and the next cap still fires.
    r.advance_ms(TICK_RETRY_MS);
    r.lease_of("a1", "hands_free");
    r.advance_ms(HANDS_FREE_MAX_MS);
    assert_eq!(r.stops().last().unwrap()["reason"], "hands_free_cap");
    assert!(!r.device.log().mic_open);
}

#[test]
fn mb_05_the_ipc_thread_admits_every_capture_call_and_only_capture_calls() {
    // `handler` calls `admit` before `schedule`; without a ticket the order guard is gone.
    let r = rig();
    for command in CAPTURE_COMMANDS {
        assert_eq!(
            admit(command, Some(&r.host)),
            Some(r.host.ticket()),
            "{command}"
        );
        assert_eq!(admit(command, None), None, "{command}: no host");
    }
    for other in [
        "window_mog_open",
        "hotkeys_set",
        "view_emit",
        "consent_records_list",
    ] {
        assert_eq!(admit(other, Some(&r.host)), None, "{other}");
    }
    let before = r.host.ticket();
    r.call("capture_stop_all", json!({})).unwrap();
    assert_ne!(admit("capture_start", Some(&r.host)), Some(before));
}
