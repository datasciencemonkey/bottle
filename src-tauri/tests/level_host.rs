//! #476 (CAP-16a, CAP-16b, ADR 0067): the host wires the mic level stream end to end.
//!
//! The chain under test is the one that `install_capture` builds: a mic test double (a fake
//! [`MicHost`]) under the real [`MicDevice`], whose frame sink is a `TeeSink` of the
//! `LevelSink` and the PTT buffer; the real `CaptureCore` with `LevelCaptureEvents`; the real
//! [`CaptureHost`] behind the hardened gateway; and a real Tauri [`Channel`] as the
//! `capture_subscribe` `onLevel` Channel. No real mic opens. The samples are a synthetic tone.

use bottle_lib::capture_host::{
    brain_level_resolver, spawn_level_mic, CaptureHost, ChannelLevelOut, HostLevels, LevelResolver,
};
use bottle_lib::ipc::{route, BrainLink, HostComponents};
use mog_core::audio_device::level::LevelOut;
use mog_core::audio_device::{DataFn, DiscardSink, FaultFn, Interleaved, MicHost};
use mog_core::capture::DeviceError;
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::ipc::{events, ChannelRef};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};
use tauri::ipc::{Channel, InvokeResponseBody};

// ---------- the mic test double ----------

/// The data callback of the open fake stream, shared with the test thread.
#[derive(Clone, Default)]
struct FakeMic(Arc<Mutex<Option<DataFn>>>);

impl FakeMic {
    /// One block from the "device": mono samples at 48 kHz. `false` if no stream was built.
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

/// A fake audio API. It keeps the last data callback, so the test can call it after the stream
/// drops (a late callback); the backend must not let that audio through.
struct FakeHost {
    mic: FakeMic,
    _not_send: PhantomData<*const ()>,
}

struct FakeStream;

impl MicHost for FakeHost {
    type Stream = FakeStream;

    fn build(&mut self, on_data: DataFn, _on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        *self.mic.0.lock().unwrap() = Some(on_data);
        Ok(FakeStream)
    }

    fn play(&mut self, _stream: &FakeStream) -> Result<(), DeviceError> {
        Ok(())
    }
}

const RATE: u32 = 48_000;
/// One level window at 48 kHz: 48 000 / 30 samples (CAP-16a).
const WINDOW: usize = 1_600;

// ---------- the onLevel Channels ----------

/// The messages that one Tauri Channel got, as JSON.
#[derive(Clone, Default)]
struct Received(Arc<Mutex<Vec<Value>>>);

impl Received {
    fn channel(&self) -> Channel<f32> {
        let got = Arc::clone(&self.0);
        Channel::new(move |body| {
            if let InvokeResponseBody::Json(s) = body {
                got.lock().unwrap().push(serde_json::from_str(&s).unwrap());
            }
            Ok(())
        })
    }

    fn all(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }
}

/// The resolver of the test: the references in `known` are Channels; any other one does not
/// resolve (a webview that is gone).
fn resolver(known: HashMap<&'static str, Received>) -> LevelResolver {
    Arc::new(move |c: &ChannelRef| {
        known
            .get(c.as_str())
            .map(|r| Arc::new(ChannelLevelOut::new(r.channel())) as Arc<dyn LevelOut>)
    })
}

// ---------- the rig ----------

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
    relayed: Arc<Mutex<Vec<(String, Value)>>>,
}

fn rig(known: HashMap<&'static str, Received>) -> Rig {
    let mic = FakeMic::default();
    let levels = HostLevels::new(resolver(known));
    let host_mic = mic.clone();
    let (device, link) = spawn_level_mic(
        move || FakeHost {
            mic: host_mic,
            _not_send: PhantomData,
        },
        DiscardSink,
        levels.gate(),
    )
    .expect("the audio thread starts");
    let relayed: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
    let sink = Arc::clone(&relayed);
    let host = CaptureHost::with_levels(
        Box::new(device),
        Arc::new(SimClock::new()) as Arc<dyn MonotonicClock>,
        Arc::new(move |event: &str, payload: Value| {
            sink.lock().unwrap().push((event.to_owned(), payload));
            true
        }),
        levels,
    )
    .expect("the cap timer thread starts");
    host.connect_failures(&link);
    Rig { host, mic, relayed }
}

impl Rig {
    fn call(&self, command: &str, args: Value) -> Result<Value, Value> {
        let components = HostComponents {
            from: "brain",
            brain: &NoBrain,
            windows: None,
            hotkeys: None,
            permissions: None,
            consent: None,
            config: None,
            mcp: None,
            capture: Some(&self.host),
            capture_ticket: None,
            app_settings: None,
            insert: None,
        };
        route(components, command, args)
    }

    fn subscribe(&self, on_level: &str) -> Result<Value, Value> {
        self.call(
            "capture_subscribe",
            json!({"onLevel": on_level, "onPartial": "__CHANNEL__:99"}),
        )
    }

    fn start(&self, mog: &str) -> String {
        let state = self
            .call("capture_start", json!({"mogId": mog, "mode": "ptt"}))
            .expect("the capture starts");
        state["leaseId"].as_str().expect("a lease ID").to_owned()
    }

    fn stop(&self, mog: &str, lease: &str) {
        self.call("capture_stop", json!({"mogId": mog, "leaseId": lease}))
            .expect("the capture stops");
    }

    fn last_state(&self) -> Value {
        self.relayed
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(e, _)| e == events::CAPTURE_STATE)
            .map(|(_, p)| p.clone())
            .expect("a capture:state item")
    }
}

/// A constant block at `amp`: its RMS is `amp`, so its level is `20 * log10(amp)` dBFS.
fn tone(amp: f32, n: usize) -> Vec<f32> {
    vec![amp; n]
}

fn assert_dbfs(v: &Value, want: f64) {
    let got = v.as_f64().expect("a level is a JSON number");
    assert!((got - want).abs() < 0.01, "level {got} dBFS, want {want}");
}

const ON_LEVEL: &str = "__CHANNEL__:7";

// ---------- CAP-16a ----------

#[test]
fn cap_16a_at_14_a_mic_level_reaches_the_subscribed_channel_end_to_end() {
    let got = Received::default();
    let r = rig(HashMap::from([(ON_LEVEL, got.clone())]));
    assert_eq!(r.subscribe(ON_LEVEL), Ok(json!({})));
    r.start("a1");
    // The brain routes the level to the target MOG that `capture:state` names (level-stream.ts).
    let state = r.last_state();
    assert_eq!(state["recording"], true);
    assert_eq!(state["owner"], "a1");
    assert!(r.mic.feed(&tone(0.5, WINDOW)), "the mic stream is open");
    assert!(r.mic.feed(&tone(0.1, WINDOW)));
    let levels = got.all();
    assert_eq!(levels.len(), 2, "one level per window: {levels:?}");
    assert_dbfs(&levels[0], 20.0 * 0.5f64.log10());
    assert_dbfs(&levels[1], -20.0);
}

#[test]
fn cap_16a_a_level_goes_out_at_30_hz_or_less_through_the_host() {
    let got = Received::default();
    let r = rig(HashMap::from([(ON_LEVEL, got.clone())]));
    r.subscribe(ON_LEVEL).unwrap();
    r.start("a1");
    // One second of audio in 10 ms blocks.
    for _ in 0..100 {
        r.mic.feed(&tone(0.3, RATE as usize / 100));
    }
    let n = got.all().len();
    assert!((29..=30).contains(&n), "{n} levels in 1 s of audio");
}

#[test]
fn cap_16a_no_level_before_capture_subscribe_and_no_sample_crosses() {
    let got = Received::default();
    let r = rig(HashMap::from([(ON_LEVEL, got.clone())]));
    let lease = r.start("a1");
    r.mic.feed(&tone(0.5, WINDOW));
    assert!(got.all().is_empty(), "no Channel yet: the level is dropped");
    r.stop("a1", &lease);
    r.subscribe(ON_LEVEL).unwrap();
    r.start("a1");
    r.mic.feed(&tone(0.5, WINDOW));
    let levels = got.all();
    assert_eq!(levels.len(), 1);
    // Only one number per window crosses IPC, never a sample (CAP-01b).
    assert!(levels.iter().all(Value::is_number));
}

#[test]
fn cap_16a_a_new_capture_subscribe_replaces_the_level_channel() {
    let (old, new) = (Received::default(), Received::default());
    let r = rig(HashMap::from([
        (ON_LEVEL, old.clone()),
        ("__CHANNEL__:8", new.clone()),
    ]));
    r.subscribe(ON_LEVEL).unwrap();
    r.start("a1");
    r.mic.feed(&tone(0.5, WINDOW));
    // A brain reload subscribes again with new Channels.
    r.subscribe("__CHANNEL__:8").unwrap();
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(old.all().len(), 1, "the old Channel gets nothing more");
    assert_eq!(new.all().len(), 1);
}

#[test]
fn cap_16a_capture_subscribe_fails_closed_when_the_channel_does_not_resolve() {
    let got = Received::default();
    let r = rig(HashMap::from([(ON_LEVEL, got.clone())]));
    r.subscribe(ON_LEVEL).unwrap();
    // A reference that does not resolve (no brain webview) is the host's failure.
    assert_eq!(
        r.subscribe("__CHANNEL__:5"),
        Err(json!({"error": "host_failed"}))
    );
    // The Channel that worked before stays.
    r.start("a1");
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 1);
}

// ---------- CAP-16b ----------

#[test]
fn cap_16b_at_14_no_level_reaches_the_channel_after_the_stop() {
    let got = Received::default();
    let r = rig(HashMap::from([(ON_LEVEL, got.clone())]));
    r.subscribe(ON_LEVEL).unwrap();
    let lease = r.start("a1");
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 1);
    r.stop("a1", &lease);
    assert_eq!(r.last_state()["recording"], false);
    // A late data callback after the stop: no level goes out.
    r.mic.feed(&tone(0.5, WINDOW));
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 1, "no level after the stop");
    // The next capture streams again.
    r.start("b2");
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 2);
}

#[test]
fn cap_16b_no_level_reaches_the_channel_after_stop_all() {
    let got = Received::default();
    let r = rig(HashMap::from([(ON_LEVEL, got.clone())]));
    r.subscribe(ON_LEVEL).unwrap();
    r.start("a1");
    // Positive control: the stream runs before the panic stop.
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(
        got.all().len(),
        1,
        "a level reaches the Channel before stop_all"
    );
    r.call("capture_stop_all", json!({})).unwrap();
    r.mic.feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 1, "no level after stop_all");
}

// ---------- CAP-16b: the host's gate alone ----------
//
// In the tests above, `MicDevice` also drops a late data callback, because `CaptureCore`
// closes the mic before it sends the stop state ("decide, close, then tell", MB-05). So those
// tests pass even if the gate never closes. A frame source that stays open while the capture
// stops cannot come from the real `MicDevice` today: a paused session keeps its mic open, but
// no session starts before #65 (TODO(#65)). This test keeps the frame source open (a
// `LevelSink` on the host's gate, fed directly, over the fake device), so only the gate that
// the host drives through `LevelCaptureEvents` stops the levels. The gate itself is also
// covered by `crates/mog-core/tests/level_stream.rs`.

#[test]
fn cap_16b_the_host_gate_alone_stops_the_levels_when_the_capture_stops() {
    use mog_core::audio_device::level::LevelSink;
    use mog_core::audio_device::{FrameSink, MicFrame};
    use mog_core::testkit::capture::FakeAudioDevice;

    let got = Received::default();
    let levels = HostLevels::new(resolver(HashMap::from([(ON_LEVEL, got.clone())])));
    // The frame source stays open the whole test: it never sees the mic close.
    let mut open_source = LevelSink::new(levels.gate().clone());
    let mut feed = |samples: &[f32]| {
        open_source.frame(MicFrame {
            samples,
            sample_rate: RATE,
        })
    };
    let host = CaptureHost::with_levels(
        Box::new(FakeAudioDevice::new()),
        Arc::new(SimClock::new()) as Arc<dyn MonotonicClock>,
        Arc::new(|_: &str, _| true),
        levels,
    )
    .expect("the cap timer thread starts");
    let r = Rig {
        host,
        mic: FakeMic::default(),
        relayed: Arc::default(),
    };
    r.subscribe(ON_LEVEL).unwrap();
    feed(&tone(0.5, WINDOW));
    assert!(
        got.all().is_empty(),
        "no capture records: the gate is closed"
    );
    let lease = r.start("a1");
    feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 1, "the capture opened the gate");
    r.stop("a1", &lease);
    feed(&tone(0.5, WINDOW));
    feed(&tone(0.5, WINDOW));
    assert_eq!(got.all().len(), 1, "the stop closed the gate (CAP-16b)");
    let _ = r.start("b2");
    r.call("capture_stop_all", json!({})).unwrap();
    let before = got.all().len();
    feed(&tone(0.5, WINDOW));
    assert_eq!(
        got.all().len(),
        before,
        "stop_all closed the gate (CAP-16b)"
    );
}

// ---------- the app resolver ----------

#[test]
fn cap_16a_the_app_resolves_onlevel_only_on_the_brain_webview() {
    let app = tauri::test::mock_app();
    let resolve = brain_level_resolver(app.handle().clone());
    let on_level = ChannelRef::try_from(ON_LEVEL.to_owned()).unwrap();
    assert!(resolve(&on_level).is_none(), "no brain webview yet");
    tauri::WebviewWindowBuilder::new(&app, "hub", tauri::WebviewUrl::default())
        .build()
        .unwrap();
    assert!(
        resolve(&on_level).is_none(),
        "only the brain owns capture_subscribe"
    );
    tauri::WebviewWindowBuilder::new(&app, "brain", tauri::WebviewUrl::default())
        .build()
        .unwrap();
    let out = resolve(&on_level).expect("the brain webview resolves the Channel");
    // A send on the mock webview must not panic on the audio thread.
    out.level(-12.5);
}

#[test]
fn cap_16a_a_channel_level_out_stops_sending_after_a_failed_send() {
    // A failed send means the webview is gone. Each later send would only fail again on the
    // audio thread, so the receiver stops until the next `capture_subscribe` replaces it.
    let calls = Arc::new(Mutex::new(0u32));
    let seen = Arc::clone(&calls);
    let channel: Channel<f32> = Channel::new(move |_| {
        *seen.lock().unwrap() += 1;
        Err(tauri::Error::WebviewNotFound)
    });
    let out = ChannelLevelOut::new(channel);
    out.level(-20.0);
    out.level(-20.0);
    out.level(-20.0);
    assert_eq!(*calls.lock().unwrap(), 1, "one failed send, then no more");
}
