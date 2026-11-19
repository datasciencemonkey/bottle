//! ADR 0103 (Refs #65 #107 #108): the host side of call recording capture. The recording
//! session runs over the app's one R2 CaptureCore (CAP-01a); the host relays lease stops to
//! it, applies the knob, the length preset tick, sleep and quit. Fake devices and simulated
//! clocks only.

use bottle_lib::capture_host::CaptureHost;
use bottle_lib::recording_host::{RecordingHost, TICK_EVERY};
use mog_core::audio_device::system::{RecordingDevice, SystemAudioDevice};
use mog_core::capture::{CaptureService, StopReason};
use mog_core::clock::{MonotonicClock, SimClock, SimWallClock};
use mog_core::recording_capture::permissions::PermissionAsker;
use mog_core::recording_capture::session::{
    FinishedRecording, PartsHandoff, SessionConfig, Snapshot, StartOptions, StopCause,
};
use mog_core::recording_capture::tap::RecordingTap;
use mog_core::testkit::capture::FakeAudioDevice;
use mog_core::token_events::{SessionEvent, SessionEventSource, SessionSink, SessionWatch};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Handoff(Mutex<Vec<FinishedRecording>>);
impl PartsHandoff for Handoff {
    fn finished(&self, rec: FinishedRecording) {
        self.0.lock().unwrap().push(rec);
    }
}

#[derive(Default)]
struct Asker(Mutex<Vec<&'static str>>);
impl PermissionAsker for Asker {
    fn ask_mic(&self) {
        self.0.lock().unwrap().push("mic");
    }
    fn ask_system_audio(&self) {
        self.0.lock().unwrap().push("system_audio");
    }
}

/// An OS event source the test fires by hand.
#[derive(Default, Clone)]
struct Source(Arc<Mutex<Option<SessionSink>>>);
impl SessionEventSource for Source {
    fn watch(&self, sink: SessionSink) -> mog_core::error::CoreResult<SessionWatch> {
        *self.0.lock().unwrap() = Some(sink);
        let me = Arc::clone(&self.0);
        Ok(SessionWatch::on_drop(move || {
            me.lock().unwrap().take();
        }))
    }
}
impl Source {
    fn fire(&self, e: SessionEvent) {
        let sink = self.0.lock().unwrap().clone();
        if let Some(s) = sink {
            s(e);
        }
    }
}

struct Rig {
    capture: Arc<CaptureHost>,
    rec: RecordingHost,
    mic: FakeAudioDevice,
    clock: SimClock,
    handoff: Arc<Handoff>,
    root: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn rig() -> Rig {
    let clock = SimClock::new();
    let mono: Arc<dyn MonotonicClock> = Arc::new(clock.clone());
    let tap = RecordingTap::new(Arc::clone(&mono));
    let mic = FakeAudioDevice::new();
    let (sys, status) = SystemAudioDevice::unavailable();
    let capture = Arc::new(
        CaptureHost::new(
            Box::new(RecordingDevice::new(Box::new(mic.clone()), Box::new(sys))),
            Arc::clone(&mono),
            Arc::new(|_: &str, _: Value| true),
        )
        .unwrap(),
    );
    let handoff = Arc::new(Handoff::default());
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "bottle-rec-host-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let rec = RecordingHost::new(
        &capture,
        tap,
        status,
        mono,
        Arc::new(SimWallClock::at(1_763_544_600_000)),
        handoff.clone(),
        SessionConfig {
            temp_root: root.clone(),
            ..SessionConfig::default()
        },
    )
    .unwrap();
    Rig {
        capture,
        rec,
        mic,
        clock,
        handoff,
        root,
    }
}

fn opts() -> StartOptions {
    StartOptions {
        name: "2026-11-19_0930".into(),
        length_min: 5,
    }
}

#[test]
fn adr_0103_the_knob_is_off_by_default_and_gates_every_start() {
    let r = rig();
    assert!(!r.rec.enabled());
    assert_eq!(
        r.rec.session().start(opts()).unwrap_err().code(),
        "recording_disabled"
    );
    assert_eq!(r.mic.log().mic_opens, 0, "no mic open with the knob off");
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    // The knob turned off stops nothing that runs, but blocks the next start.
    r.rec.set_enabled(false, &Asker::default());
    r.rec.session().stop(StopCause::User).unwrap();
    assert_eq!(
        r.rec.session().start(opts()).unwrap_err().code(),
        "recording_disabled"
    );
}

#[test]
fn adr_0103_turning_the_knob_on_asks_for_permissions_once() {
    let r = rig();
    let a = Asker::default();
    r.rec.set_enabled(true, &a);
    r.rec.set_enabled(true, &a);
    assert_eq!(*a.0.lock().unwrap(), vec!["mic", "system_audio"]);
}

#[test]
fn cap_01a_the_recording_holds_an_r2_session_lease_with_the_others_track() {
    let r = rig();
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    let state = r.capture.service().status();
    let v = serde_json::to_value(&state).unwrap();
    assert_eq!(v["mode"], json!("session"), "{v}");
    assert_eq!(v["recording"], json!(true));
    assert_eq!(r.mic.log().mic_opens, 1);
}

#[test]
fn adr_0103_a_panic_stop_on_the_host_finalizes_the_recording() {
    let r = rig();
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    r.capture
        .call("capture_stop_all", json!({}))
        .expect("stop all");
    assert_eq!(r.rec.session().snapshot(), Snapshot::Idle);
    assert_eq!(r.handoff.0.lock().unwrap()[0].cause, StopCause::Panic);
}

#[test]
fn adr_0103_the_host_stop_observer_sees_every_lease_stop() {
    let r = rig();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s2 = Arc::clone(&seen);
    r.capture
        .add_stop_observer(Arc::new(move |st: &mog_core::capture::Stopped| {
            s2.lock().unwrap().push(st.reason)
        }));
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    r.rec.session().stop(StopCause::User).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![StopReason::Manual]);
}

#[test]
fn adr_0103_sleep_stops_the_recording_and_quit_finalizes_it() {
    let r = rig();
    let src = Source::default();
    r.rec.watch_sleep(&src).unwrap();
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    src.fire(SessionEvent::Sleep);
    assert_eq!(r.handoff.0.lock().unwrap()[0].cause, StopCause::Sleep);
    r.rec.session().start(opts()).unwrap();
    r.rec.shutdown();
    assert_eq!(r.handoff.0.lock().unwrap()[1].cause, StopCause::Quit);
    assert_eq!(r.rec.session().snapshot(), Snapshot::Idle);
    // After the quit no new recording starts.
    assert_eq!(
        r.rec.session().start(opts()).unwrap_err().code(),
        "recording_disabled"
    );
}

#[test]
fn adr_0103_the_host_ticker_applies_the_length_preset() {
    let r = rig();
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    r.clock.advance(Duration::from_secs(5 * 60));
    // The ticker runs every TICK_EVERY of real time and reads the simulated clock.
    for _ in 0..200 {
        if r.rec.session().snapshot() == Snapshot::Idle {
            break;
        }
        std::thread::sleep(TICK_EVERY);
    }
    assert_eq!(r.rec.session().snapshot(), Snapshot::Idle);
    assert_eq!(
        r.handoff.0.lock().unwrap()[0].cause,
        StopCause::LengthReached
    );
}

#[test]
fn adr_0103_the_handoff_slot_forwards_once_the_cloud_side_connects() {
    use bottle_lib::recording_host::HandoffSlot;
    let slot = HandoffSlot::default();
    let rec = FinishedRecording {
        id: "a1".into(),
        name: "n".into(),
        dir: std::env::temp_dir().join("bottle-rec-none"),
        parts: Vec::new(),
        started_at_ms: 0,
        duration_ms: 0,
        cause: StopCause::User,
        system_audio: mog_core::recording_capture::session::SystemAudioState::Ok,
    };
    // Not connected: the parts stay in the temp dir for the launch scan ("Upload failed").
    slot.finished(rec.clone());
    assert_eq!(slot.pending(), vec![rec.clone()]);
    let cloud = Arc::new(Handoff::default());
    slot.connect(cloud.clone());
    // The waiting recording goes out at the connect; later ones go straight through.
    assert_eq!(cloud.0.lock().unwrap().len(), 1);
    slot.finished(rec);
    assert_eq!(cloud.0.lock().unwrap().len(), 2);
    assert!(slot.pending().is_empty());
}

fn code(r: mog_core::error::CoreResult<Value>) -> &'static str {
    match r {
        Err(mog_core::error::CoreError::IpcRefused { code }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn adr_0103_no_dictation_while_a_recording_runs() {
    let r = rig();
    r.rec.set_enabled(true, &Asker::default());
    r.rec.session().start(opts()).unwrap();
    for mode in ["ptt", "hands_free"] {
        assert_eq!(
            code(
                r.capture
                    .call("capture_start", json!({"mogId": "mog-a1", "mode": mode}))
            ),
            "recording_in_progress",
            "{mode}"
        );
    }
    // No overlay lease: R2 still has only the recording's lease, and the recording runs.
    let v = serde_json::to_value(r.capture.service().status()).unwrap();
    assert_eq!(v["mode"], json!("session"));
    assert_eq!(v["owner"], json!("bottle-recording"));
    // The recording's stop works, and then dictation is free again.
    r.rec.session().stop(StopCause::User).unwrap();
    r.capture
        .call("capture_start", json!({"mogId": "mog-a1", "mode": "ptt"}))
        .expect("PTT after the recording");
}
