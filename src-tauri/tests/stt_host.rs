//! #46 (CAP-51c, CAP-52, ADR 0082): the host feeds the mic to R4 SttEngine next to the PTT
//! buffer, and R2's events drive both.
//!
//! The chain under test is the one that `install_capture` builds: a mic test double (a fake
//! [`MicHost`]) under the real `MicDevice`, whose frame sink tees the `LevelSink`, the PTT
//! buffer and R4; the real `CaptureCore` with the PTT buffer and R4 in its event chain; and the
//! real [`CaptureHost`] behind the hardened gateway. No real mic and no model: R4 has a fake
//! recognizer. The samples are a synthetic tone.

use bottle_lib::capture_host::{spawn_level_mic, CaptureHost, HostLevels, LevelResolver};
use bottle_lib::ipc::{route, BrainLink, HostComponents};
use mog_core::audio_device::level::{NoLevelOut, TeeSink};
use mog_core::audio_device::ptt_buffer::PttBuffer;
use mog_core::audio_device::{DataFn, FaultFn, Interleaved, MicHost};
use mog_core::capture::DeviceError;
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::ipc::ChannelRef;
use mog_core::stt_engine::{
    EngineParts, LocalSttEngine, PowerProbe, PowerSource, Recognizer, RecognizerLoader, SttDone,
    SttIdentity, TranscriptSink,
};
use mog_core::types::TranscriptProvider;
use serde_json::{json, Value};
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

const RATE: u32 = 48_000;
const MOG: &str = "a1";

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

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

struct Words;

impl Recognizer for Words {
    fn transcribe(&mut self, _samples: &[f32]) -> String {
        "words".into()
    }
}

struct Loader;

impl RecognizerLoader for Loader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        Ok(Box::new(Words))
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
struct Out(Mutex<Vec<SttDone>>);

impl TranscriptSink for Out {
    fn done(&self, done: SttDone) {
        self.0.lock().unwrap().push(done);
    }
}

struct Rig {
    host: CaptureHost,
    mic: FakeMic,
    buf: PttBuffer,
    stt: LocalSttEngine,
    out: Arc<Out>,
}

fn rig() -> Rig {
    let mic = FakeMic::default();
    let out = Arc::new(Out::default());
    let mut parts = EngineParts::new(
        Arc::new(Loader),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    parts.power = Arc::new(Ac);
    let stt = LocalSttEngine::spawn(parts);
    let buf = PttBuffer::new(Arc::new(stt.clone()));
    let none: LevelResolver = Arc::new(|_: &ChannelRef| Some(Arc::new(NoLevelOut) as _));
    let levels = HostLevels::new(none);
    let host_mic = mic.clone();
    let (device, link) = spawn_level_mic(
        move || FakeHost {
            mic: host_mic,
            _not_send: PhantomData,
        },
        TeeSink(buf.sink(), stt.frame_sink()),
        levels.gate(),
    )
    .expect("the audio thread starts");
    let host = CaptureHost::with_stt(
        Box::new(device),
        Arc::new(SimClock::new()) as Arc<dyn MonotonicClock>,
        Arc::new(|_: &str, _: Value| true),
        levels,
        buf.clone(),
        stt.clone(),
    )
    .expect("the cap timer thread starts");
    host.connect_failures(&link);
    Rig {
        host,
        mic,
        buf,
        stt,
        out,
    }
}

impl Rig {
    fn call(&self, command: &str, args: Value) -> Result<Value, Value> {
        let components = HostComponents {
            capture: Some(&self.host),
            ..HostComponents::none("brain", &NoBrain)
        };
        route(components, command, args)
    }

    fn start(&self) -> String {
        let state = self
            .call("capture_start", json!({"mogId": MOG, "mode": "ptt"}))
            .expect("the capture starts");
        state["leaseId"].as_str().expect("a lease ID").to_owned()
    }

    fn stop(&self, lease: &str) {
        self.call("capture_stop", json!({"mogId": MOG, "leaseId": lease}))
            .expect("the capture stops");
    }
}

#[test]
fn cap_52_the_host_mic_feeds_r4_and_a_stop_gives_one_transcript() {
    let r = rig();
    let lease = r.start();
    // 1 s of a tone in 10 ms blocks.
    let block: Vec<f32> = (0..480)
        .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / RATE as f32).sin())
        .collect();
    for _ in 0..100 {
        assert!(r.mic.feed(&block), "the mic stream is open");
    }
    r.stop(&lease);
    r.stt.sync();
    let done = r.out.0.lock().unwrap().clone();
    assert_eq!(done.len(), 1, "one transcript for the lease");
    assert_eq!(done[0].lease_id.to_string(), lease);
    let t = done[0].result.clone().expect("a transcript");
    assert_eq!(t.text, "words");
    assert_eq!(t.provider, TranscriptProvider::LocalParakeet);
    assert_eq!(r.buf.held_samples(), 0, "the PTT buffer keeps no copy");
    assert_eq!(r.stt.status().held_samples, 0, "R4 keeps no audio");
}

#[test]
fn cap_51c_mb_05_a_granted_ptt_preloads_and_the_panic_stop_makes_no_transcript() {
    let r = rig();
    let lease = r.start();
    r.stt.sync();
    assert!(r.stt.status().loaded, "the model is in memory at key-down");
    assert!(r.mic.feed(&[0.2; 480]));
    r.call("capture_stop_all", json!({}))
        .expect("the panic stop");
    r.stt.sync();
    assert!(
        r.out.0.lock().unwrap().is_empty(),
        "no text from a panic-stopped lease {lease}"
    );
    assert_eq!(r.stt.status().open_takes, 0);
    assert_eq!(r.buf.held_samples(), 0);
}

#[test]
fn cap_51c_install_capture_wires_r4_into_the_mic_tee_and_the_events() {
    let src = include_str!("../src/lib.rs");
    let body: String = src[src.find("fn install_capture").expect("install_capture")..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        body.contains("TeeSink(ptt.sink(),stt.engine().frame_sink())"),
        "the mic frames go to the PTT buffer and to R4"
    );
    assert!(
        body.contains("CaptureHost::with_stt("),
        "R4 is in the R2 event chain"
    );
    assert!(
        body.contains("PttBuffer::new(std::sync::Arc::new(stt.engine().clone()"),
        "R4 consumes the PTT buffer utterances"
    );
}
