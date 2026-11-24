//! ADR 0118 (UX slice 6, host half): the talk key during a call recording, on the production
//! chain of `install_capture`. A fake mic feeds the real level mic device, whose sink tees the
//! PTT buffer, R4 and the recording tap (in that order); the real [`CaptureHost`] with the mic
//! gate outermost in its event chain; the real [`RecordingHost`]. No real mic, no model: R4 has
//! a fake recognizer.

use bottle_lib::capture_host::{
    spawn_level_mic, BrainRelay, BrainSegments, CaptureHost, HostLevels, LevelResolver,
};
use bottle_lib::recording_host::RecordingHost;
use mog_core::audio_device::level::{NoLevelOut, TeeSink};
use mog_core::audio_device::ptt_buffer::PttBuffer;
use mog_core::audio_device::system::{RecordingDevice, SystemAudioDevice};
use mog_core::audio_device::{DataFn, FaultFn, Interleaved, MicHost};
use mog_core::capture::DeviceError;
use mog_core::clock::{MonotonicClock, SimClock, SimWallClock};
use mog_core::ipc::ChannelRef;
use mog_core::recording_capture::permissions::PermissionAsker;
use mog_core::recording_capture::session::{
    FinishedRecording, PartsHandoff, SessionConfig, StartOptions, StopCause,
};
use mog_core::recording_capture::tap::{RecordingTap, GATE_TAIL_MS};
use mog_core::stt_engine::{
    LocalStt, PowerProbe, PowerSource, Recognizer, RecognizerLoader, SttIdentity,
};
use serde_json::{json, Value};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

const RATE: u32 = 48_000;
const MOG: &str = "a1";
const SEGMENT: &str = "capture:segment";

#[derive(Clone, Default)]
struct FakeMic(Arc<Mutex<Option<DataFn>>>);

impl FakeMic {
    fn feed(&self, samples: &[f32]) -> bool {
        match self.0.lock().unwrap().as_mut() {
            Some(on_data) => {
                on_data(Interleaved {
                    samples,
                    channels: 1,
                    sample_rate: RATE,
                });
                true
            }
            None => false,
        }
    }

    /// 1 s of a tone in 10 ms blocks.
    fn speak_one_second(&self) {
        let block: Vec<f32> = (0..480)
            .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / RATE as f32).sin())
            .collect();
        for _ in 0..100 {
            assert!(self.feed(&block), "the mic stream is open");
        }
    }
}

struct FakeHost {
    mic: FakeMic,
    _not_send: PhantomData<*const ()>,
}

struct FakeStream(Arc<Mutex<Option<DataFn>>>);

impl Drop for FakeStream {
    fn drop(&mut self) {
        self.0.lock().unwrap().take();
    }
}

impl MicHost for FakeHost {
    type Stream = FakeStream;

    fn build(&mut self, on_data: DataFn, _on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        *self.mic.0.lock().unwrap() = Some(on_data);
        Ok(FakeStream(Arc::clone(&self.mic.0)))
    }

    fn play(&mut self, _stream: &FakeStream) -> Result<(), DeviceError> {
        Ok(())
    }
}

const SPOKEN: &str = "zebracadabra";

struct Words;

impl Recognizer for Words {
    fn transcribe(&mut self, _samples: &[f32]) -> String {
        SPOKEN.into()
    }
}

/// Loads [`Words`], or fails with `stt_model_missing` (no model installed).
struct Loader {
    installed: bool,
}

impl RecognizerLoader for Loader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        if self.installed {
            Ok(Box::new(Words))
        } else {
            Err("stt_model_missing")
        }
    }
    fn identity(&self) -> SttIdentity {
        SttIdentity::local_parakeet()
    }
}

struct Ac;

impl PowerProbe for Ac {
    fn source(&self) -> PowerSource {
        PowerSource::Ac
    }
}

#[derive(Default)]
struct Handoff(Mutex<Vec<FinishedRecording>>);
impl PartsHandoff for Handoff {
    fn finished(&self, rec: FinishedRecording) {
        self.0.lock().unwrap().push(rec);
    }
}

struct NoAsk;
impl PermissionAsker for NoAsk {
    fn ask_mic(&self) {}
    fn ask_system_audio(&self) {}
}

/// Every item that the host sends on the brain relay, in order.
type Sent = Arc<Mutex<Vec<(String, Value)>>>;

struct Rig {
    host: Arc<CaptureHost>,
    rec: RecordingHost,
    tap: RecordingTap,
    mic: FakeMic,
    stt: LocalStt,
    sent: Sent,
    clock: SimClock,
    root: std::path::PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.rec.shutdown();
        self.host.shutdown();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn rig() -> Rig {
    let sent: Sent = Arc::default();
    let log = Arc::clone(&sent);
    let relay: BrainRelay = Arc::new(move |event: &str, payload: Value| {
        log.lock().unwrap().push((event.to_owned(), payload));
        true
    });
    let stt = LocalStt::new(
        Arc::new(Loader { installed: true }),
        Arc::new(BrainSegments::new(Arc::clone(&relay))),
        |parts| parts.power = Arc::new(Ac),
    );
    let mic = FakeMic::default();
    let buf = PttBuffer::new(Arc::new(stt.engine().clone()));
    let none: LevelResolver = Arc::new(|_: &ChannelRef| Some(Arc::new(NoLevelOut) as _));
    let levels = HostLevels::new(none);
    let clock = SimClock::new();
    let mono: Arc<dyn MonotonicClock> = Arc::new(clock.clone());
    let tap = RecordingTap::new(Arc::clone(&mono));
    let host_mic = mic.clone();
    // As `install_capture`: the PTT buffer and R4 get the frames first, then the tap.
    let (device, _link) = spawn_level_mic(
        move || FakeHost {
            mic: host_mic,
            _not_send: PhantomData,
        },
        TeeSink(
            TeeSink(buf.sink(), stt.engine().frame_sink()),
            tap.mic_sink(),
        ),
        levels.gate(),
    )
    .expect("the audio thread starts");
    let (sys, status) = SystemAudioDevice::unavailable();
    let device = RecordingDevice::new(Box::new(device), Box::new(sys));
    let host = Arc::new(
        CaptureHost::with_stt_gated(
            Box::new(device),
            Arc::clone(&mono),
            relay,
            levels,
            buf,
            stt.engine().clone(),
            tap.clone(),
        )
        .expect("the cap timer thread starts"),
    );
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "bottle-u6h-{}-{}",
        std::process::id(),
        N.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    ));
    let rec = RecordingHost::new(
        &host,
        tap.clone(),
        status,
        mono,
        Arc::new(SimWallClock::at(1_763_544_600_000)),
        Arc::new(Handoff::default()),
        SessionConfig {
            temp_root: root.clone(),
            ..SessionConfig::default()
        },
    )
    .unwrap();
    rec.set_enabled(true, &NoAsk);
    Rig {
        host,
        rec,
        tap,
        mic,
        stt,
        sent,
        clock,
        root,
    }
}

fn opts() -> StartOptions {
    StartOptions {
        name: "2026-11-23_0930".into(),
        length_min: 5,
    }
}

fn code(r: mog_core::error::CoreResult<Value>) -> &'static str {
    match r {
        Err(mog_core::error::CoreError::IpcRefused { code }) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

impl Rig {
    fn start(&self, mode: &str) -> mog_core::error::CoreResult<Value> {
        self.host
            .call("capture_start", json!({"mogId": MOG, "mode": mode}))
    }

    fn segments(&self) -> Vec<Value> {
        self.stt.engine().sync();
        self.sent
            .lock()
            .unwrap()
            .iter()
            .filter(|(event, _)| event == SEGMENT)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

/// AC-6.1, AC-6.8: `ptt` during an active recording is an overlay. The PTT buffer and R4 get
/// the overlay lease through the whole production chain, so the hold makes one utterance whose
/// segment names the overlay lease (requirement 4).
#[test]
fn ux6h_ac_6_1_a_ptt_hold_in_a_recording_is_an_overlay_and_r4_makes_its_segment() {
    let r = rig();
    r.rec.session().start(opts()).unwrap();
    let state = r.start("ptt").expect("AC-6.1: a lease");
    let lease = state["leaseId"].as_str().unwrap().to_owned();
    // The recording keeps its session lease (AC-6.12).
    assert_eq!(state["mode"], json!("session"));
    r.mic.speak_one_second();
    r.host
        .call("capture_stop", json!({"mogId": MOG, "leaseId": lease}))
        .unwrap();
    let segments = r.segments();
    assert_eq!(segments.len(), 1, "one utterance: {segments:?}");
    assert_eq!(
        segments[0]["leaseId"],
        json!(lease),
        "AC-6.8: overlay leaseId"
    );
    assert!(
        r.rec.session().is_recording(),
        "AC-6.12: the hold does not stop the recording"
    );
    r.rec.session().stop(StopCause::User).unwrap();
}

/// AC-6.2, AC-6.3: hands-free is refused during a recording, and every start is refused while
/// the recording starts or stops.
#[test]
fn ux6h_ac_6_2_and_6_3_hands_free_and_a_changing_recording_answer_recording_in_progress() {
    let r = rig();
    r.rec.session().start(opts()).unwrap();
    assert_eq!(code(r.start("hands_free")), "recording_in_progress");
    r.rec.session().stop(StopCause::User).unwrap();
    // The recording is over: dictation is free again, and no gate is stuck open or closed.
    r.start("ptt").expect("PTT after the recording");
}

/// ADR 0118 decisions 5 and 6: the hold closes the gate at once, an overlay lease keeps it
/// closed after the key is up, and it opens 250 ms after the lease ends.
#[test]
fn ux6h_the_overlay_lease_keeps_the_gate_closed_until_its_stop_and_the_tail() {
    let r = rig();
    r.rec.session().start(opts()).unwrap();
    assert!(!r.tap.mic_gated());
    // The key hook runs before the brain starts the capture.
    r.tap.talk_key_down();
    assert!(r.tap.mic_gated(), "closed at key-down, before any lease");
    let lease = r.start("ptt").unwrap()["leaseId"]
        .as_str()
        .unwrap()
        .to_owned();
    r.tap.talk_key_up();
    r.clock
        .advance(std::time::Duration::from_millis(GATE_TAIL_MS + 50));
    assert!(r.tap.mic_gated(), "a live overlay lease keeps it closed");
    r.host
        .call("capture_stop", json!({"mogId": MOG, "leaseId": lease}))
        .unwrap();
    assert!(r.tap.mic_gated(), "the tail runs after the lease stop");
    r.clock
        .advance(std::time::Duration::from_millis(GATE_TAIL_MS - 50));
    assert!(r.tap.mic_gated());
    r.clock.advance(std::time::Duration::from_millis(100));
    assert!(!r.tap.mic_gated(), "open after the tail");
    r.rec.session().stop(StopCause::User).unwrap();
}

/// Requirement 8 and the stop-all item of requirement 2: `capture_stop_all` ends the overlay
/// lease, which starts the tail. It does not open the gate while the key is physically down,
/// and no IPC command touches the key flag.
#[test]
fn ux6h_stop_all_ends_the_overlay_with_a_tail_and_the_key_still_holds_the_gate() {
    let r = rig();
    r.rec.session().start(opts()).unwrap();
    r.tap.talk_key_down();
    r.start("ptt").unwrap();
    r.host.call("capture_stop_all", json!({})).unwrap();
    r.clock
        .advance(std::time::Duration::from_millis(GATE_TAIL_MS + 50));
    assert!(
        r.tap.mic_gated(),
        "the key is still down: the brain cannot open the gate"
    );
    r.tap.talk_key_up();
    r.clock
        .advance(std::time::Duration::from_millis(GATE_TAIL_MS + 50));
    assert!(!r.tap.mic_gated());
}
