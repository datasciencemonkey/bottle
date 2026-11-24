//! R4 SttEngine (#46, ADR 0082): model residency (CAP-51a, CAP-51b), preload on key-down
//! (CAP-51c), the decode of closed VAD segments during speech (CAP-52), the model identity on
//! each transcript (CAP-72) and the -3 dBFS input that R4 reads (CAP-20, the #23 hand-off).
//!
//! Fakes only: no model, no device, no network. The engine runs on its own worker thread;
//! `sync()` waits until the worker has done every earlier call.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::audio_device::normalize::{peak_dbfs, TARGET_PEAK_DBFS};
use mog_core::audio_device::sink::{FrameSink, MicFrame, SinkEvent};
use mog_core::capture::{CaptureEvents, StopReason, Stopped};
use mog_core::clock::{MonotonicClock, SimClock};
use mog_core::stt::{SttEngine as _, LOCAL_STT_MODEL, LOCAL_STT_RUNTIME};
use mog_core::stt_engine::{
    residency, EnergyVad, EngineConfig, EngineParts, LocalStt, LocalSttEngine, PowerProbe,
    PowerSource, Recognizer, RecognizerLoader, Resampler, SegmentCutter, SegmentRule,
    SttCaptureEvents, SttDone, SttIdentity, TranscriptSink, Vad, STT_SAMPLE_RATE, VAD_WINDOW,
    VAD_WINDOW_MS,
};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack, TranscriptProvider};

// ---------- Fakes ----------

/// One call to the fake recognizer: the length (16 kHz samples) and the peak it read.
#[derive(Debug, Clone, Copy)]
struct Call {
    len: usize,
    peak_dbfs: f32,
}

#[derive(Default)]
struct Log {
    calls: Mutex<Vec<Call>>,
    loads: AtomicU32,
    fail_load: AtomicBool,
    panic_next: AtomicBool,
}

struct FakeRecognizer {
    log: Arc<Log>,
    clock: SimClock,
    decode_ms: u64,
}

impl Recognizer for FakeRecognizer {
    fn transcribe(&mut self, samples: &[f32]) -> String {
        if self.log.panic_next.swap(false, Ordering::SeqCst) {
            panic!("fake recognizer fault");
        }
        let mut calls = self.log.calls.lock().unwrap();
        calls.push(Call {
            len: samples.len(),
            peak_dbfs: peak_dbfs(samples),
        });
        self.clock.advance(Duration::from_millis(self.decode_ms));
        format!("s{}", calls.len())
    }
}

struct FakeLoader {
    log: Arc<Log>,
    clock: SimClock,
    decode_ms: u64,
}

impl RecognizerLoader for FakeLoader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        if self.log.fail_load.load(Ordering::SeqCst) {
            return Err("stt_model_missing");
        }
        self.log.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeRecognizer {
            log: Arc::clone(&self.log),
            clock: self.clock.clone(),
            decode_ms: self.decode_ms,
        }))
    }

    fn identity(&self) -> SttIdentity {
        SttIdentity::local_parakeet()
    }
}

#[derive(Debug)]
struct FakePower(Mutex<PowerSource>);

impl FakePower {
    fn set(&self, p: PowerSource) {
        *self.0.lock().unwrap() = p;
    }
}

impl PowerProbe for FakePower {
    fn source(&self) -> PowerSource {
        *self.0.lock().unwrap()
    }
}

#[derive(Default)]
struct Out(Mutex<Vec<SttDone>>);

impl TranscriptSink for Out {
    fn done(&self, done: SttDone) {
        self.0.lock().unwrap().push(done);
    }
}

impl Out {
    fn all(&self) -> Vec<SttDone> {
        self.0.lock().unwrap().clone()
    }
}

struct Rig {
    engine: LocalSttEngine,
    log: Arc<Log>,
    power: Arc<FakePower>,
    clock: SimClock,
    out: Arc<Out>,
}

fn rig_with(power: PowerSource, decode_ms: u64) -> Rig {
    let log = Arc::new(Log::default());
    let clock = SimClock::new();
    let power = Arc::new(FakePower(Mutex::new(power)));
    let out = Arc::new(Out::default());
    let loader = Arc::new(FakeLoader {
        log: Arc::clone(&log),
        clock: clock.clone(),
        decode_ms,
    });
    let mut parts = EngineParts::new(loader, Arc::clone(&out) as Arc<dyn TranscriptSink>);
    parts.power = Arc::clone(&power) as Arc<dyn PowerProbe>;
    parts.clock = Arc::new(clock.clone()) as Arc<dyn MonotonicClock>;
    let engine = LocalSttEngine::spawn(parts);
    engine.sync();
    Rig {
        engine,
        log,
        power,
        clock,
        out,
    }
}

fn rig(power: PowerSource) -> Rig {
    rig_with(power, 0)
}

const RATE: u32 = 48_000;

/// A 220 Hz tone at `dbfs` peak, `ms` long, at `rate`.
fn tone(ms: u64, dbfs: f32, rate: u32) -> Vec<f32> {
    let amp = 10f32.powf(dbfs / 20.0);
    let n = (u64::from(rate) * ms / 1_000) as usize;
    (0..n)
        .map(|i| amp * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / rate as f32).sin())
        .collect()
}

fn silence(ms: u64, rate: u32) -> Vec<f32> {
    vec![0.0; (u64::from(rate) * ms / 1_000) as usize]
}

/// Feed `samples` in 10 ms blocks, like a mic data callback.
fn feed(engine: &LocalSttEngine, samples: &[f32], rate: u32) {
    for block in samples.chunks((rate / 100) as usize) {
        engine.frames(block, rate);
    }
}

fn calls(log: &Log) -> Vec<Call> {
    log.calls.lock().unwrap().clone()
}

fn ms_of(len: usize) -> u64 {
    len as u64 * 1_000 / u64::from(STT_SAMPLE_RATE)
}

fn state(lease: u64, mode: CaptureMode, recording: bool) -> CaptureState {
    CaptureState {
        owner: Some("mog-a".into()),
        mode: Some(mode),
        lease_id: Some(lease.to_string()),
        recording,
        paused: false,
        session_id: None,
        started_at: None,
        directed_to: None,
        others_track: OthersTrack::Off,
        mic: None,
    }
}

fn idle_state() -> CaptureState {
    CaptureState {
        owner: None,
        mode: None,
        lease_id: None,
        recording: false,
        paused: false,
        session_id: None,
        started_at: None,
        directed_to: None,
        others_track: OthersTrack::Off,
        mic: None,
    }
}

fn stopped(lease: u64, reason: StopReason) -> Stopped {
    Stopped {
        lease_id: lease,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Ptt,
        reason,
    }
}

#[derive(Default)]
struct NoEvents;

impl CaptureEvents for NoEvents {
    fn overlay_granted(&self, _lease: &mog_core::capture::Lease) {}
    fn state(&self, _state: &CaptureState) {}
    fn stopped(&self, _stopped: &Stopped) {}
}

// ---------- CAP-51a / CAP-51b: residency ----------

#[test]
fn cap_51a_the_policy_never_unloads_on_ac_power() {
    let eight_h = Duration::from_secs(8 * 3_600);
    assert!(!residency::should_unload(PowerSource::Ac, eight_h));
    // A desktop with no battery reports no source: it counts as AC power.
    assert!(!residency::should_unload(PowerSource::Unknown, eight_h));
}

#[test]
fn cap_51b_the_policy_unloads_after_30_min_idle_on_battery() {
    assert_eq!(residency::BATTERY_IDLE_UNLOAD, Duration::from_secs(30 * 60));
    assert!(!residency::should_unload(
        PowerSource::Battery,
        Duration::from_secs(30 * 60 - 1)
    ));
    assert!(residency::should_unload(
        PowerSource::Battery,
        Duration::from_secs(30 * 60)
    ));
}

#[test]
fn cap_51a_model_loads_at_start_and_stays_resident_for_8_h_on_ac() {
    let r = rig(PowerSource::Ac);
    assert!(r.engine.status().loaded, "on AC the model loads at start");
    for _ in 0..(8 * 6) {
        r.clock.advance(Duration::from_secs(10 * 60));
        r.engine.tick();
    }
    r.engine.sync();
    let s = r.engine.status();
    assert!(s.loaded, "the model stays loaded for 8 h on AC");
    assert_eq!(s.unloads, 0);
    assert_eq!(r.log.loads.load(Ordering::SeqCst), 1);
}

#[test]
fn cap_51b_model_unloads_after_30_min_idle_on_battery_only() {
    let r = rig(PowerSource::Battery);
    assert!(
        !r.engine.status().loaded,
        "on battery nothing loads before a key-down"
    );
    r.engine.preload();
    r.engine.sync();
    assert!(r.engine.status().loaded);

    r.clock.advance(Duration::from_secs(20 * 60));
    r.engine.preload(); // a key-down at 20 min resets the idle time
    r.engine.sync();
    r.clock.advance(Duration::from_secs(29 * 60));
    r.engine.tick();
    r.engine.sync();
    assert!(r.engine.status().loaded, "29 min idle keeps the model");

    r.clock.advance(Duration::from_secs(60));
    r.engine.tick();
    r.engine.sync();
    let s = r.engine.status();
    assert!(!s.loaded, "30 min idle on battery unloads the model");
    assert_eq!(s.unloads, 1);
}

#[test]
fn cap_51a_cap_51b_unplug_then_plug_in_follows_the_power_source() {
    let r = rig(PowerSource::Ac);
    r.power.set(PowerSource::Battery);
    r.clock.advance(Duration::from_secs(31 * 60));
    r.engine.tick();
    r.engine.sync();
    assert!(!r.engine.status().loaded, "idle on battery unloads");
    r.power.set(PowerSource::Ac);
    r.engine.tick();
    r.engine.sync();
    assert!(
        r.engine.status().loaded,
        "back on AC the model is resident again"
    );
    assert_eq!(r.log.loads.load(Ordering::SeqCst), 2);
}

// ---------- CAP-51c: preload on key-down ----------

#[test]
fn cap_51c_key_down_preloads_so_key_up_does_not_load() {
    let r = rig(PowerSource::Battery);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    // Key-down: the hotkey host preloads; the granted PTT state also preloads (idempotent).
    r.engine.preload();
    events.state(&state(1, CaptureMode::Ptt, true));
    r.engine.sync();
    assert_eq!(
        r.log.loads.load(Ordering::SeqCst),
        1,
        "one load per key-down"
    );
    feed(&r.engine, &tone(1_000, -6.0, RATE), RATE);
    events.stopped(&stopped(1, StopReason::Released));
    r.engine.sync();
    assert_eq!(
        r.log.loads.load(Ordering::SeqCst),
        1,
        "the key-up path does not load the model"
    );
    assert_eq!(r.out.all().len(), 1);
}

#[test]
fn cap_51c_first_ptt_after_an_unload_reloads_at_key_down() {
    let r = rig(PowerSource::Battery);
    r.engine.preload();
    r.engine.sync();
    r.clock.advance(Duration::from_secs(31 * 60));
    r.engine.tick();
    r.engine.sync();
    assert!(!r.engine.status().loaded);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    events.state(&state(2, CaptureMode::Ptt, true));
    r.engine.sync();
    assert!(
        r.engine.status().loaded,
        "the granted PTT reloads the model"
    );
    assert_eq!(r.log.loads.load(Ordering::SeqCst), 2);
}

#[test]
fn cap_51c_a_failed_load_gives_a_content_free_error_transcript() {
    let r = rig(PowerSource::Battery);
    r.log.fail_load.store(true, Ordering::SeqCst);
    r.engine.preload();
    r.engine.begin(3, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(800, -6.0, RATE), RATE);
    r.engine.commit(3, StopReason::Released);
    r.engine.sync();
    let done = r.out.all();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].result, Err("stt_model_missing"));
}

// ---------- CAP-52: decode closed segments during speech ----------

#[test]
fn cap_52_closed_segment_decodes_during_speech_and_only_the_tail_after_key_up() {
    // Each decode costs 100 ms of (simulated) time.
    let r = rig_with(PowerSource::Ac, 100);
    r.engine.begin(4, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(3_200, -6.0, RATE), RATE);
    feed(&r.engine, &silence(400, RATE), RATE);
    feed(&r.engine, &tone(1_000, -6.0, RATE), RATE);
    r.engine.sync();
    let during = calls(&r.log);
    assert_eq!(
        during.len(),
        1,
        "the first segment decodes while the key is down"
    );
    assert!(
        (3_200..=3_700).contains(&ms_of(during[0].len)),
        "the first segment ends in the pause: {} ms",
        ms_of(during[0].len)
    );

    r.engine.commit(4, StopReason::Released);
    r.engine.sync();
    let all = calls(&r.log);
    assert_eq!(all.len(), 2, "after key-up only the tail decodes");
    assert!(
        ms_of(all[1].len) <= 1_200,
        "the tail is the audio after the cut: {} ms",
        ms_of(all[1].len)
    );
    let done = r.out.all();
    assert_eq!(done.len(), 1);
    let t = done[0].result.as_ref().expect("a transcript");
    assert_eq!(t.raw, "s1 s2");
    assert_eq!(t.text, "s1 s2");
    // CAP-52 / section 8.1: the latency is the tail decode after key-up, not the whole hold.
    assert_eq!(t.latency_ms, 100);
    assert_eq!(done[0].lease_id, 4);
    assert_eq!(done[0].mog_id, "mog-a");
}

#[test]
fn cap_52_short_pause_before_3_s_does_not_cut() {
    let r = rig(PowerSource::Ac);
    r.engine.begin(5, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(1_000, -6.0, RATE), RATE);
    feed(&r.engine, &silence(500, RATE), RATE);
    feed(&r.engine, &tone(1_000, -6.0, RATE), RATE);
    r.engine.sync();
    assert!(calls(&r.log).is_empty(), "no cut before 3 s of audio");
    r.engine.commit(5, StopReason::Released);
    r.engine.sync();
    assert_eq!(
        calls(&r.log).len(),
        1,
        "a short utterance is one tail decode"
    );
}

#[test]
fn cap_52_continuous_speech_is_cut_at_30_s() {
    let r = rig(PowerSource::Ac);
    r.engine.begin(6, "mog-a", CaptureMode::HandsFree);
    feed(&r.engine, &tone(31_000, -6.0, 16_000), 16_000);
    r.engine.sync();
    let c = calls(&r.log);
    assert_eq!(c.len(), 1, "a hard cut at 30 s");
    assert!(ms_of(c[0].len) <= 30_000);
}

#[test]
fn cap_52_segment_cutter_rule() {
    let rule = SegmentRule::default();
    assert_eq!(rule.min_segment_ms, 3_000);
    assert_eq!(rule.pause_ms, 300);
    assert_eq!(rule.max_segment_ms, 30_000);
    let mut cut = SegmentCutter::new(rule);
    let win_ms = VAD_WINDOW as u64 * 1_000 / u64::from(STT_SAMPLE_RATE);
    let mut at = None;
    for i in 0..((4_000 / win_ms) as usize) {
        // Speech to 3.1 s, then silence.
        let speech = (i as u64) * win_ms < 3_100;
        if cut.push(speech) {
            at = Some(i as u64 * win_ms);
            break;
        }
    }
    let at = at.expect("a cut in the pause after 3 s");
    assert!((3_300..=3_500).contains(&at), "cut at {at} ms");
}

#[test]
fn cap_52_silence_only_segment_is_not_decoded() {
    let r = rig(PowerSource::Ac);
    r.engine.begin(7, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(500, -6.0, RATE), RATE);
    feed(&r.engine, &silence(3_500, RATE), RATE);
    r.engine.sync();
    let c = calls(&r.log);
    assert_eq!(c.len(), 1, "the first segment has speech");
    feed(&r.engine, &silence(3_500, RATE), RATE);
    r.engine.commit(7, StopReason::Released);
    r.engine.sync();
    assert_eq!(
        calls(&r.log).len(),
        1,
        "segments and a tail with no speech make no decode"
    );
}

#[test]
fn cap_52_discarded_and_panic_stops_make_no_transcript_and_hold_no_audio() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    for (lease, reason) in [
        (10, StopReason::Discarded),
        (11, StopReason::Escape),
        (12, StopReason::ZeroBuffers),
        (13, StopReason::Panic),
    ] {
        events.state(&state(lease, CaptureMode::Ptt, true));
        feed(&r.engine, &tone(3_500, -6.0, RATE), RATE);
        feed(&r.engine, &silence(400, RATE), RATE);
        feed(&r.engine, &tone(500, -6.0, RATE), RATE);
        events.stopped(&stopped(lease, reason));
    }
    r.engine.sync();
    assert!(
        r.out.all().is_empty(),
        "no transcript for a dropped capture"
    );
    let s = r.engine.status();
    assert_eq!(s.open_takes, 0);
    assert_eq!(s.held_samples, 0);
}

#[test]
fn cap_52_zero_buffer_sink_event_drops_every_take() {
    let r = rig(PowerSource::Ac);
    r.engine.begin(14, "mog-a", CaptureMode::Ptt);
    let mut sink = r.engine.frame_sink();
    let block = tone(10, -6.0, RATE);
    for _ in 0..50 {
        sink.frame(MicFrame {
            samples: &block,
            sample_rate: RATE,
        });
    }
    sink.event(SinkEvent::Discard);
    r.engine.commit(14, StopReason::Released);
    r.engine.sync();
    assert!(r.out.all().is_empty());
    assert_eq!(r.engine.status().held_samples, 0);
}

#[test]
fn cap_52_frames_with_no_live_lease_are_dropped() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    feed(&r.engine, &tone(500, -6.0, RATE), RATE); // before any lease: no pre-roll
    events.state(&state(15, CaptureMode::Ptt, true));
    feed(&r.engine, &tone(500, -6.0, RATE), RATE);
    events.state(&idle_state());
    feed(&r.engine, &tone(2_000, -6.0, RATE), RATE); // after the lease went idle
    r.engine.sync();
    let held = r.engine.status().held_samples;
    assert!(
        (7_900..=8_100).contains(&held),
        "only the 500 ms of the live lease is held at 16 kHz: {held}"
    );
    events.stopped(&stopped(15, StopReason::Released));
    r.engine.sync();
    assert_eq!(calls(&r.log).len(), 1);
    assert_eq!(
        r.engine.status().held_samples,
        0,
        "the audio is gone after the decode"
    );
}

#[test]
fn cap_52_a_recognizer_panic_drops_the_model_and_the_next_use_reloads() {
    let r = rig(PowerSource::Ac);
    r.log.panic_next.store(true, Ordering::SeqCst);
    r.engine.begin(16, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(800, -6.0, RATE), RATE);
    r.engine.commit(16, StopReason::Released);
    r.engine.sync();
    let done = r.out.all();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].result, Err("stt_engine_failed"));
    assert!(!r.engine.status().loaded, "a faulted model is dropped");
    r.engine.begin(17, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(800, -6.0, RATE), RATE);
    r.engine.commit(17, StopReason::Released);
    r.engine.sync();
    assert!(
        r.out.all()[1].result.is_ok(),
        "the next PTT loads a new model"
    );
    assert_eq!(r.log.loads.load(Ordering::SeqCst), 2);
}

#[test]
fn cap_52_ptt_over_hands_free_keeps_two_takes_apart() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    events.state(&state(20, CaptureMode::HandsFree, true));
    feed(&r.engine, &tone(500, -6.0, RATE), RATE);
    events.state(&state(21, CaptureMode::Ptt, true));
    feed(&r.engine, &tone(700, -6.0, RATE), RATE);
    events.stopped(&stopped(21, StopReason::Released));
    events.state(&state(20, CaptureMode::HandsFree, true));
    feed(&r.engine, &tone(300, -6.0, RATE), RATE);
    events.stopped(&Stopped {
        lease_id: 20,
        mog_id: "mog-a".into(),
        mode: CaptureMode::HandsFree,
        reason: StopReason::VadEnd,
    });
    r.engine.sync();
    let c = calls(&r.log);
    assert_eq!(c.len(), 2);
    assert!((650..=750).contains(&ms_of(c[0].len)), "the PTT take alone");
    assert!(
        (750..=850).contains(&ms_of(c[1].len)),
        "the hands-free take, both parts"
    );
    let done = r.out.all();
    assert_eq!(done[0].lease_id, 21);
    assert_eq!(done[1].lease_id, 20);
    assert_eq!(done[1].mode, CaptureMode::HandsFree);
}

// ---------- CAP-20: the input that R4 reads ----------

#[test]
fn cap_20_every_segment_r4_reads_peaks_at_minus_3_dbfs() {
    let r = rig(PowerSource::Ac);
    r.engine.begin(30, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(3_200, -30.0, RATE), RATE);
    feed(&r.engine, &silence(400, RATE), RATE);
    feed(&r.engine, &tone(800, -12.0, RATE), RATE);
    r.engine.commit(30, StopReason::Released);
    r.engine.sync();
    let c = calls(&r.log);
    assert_eq!(c.len(), 2);
    for call in c {
        assert!(
            (call.peak_dbfs - TARGET_PEAK_DBFS).abs() < 0.5,
            "R4 read a peak of {} dBFS",
            call.peak_dbfs
        );
    }
}

// ---------- CAP-72: identity on each transcript ----------

#[test]
fn cap_72_each_transcript_names_provider_model_and_runtime() {
    let id = SttIdentity::local_parakeet();
    assert_eq!(id.provider, TranscriptProvider::LocalParakeet);
    assert_eq!(id.model, LOCAL_STT_MODEL);
    assert_eq!(id.runtime, LOCAL_STT_RUNTIME);
    assert!(LOCAL_STT_MODEL.contains("parakeet"));
    assert!(LOCAL_STT_RUNTIME.contains("sherpa-onnx"));

    let r = rig(PowerSource::Ac);
    for lease in [40, 41] {
        r.engine.begin(lease, "mog-a", CaptureMode::Ptt);
        feed(&r.engine, &tone(600, -6.0, RATE), RATE);
        r.engine.commit(lease, StopReason::Released);
    }
    r.engine.sync();
    let done = r.out.all();
    assert_eq!(done.len(), 2);
    for d in done {
        let t = d.result.expect("a transcript");
        assert_eq!(t.provider, TranscriptProvider::LocalParakeet);
        assert_eq!(t.model, LOCAL_STT_MODEL);
        assert_eq!(t.runtime, LOCAL_STT_RUNTIME);
        assert_eq!(t.segment_id, format!("ptt-{}", d.lease_id));
        assert!(t.lang.is_none());
    }
}

#[test]
fn cap_72_stt_transcribe_returns_the_stored_transcript_once() {
    let log = Arc::new(Log::default());
    let clock = SimClock::new();
    let loader = Arc::new(FakeLoader {
        log: Arc::clone(&log),
        clock: clock.clone(),
        decode_ms: 0,
    });
    let out = Arc::new(Out::default());
    let stt = LocalStt::new(
        loader,
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
        |parts| {
            parts.power = Arc::new(FakePower(Mutex::new(PowerSource::Battery)));
            parts.clock = Arc::new(clock.clone());
        },
    );
    stt.load_model().expect("the fake model loads");
    stt.engine().begin(50, "mog-a", CaptureMode::Ptt);
    feed(stt.engine(), &tone(600, -6.0, RATE), RATE);
    stt.engine().commit(50, StopReason::Released);
    stt.engine().sync();
    assert_eq!(out.all().len(), 1, "the host still gets each result");
    let t = stt.transcribe("ptt-50").expect("the stored transcript");
    assert_eq!(t.provider, TranscriptProvider::LocalParakeet);
    assert_eq!(t.model, LOCAL_STT_MODEL);
    assert!(
        stt.transcribe("ptt-50").is_err(),
        "a transcript is handed out once"
    );
    assert!(stt.transcribe("ptt-999").is_err());
}

// ---------- Parts: resampler, VAD, config ----------

#[test]
fn cap_52_resampler_brings_any_device_rate_to_16_khz() {
    for (rate, n) in [(48_000u32, 48_000usize), (44_100, 44_100), (16_000, 16_000)] {
        let mut rs = Resampler::new(rate);
        let mut out = Vec::new();
        let input = tone(1_000, -6.0, rate);
        assert_eq!(input.len(), n);
        for block in input.chunks(441) {
            rs.process(block, &mut out);
        }
        assert!(
            (15_990..=16_010).contains(&out.len()),
            "{rate} Hz gave {} samples",
            out.len()
        );
    }
    let mut same = Resampler::new(16_000);
    let mut out = Vec::new();
    same.process(&[0.1, 0.2, 0.3], &mut out);
    assert_eq!(out, vec![0.1, 0.2, 0.3], "16 kHz passes through unchanged");
}

#[test]
fn cap_52_energy_vad_tells_speech_from_silence() {
    let mut vad = EnergyVad::default();
    let speech = tone(32, -30.0, STT_SAMPLE_RATE);
    assert_eq!(speech.len(), VAD_WINDOW);
    assert!(vad.is_speech(&speech[..VAD_WINDOW]));
    assert!(!vad.is_speech(&vec![0.0; VAD_WINDOW]));
    assert!(
        !vad.is_speech(&vec![1e-4; VAD_WINDOW]),
        "room noise is not speech"
    );
}

#[test]
fn cap_51a_engine_config_defaults() {
    let c = EngineConfig::default();
    assert_eq!(c.idle_unload, residency::BATTERY_IDLE_UNLOAD);
    assert_eq!(c.tick, Duration::from_secs(60));
    assert_eq!(c.rule, SegmentRule::default());
}

// ---------- #21 hookup: R4 is the PTT buffer's consumer ----------

#[test]
fn cap_52_cap_34_r4_reads_the_mic_tee_and_consumes_the_ptt_buffer_utterance() {
    use mog_core::audio_device::level::TeeSink;
    use mog_core::audio_device::ptt_buffer::{PttBuffer, PttCaptureEvents};
    let r = rig(PowerSource::Ac);
    // The host chain (ADR 0082): the mic tees the PTT buffer and R4; R2's events reach the PTT
    // buffer, then R4; the PTT buffer hands its utterance to R4.
    let buf = PttBuffer::new(Arc::new(r.engine.clone()));
    let mut mic = TeeSink(buf.sink(), r.engine.frame_sink());
    let events = PttCaptureEvents::new(
        SttCaptureEvents::new(NoEvents, r.engine.clone()),
        buf.clone(),
    );
    for (lease, reason, kept) in [(60, StopReason::Released, 1), (61, StopReason::Panic, 1)] {
        events.state(&state(lease, CaptureMode::Ptt, true));
        for block in tone(1_000, -6.0, RATE).chunks(480) {
            mic.frame(MicFrame {
                samples: block,
                sample_rate: RATE,
            });
        }
        events.stopped(&stopped(lease, reason));
        r.engine.sync();
        assert_eq!(r.out.all().len(), kept, "lease {lease}");
        assert_eq!(buf.held_samples(), 0, "the PTT buffer keeps no copy");
        assert_eq!(r.engine.status().held_samples, 0, "R4 keeps no audio");
    }
    let t = r.out.all()[0].result.clone().expect("a transcript");
    assert_eq!(t.segment_id, "ptt-60");
}

// ---------- Review of PR #520: privacy of Debug, supervision, guards ----------

#[test]
fn sec_cap_01b_stt_done_debug_shows_no_transcript_text() {
    let r = rig(PowerSource::Ac);
    r.engine.begin(70, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(600, -6.0, RATE), RATE);
    r.engine.commit(70, StopReason::Released);
    r.engine.sync();
    let done = r.out.all().pop().expect("a result");
    // The fake recognizer's text is "s1"; give the transcript a marker too.
    let mut marked = done.clone();
    if let Ok(t) = marked.result.as_mut() {
        t.text = "SECRET-MARKER-TEXT".into();
        t.raw = "SECRET-MARKER-RAW".into();
    }
    let shown = format!("{marked:?} {:#?}", marked);
    assert!(
        !shown.contains("SECRET-MARKER"),
        "Debug leaked the text: {shown}"
    );
    assert!(shown.contains("70"), "Debug still shows the lease");
}

struct PanicVad(Arc<AtomicU32>);
impl Vad for PanicVad {
    fn is_speech(&mut self, _w: &[f32]) -> bool {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("fake VAD fault");
        }
        true
    }
}

#[test]
fn cap_52_a_vad_panic_does_not_kill_the_worker() {
    let log = Arc::new(Log::default());
    let clock = SimClock::new();
    let out = Arc::new(Out::default());
    let mut parts = EngineParts::new(
        Arc::new(FakeLoader {
            log: Arc::clone(&log),
            clock: clock.clone(),
            decode_ms: 0,
        }),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    let calls = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&calls);
    parts.vad = Arc::new(move || Box::new(PanicVad(Arc::clone(&c))) as Box<dyn Vad>);
    parts.power = Arc::new(FakePower(Mutex::new(PowerSource::Ac)));
    parts.clock = Arc::new(clock);
    let engine = LocalSttEngine::spawn(parts);
    for lease in [80, 81] {
        engine.begin(lease, "mog-a", CaptureMode::Ptt);
        feed(&engine, &tone(600, -6.0, RATE), RATE);
        engine.commit(lease, StopReason::Released);
    }
    engine.sync();
    assert_eq!(out.all().len(), 2, "the worker lives on after a VAD panic");
}

struct PanicOnce(AtomicBool, Out);
impl TranscriptSink for PanicOnce {
    fn done(&self, done: SttDone) {
        if !self.0.swap(true, Ordering::SeqCst) {
            panic!("fake sink fault");
        }
        self.1.done(done);
    }
}

#[test]
fn cap_52_a_transcript_sink_panic_does_not_kill_the_worker() {
    let log = Arc::new(Log::default());
    let clock = SimClock::new();
    let out = Arc::new(PanicOnce(AtomicBool::new(false), Out::default()));
    let mut parts = EngineParts::new(
        Arc::new(FakeLoader {
            log: Arc::clone(&log),
            clock: clock.clone(),
            decode_ms: 0,
        }),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    parts.power = Arc::new(FakePower(Mutex::new(PowerSource::Ac)));
    parts.clock = Arc::new(clock);
    let engine = LocalSttEngine::spawn(parts);
    for lease in [82, 83] {
        engine.begin(lease, "mog-a", CaptureMode::Ptt);
        feed(&engine, &tone(600, -6.0, RATE), RATE);
        engine.commit(lease, StopReason::Released);
    }
    engine.sync();
    assert_eq!(
        out.1.all().len(),
        1,
        "the second result arrives after a sink panic"
    );
}

struct PanicLoader;
impl RecognizerLoader for PanicLoader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        panic!("fake loader fault")
    }
    fn identity(&self) -> SttIdentity {
        SttIdentity::local_parakeet()
    }
}

#[test]
fn cap_51c_a_loader_panic_gives_a_content_free_code_and_the_worker_lives() {
    let out = Arc::new(Out::default());
    let mut parts = EngineParts::new(
        Arc::new(PanicLoader),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    parts.power = Arc::new(FakePower(Mutex::new(PowerSource::Battery)));
    let engine = LocalSttEngine::spawn(parts);
    engine.preload();
    engine.sync();
    assert_eq!(engine.status().last_error, Some("stt_engine_failed"));
    engine.begin(84, "mog-a", CaptureMode::Ptt);
    feed(&engine, &tone(600, -6.0, RATE), RATE);
    engine.commit(84, StopReason::Released);
    engine.sync();
    assert_eq!(out.all()[0].result, Err("stt_engine_failed"));
}

#[test]
fn cap_51b_the_model_never_unloads_while_a_take_is_open() {
    let r = rig(PowerSource::Battery);
    r.engine.begin(85, "mog-a", CaptureMode::HandsFree);
    r.engine.sync();
    r.clock.advance(Duration::from_secs(31 * 60));
    r.engine.tick();
    r.engine.sync();
    assert!(r.engine.status().loaded, "an open take keeps the model");
    r.engine.commit(85, StopReason::Released);
    r.engine.tick();
    r.engine.sync();
    assert!(
        !r.engine.status().loaded,
        "after the take, the idle rule applies"
    );
}

#[test]
fn cap_52_a_pause_cut_needs_speech_in_the_segment() {
    let mut cut = SegmentCutter::new(SegmentRule::default());
    // 10 s of non-speech: no pause cut (a quiet input that the VAD misses is not cut up).
    for i in 0..(10_000 / VAD_WINDOW_MS) {
        assert!(!cut.push(false), "no pause cut without speech, window {i}");
    }
}

#[test]
fn cap_13a_no_frame_is_copied_to_the_worker_with_no_live_lease() {
    let r = rig(PowerSource::Ac);
    feed(&r.engine, &tone(500, -6.0, RATE), RATE);
    r.engine.sync();
    assert_eq!(
        r.engine.status().frame_messages,
        0,
        "the audio thread copied no frame"
    );
    r.engine.begin(86, "mog-a", CaptureMode::Ptt);
    feed(&r.engine, &tone(100, -6.0, RATE), RATE);
    r.engine.sync();
    assert_eq!(r.engine.status().frame_messages, 10);
}

// ---------- ADR 0118: a PTT overlay on a session lease (a hold during a recording) ----------

fn overlay_lease(id: u64) -> mog_core::capture::Lease {
    mog_core::capture::Lease {
        id,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Ptt,
    }
}

/// What `capture.state` shows while an overlay PTT lives on session lease `session`.
fn session_state(session: u64, directed: bool) -> CaptureState {
    CaptureState {
        directed_to: directed.then(|| "mog-a".to_owned()),
        ..state(session, CaptureMode::Session, true)
    }
}

fn mic_tone(r: &Rig, ms: u64) {
    feed(&r.engine, &tone(ms, -6.0, RATE), RATE);
}

#[test]
fn adr_0118_overlay_granted_begins_a_take_with_the_overlay_lease_id() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    events.state(&session_state(1, false));
    events.overlay_granted(&overlay_lease(2));
    events.state(&session_state(1, true));
    mic_tone(&r, 1_000);
    events.stopped(&stopped(2, StopReason::Released));
    r.engine.sync();
    let done = r.out.all();
    assert_eq!(done.len(), 1);
    assert_eq!(done[0].lease_id, 2, "the overlay lease, not the session");
    assert_eq!(done[0].mode, CaptureMode::Ptt);
    assert_eq!(r.engine.status().held_samples, 0);
}

#[test]
fn adr_0118_a_session_state_with_directed_to_does_not_idle_the_engine() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    events.overlay_granted(&overlay_lease(2));
    events.state(&session_state(1, true));
    events.state(&session_state(1, true));
    mic_tone(&r, 1_000);
    events.stopped(&stopped(2, StopReason::Released));
    r.engine.sync();
    let done = r.out.all();
    assert_eq!(done.len(), 1);
    assert!(
        (900..=1_100).contains(&done[0].duration_ms),
        "{}",
        done[0].duration_ms
    );
}

#[test]
fn adr_0118_a_session_state_without_directed_to_idles_and_clears_the_overlay() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    events.overlay_granted(&overlay_lease(2));
    events.state(&session_state(1, true));
    mic_tone(&r, 500);
    // The overlay ended: the state shows the bare session. Later frames go nowhere.
    events.state(&session_state(1, false));
    mic_tone(&r, 500);
    events.stopped(&stopped(2, StopReason::Released));
    r.engine.sync();
    let done = r.out.all();
    assert_eq!(done.len(), 1);
    assert!(done[0].duration_ms < 700, "{}", done[0].duration_ms);
    // The overlay is cleared: a directed state now idles the engine again.
    events.state(&session_state(1, true));
    mic_tone(&r, 500);
    r.engine.sync();
    assert_eq!(r.out.all().len(), 1);
}

#[test]
fn adr_0118_no_frames_reach_the_session_lease() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    events.state(&session_state(1, false));
    mic_tone(&r, 1_000);
    events.stopped(&Stopped {
        lease_id: 1,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Session,
        reason: StopReason::Manual,
    });
    r.engine.sync();
    assert!(r.out.all().is_empty());
    assert_eq!(r.engine.status().held_samples, 0);
}

#[test]
fn adr_0118_an_overlay_grant_after_its_stop_begins_no_take() {
    let r = rig(PowerSource::Ac);
    let events = SttCaptureEvents::new(NoEvents, r.engine.clone());
    // The events of one change can arrive out of order: the stop first, the grant late.
    events.stopped(&stopped(2, StopReason::Released));
    events.overlay_granted(&overlay_lease(2));
    events.state(&session_state(1, true));
    mic_tone(&r, 1_000);
    events.state(&session_state(1, false));
    r.engine.sync();
    assert!(r.out.all().is_empty());
    assert_eq!(r.engine.status().held_samples, 0, "no audio is kept");
}

#[test]
fn adr_0118_the_production_wrapper_order_hands_the_overlay_lease_to_the_buffer_and_r4() {
    use mog_core::audio_device::level::{LevelCaptureEvents, LevelGate, LevelOut, TeeSink};
    use mog_core::audio_device::ptt_buffer::{
        PttBuffer, PttCaptureEvents, Utterance, UtteranceSink,
    };
    #[derive(Default)]
    struct Got(Mutex<Vec<u64>>);
    impl UtteranceSink for Got {
        fn utterance(&self, u: Utterance) {
            self.0.lock().unwrap().push(u.lease_id());
        }
    }
    struct NoLevel;
    impl LevelOut for NoLevel {
        fn level(&self, _dbfs: f32) {}
    }
    let r = rig(PowerSource::Ac);
    let got = Arc::new(Got::default());
    let buf = PttBuffer::new(got.clone());
    let mut mic = TeeSink(buf.sink(), r.engine.frame_sink());
    // capture_host.rs: Level -> Ptt -> Stt -> inner.
    let events = LevelCaptureEvents::new(
        PttCaptureEvents::new(
            SttCaptureEvents::new(NoEvents, r.engine.clone()),
            buf.clone(),
        ),
        LevelGate::new(Arc::new(NoLevel)),
    );
    events.state(&session_state(1, false));
    events.overlay_granted(&overlay_lease(2));
    events.state(&session_state(1, true));
    for block in tone(1_000, -6.0, RATE).chunks(480) {
        mic.frame(MicFrame {
            samples: block,
            sample_rate: RATE,
        });
    }
    events.stopped(&stopped(2, StopReason::Released));
    events.state(&session_state(1, false));
    r.engine.sync();
    assert_eq!(*got.0.lock().unwrap(), vec![2], "the PTT buffer made one");
    let done = r.out.all();
    assert_eq!(done.len(), 1, "R4 made one");
    assert_eq!(done[0].lease_id, 2);
}
