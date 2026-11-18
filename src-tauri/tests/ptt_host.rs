//! #21 (CAP-12c, CAP-13a, ADR 0078): the host feeds the mic to the RAM-only PTT buffer.
//!
//! The chain under test is the one that `install_capture` builds: a mic test double (a fake
//! [`MicHost`]) under the real `MicDevice`, whose frame sink is a `TeeSink` of the `LevelSink`
//! and the PTT buffer sink; the real `CaptureCore` with the PTT buffer in its event chain; and
//! the real [`CaptureHost`] behind the hardened gateway. No real mic opens. The samples are a
//! synthetic tone. The consumer here is a stand-in; R4 SttEngine (#46) is the app's consumer (tests/stt_host.rs).

use bottle_lib::capture_host::{spawn_level_mic, CaptureHost, HostLevels, LevelResolver};
use bottle_lib::ipc::{route, BrainLink, HostComponents};
use mog_core::audio_device::level::NoLevelOut;
use mog_core::audio_device::ptt_buffer::{PttBuffer, Utterance, UtteranceSink};
use mog_core::audio_device::{DataFn, FaultFn, Interleaved, MicHost};
use mog_core::capture::{DeviceError, StopReason};
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::ipc::ChannelRef;
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

#[derive(Default)]
struct Kept(Mutex<Vec<Utterance>>);

impl UtteranceSink for Kept {
    fn utterance(&self, utterance: Utterance) {
        self.0.lock().unwrap().push(utterance);
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

struct Rig {
    host: CaptureHost,
    mic: FakeMic,
    buf: PttBuffer,
    kept: Arc<Kept>,
}

fn rig() -> Rig {
    let mic = FakeMic::default();
    let kept = Arc::new(Kept::default());
    let buf = PttBuffer::new(kept.clone());
    let none: LevelResolver = Arc::new(|_: &ChannelRef| Some(Arc::new(NoLevelOut) as _));
    let levels = HostLevels::new(none);
    let host_mic = mic.clone();
    let (device, link) = spawn_level_mic(
        move || FakeHost {
            mic: host_mic,
            _not_send: PhantomData,
        },
        buf.sink(),
        levels.gate(),
    )
    .expect("the audio thread starts");
    let host = CaptureHost::with_audio(
        Box::new(device),
        Arc::new(SimClock::new()) as Arc<dyn MonotonicClock>,
        Arc::new(|_: &str, _: Value| true),
        levels,
        buf.clone(),
    )
    .expect("the cap timer thread starts");
    host.connect_failures(&link);
    Rig {
        host,
        mic,
        buf,
        kept,
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
fn cap_12c_cap_13a_the_host_mic_feeds_the_ram_only_ptt_buffer() {
    let r = rig();
    // CAP-13a: no stream before key-down, so no frame exists.
    assert!(!r.mic.feed(&[0.9; 480]), "no stream before capture_start");
    let lease = r.start();
    assert!(r.buf.is_taking());
    for _ in 0..4 {
        assert!(r.mic.feed(&[0.2; 480]), "the mic stream is open");
    }
    assert_eq!(r.buf.held_samples(), 1_920);
    r.stop(&lease);
    assert!(
        !r.mic.feed(&[0.9; 480]),
        "the stream is gone after the stop"
    );
    let kept = std::mem::take(&mut *r.kept.0.lock().unwrap());
    assert_eq!(kept.len(), 1, "one utterance for the consumer");
    assert_eq!(kept[0].lease_id().to_string(), lease);
    assert_eq!(kept[0].reason(), StopReason::Manual);
    assert_eq!(
        kept[0].samples().len(),
        1_920,
        "only the audio after key-down"
    );
    assert_eq!(r.buf.held_samples(), 0, "the buffer keeps no copy (CAP-34)");
}

#[test]
fn cap_12c_mb_05_a_capture_stop_all_hands_the_audio_on_and_holds_nothing() {
    let r = rig();
    let lease = r.start();
    r.mic.feed(&[0.2; 480]);
    r.call("capture_stop_all", json!({}))
        .expect("the panic stop");
    assert_eq!(r.buf.held_samples(), 0);
    assert_eq!(r.buf.takes(), 0);
    assert!(!r.buf.is_taking());
    // ADR 0078 decision 7: the panic stop keeps the audio, like a manual stop. The consumer
    // sees the reason `Panic` and decides what to do with it.
    let kept = std::mem::take(&mut *r.kept.0.lock().unwrap());
    assert_eq!(kept.len(), 1, "the panic stop hands the take on");
    assert_eq!(kept[0].lease_id().to_string(), lease);
    assert_eq!(kept[0].reason(), StopReason::Panic);
    assert_eq!(kept[0].samples().len(), 480);
}

// ---------- the production wiring in `install_capture` ----------

fn install_capture_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/lib.rs");
    let text = std::fs::read_to_string(&path).expect("read src/lib.rs");
    let body = text
        .split("fn install_capture(")
        .nth(1)
        .expect("install_capture in lib.rs");
    let end = body.find("\n}\n").expect("the end of install_capture");
    body[..end].to_owned()
}

/// Whitespace-free text, so a rustfmt line break does not matter.
fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
fn cap_12c_install_capture_feeds_the_mic_to_the_same_ptt_buffer_that_the_core_drives() {
    let body = squash(&install_capture_source());
    assert!(
        body.contains("letptt=mog_core::audio_device::ptt_buffer::PttBuffer::new("),
        "install_capture builds one PttBuffer named `ptt`"
    );
    assert_eq!(
        body.matches("PttBuffer::new(").count(),
        1,
        "exactly one PttBuffer in install_capture"
    );
    assert!(
        // #46: the same tee also feeds R4 SttEngine (ADR 0082).
        body.contains("spawn_level_mic(CpalMicHost::new,mog_core::audio_device::level::TeeSink(ptt.sink(),stt.engine().frame_sink()),levels.gate()"),
        "the mic frames go to `ptt.sink()`"
    );
    assert!(
        !body.contains("DiscardSink"),
        "the mic frames must not go to DiscardSink"
    );
    // #46: `with_stt` is `with_audio` with R4 in the event chain (ADR 0082).
    let host = body
        .split("CaptureHost::with_stt(")
        .nth(1)
        .expect("the host is built with with_stt");
    let args = &host[..host.find(")?;").expect("the end of the with_stt call")];
    assert!(
        args.contains(",ptt,stt.engine().clone()"),
        "the same `ptt` goes to with_stt: {args}"
    );
    assert!(!body.contains("CaptureHost::with_levels("));
}
