//! Tests for R2 CaptureService core (#4): one mic owner, leases, the three capture modes,
//! the locked 120 s hold and 5 min hands-free caps, ARC-F9 priority and the panic stop.
//! A fake clock and a fake device keep these tests free of real audio hardware.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use mog_core::capture::{
    AudioDevice, AudioSource, CaptureCore, CaptureError, CaptureEvents, CaptureRequest,
    CaptureService, Clock, DeviceError, StopReason, Stopped, HANDS_FREE_MAX_MS, HOLD_MAX_MS,
};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack};

// ---------- fakes ----------

#[derive(Default)]
struct FakeClock {
    mono_ms: AtomicU64,
}

impl FakeClock {
    fn advance(&self, ms: u64) {
        self.mono_ms.fetch_add(ms, Ordering::SeqCst);
    }
}

const EPOCH_BASE_MS: u64 = 1_800_000_000_000;

impl Clock for FakeClock {
    fn monotonic_ms(&self) -> u64 {
        self.mono_ms.load(Ordering::SeqCst)
    }
    fn epoch_ms(&self) -> u64 {
        EPOCH_BASE_MS + self.monotonic_ms()
    }
}

#[derive(Default)]
struct DeviceLog {
    mic_open: bool,
    system_open: bool,
    mic_opens: u32,
    system_opens: u32,
    fail_next_open: Option<&'static str>,
    panic_next_open: bool,
}

#[derive(Default, Clone)]
struct FakeDevice(Arc<Mutex<DeviceLog>>);

impl FakeDevice {
    fn log(&self) -> std::sync::MutexGuard<'_, DeviceLog> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl AudioDevice for FakeDevice {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        let mut log = self.log();
        if log.panic_next_open {
            log.panic_next_open = false;
            drop(log);
            panic!("fake device panic");
        }
        if let Some(code) = log.fail_next_open.take() {
            return Err(DeviceError { code });
        }
        match source {
            AudioSource::Mic => {
                assert!(!log.mic_open, "the mic must not open twice");
                log.mic_open = true;
                log.mic_opens += 1;
            }
            AudioSource::SystemAudio => {
                assert!(!log.system_open, "system audio must not open twice");
                log.system_open = true;
                log.system_opens += 1;
            }
        }
        Ok(())
    }

    fn close(&self, source: AudioSource) {
        let mut log = self.log();
        match source {
            AudioSource::Mic => log.mic_open = false,
            AudioSource::SystemAudio => log.system_open = false,
        }
    }
}

#[derive(Default)]
struct RecordingEvents {
    states: Mutex<Vec<CaptureState>>,
    stops: Mutex<Vec<Stopped>>,
}

impl CaptureEvents for RecordingEvents {
    fn state(&self, state: &CaptureState) {
        self.states.lock().unwrap().push(state.clone());
    }
    fn stopped(&self, stopped: &Stopped) {
        self.stops.lock().unwrap().push(stopped.clone());
    }
}

struct Rig {
    svc: CaptureCore,
    clock: Arc<FakeClock>,
    device: FakeDevice,
    events: Arc<RecordingEvents>,
}

fn rig() -> Rig {
    let clock = Arc::new(FakeClock::default());
    let device = FakeDevice::default();
    let events = Arc::new(RecordingEvents::default());
    let svc = CaptureCore::new(Box::new(device.clone()), clock.clone(), events.clone());
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
    r.events
        .stops
        .lock()
        .unwrap()
        .last()
        .cloned()
        .expect("a stop event")
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
    r.device.log().fail_next_open = Some("permission_denied");
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
        "recording",
        "paused",
        "sessionId",
        "startedAt",
        "directedTo",
        "othersTrack",
    ];
    let states = r.events.states.lock().unwrap();
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
    r.clock.advance(10_000);
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
fn cap_04a_caps_are_locked_at_120_s_and_5_min() {
    assert_eq!(HOLD_MAX_MS, 120_000);
    assert_eq!(HANDS_FREE_MAX_MS, 300_000);
}

#[test]
fn cap_04a_missing_key_up_stops_ptt_at_120_s() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    r.clock.advance(HOLD_MAX_MS - 1);
    r.svc.tick();
    assert!(r.svc.status().recording, "still recording at 119.999 s");
    r.clock.advance(1);
    r.svc.tick();
    assert!(!r.svc.status().recording);
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, lease.id);
    assert_eq!(stop.reason, StopReason::HoldCap);
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
    r.clock.advance(HOLD_MAX_MS + 5_000);
    assert!(!r.svc.status().recording);
    assert_eq!(last_stop(&r).reason, StopReason::HoldCap);
}

#[test]
fn cap_04a_tick_returns_the_next_deadline() {
    let r = rig();
    assert_eq!(r.svc.tick(), None, "no deadline when idle");
    r.clock.advance(1_000);
    r.svc.start(req("a", CaptureMode::Ptt)).unwrap();
    assert_eq!(r.svc.tick(), Some(1_000 + HOLD_MAX_MS));
}

#[test]
fn cap_04a_directed_ptt_in_a_session_stops_at_120_s_and_the_session_goes_on() {
    let r = rig();
    let session = r.svc.start(req("a", CaptureMode::Session)).unwrap();
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.clock.advance(HOLD_MAX_MS);
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
fn cap_04b_hands_free_stops_at_5_min() {
    let r = rig();
    let lease = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.clock.advance(HANDS_FREE_MAX_MS - 1);
    r.svc.tick();
    assert!(r.svc.status().recording);
    r.clock.advance(1);
    r.svc.tick();
    assert!(!r.svc.status().recording);
    assert!(!r.device.log().mic_open);
    let stop = last_stop(&r);
    assert_eq!(stop.lease_id, lease.id);
    assert_eq!(stop.reason, StopReason::HandsFreeCap);
    assert_eq!(stop.reason.code(), "hands_free_cap");
}

#[test]
fn cap_04b_suspended_hands_free_still_stops_at_5_min() {
    let r = rig();
    let hf = r.svc.start(req("a", CaptureMode::HandsFree)).unwrap();
    r.clock.advance(HANDS_FREE_MAX_MS - 10_000);
    let ptt = r.svc.start(req("b", CaptureMode::Ptt)).unwrap();
    r.clock.advance(10_000);
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
    r.clock.advance(HANDS_FREE_MAX_MS * 4);
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
    let stops = r.events.stops.lock().unwrap().clone();
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
    assert!(r.events.stops.lock().unwrap().is_empty());
}

// ---------- MOG-09 / UX-07: idle and owner ----------

#[test]
fn mog_09_an_idle_service_never_opens_the_mic() {
    let r = rig();
    r.clock.advance(3_600_000);
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
    let owners: Vec<Option<String>> = r
        .events
        .states
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.owner.clone())
        .collect();
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
