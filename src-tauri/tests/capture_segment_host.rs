//! #514 part 2 (F1 steps 5 and 8, CAP-52, CAP-02a, CAP-01b, ADR 0082 amendment 1): the host
//! sends one final `capture:segment` (`track: 'you'`, `segmentId` `ptt-<lease>`) to the brain
//! for each R4 result, so the brain can call `stt_transcribe` for it.
//!
//! The chain under test is the one that `install_capture` builds: a mic test double (a fake
//! [`MicHost`]) under the real `MicDevice`, whose frame sink tees the PTT buffer and R4; the real
//! `CaptureCore` with the PTT buffer and R4 in its event chain; the real [`CaptureHost`]; and
//! the real [`LocalStt`] whose results go to [`BrainSegments`] on the same brain relay as
//! `capture:state`. No real mic and no model: R4 has a fake recognizer. The samples are a
//! synthetic tone. This file builds no `HostComponents`: it calls the host directly.

use bottle_lib::capture_host::{
    spawn_level_mic, BrainRelay, BrainSegments, CaptureHost, HostLevels, LevelResolver,
};
use mog_core::audio_device::level::{NoLevelOut, TeeSink};
use mog_core::audio_device::ptt_buffer::PttBuffer;
use mog_core::audio_device::{DataFn, FaultFn, Interleaved, MicHost};
use mog_core::capture::DeviceError;
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::error::CoreError;
use mog_core::ipc::ChannelRef;
use mog_core::stt::SttEngine;
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

/// A distinct word, so a test can prove that it never reaches an event.
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

/// Every item that the host sends on the brain relay, in order.
type Sent = Arc<Mutex<Vec<(String, Value)>>>;

type Answers = Arc<Mutex<Vec<Result<String, &'static str>>>>;

struct Rig {
    host: CaptureHost,
    mic: FakeMic,
    stt: LocalStt,
    sent: Sent,
    /// For each `capture:segment`, what `stt_transcribe` answered at the moment it was sent:
    /// the text, or the content-free code.
    answers: Answers,
}

fn rig(installed: bool) -> Rig {
    let sent: Sent = Arc::default();
    let answers: Answers = Arc::default();
    let slot: Arc<Mutex<Option<LocalStt>>> = Arc::default();
    let (log, stored, seen) = (Arc::clone(&sent), Arc::clone(&answers), Arc::clone(&slot));
    let relay: BrainRelay = Arc::new(move |event: &str, payload: Value| {
        if event == SEGMENT {
            // As the brain: call `stt_transcribe` with the segmentId of the event, at once.
            let id = payload["segmentId"].as_str().unwrap_or_default();
            if let Some(stt) = seen.lock().unwrap().as_ref() {
                let answer = match stt.transcribe(id) {
                    Ok(t) => Ok(t.text),
                    Err(CoreError::Unavailable { reason, .. }) => Err(reason),
                    Err(_) => Err("other_error"),
                };
                stored.lock().unwrap().push(answer);
            }
        }
        log.lock().unwrap().push((event.to_owned(), payload));
        true
    });
    // As `install_capture`: R4's results go to the brain relay.
    let stt = LocalStt::new(
        Arc::new(Loader { installed }),
        Arc::new(BrainSegments::new(Arc::clone(&relay))),
        |parts| parts.power = Arc::new(Ac),
    );
    *slot.lock().unwrap() = Some(stt.clone());
    let mic = FakeMic::default();
    let buf = PttBuffer::new(Arc::new(stt.engine().clone()));
    let none: LevelResolver = Arc::new(|_: &ChannelRef| Some(Arc::new(NoLevelOut) as _));
    let levels = HostLevels::new(none);
    let host_mic = mic.clone();
    let (device, link) = spawn_level_mic(
        move || FakeHost {
            mic: host_mic,
            _not_send: PhantomData,
        },
        TeeSink(buf.sink(), stt.engine().frame_sink()),
        levels.gate(),
    )
    .expect("the audio thread starts");
    let host = CaptureHost::with_stt(
        Box::new(device),
        Arc::new(SimClock::new()) as Arc<dyn MonotonicClock>,
        relay,
        levels,
        buf,
        stt.engine().clone(),
    )
    .expect("the cap timer thread starts");
    host.connect_failures(&link);
    Rig {
        host,
        mic,
        stt,
        sent,
        answers,
    }
}

impl Rig {
    fn start(&self) -> String {
        let state = self
            .host
            .call("capture_start", json!({"mogId": MOG, "mode": "ptt"}))
            .expect("the capture starts");
        state["leaseId"].as_str().expect("a lease ID").to_owned()
    }

    fn stop(&self, lease: &str) {
        self.host
            .call("capture_stop", json!({"mogId": MOG, "leaseId": lease}))
            .expect("the capture stops");
    }

    /// The `capture:segment` payloads, after R4 has handled every queued message.
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

#[test]
fn f1_cap_52_cap_02a_a_ptt_stop_emits_one_final_capture_segment_for_you() {
    let r = rig(true);
    let lease = r.start();
    r.mic.speak_one_second();
    r.stop(&lease);
    let segments = r.segments();
    assert_eq!(segments.len(), 1, "one capture:segment for the lease");
    assert_eq!(
        segments[0],
        json!({
            "segmentId": format!("ptt-{lease}"),
            "mogId": MOG,
            "leaseId": lease,
            "track": "you",
            "tStartMs": 0,
            "tEndMs": 1000,
            "final": true,
        }),
        "a final SegmentRef of the user's track that names its lease (CAP-02a)"
    );
    // F1 step 8: `stt_transcribe` with the segmentId of the event gives the R4 text.
    assert_eq!(*r.answers.lock().unwrap(), [Ok(SPOKEN.to_owned())]);
}

#[test]
fn f1_capture_segment_comes_after_the_stop_and_the_result_is_already_stored() {
    // The brain may call `stt_transcribe` at once on the event: R4's store must hold the result.
    let r = rig(true);
    let lease = r.start();
    r.mic.speak_one_second();
    r.stop(&lease);
    assert_eq!(r.segments().len(), 1);
    assert_eq!(
        *r.answers.lock().unwrap(),
        [Ok(SPOKEN.to_owned())],
        "stt_transcribe finds the text when capture:segment arrives"
    );
    let events: Vec<String> = r
        .sent
        .lock()
        .unwrap()
        .iter()
        .map(|(e, _)| e.clone())
        .collect();
    let stopped = events.iter().position(|e| e == "capture:stopped");
    let segment = events.iter().position(|e| e == SEGMENT);
    assert!(
        stopped.is_some() && segment > stopped,
        "capture:stopped, then capture:segment: {events:?}"
    );
}

#[test]
fn cap_01b_capture_segment_holds_no_text_and_no_audio() {
    let r = rig(true);
    let lease = r.start();
    r.mic.speak_one_second();
    r.stop(&lease);
    let segments = r.segments();
    assert_eq!(segments.len(), 1);
    let wire = segments[0].to_string();
    assert!(!wire.contains(SPOKEN), "no transcript text in the event");
    let keys: Vec<&str> = segments[0]
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    let mut keys = keys;
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "final",
            "leaseId",
            "mogId",
            "segmentId",
            "tEndMs",
            "tStartMs",
            "track"
        ],
        "only the SegmentRef fields, no text and no samples"
    );
}

#[test]
fn f1_a_failed_r4_result_still_emits_capture_segment_and_stt_transcribe_gives_its_code() {
    // No model installed: the brain still gets the event, and `stt_transcribe` answers the
    // content-free code, so the card can say why no text came.
    let r = rig(false);
    let lease = r.start();
    r.mic.speak_one_second();
    r.stop(&lease);
    let segments = r.segments();
    assert_eq!(
        segments.len(),
        1,
        "one capture:segment for the failed lease"
    );
    assert_eq!(segments[0]["segmentId"], json!(format!("ptt-{lease}")));
    assert_eq!(segments[0]["final"], json!(true));
    assert_eq!(*r.answers.lock().unwrap(), [Err("stt_model_missing")]);
}

#[test]
fn mb_05_cap_17b_a_panic_stop_or_an_empty_capture_emits_no_capture_segment() {
    let r = rig(true);
    // MB-05: the panic stop makes no text, so no segment.
    let lease = r.start();
    r.mic.speak_one_second();
    r.host
        .call("capture_stop_all", json!({}))
        .expect("the panic stop");
    // CAP-17b: a capture with no audio frames makes no text, so no segment.
    let empty = r.start();
    r.stop(&empty);
    assert!(
        r.segments().is_empty(),
        "no capture:segment for lease {lease} (panic) or {empty} (no audio)"
    );
}

#[test]
fn f1_install_capture_sends_r4_results_to_the_brain_relay() {
    let src = include_str!("../src/lib.rs");
    let body: String = src[src.find("fn install_capture").expect("install_capture")..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        body.contains("capture_host::BrainSegments::new(std::sync::Arc::clone(&relay))"),
        "R4's results go to capture:segment on the brain relay"
    );
    assert!(
        !body.contains("NoTranscriptSink"),
        "the app no longer drops R4's results"
    );
}
