//! Tests for R2 CaptureService core (#4): one mic owner, leases, the three capture modes,
//! the locked 120 s hold and 5 min hands-free caps, ARC-F9 priority and the panic stop.
//! A fake clock and a fake device keep these tests free of real audio hardware.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, OnceLock, Weak};
use std::time::{Duration, Instant};

use mog_core::capture::{
    parse_lease, AudioDevice, AudioSource, CaptureCore, CaptureError, CaptureEvents,
    CaptureRequest, CaptureService, DeviceError, StopReason, Stopped, HANDS_FREE_MAX_MS,
    HOLD_MAX_MS,
};
use mog_core::clock::SimClock;
use mog_core::ipc::IpcError;
use mog_core::testkit::capture::{FakeAudioDevice, RecordingCaptureEvents};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack};

// ---------- rig: one SimClock (#229), the testkit fake device and event sink ----------

const EPOCH_BASE_MS: u64 = 1_800_000_000_000;

struct Rig {
    svc: CaptureCore,
    clock: SimClock,
    device: FakeAudioDevice,
    events: Arc<RecordingCaptureEvents>,
}

impl Rig {
    fn advance(&self, ms: u64) {
        self.clock.advance(Duration::from_millis(ms));
    }
}

fn rig() -> Rig {
    let clock = SimClock::new();
    let device = FakeAudioDevice::new();
    let events = Arc::new(RecordingCaptureEvents::default());
    let svc = CaptureCore::new(
        Box::new(device.clone()),
        Arc::new(clock.clone()),
        events.clone(),
    )
    .with_epoch_origin_ms(EPOCH_BASE_MS);
    Rig {
        svc,
        clock,
        device,
        events,
    }
}

fn req(mog: &str, mode: CaptureMode) -> CaptureRequest {
    CaptureRequest {
        mog_id: mog.to_string(),
        mode,
        others_track: false,
    }
}

fn last_stop(r: &Rig) -> Stopped {
    r.events.last_stop().expect("a stop event")
}

// ---------- CAP-01a: one owner, mic_busy ----------

#[test]
fn cap_01a_second_hands_free_request_gets_mic_busy() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let err = r.svc.start(req("b", CaptureMode::HandsFree)).unwrap_err();
    assert_eq!(err, CaptureError::MicBusy);
    assert_eq!(err.code(), "mic_busy");
    assert_eq!(r.svc.status().owner.as_deref(), Some("a"));
}

#[test]
fn cap_01a_ptt_during_ptt_gets_mic_busy() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(
        r.svc.start(req("b", CaptureMode::Ptt)).unwrap_err(),
        CaptureError::MicBusy
    );
    assert_eq!(
        r.svc.start(req("b", CaptureMode::HandsFree)).unwrap_err(),
        CaptureError::MicBusy
    );
    assert_eq!(
        r.svc.start(req("b", CaptureMode::Session)).unwrap_err(),
        CaptureError::MicBusy
    );
}

#[test]
fn cap_01a_second_session_gets_mic_busy() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Session)).unwrap();
    assert_eq!(
        r.svc.start(req("b", CaptureMode::Session)).unwrap_err(),
        CaptureError::MicBusy
    );
    assert_eq!(
        r.svc.start(req("b", CaptureMode::HandsFree)).unwrap_err(),
        CaptureError::MicBusy
    );
}

#[test]
fn cap_01a_one_mic_stream_serves_every_lease() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.stop(ptt.id).unwrap();
    r.svc.stop(hf.id).unwrap();
    let log = r.device.log();
    assert_eq!(log.mic_opens, 1);
    assert!(!log.mic_open);
}

#[test]
fn cap_01a_device_failure_leaves_no_lease() {
    let r = rig();
    r.device.log().fail_next_open = Some((None, "permission_denied"));
    let err = r.svc.start(req("a", CaptureMode::Ptt)).unwrap_err();
    assert_eq!(
        err,
        CaptureError::Device(DeviceError {
            code: "permission_denied"
        })
    );
    assert_eq!(err.code(), "permission_denied");
    assert!(!r.svc.status().recording);
    assert!(r.svc.status().owner.is_none());
    // The mic is free again.
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
}

#[test]
fn cap_01a_mic_is_released_after_a_panic_inside_the_service() {
    let r = rig();
    r.device.log().panic_next_open = true;
    let res = catch_unwind(AssertUnwindSafe(|| r.svc.start(req("a", CaptureMode::Ptt))));
    assert!(res.is_err(), "the fake device panics");
    // The service still works after the panic and the panic stop releases everything.
    r.svc.stop_all();
    assert!(!r.device.log().mic_open);
    assert!(!r.svc.status().recording);
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert!(r.device.log().mic_open);
}

#[test]
fn cap_01a_service_recovers_from_a_panic_without_stop_all() {
    let r = rig();
    r.device.log().panic_next_open = true;
    let res = catch_unwind(AssertUnwindSafe(|| r.svc.start(req("a", CaptureMode::Ptt))));
    assert!(res.is_err(), "the fake device panics");
    // The half-made lease is gone: the next caller gets the mic, not mic_busy. `start`
    // never returned that lease, so no stop event names it.
    assert!(!r.svc.status().recording);
    assert!(r.svc.status().owner.is_none());
    assert!(r.events.stops().is_empty());
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert!(r.device.log().mic_open);
}

#[test]
fn cap_01a_dropping_the_service_closes_every_stream() {
    let r = rig();
    r.svc
        .start(CaptureRequest {
            others_track: true,
            ..req("a", CaptureMode::Session)
        })
        .unwrap();
    let device = r.device.clone();
    drop(r);
    let log = device.log();
    assert!(!log.mic_open);
    assert!(!log.system_open);
}

// ---------- CAP-01b: no audio over IPC ----------

#[test]
fn cap_01b_state_events_carry_only_capture_state_fields() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.stop(ptt.id).unwrap();
    r.svc.stop(hf.id).unwrap();
    let allowed = [
        "owner",
        "mode",
        "leaseId",
        "recording",
        "paused",
        "sessionId",
        "startedAt",
        "directedTo",
        "othersTrack",
    ];
    let states = r.events.states();
    assert!(!states.is_empty());
    for s in states.iter() {
        let json = serde_json::to_value(s).unwrap();
        for (key, value) in json.as_object().unwrap() {
            assert!(allowed.contains(&key.as_str()), "unexpected field {key}");
            assert!(!value.is_array(), "no sample arrays in {key}");
        }
    }
}

// ---------- CAP-02a: leases ----------

#[test]
fn cap_02a_each_capture_gets_a_unique_lease_id() {
    let r = rig();
    let a = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.svc.stop(a.id).unwrap();
    let b = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_ne!(a.id, b.id);
    assert!(a.id > 0 && b.id > 0);
    assert_eq!(b.mog_id, "a");
    assert_eq!(b.mode, CaptureMode::Ptt);
}

#[test]
fn cap_02a_stop_needs_a_live_lease() {
    let r = rig();
    let a = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(
        r.svc.stop(a.id + 100).unwrap_err(),
        CaptureError::UnknownLease
    );
    assert!(r.svc.status().recording, "a wrong lease stops nothing");
    r.svc.stop(a.id).unwrap();
    let err = r.svc.stop(a.id).unwrap_err();
    assert_eq!(err, CaptureError::UnknownLease);
    assert_eq!(err.code(), "unknown_lease");
}

#[test]
fn cap_02a_capture_state_carries_the_lease_that_owns_the_mic() {
    let r = rig();
    assert_eq!(r.svc.status().lease_id, None, "no lease when idle");
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    assert_eq!(r.svc.status().lease_id, Some(hf.lease_id()));
    // ADR 0007: a PTT over a suspended hands-free shows the PTT lease.
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.svc.status().lease_id, Some(ptt.lease_id()));
    r.svc.stop(ptt.id).unwrap();
    assert_eq!(r.svc.status().lease_id, Some(hf.lease_id()));
    r.svc.stop(hf.id).unwrap();
    assert_eq!(r.svc.status().lease_id, None);
    // Every state event with an owner names a lease.
    for s in r.events.states().iter() {
        assert_eq!(s.owner.is_some(), s.lease_id.is_some());
    }
}

#[test]
fn cap_02a_directed_ptt_keeps_the_session_lease_in_the_state() {
    let r = rig();
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("a"));
    assert_eq!(s.lease_id, Some(session.lease_id()));
}

#[test]
fn cap_02a_wire_lease_ids_match_the_contract_and_parse_back() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    let wire = lease.lease_id();
    assert!(!wire.is_empty() && wire.len() <= 64);
    assert!(wire.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    assert_eq!(parse_lease(&wire), Ok(lease.id));
    for bad in ["", "abc", "-1", "0", "007", "+7", "99999999999999999999999"] {
        assert_eq!(parse_lease(bad), Err(CaptureError::UnknownLease), "{bad:?}");
    }
}

#[test]
fn cap_02a_stale_lease_maps_to_the_typed_ipc_error() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.svc.stop(lease.id).unwrap();
    let err = r.svc.stop(lease.id).unwrap_err();
    let ipc = err.to_ipc("capture_stop");
    assert_eq!(
        ipc,
        IpcError::UnknownLease {
            command: "capture_stop".into()
        }
    );
    let json = serde_json::to_value(&ipc).unwrap();
    assert_eq!(json["error"], err.code());
    let cmd = || "capture_pause".to_string();
    assert_eq!(
        CaptureError::MicBusy.to_ipc("capture_pause"),
        IpcError::MicBusy { command: cmd() }
    );
    assert_eq!(
        CaptureError::InvalidMode.to_ipc("capture_pause"),
        IpcError::InvalidMode { command: cmd() }
    );
    assert_eq!(
        CaptureError::Device(DeviceError { code: "no_device" }).to_ipc("capture_pause"),
        IpcError::HostFailed { command: cmd() }
    );
}

// ---------- CAP-03: three modes ----------

#[test]
fn cap_03_ptt_starts_and_stops() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    let s = r.svc.status();
    assert_eq!(s.mode, Some(CaptureMode::Ptt));
    assert!(s.recording);
    assert_eq!(s.started_at, Some(EPOCH_BASE_MS));
    assert!(r.device.log().mic_open);
    let after = r.svc.stop(lease.id).unwrap();
    assert!(!after.recording);
    assert!(after.owner.is_none());
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, lease.id);
    assert_eq!(stop.reason, StopReason::Manual);
}

#[test]
fn cap_03_hands_free_starts_and_stops() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    assert_eq!(r.svc.status().mode, Some(CaptureMode::HandsFree));
    assert!(r.device.log().mic_open);
    r.advance(10_000);
    r.svc.stop(lease.id).unwrap();
    assert!(!r.device.log().mic_open);
    assert_eq!(last_stop(&r).reason, StopReason::Manual);
}

#[test]
fn cap_03_session_starts_pauses_resumes_and_stops() {
    let r = rig();
    let lease = r
        .svc
        .start(CaptureRequest {
            others_track: true,
            ..req("a", CaptureMode::Session)
        })
        .unwrap();
    let s = r.svc.status();
    assert_eq!(s.mode, Some(CaptureMode::Session));
    assert!(s.session_id.is_some());
    assert_eq!(s.others_track, OthersTrack::Ok);
    assert!(r.device.log().system_open);

    let paused = r.svc.pause(lease.id).unwrap();
    assert!(paused.paused && !paused.recording);
    assert_eq!(paused.owner.as_deref(), Some("a"));

    let resumed = r.svc.resume(lease.id).unwrap();
    assert!(resumed.recording && !resumed.paused);
    assert!(r.device.log().mic_open);

    r.svc.stop(lease.id).unwrap();
    let log = r.device.log();
    assert!(!log.mic_open && !log.system_open);
}

#[test]
fn cap_03_session_without_others_track_leaves_system_audio_closed() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Session)).unwrap();
    assert_eq!(r.svc.status().others_track, OthersTrack::Off);
    assert!(!r.device.log().system_open);
}

#[test]
fn cap_03_others_track_is_only_for_sessions() {
    let r = rig();
    let err = r
        .svc
        .start(CaptureRequest {
            others_track: true,
            ..req("a", CaptureMode::HandsFree)
        })
        .unwrap_err();
    assert_eq!(err, CaptureError::InvalidMode);
    assert_eq!(err.code(), "invalid_mode");
    assert!(!r.device.log().mic_open);
}

#[test]
fn cap_03_only_a_session_can_pause() {
    let r = rig();
    let ptt = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.svc.pause(ptt.id).unwrap_err(), CaptureError::InvalidMode);
}

#[test]
fn cap_13b_pause_closes_the_mic_stream() {
    let r = rig();
    let lease = r
        .svc
        .start(CaptureRequest {
            others_track: true,
            ..req("a", CaptureMode::Session)
        })
        .unwrap();
    r.svc.pause(lease.id).unwrap();
    let log = r.device.log();
    assert!(!log.mic_open);
    assert!(!log.system_open);
}

// ---------- CAP-04a / CAP-04b: locked caps (AT-03) ----------

#[test]
fn cap_04a_caps_are_locked_after_120_s_and_5_min() {
    assert_eq!(HOLD_MAX_MS, 120_000);
    assert_eq!(HANDS_FREE_MAX_MS, 300_000);
}

#[test]
fn cap_04a_missing_key_up_stops_ptt_after_120_s() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.advance(HOLD_MAX_MS - 1);
    r.svc.tick();
    assert!(r.svc.status().recording, "still recording at 119.999 s");
    r.advance(1);
    r.svc.tick();
    // tick() did the stop: check the device and the event before status() runs.
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, lease.id);
    assert_eq!(stop.reason, StopReason::HoldCap);
    assert!(!r.svc.status().recording);
    assert_eq!(stop.reason.code(), "hold_cap");
    assert_eq!(
        r.svc.stop(lease.id).unwrap_err(),
        CaptureError::UnknownLease
    );
}

#[test]
fn cap_04a_status_enforces_the_cap_without_a_tick() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.advance(HOLD_MAX_MS + 5_000);
    assert!(!r.svc.status().recording);
    assert_eq!(last_stop(&r).reason, StopReason::HoldCap);
}

#[test]
fn cap_04a_tick_returns_the_next_deadline() {
    let r = rig();
    assert_eq!(r.svc.tick(), None, "no deadline when idle");
    r.advance(1_000);
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.svc.tick(), Some(1_000 + HOLD_MAX_MS));
}

#[test]
fn cap_04a_directed_ptt_in_a_session_stops_after_120_s_and_the_session_goes_on() {
    let r = rig();
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.advance(HOLD_MAX_MS);
    r.svc.tick();
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, ptt.id);
    assert_eq!(stop.reason, StopReason::HoldCap);
    let s = r.svc.status();
    assert!(s.recording);
    assert_eq!(s.owner.as_deref(), Some("a"));
    assert!(s.directed_to.is_none());
    r.svc.stop(session.id).unwrap();
}

#[test]
fn cap_04b_hands_free_stops_after_5_min() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.advance(HANDS_FREE_MAX_MS - 1);
    r.svc.tick();
    assert!(r.svc.status().recording);
    r.advance(1);
    r.svc.tick();
    // tick() did the stop: check the device and the event before status() runs.
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, lease.id);
    assert_eq!(stop.reason, StopReason::HandsFreeCap);
    assert!(!r.svc.status().recording);
    assert_eq!(stop.reason.code(), "hands_free_cap");
}

#[test]
fn cap_04b_suspended_hands_free_still_stops_after_5_min() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.advance(HANDS_FREE_MAX_MS - 10_000);
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.advance(10_000);
    r.svc.tick();
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, hf.id);
    assert_eq!(stop.reason, StopReason::HandsFreeCap);
    // The PTT keeps the mic.
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("b"));
    assert_eq!(s.mode, Some(CaptureMode::Ptt));
    assert!(r.device.log().mic_open);
    r.svc.stop(ptt.id).unwrap();
    assert!(!r.device.log().mic_open);
}

#[test]
fn cap_04b_session_has_no_hands_free_cap() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.advance(HANDS_FREE_MAX_MS * 4);
    assert_eq!(r.svc.tick(), None);
    assert!(r.svc.status().recording);
}

// ---------- ARC-F9: priority PTT > session > hands-free ----------

#[test]
fn arc_f9_ptt_pauses_another_mogs_hands_free_then_resumes_it() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("b"));
    assert_eq!(s.mode, Some(CaptureMode::Ptt));
    r.svc.stop(ptt.id).unwrap();
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("a"));
    assert_eq!(s.mode, Some(CaptureMode::HandsFree));
    assert!(s.recording);
    r.svc.stop(hf.id).unwrap();
}

#[test]
fn arc_f9_stopping_a_suspended_hands_free_keeps_the_ptt() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.stop(hf.id).unwrap();
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("b"));
    assert!(r.device.log().mic_open);
}

#[test]
fn arc_f9_ptt_in_a_session_is_a_directed_segment() {
    let r = rig();
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("a"));
    assert_eq!(s.mode, Some(CaptureMode::Session));
    assert_eq!(s.directed_to.as_deref(), Some("b"));
    assert_eq!(r.device.log().mic_opens, 1, "no second mic stream");
    r.svc.stop(ptt.id).unwrap();
    assert!(r.svc.status().directed_to.is_none());
    r.svc.stop(session.id).unwrap();
}

#[test]
fn arc_f9_directed_ptt_on_a_paused_session_opens_the_mic_only_for_the_ptt() {
    let r = rig();
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.svc.pause(session.id).unwrap();
    assert!(!r.device.log().mic_open);
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert!(r.device.log().mic_open);
    r.svc.stop(ptt.id).unwrap();
    assert!(!r.device.log().mic_open);
    assert!(r.svc.status().paused);
}

#[test]
fn arc_f9_session_preempts_hands_free() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let session = r.svc.start(req("b", CaptureMode::Session)).unwrap();
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, hf.id);
    assert_eq!(stop.reason, StopReason::Preempted);
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("b"));
    assert_eq!(s.mode, Some(CaptureMode::Session));
    assert_eq!(r.device.log().mic_opens, 1);
    assert_eq!(r.svc.stop(hf.id).unwrap_err(), CaptureError::UnknownLease);
    r.svc.stop(session.id).unwrap();
}

#[test]
fn arc_f9_ptt_over_hands_free_blocks_a_session() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert_eq!(
        r.svc.start(req("c", CaptureMode::Session)).unwrap_err(),
        CaptureError::MicBusy
    );
}

#[test]
fn arc_f9_second_ptt_during_a_directed_segment_gets_mic_busy() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert_eq!(
        r.svc.start(req("c", CaptureMode::Ptt)).unwrap_err(),
        CaptureError::MicBusy
    );
}

// ---------- MB-05: panic stop ----------

#[test]
fn mb_05_stop_all_stops_every_lease_and_closes_every_stream() {
    let r = rig();
    let session = r
        .svc
        .start(CaptureRequest {
            others_track: true,
            ..req("a", CaptureMode::Session)
        })
        .unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    let s = r.svc.stop_all();
    assert!(!s.recording && s.owner.is_none());
    let log = r.device.log();
    assert!(!log.mic_open && !log.system_open);
    drop(log);
    let stops = r.events.stops();
    let mut ids: Vec<u64> = stops
        .iter()
        .filter(|s| s.reason == StopReason::Panic)
        .map(|s| s.lease_id)
        .collect();
    ids.sort_unstable();
    assert_eq!(ids, vec![session.id, ptt.id]);
    assert_eq!(StopReason::Panic.code(), "panic");
}

#[test]
fn mb_05_stop_all_when_idle_is_harmless() {
    let r = rig();
    let s = r.svc.stop_all();
    assert!(!s.recording);
    assert!(r.events.stops().is_empty());
}

// ---------- MOG-09 / UX-07: idle and owner ----------

#[test]
fn mog_09_an_idle_service_never_opens_the_mic() {
    let r = rig();
    r.advance(3_600_000);
    r.svc.tick();
    let _ = r.svc.status();
    assert_eq!(r.device.log().mic_opens, 0);
    assert_eq!(r.device.log().system_opens, 0);
}

#[test]
fn ux_07_state_names_only_the_mog_that_owns_the_capture() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.stop(ptt.id).unwrap();
    r.svc.stop(hf.id).unwrap();
    let owners: Vec<Option<String>> = r.events.states().iter().map(|s| s.owner.clone()).collect();
    assert_eq!(
        owners,
        vec![
            Some("a".to_string()),
            Some("b".to_string()),
            Some("a".to_string()),
            None
        ]
    );
}

// ---------- review fixes: N2, N3, N4, N9 ----------

#[test]
fn arc_f9_a_failed_session_start_keeps_the_hands_free_capture() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.device.log().fail_next_open = Some((Some(AudioSource::SystemAudio), "no_device"));
    let err = r
        .svc
        .start(CaptureRequest {
            others_track: true,
            ..req("b", CaptureMode::Session)
        })
        .unwrap_err();
    assert_eq!(err, CaptureError::Device(DeviceError { code: "no_device" }));
    // The hands-free capture was not preempted and still owns the mic.
    assert!(r.events.stops().is_empty(), "no preempted stop");
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("a"));
    assert_eq!(s.mode, Some(CaptureMode::HandsFree));
    assert_eq!(s.lease_id, Some(hf.lease_id()));
    let log = r.device.log();
    assert!(log.mic_open && !log.system_open);
}

#[test]
fn mb_05_stop_all_does_not_wait_for_a_slow_device_open() {
    let r = Arc::new(rig());
    r.device.log().open_delay = Duration::from_millis(600);
    let starter = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.start(req("a", CaptureMode::Session)))
    };
    let waited = Instant::now();
    while !r.device.log().opening {
        assert!(waited.elapsed() < Duration::from_secs(5), "the open began");
        std::thread::yield_now();
    }
    // status() and stop_all() answer while the open is still running (MB-05: 200 ms).
    let t = Instant::now();
    assert!(!r.svc.status().recording);
    r.svc.stop_all();
    assert!(
        t.elapsed() < Duration::from_millis(200),
        "stop_all took {:?}",
        t.elapsed()
    );
    assert!(r.device.log().opening, "the slow open is still running");
    // The start ends without a lease, and the stream that opened late is closed.
    assert_eq!(starter.join().unwrap(), Err(CaptureError::UnknownLease));
    assert!(!r.device.log().mic_open);
    assert!(!r.svc.status().recording);
    r.device.log().open_delay = Duration::ZERO;
    r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
}

/// A sink that reads the service back while it handles an event.
#[derive(Default)]
struct ReentrantEvents {
    svc: OnceLock<Weak<CaptureCore>>,
    seen: std::sync::Mutex<Vec<Option<String>>>,
}

impl CaptureEvents for ReentrantEvents {
    fn state(&self, _state: &CaptureState) {
        if let Some(svc) = self.svc.get().and_then(Weak::upgrade) {
            self.seen.lock().unwrap().push(svc.status().owner);
        }
    }
    fn stopped(&self, _stopped: &Stopped) {}
}

#[test]
fn cap_01a_events_are_sent_without_the_state_lock() {
    let events = Arc::new(ReentrantEvents::default());
    let svc = Arc::new(CaptureCore::new(
        Box::new(FakeAudioDevice::new()),
        Arc::new(SimClock::new()),
        events.clone(),
    ));
    events.svc.set(Arc::downgrade(&svc)).ok();
    let lease = svc.start(req("a", CaptureMode::Ptt)).unwrap();
    svc.stop(lease.id).unwrap();
    assert_eq!(
        *events.seen.lock().unwrap(),
        vec![Some("a".to_string()), None]
    );
}

#[test]
fn cap_01a_a_lost_mic_stops_the_capture_and_the_next_start_reopens_it() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.device_failed(AudioSource::Mic);
    let mut stopped: Vec<(u64, StopReason)> = r
        .events
        .stops()
        .iter()
        .map(|s| (s.lease_id, s.reason))
        .collect();
    stopped.sort_unstable_by_key(|s| s.0);
    assert_eq!(
        stopped,
        vec![
            (hf.id, StopReason::DeviceLost),
            (ptt.id, StopReason::DeviceLost)
        ]
    );
    assert_eq!(StopReason::DeviceLost.code(), "device_lost");
    assert!(!r.device.log().mic_open);
    assert!(!r.svc.status().recording);
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.device.log().mic_opens, 2, "the mic opens again");
}

#[test]
fn cap_01a_lost_system_audio_stops_only_the_session_that_uses_it() {
    let r = rig();
    let session = r
        .svc
        .start(CaptureRequest {
            others_track: true,
            ..req("a", CaptureMode::Session)
        })
        .unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.device_failed(AudioSource::SystemAudio);
    let stop = last_stop(&r);
    assert_eq!(
        (stop.lease_id, stop.reason),
        (session.id, StopReason::DeviceLost)
    );
    // The directed PTT keeps the mic.
    let s = r.svc.status();
    assert_eq!(s.lease_id, Some(ptt.lease_id()));
    assert_eq!(s.others_track, OthersTrack::Off);
    assert!(r.device.log().mic_open && !r.device.log().system_open);
}

#[test]
fn arc_f9_stopping_a_session_turns_its_directed_ptt_into_a_plain_ptt() {
    let r = rig();
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.svc.stop(session.id).unwrap();
    let s = r.svc.status();
    assert_eq!(s.owner.as_deref(), Some("b"));
    assert_eq!(s.mode, Some(CaptureMode::Ptt));
    assert_eq!(s.lease_id, Some(ptt.lease_id()));
    assert!(s.directed_to.is_none() && s.session_id.is_none());
    assert!(r.device.log().mic_open);
    assert_eq!(r.device.log().mic_opens, 1, "the stream stays open");
    // It keeps its own 120 s cap.
    r.advance(HOLD_MAX_MS);
    r.svc.tick();
    assert_eq!(last_stop(&r).reason, StopReason::HoldCap);
}

// ---------- re-review fixes: R1, N11 ----------

#[test]
fn cap_01a_a_start_after_stop_all_waits_for_the_cancelled_open() {
    let r = Arc::new(rig());
    r.device.log().open_delay = Duration::from_millis(400);
    let starter = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.start(req("a", CaptureMode::Ptt)))
    };
    let waited = Instant::now();
    while !r.device.log().opening {
        assert!(waited.elapsed() < Duration::from_secs(5), "the open began");
        std::thread::yield_now();
    }
    r.svc.stop_all();
    // A's open still runs: B must not open the mic beside it.
    assert_eq!(
        r.svc.start(req("b", CaptureMode::Ptt)).unwrap_err(),
        CaptureError::MicBusy
    );
    assert_eq!(starter.join().unwrap(), Err(CaptureError::UnknownLease));
    {
        let log = r.device.log();
        assert_eq!(log.max_in_flight, 1, "one open at a time");
        assert!(!log.mic_open, "the late stream of A is closed");
    }
    // After A's open returned, B gets the mic, and a final stop_all leaves nothing open.
    r.device.log().open_delay = Duration::ZERO;
    let b = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.svc.status().lease_id, Some(b.lease_id()));
    r.svc.stop_all();
    let log = r.device.log();
    assert!(!log.mic_open && !log.system_open);
    assert_eq!(log.max_in_flight, 1);
}

/// A device whose mic fails while (or right after) it opens, before the lease is granted.
struct FailsDuringOpen {
    inner: FakeAudioDevice,
    svc: OnceLock<Weak<CaptureCore>>,
    armed: std::sync::atomic::AtomicBool,
}

impl AudioDevice for FailsDuringOpen {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        self.inner.open(source)?;
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            if let Some(svc) = self.svc.get().and_then(Weak::upgrade) {
                svc.device_failed(source);
            }
        }
        Ok(())
    }
    fn close(&self, source: AudioSource) {
        self.inner.close(source);
    }
}

#[test]
fn cap_01a_a_device_failure_before_the_grant_gives_no_lease() {
    let fake = FakeAudioDevice::new();
    let device = Arc::new(FailsDuringOpen {
        inner: fake.clone(),
        svc: OnceLock::new(),
        armed: std::sync::atomic::AtomicBool::new(true),
    });
    struct Shared(Arc<FailsDuringOpen>);
    impl AudioDevice for Shared {
        fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
            self.0.open(source)
        }
        fn close(&self, source: AudioSource) {
            self.0.close(source);
        }
    }
    let events = Arc::new(RecordingCaptureEvents::default());
    let svc = Arc::new(CaptureCore::new(
        Box::new(Shared(Arc::clone(&device))),
        Arc::new(SimClock::new()),
        events.clone(),
    ));
    device.svc.set(Arc::downgrade(&svc)).ok();
    let err = svc.start(req("a", CaptureMode::Ptt)).unwrap_err();
    assert_eq!(
        err,
        CaptureError::Device(DeviceError {
            code: "device_lost"
        })
    );
    assert!(!svc.status().recording, "no lease on a closed stream");
    assert!(!fake.log().mic_open);
    assert!(events.stops().is_empty(), "no lease was granted");
    // The next start opens the mic again.
    svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert!(fake.log().mic_open);
    assert_eq!(fake.log().mic_opens, 2);
}

// ---------- round 3: N14 ----------

/// A sink that panics once, on the first stop event after it is armed.
#[derive(Default)]
struct PanicsOnStop {
    armed: std::sync::atomic::AtomicBool,
}

impl CaptureEvents for PanicsOnStop {
    fn state(&self, _state: &CaptureState) {}
    fn stopped(&self, _stopped: &Stopped) {
        if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
            panic!("sink panic");
        }
    }
}

fn panicking_sink_rig() -> (CaptureCore, SimClock, FakeAudioDevice, Arc<PanicsOnStop>) {
    let clock = SimClock::new();
    let device = FakeAudioDevice::new();
    let events = Arc::new(PanicsOnStop::default());
    let svc = CaptureCore::new(
        Box::new(device.clone()),
        Arc::new(clock.clone()),
        events.clone(),
    );
    (svc, clock, device, events)
}

#[test]
fn cap_01a_a_sink_panic_during_start_leaves_the_core_usable() {
    let (svc, clock, device, events) = panicking_sink_rig();
    svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    clock.advance(Duration::from_millis(HANDS_FREE_MAX_MS + 1));
    events
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // The start expires the hands-free lease first; the sink panics on that stop event.
    let res = catch_unwind(AssertUnwindSafe(|| svc.start(req("b", CaptureMode::Ptt))));
    assert!(res.is_err(), "the sink panics");
    let lease = svc.start(req("b", CaptureMode::Ptt)).unwrap();
    assert_eq!(svc.status().lease_id, Some(lease.lease_id()));
    svc.stop_all();
    assert!(!device.log().mic_open);
    svc.start(req("c", CaptureMode::Ptt)).unwrap();
    assert!(device.log().mic_open);
}

#[test]
fn cap_01a_a_sink_panic_during_resume_leaves_the_core_usable() {
    let (svc, clock, device, events) = panicking_sink_rig();
    let session = svc.start(req("a", CaptureMode::Session)).unwrap();
    svc.pause(session.id).unwrap();
    svc.start(req("b", CaptureMode::Ptt)).unwrap();
    clock.advance(Duration::from_millis(HOLD_MAX_MS));
    events
        .armed
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // The resume expires the directed PTT first; the sink panics on that stop event.
    let res = catch_unwind(AssertUnwindSafe(|| svc.resume(session.id)));
    assert!(res.is_err(), "the sink panics");
    assert!(svc.status().paused, "the session stays paused");
    svc.resume(session.id).unwrap();
    assert!(svc.status().recording);
    assert!(device.log().mic_open);
    svc.stop_all();
    svc.start(req("c", CaptureMode::Ptt)).unwrap();
}

// ---------- round 4: R2 (grant on a closed mic) ----------

#[test]
fn cap_01a_a_start_during_a_close_is_granted_only_on_an_open_mic() {
    let r = Arc::new(rig());
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    r.device.log().close_gate = Some(gate);
    // Thread X stops the only lease; its mic close waits at the gate.
    let stopper = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.stop(hf.id).map(|_| ()))
    };
    let waited = Instant::now();
    while !r.device.log().closing {
        assert!(waited.elapsed() < Duration::from_secs(5), "the close began");
        std::thread::yield_now();
    }
    // Thread Y starts while the close is in progress.
    let starter = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.start(req("b", CaptureMode::Ptt)))
    };
    std::thread::sleep(Duration::from_millis(50));
    release.send(()).unwrap();
    stopper.join().unwrap().unwrap();
    let lease = starter.join().unwrap().unwrap();
    // The granted lease has an open mic: Y opened it again after X's close.
    let log = r.device.log();
    assert!(log.mic_open, "the lease of B is on an open mic");
    assert_eq!(log.mic_opens, 2);
    drop(log);
    assert_eq!(r.svc.status().lease_id, Some(lease.lease_id()));
}

#[test]
fn cap_01a_stress_every_granted_lease_has_an_open_mic() {
    let r = Arc::new(rig());
    let workers: Vec<_> = (0..3)
        .map(|t| {
            let r = Arc::clone(&r);
            std::thread::spawn(move || {
                let modes = [CaptureMode::HandsFree, CaptureMode::Ptt, CaptureMode::Ptt];
                let mut grants = 0u32;
                for i in 0..10_000u32 {
                    let mode = modes[((i as usize) + t) % 3];
                    if let Ok(lease) = r.svc.start(req(&format!("m{t}"), mode)) {
                        grants += 1;
                        // Only this thread stops its lease, and no cap runs: the mic must be open.
                        assert!(
                            r.device.log().mic_open,
                            "lease {} on a closed mic",
                            lease.id
                        );
                        r.svc.stop(lease.id).unwrap();
                    }
                }
                grants
            })
        })
        .collect();
    let poller = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || {
            for _ in 0..10_000 {
                r.svc.tick();
                r.svc.status();
            }
        })
    };
    let grants: u32 = workers.into_iter().map(|w| w.join().unwrap()).sum();
    poller.join().unwrap();
    assert!(grants > 0);
    // No lease is left, so no stream is left open.
    let s = r.svc.status();
    assert!(s.owner.is_none());
    let log = r.device.log();
    assert!(
        !log.mic_open && !log.system_open,
        "no stream open with no lease"
    );
    assert_eq!(log.max_in_flight, 1, "one open at a time");
}

// ---------- round 5: one commit path (sync before events), device loss, resume ----------

use std::sync::atomic::AtomicBool as Flag;
use std::sync::atomic::{AtomicU64, Ordering as AtOrd};

/// A host-like sink: it can restart capture on `device_lost`, or panic on one kind of event.
#[derive(Default)]
struct HostSink {
    svc: OnceLock<Weak<CaptureCore>>,
    restart_on_lost: Flag,
    restarted: std::sync::Mutex<Option<Result<mog_core::capture::Lease, CaptureError>>>,
    panic_on_stop: std::sync::Mutex<Option<StopReason>>,
    panic_on_paused_state: Flag,
    panic_on_recording_state: Flag,
    panic_every_nth_stop: AtomicU64,
    stops: AtomicU64,
}

impl CaptureEvents for HostSink {
    fn state(&self, state: &CaptureState) {
        if state.paused && self.panic_on_paused_state.swap(false, AtOrd::SeqCst) {
            panic!("sink panic on a paused state");
        }
        if state.recording && self.panic_on_recording_state.swap(false, AtOrd::SeqCst) {
            panic!("sink panic on a recording state");
        }
    }
    fn stopped(&self, stopped: &Stopped) {
        let n = self.stops.fetch_add(1, AtOrd::SeqCst) + 1;
        let every = self.panic_every_nth_stop.load(AtOrd::SeqCst);
        if every > 0 && n % every == 0 {
            panic!("sink panic on stop {n}");
        }
        let once = self
            .panic_on_stop
            .lock()
            .unwrap()
            .take_if(|r| *r == stopped.reason);
        if once.is_some() {
            panic!("sink panic on {:?}", stopped.reason);
        }
        if stopped.reason == StopReason::DeviceLost
            && self.restart_on_lost.swap(false, AtOrd::SeqCst)
        {
            if let Some(svc) = self.svc.get().and_then(Weak::upgrade) {
                *self.restarted.lock().unwrap() = Some(svc.start(req("a", CaptureMode::HandsFree)));
            }
        }
    }
}

fn host_rig() -> (Arc<CaptureCore>, FakeAudioDevice, Arc<HostSink>, SimClock) {
    let clock = SimClock::new();
    let device = FakeAudioDevice::new();
    let sink = Arc::new(HostSink::default());
    let svc = Arc::new(CaptureCore::new(
        Box::new(device.clone()),
        Arc::new(clock.clone()),
        sink.clone(),
    ));
    sink.svc.set(Arc::downgrade(&svc)).ok();
    (svc, device, sink, clock)
}

fn wait_for(what: &str, cond: impl Fn() -> bool) {
    let t = Instant::now();
    while !cond() {
        assert!(t.elapsed() < Duration::from_secs(5), "waited for {what}");
        std::thread::yield_now();
    }
}

#[test]
fn cap_01a_a_sink_that_restarts_on_device_lost_gets_an_open_mic() {
    let (svc, device, sink, _) = host_rig();
    svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    sink.restart_on_lost.store(true, AtOrd::SeqCst);
    svc.device_failed(AudioSource::Mic);
    let lease = sink
        .restarted
        .lock()
        .unwrap()
        .clone()
        .expect("the sink restarted capture")
        .expect("the restart is granted");
    assert_eq!(svc.status().lease_id, Some(lease.lease_id()));
    let log = device.log();
    assert!(log.mic_open, "the restarted lease has an open mic");
    assert_eq!(log.mic_opens, 2, "the dead stream was not reused");
}

#[test]
fn cap_01a_device_failed_against_a_start_loop_never_leaves_a_lease_on_a_closed_mic() {
    for run in 0..300 {
        let r = Arc::new(rig());
        r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
        let failer = {
            let r = Arc::clone(&r);
            std::thread::spawn(move || r.svc.device_failed(AudioSource::Mic))
        };
        let starter = {
            let r = Arc::clone(&r);
            std::thread::spawn(move || {
                for _ in 0..200 {
                    if r.svc.start(req("b", CaptureMode::HandsFree)).is_ok() {
                        return;
                    }
                    std::thread::yield_now();
                }
            })
        };
        failer.join().unwrap();
        starter.join().unwrap();
        if r.svc.status().recording {
            assert!(
                r.device.log().mic_open,
                "run {run}: a live lease on a closed mic"
            );
        }
    }
}

#[test]
fn cap_13b_a_sink_panic_in_stop_all_stop_or_pause_still_closes_the_mic() {
    // stop_all
    let (svc, device, sink, _) = host_rig();
    svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    *sink.panic_on_stop.lock().unwrap() = Some(StopReason::Panic);
    assert!(catch_unwind(AssertUnwindSafe(|| svc.stop_all())).is_err());
    assert!(
        !device.log().mic_open,
        "stop_all: the mic closed before the event"
    );
    // stop
    let (svc, device, sink, _) = host_rig();
    let hf = svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    *sink.panic_on_stop.lock().unwrap() = Some(StopReason::Manual);
    assert!(catch_unwind(AssertUnwindSafe(|| svc.stop(hf.id))).is_err());
    assert!(
        !device.log().mic_open,
        "stop: the mic closed before the event"
    );
    // pause
    let (svc, device, sink, _) = host_rig();
    let session = svc.start(req("a", CaptureMode::Session)).unwrap();
    sink.panic_on_paused_state.store(true, AtOrd::SeqCst);
    assert!(catch_unwind(AssertUnwindSafe(|| svc.pause(session.id))).is_err());
    assert!(
        !device.log().mic_open,
        "pause: the mic closed before the event"
    );
}

#[test]
fn cap_01a_the_unwind_guard_does_not_lose_a_concurrent_stop() {
    let r = Arc::new(rig());
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let (release, gate) = std::sync::mpsc::channel();
    {
        let mut log = r.device.log();
        log.panic_next_open = true;
        log.close_gate = Some(gate);
    }
    let starter = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || {
            catch_unwind(AssertUnwindSafe(|| {
                r.svc.start(CaptureRequest {
                    others_track: true,
                    ..req("b", CaptureMode::Session)
                })
            }))
            .is_err()
        })
    };
    // The system-audio open panics; the guard's close waits at the gate.
    wait_for("the guard's close", || r.device.log().closing);
    r.svc.stop(hf.id).unwrap();
    release.send(()).unwrap();
    assert!(starter.join().unwrap(), "the open panicked");
    let log = r.device.log();
    assert!(
        !log.mic_open && !log.system_open,
        "no stream left with no lease"
    );
}

#[test]
fn cap_13b_a_resume_stays_paused_until_its_mic_opens_and_a_device_loss_keeps_the_session() {
    let r = Arc::new(rig());
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.svc.pause(session.id).unwrap();
    r.device.log().open_delay = Duration::from_millis(300);
    let resumer = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.resume(session.id))
    };
    wait_for("the resume's open", || r.device.log().opening);
    let s = r.svc.status();
    assert!(
        s.paused && !s.recording,
        "no recording before the mic opens"
    );
    r.svc.device_failed(AudioSource::Mic);
    assert_eq!(
        resumer.join().unwrap(),
        Err(CaptureError::Device(DeviceError {
            code: "device_lost"
        }))
    );
    let s = r.svc.status();
    assert_eq!(s.lease_id, Some(session.lease_id()), "the session is kept");
    assert!(s.paused);
    assert!(!r.device.log().mic_open);
    r.device.log().open_delay = Duration::ZERO;
    assert!(r.svc.resume(session.id).unwrap().recording);
}

#[test]
fn cap_02a_a_stop_during_a_resume_makes_the_resume_unknown_lease() {
    let r = Arc::new(rig());
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.svc.pause(session.id).unwrap();
    r.device.log().open_delay = Duration::from_millis(200);
    let resumer = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.resume(session.id))
    };
    wait_for("the resume's open", || r.device.log().opening);
    r.svc.stop(session.id).unwrap();
    assert_eq!(resumer.join().unwrap(), Err(CaptureError::UnknownLease));
    assert!(!r.device.log().mic_open);
}

#[test]
fn mb_05_stop_all_recovers_a_lease_whose_grant_event_panicked() {
    let (svc, device, sink, _) = host_rig();
    sink.panic_on_recording_state.store(true, AtOrd::SeqCst);
    assert!(catch_unwind(AssertUnwindSafe(
        || svc.start(req("a", CaptureMode::Session))
    ))
    .is_err());
    // The caller never got the lease ID, but the lease exists: stop_all is the recovery.
    assert!(svc.status().recording);
    svc.stop_all();
    assert!(!device.log().mic_open);
    svc.start(req("b", CaptureMode::Ptt)).unwrap();
}

#[test]
fn cap_01a_stress_with_pause_resume_device_loss_and_sink_panics_ends_consistent() {
    let (svc, device, sink, clock) = host_rig();
    sink.panic_every_nth_stop.store(97, AtOrd::SeqCst);
    let call = |f: &dyn Fn()| {
        let _ = catch_unwind(AssertUnwindSafe(f));
    };
    let workers: Vec<_> = (0..3u64)
        .map(|t| {
            let svc = Arc::clone(&svc);
            std::thread::spawn(move || {
                let modes = [
                    CaptureMode::HandsFree,
                    CaptureMode::Ptt,
                    CaptureMode::Session,
                ];
                for i in 0..2_000u64 {
                    let mode = modes[((i + t) % 3) as usize];
                    let svc2 = Arc::clone(&svc);
                    let got = catch_unwind(AssertUnwindSafe(move || {
                        svc2.start(req(&format!("m{t}"), mode))
                    }));
                    if let Ok(Ok(lease)) = got {
                        if mode == CaptureMode::Session {
                            let _ = catch_unwind(AssertUnwindSafe(|| svc.pause(lease.id)));
                            let _ = catch_unwind(AssertUnwindSafe(|| svc.resume(lease.id)));
                        }
                        let _ = catch_unwind(AssertUnwindSafe(|| svc.stop(lease.id)));
                    }
                }
            })
        })
        .collect();
    for i in 0..2_000u32 {
        call(&|| {
            svc.tick();
        });
        call(&|| {
            svc.status();
        });
        if i % 50 == 0 {
            call(&|| svc.device_failed(AudioSource::Mic));
            clock.advance(Duration::from_millis(1_000));
        }
    }
    for w in workers {
        w.join().unwrap();
    }
    sink.panic_every_nth_stop.store(0, AtOrd::SeqCst);
    // Quiescent: a live lease has an open mic.
    if svc.status().recording {
        assert!(device.log().mic_open, "a live lease on a closed mic");
    }
    // After stop_all nothing is open, no open slot is stuck, and no open ran twice at once.
    svc.stop_all();
    {
        let log = device.log();
        assert!(!log.mic_open && !log.system_open, "a stream with no lease");
        assert_eq!(log.max_in_flight, 1);
    }
    svc.start(req("z", CaptureMode::Ptt)).unwrap();
    assert!(device.log().mic_open);
}

// ---------- round 6: a pause during a resume wins (CAP-13b) ----------

#[test]
fn cap_13b_a_pause_during_a_resume_keeps_the_session_paused() {
    let r = Arc::new(rig());
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    r.svc.pause(session.id).unwrap();
    r.device.log().open_delay = Duration::from_millis(300);
    let resumer = {
        let r = Arc::clone(&r);
        std::thread::spawn(move || r.svc.resume(session.id))
    };
    wait_for("the resume's open", || r.device.log().opening);
    let paused = r.svc.pause(session.id).unwrap();
    assert!(paused.paused && !paused.recording);
    // The resume ends without making the session live: the user's pause wins.
    let resumed = resumer.join().unwrap().unwrap();
    assert!(resumed.paused && !resumed.recording);
    let s = r.svc.status();
    assert!(s.paused && !s.recording, "the session stays paused");
    assert!(!r.device.log().mic_open, "the mic is closed");
    // A later resume works.
    r.device.log().open_delay = Duration::ZERO;
    assert!(r.svc.resume(session.id).unwrap().recording);
}

// ---------- #14: the PTT and hands-free lifecycle rules in R2 ----------
//
// "200 ms or less" (CAP-14b): every stop below runs on the sim clock with no advance between
// the trigger and the check, so the mic is closed 0 ms after the trigger. A cap or VAD end
// fires at the exact deadline that `tick()` returns; the host timer calls `tick()` then
// (TODO(#425)).

use mog_core::capture::{HandsFreeStop, DEFAULT_VAD_END_MS, POST_ROLL_MS};

#[test]
fn cap_12a_key_down_lease_opens_the_mic_at_once() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    // No advance: the mic is live when `start` returns (NFR-02 budget left to the device).
    assert!(r.device.log().mic_open);
    let state = r.events.states().last().cloned().unwrap();
    assert!(state.recording);
    assert_eq!(state.mode, Some(CaptureMode::Ptt));
}

#[test]
fn cap_12b_release_keeps_the_mic_open_for_the_120_ms_post_roll() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.advance(1_000);
    let deadline = r.svc.release(lease.id).unwrap();
    assert_eq!(POST_ROLL_MS, 120);
    assert_eq!(deadline, 1_000 + POST_ROLL_MS);
    assert_eq!(r.svc.tick(), Some(1_000 + POST_ROLL_MS));
    r.advance(POST_ROLL_MS - 1);
    r.svc.tick();
    assert!(r.device.log().mic_open, "the last word is still recorded");
    assert!(r.svc.status().recording);
    r.advance(1);
    assert_eq!(r.svc.tick(), None);
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(
        (stop.lease_id, stop.reason),
        (lease.id, StopReason::Released)
    );
    assert!(!stop.reason.discards_audio());
    assert_eq!(StopReason::Released.code(), "released");
}

#[test]
fn cap_12b_a_second_release_does_not_move_the_post_roll() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.svc.release(lease.id).unwrap(), POST_ROLL_MS);
    r.advance(100);
    assert_eq!(r.svc.release(lease.id).unwrap(), POST_ROLL_MS);
}

#[test]
fn cap_12b_release_is_only_for_ptt() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    assert_eq!(r.svc.release(hf.id).unwrap_err(), CaptureError::InvalidMode);
    assert_eq!(r.svc.release(999).unwrap_err(), CaptureError::UnknownLease);
}

#[test]
fn cap_12b_post_roll_of_a_ptt_over_hands_free_keeps_hands_free() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    let ptt = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.svc.release(ptt.id).unwrap();
    r.advance(POST_ROLL_MS);
    r.svc.tick();
    let state = r.svc.status();
    assert_eq!(state.lease_id, Some(hf.id.to_string()));
    assert_eq!(state.mode, Some(CaptureMode::HandsFree));
    assert!(r.device.log().mic_open);
}

#[test]
fn cap_44b_discard_stops_the_tap_at_once_and_drops_its_audio() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.advance(100);
    let state = r.svc.discard(lease.id).unwrap();
    assert!(!state.recording);
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(
        (stop.lease_id, stop.reason),
        (lease.id, StopReason::Discarded)
    );
    // The segment pipeline (#116) drops the audio of a lease that stops with this reason.
    assert!(stop.reason.discards_audio());
    assert_eq!(StopReason::Discarded.code(), "discarded");
    assert!(!StopReason::Manual.discards_audio());
}

#[test]
fn cap_44b_discard_during_the_post_roll_still_drops_the_audio() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.svc.release(lease.id).unwrap();
    r.svc.discard(lease.id).unwrap();
    assert_eq!(last_stop(&r).reason, StopReason::Discarded);
    assert_eq!(
        r.svc.discard(lease.id).unwrap_err(),
        CaptureError::UnknownLease
    );
}

#[test]
fn cap_14b_a_tap_stops_hands_free_and_keeps_its_audio() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.advance(30_000);
    let state = r.svc.stop_hands_free(HandsFreeStop::Tap);
    assert!(!state.recording);
    assert!(
        !r.device.log().mic_open,
        "closed 0 ms after the tap (<= 200 ms)"
    );
    let stop = last_stop(&r);
    assert_eq!((stop.lease_id, stop.reason), (hf.id, StopReason::Tap));
    assert!(!stop.reason.discards_audio());
}

#[test]
fn cap_14b_esc_stops_hands_free_and_drops_its_audio() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.advance(30_000);
    r.svc.stop_hands_free(HandsFreeStop::Escape);
    assert!(
        !r.device.log().mic_open,
        "closed 0 ms after Esc (<= 200 ms)"
    );
    let stop = last_stop(&r);
    assert_eq!((stop.lease_id, stop.reason), (hf.id, StopReason::Escape));
    assert!(stop.reason.discards_audio());
}

#[test]
fn cap_14b_stop_hands_free_leaves_ptt_and_sessions_alone() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert!(r.svc.stop_hands_free(HandsFreeStop::Escape).recording);
    let r = rig();
    r.svc.start(req("a", CaptureMode::Session)).unwrap();
    assert!(r.svc.stop_hands_free(HandsFreeStop::Tap).recording);
    assert!(r.events.stops().is_empty());
}

#[test]
fn cap_14b_the_5_min_cap_closes_the_mic_at_its_deadline() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    assert_eq!(r.svc.tick(), Some(HANDS_FREE_MAX_MS));
    r.advance(HANDS_FREE_MAX_MS);
    r.svc.tick();
    assert!(
        !r.device.log().mic_open,
        "closed 0 ms after the cap (<= 200 ms)"
    );
    assert_eq!(
        (last_stop(&r).lease_id, last_stop(&r).reason),
        (hf.id, StopReason::HandsFreeCap)
    );
}

#[test]
fn cap_14c_hands_free_stops_after_1500_ms_of_silence_by_default() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    assert_eq!(DEFAULT_VAD_END_MS, 1_500);
    r.advance(500);
    r.svc.voice_activity(true);
    r.advance(2_000);
    // Speech ends at 2,500 ms: the VAD end is 1,500 ms later.
    assert_eq!(r.svc.voice_activity(false), Some(4_000));
    r.advance(1_499);
    r.svc.tick();
    assert!(r.svc.status().recording);
    r.advance(1);
    r.svc.tick();
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!((stop.lease_id, stop.reason), (hf.id, StopReason::VadEnd));
    assert!(!stop.reason.discards_audio());
    assert_eq!(StopReason::VadEnd.code(), "vad_end");
}

#[test]
fn cap_14c_speech_again_resets_the_silence() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.svc.voice_activity(true);
    r.svc.voice_activity(false);
    r.advance(1_000);
    r.svc.voice_activity(true);
    r.advance(1_000);
    r.svc.tick();
    assert!(r.svc.status().recording);
    r.svc.voice_activity(false);
    r.advance(1_500);
    r.svc.tick();
    assert!(!r.svc.status().recording);
}

#[test]
fn cap_14c_no_vad_end_before_the_first_speech() {
    // ADR: the silence counts only after speech, so a user who thinks first keeps the mic.
    let r = rig();
    r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.svc.voice_activity(false);
    r.advance(10_000);
    r.svc.tick();
    assert!(r.svc.status().recording);
    assert_eq!(r.svc.tick(), Some(HANDS_FREE_MAX_MS));
}

#[test]
fn cap_14c_zero_turns_the_vad_end_off() {
    let r = rig();
    r.svc.set_vad_end_ms("a", 0);
    r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.svc.voice_activity(true);
    r.svc.voice_activity(false);
    r.advance(60_000);
    r.svc.tick();
    assert!(r.svc.status().recording);
    assert_eq!(r.svc.tick(), Some(HANDS_FREE_MAX_MS));
}

#[test]
fn cap_14c_the_vad_end_is_per_mog() {
    let r = rig();
    r.svc.set_vad_end_ms("b", 3_000);
    r.svc.start(req("b", CaptureMode::HandsFree)).unwrap();
    r.svc.voice_activity(true);
    assert_eq!(r.svc.voice_activity(false), Some(3_000));
}

#[test]
fn cap_14c_no_vad_end_while_a_ptt_holds_the_mic_and_the_silence_restarts_after_it() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.svc.voice_activity(true);
    r.svc.voice_activity(false);
    let ptt = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.advance(5_000);
    r.svc.tick();
    assert!(r.svc.status().recording);
    r.svc.stop(ptt.id).unwrap();
    // The silence counts again from the end of the PTT.
    r.advance(1_499);
    r.svc.tick();
    assert_eq!(r.svc.status().mode, Some(CaptureMode::HandsFree));
    r.advance(1);
    r.svc.tick();
    assert!(!r.svc.status().recording);
}

#[test]
fn cap_14c_voice_activity_without_hands_free_does_nothing() {
    let r = rig();
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.svc.voice_activity(true);
    r.svc.voice_activity(false);
    r.advance(5_000);
    r.svc.tick();
    assert!(r.svc.status().recording);
}
