//! The RAM-only PTT and hands-free buffer of R2 (#21, ADR 0078): CAP-12c (RAM only), CAP-13a
//! (no pre-roll), CAP-13b (the mic closes on pause), CAP-34 (no copy after STT), CAP-21 (the
//! cold stream build until SP-08), with the CAP-20 peak and the CAP-04a / CAP-04b caps.
//!
//! No real mic opens. The frames are synthetic tones, and the mic is a fake `MicHost` under the
//! real `MicDevice`.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::audio_device::level::TeeSink;
use mog_core::audio_device::normalize::{peak_dbfs, TARGET_PEAK_DBFS};
use mog_core::audio_device::ptt_buffer::{
    cap_ms, cap_samples, NoUtteranceSink, PttBuffer, PttCaptureEvents, Utterance, UtteranceSink,
    CHUNK_SAMPLES,
};
use mog_core::audio_device::{
    DataFn, DiscardSink, FaultFn, FrameSink, Interleaved, MicDevice, MicFrame, MicHost, SinkEvent,
};
use mog_core::capture::{
    CaptureCore, CaptureEvents, CaptureRequest, CaptureService, DeviceError, StopReason, Stopped,
    HANDS_FREE_MAX_MS, HOLD_MAX_MS,
};
use mog_core::clock::SimClock;
use mog_core::testkit::capture::{FakeAudioDevice, RecordingCaptureEvents};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack};

const RATE: u32 = 48_000;
const MOG_A: &str = "00000000-0000-4000-8000-0000000000a1";
const MOG_B: &str = "00000000-0000-4000-8000-0000000000b2";

// ---------- helpers ----------

/// The utterances that reached the consumer (a stand-in for R4 SttEngine, #46).
#[derive(Default)]
struct Recorded(Mutex<Vec<Utterance>>);

impl UtteranceSink for Recorded {
    fn utterance(&self, utterance: Utterance) {
        self.0.lock().unwrap().push(utterance);
    }
}

impl Recorded {
    fn take(&self) -> Vec<Utterance> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

fn buffer() -> (Arc<Recorded>, PttBuffer) {
    let out = Arc::new(Recorded::default());
    let buf = PttBuffer::new(out.clone());
    (out, buf)
}

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// A sine with the peak `amp`, `n` samples long.
fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin())
        .collect()
}

fn feed(sink: &mut impl FrameSink, samples: &[f32]) {
    for chunk in samples.chunks(480) {
        sink.frame(MicFrame {
            samples: chunk,
            sample_rate: RATE,
        });
    }
}

fn state(lease: u64, owner: &str, mode: CaptureMode) -> CaptureState {
    CaptureState {
        owner: Some(owner.to_string()),
        mode: Some(mode),
        lease_id: Some(lease.to_string()),
        recording: true,
        paused: false,
        session_id: None,
        started_at: Some(1),
        directed_to: None,
        others_track: OthersTrack::Off,
        mic: None,
    }
}

fn idle() -> CaptureState {
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

fn stopped(lease: u64, owner: &str, mode: CaptureMode, reason: StopReason) -> Stopped {
    Stopped {
        lease_id: lease,
        mog_id: owner.to_string(),
        mode,
        reason,
    }
}

// ---------- CAP-13a: no pre-roll ----------

#[test]
fn cap_13a_no_frame_before_key_down_reaches_the_utterance() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    // Loud audio before the key goes down: no capture records, so it is dropped.
    feed(&mut sink, &vec![0.9; 4_800]);
    assert_eq!(buf.held_samples(), 0, "no pre-roll is kept");
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.1, 9_600));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    buf.on_state(&idle());
    let got = out.take();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].samples().len(), 9_600, "only audio after key-down");
    // The pre-roll was a constant 0.9; the tone is a sine. No constant run of 0.9-scaled
    // samples is at the start.
    assert!(
        got[0].samples()[0].abs() < 0.01,
        "the utterance starts with the tone"
    );
}

#[test]
fn cap_13a_the_sink_keeps_no_ring_buffer_between_captures() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.2, 4_800));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    buf.on_state(&idle());
    // Between the two captures the mic would give audio: none of it is kept.
    feed(&mut sink, &vec![0.7; 48_000]);
    assert_eq!(buf.held_samples(), 0);
    assert_eq!(buf.takes(), 0);
    assert!(!buf.is_taking());
    buf.on_state(&state(2, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.2, 2_400));
    buf.on_stopped(&stopped(2, MOG_A, CaptureMode::Ptt, StopReason::Released));
    let got = out.take();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].samples().len(), 4_800);
    assert_eq!(
        got[1].samples().len(),
        2_400,
        "the second take has only its own audio"
    );
}

#[test]
fn cap_13a_a_late_state_of_a_stopped_lease_opens_no_take() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.2, 480));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    // A stale `capture.state` for lease 1 after its stop.
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.2, 4_800));
    assert_eq!(buf.held_samples(), 0);
    assert_eq!(out.take().len(), 1);
}

// ---------- CAP-13b: pause ----------

#[test]
fn cap_13b_a_paused_state_takes_no_frame() {
    let (_out, buf) = buffer();
    let mut sink = buf.sink();
    let mut paused = state(1, MOG_A, CaptureMode::HandsFree);
    buf.on_state(&paused);
    feed(&mut sink, &tone(0.2, 480));
    assert_eq!(buf.held_samples(), 480);
    paused.paused = true;
    paused.recording = false;
    buf.on_state(&paused);
    assert!(!buf.is_taking());
    feed(&mut sink, &tone(0.2, 4_800));
    assert_eq!(buf.held_samples(), 480, "no frame while paused");
}

#[test]
fn cap_13b_r2_closes_the_mic_on_pause_and_the_buffer_holds_no_session_audio() {
    let device = FakeAudioDevice::new();
    let (_out, buf) = buffer();
    let events = Arc::new(PttCaptureEvents::new(
        RecordingCaptureEvents::default(),
        buf.clone(),
    ));
    let core = CaptureCore::new(
        Box::new(device.clone()),
        Arc::new(SimClock::new()),
        events.clone(),
    );
    let lease = core
        .start(CaptureRequest {
            mog_id: MOG_A.into(),
            mode: CaptureMode::Session,
            others_track: false,
        })
        .unwrap();
    assert!(device.log().mic_open);
    feed(&mut buf.sink(), &tone(0.2, 4_800));
    assert_eq!(
        buf.held_samples(),
        0,
        "the journal owns session audio, not this buffer"
    );
    let st = core.pause(lease.id).unwrap();
    assert!(st.paused);
    assert!(!device.log().mic_open, "the mic stream is closed on pause");
    assert!(!buf.is_taking());
}

// ---------- CAP-12c: RAM only ----------

#[test]
fn cap_12c_a_released_ptt_gives_one_utterance_in_ram() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(7, MOG_A, CaptureMode::Ptt));
    let audio = tone(0.25, CHUNK_SAMPLES * 3 + 17);
    feed(&mut sink, &audio);
    assert_eq!(buf.held_samples(), audio.len());
    buf.on_stopped(&stopped(7, MOG_A, CaptureMode::Ptt, StopReason::Released));
    let got = out.take();
    assert_eq!(got.len(), 1);
    let u = &got[0];
    assert_eq!(u.lease_id(), 7);
    assert_eq!(u.mog_id(), MOG_A);
    assert_eq!(u.mode(), CaptureMode::Ptt);
    assert_eq!(u.reason(), StopReason::Released);
    assert_eq!(u.sample_rate(), RATE);
    assert_eq!(u.samples().len(), audio.len());
    // One gain over the whole take: the shape is kept (CAP-20).
    let gain = u.samples()[100] / audio[100];
    for (a, b) in audio.iter().zip(u.samples()).step_by(97) {
        assert!((a * gain - b).abs() < 1e-4);
    }
}

#[test]
fn cap_12c_a_ptt_over_hands_free_gives_two_separate_utterances() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::HandsFree));
    feed(&mut sink, &vec![0.1; 960]);
    // ARC-F9: a PTT suspends hands-free; `capture.state` shows the PTT lease.
    buf.on_state(&state(2, MOG_B, CaptureMode::Ptt));
    feed(&mut sink, &vec![0.4; 480]);
    buf.on_stopped(&stopped(2, MOG_B, CaptureMode::Ptt, StopReason::Released));
    buf.on_state(&state(1, MOG_A, CaptureMode::HandsFree));
    feed(&mut sink, &vec![0.1; 480]);
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::HandsFree, StopReason::Tap));
    let got = out.take();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].lease_id(), 2);
    assert_eq!(got[0].mog_id(), MOG_B);
    assert_eq!(got[0].samples().len(), 480, "only the PTT audio");
    assert_eq!(got[1].lease_id(), 1);
    assert_eq!(got[1].mode(), CaptureMode::HandsFree);
    assert_eq!(
        got[1].samples().len(),
        1_440,
        "the hands-free audio, without the PTT"
    );
    assert_eq!(buf.takes(), 0);
}

#[test]
fn cap_12c_session_and_directed_ptt_audio_is_not_held_here() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    let mut session = state(1, MOG_A, CaptureMode::Session);
    session.directed_to = Some(MOG_B.into());
    buf.on_state(&session);
    feed(&mut sink, &tone(0.3, 4_800));
    assert_eq!(buf.held_samples(), 0);
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Session, StopReason::Manual));
    assert!(out.take().is_empty());
}

#[test]
fn cap_12c_the_buffer_source_has_no_disk_log_or_telemetry_call() {
    let src = include_str!("../src/audio_device/ptt_buffer.rs");
    let banned = [
        concat!("std::", "fs"),
        concat!("File", "::"),
        concat!("Open", "Options"),
        concat!("temp", "_dir"),
        concat!("temp", "file"),
        concat!("mm", "ap"),
        concat!("print", "ln!"),
        concat!("eprint", "ln!"),
        concat!("print", "!("),
        concat!("eprint", "!("),
        concat!("db", "g!"),
        concat!("log", "::"),
        concat!("trac", "ing::"),
        concat!("std::", "io::Write"),
    ];
    for word in banned {
        assert!(!src.contains(word), "the PTT buffer must not use `{word}`");
    }
}

// ---------- the caps ----------

#[test]
fn cap_04a_cap_12c_a_ptt_take_holds_at_most_120_s() {
    assert_eq!(cap_ms(CaptureMode::Ptt), Some(HOLD_MAX_MS));
    assert_eq!(HOLD_MAX_MS, 120_000);
    let cap = cap_samples(CaptureMode::Ptt, 1_000).unwrap();
    assert_eq!(cap, 120_000);
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    // 125 s at 1 kHz.
    for _ in 0..125 {
        sink.frame(MicFrame {
            samples: &[0.1; 1_000],
            sample_rate: 1_000,
        });
    }
    assert_eq!(buf.held_samples(), cap, "frames after the cap are dropped");
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::HoldCap));
    let got = out.take();
    assert_eq!(got[0].samples().len(), cap);
    assert_eq!(got[0].duration_ms(), HOLD_MAX_MS);
}

#[test]
fn cap_04b_cap_12c_a_hands_free_take_holds_at_most_5_min() {
    assert_eq!(cap_ms(CaptureMode::HandsFree), Some(HANDS_FREE_MAX_MS));
    assert_eq!(HANDS_FREE_MAX_MS, 300_000);
    assert_eq!(cap_ms(CaptureMode::Session), None);
    assert_eq!(
        cap_samples(CaptureMode::HandsFree, RATE),
        Some(48_000 * 300)
    );
    let (_out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::HandsFree));
    for _ in 0..310 {
        sink.frame(MicFrame {
            samples: &[0.1; 1_000],
            sample_rate: 1_000,
        });
    }
    assert_eq!(buf.held_samples(), 300_000);
}

// ---------- discards ----------

#[test]
fn cap_44b_cap_14b_cap_17b_a_discarding_stop_gives_no_utterance_and_holds_nothing() {
    for reason in [
        StopReason::Discarded,
        StopReason::Escape,
        StopReason::ZeroBuffers,
    ] {
        assert!(reason.discards_audio());
        let (out, buf) = buffer();
        let mut sink = buf.sink();
        let mode = if reason == StopReason::Escape {
            CaptureMode::HandsFree
        } else {
            CaptureMode::Ptt
        };
        buf.on_state(&state(1, MOG_A, mode));
        feed(&mut sink, &tone(0.3, 4_800));
        buf.on_stopped(&stopped(1, MOG_A, mode, reason));
        assert!(out.take().is_empty(), "{reason:?} makes no utterance");
        assert_eq!(buf.held_samples(), 0);
        assert_eq!(buf.takes(), 0);
    }
}

#[test]
fn cap_17b_the_discard_marker_drops_the_audio_at_once() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.3, 4_800));
    sink.event(SinkEvent::Discard);
    assert_eq!(
        buf.held_samples(),
        0,
        "the audio is gone before the stop arrives"
    );
    feed(&mut sink, &tone(0.3, 480));
    assert_eq!(buf.held_samples(), 0);
    // Even a stop reason that keeps audio makes no utterance now.
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    assert!(out.take().is_empty());
}

#[test]
fn cap_12c_a_gap_marker_keeps_the_audio() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.3, 480));
    sink.event(SinkEvent::Gap { gap_ms: 30 });
    feed(&mut sink, &tone(0.3, 480));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    assert_eq!(out.take()[0].samples().len(), 960);
}

#[test]
fn cap_12c_a_frame_at_a_new_rate_is_resampled_to_the_take_rate() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    sink.frame(MicFrame {
        samples: &[0.2; 480],
        sample_rate: 48_000,
    });
    // CAP-18: the new default device runs at 16 kHz. 160 samples are 10 ms.
    sink.frame(MicFrame {
        samples: &[0.2; 160],
        sample_rate: 16_000,
    });
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    let got = out.take();
    assert_eq!(got[0].sample_rate(), 48_000);
    assert_eq!(got[0].samples().len(), 960, "10 ms + 10 ms at 48 kHz");
}

#[test]
fn cap_12c_a_stop_with_no_audio_gives_no_utterance() {
    let (out, buf) = buffer();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    assert!(out.take().is_empty());
}

#[test]
fn cap_12c_the_sink_drops_non_finite_samples() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    sink.frame(MicFrame {
        samples: &[0.5, f32::NAN, f32::INFINITY, -0.5],
        sample_rate: RATE,
    });
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    let got = out.take();
    assert!(got[0].samples().iter().all(|s| s.is_finite()));
}

// ---------- CAP-34 and section 9.2: no copy after STT ----------

#[test]
fn cap_34_the_buffer_holds_no_copy_after_the_hand_off() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.3, 9_600));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    buf.on_state(&idle());
    assert_eq!(buf.held_samples(), 0, "the utterance is the only copy");
    assert_eq!(buf.takes(), 0);
    assert_eq!(out.take().len(), 1);
}

#[test]
fn cap_34_zeroize_overwrites_every_sample() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.3, 4_800));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    let mut u = out.take().pop().unwrap();
    assert!(u.samples().iter().any(|s| *s != 0.0));
    u.zeroize();
    assert_eq!(u.samples().len(), 4_800);
    assert!(
        u.samples().iter().all(|s| *s == 0.0),
        "`Drop` runs the same wipe"
    );
}

#[test]
fn cap_34_cap_01b_utterance_debug_shows_no_sample() {
    let (out, buf) = buffer();
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    sink.frame(MicFrame {
        samples: &[0.123_456, -0.654_321],
        sample_rate: RATE,
    });
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    let u = out.take().pop().unwrap();
    let text = format!("{u:?} {buf:?} {:?}", buf.sink());
    for s in u.samples() {
        assert!(
            !text.contains(&format!("{s}")),
            "no sample in Debug: {text}"
        );
    }
    assert!(!text.contains("0.123") && !text.contains("0.654"));
    assert!(
        !text.contains(MOG_A),
        "no MOG ID in the utterance Debug either"
    );
}

#[test]
fn cap_34_the_host_default_sink_drops_each_utterance() {
    let buf = PttBuffer::new(Arc::new(NoUtteranceSink));
    let mut sink = buf.sink();
    buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
    feed(&mut sink, &tone(0.3, 4_800));
    buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
    assert_eq!(buf.held_samples(), 0);
    assert_eq!(buf.takes(), 0);
}

// ---------- CAP-20: the peak before STT ----------

#[test]
fn cap_20_the_utterance_peaks_at_minus_3_dbfs() {
    for db in [-30.0, -12.0, -0.5] {
        let (out, buf) = buffer();
        let mut sink = buf.sink();
        buf.on_state(&state(1, MOG_A, CaptureMode::Ptt));
        feed(&mut sink, &tone(db_to_amp(db), 9_600));
        buf.on_stopped(&stopped(1, MOG_A, CaptureMode::Ptt, StopReason::Released));
        let u = out.take().pop().unwrap();
        let peak = peak_dbfs(u.samples());
        assert!(
            (peak - TARGET_PEAK_DBFS).abs() < 0.5,
            "input {db} dBFS gives peak {peak} dBFS"
        );
    }
}

// ---------- end to end: the real MicDevice and CaptureCore ----------

/// The data callback of the open fake stream. Builds are counted for CAP-21.
#[derive(Clone, Default)]
struct FakeMic {
    on_data: Arc<Mutex<Option<DataFn>>>,
    builds: Arc<AtomicUsize>,
    plays: Arc<AtomicUsize>,
}

impl FakeMic {
    fn feed(&self, samples: &[f32]) -> bool {
        match self.on_data.lock().unwrap().as_mut() {
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
        // The stream is gone: no more callbacks.
        self.0.lock().unwrap().take();
    }
}

impl MicHost for FakeHost {
    type Stream = FakeStream;

    fn build(&mut self, on_data: DataFn, _on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        self.mic.builds.fetch_add(1, Ordering::SeqCst);
        *self.mic.on_data.lock().unwrap() = Some(on_data);
        Ok(FakeStream(Arc::clone(&self.mic.on_data)))
    }

    fn play(&mut self, _stream: &FakeStream) -> Result<(), DeviceError> {
        self.mic.plays.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

struct Rig {
    core: Arc<CaptureCore>,
    mic: FakeMic,
    buf: PttBuffer,
    out: Arc<Recorded>,
}

fn rig() -> Rig {
    let mic = FakeMic::default();
    let (out, buf) = buffer();
    let host_mic = mic.clone();
    let (device, _link) = MicDevice::spawn(
        move || FakeHost {
            mic: host_mic,
            _not_send: PhantomData,
        },
        Box::new(TeeSink(DiscardSink, buf.sink())),
    )
    .expect("the audio thread starts");
    let events: Arc<dyn CaptureEvents> = Arc::new(PttCaptureEvents::new(
        RecordingCaptureEvents::default(),
        buf.clone(),
    ));
    let core = Arc::new(CaptureCore::new(
        Box::new(device),
        Arc::new(SimClock::new()),
        events,
    ));
    Rig {
        core,
        mic,
        buf,
        out,
    }
}

fn ptt() -> CaptureRequest {
    CaptureRequest {
        mog_id: MOG_A.into(),
        mode: CaptureMode::Ptt,
        others_track: false,
    }
}

#[test]
fn cap_21_cap_13a_no_stream_is_built_or_plays_before_key_down() {
    let r = rig();
    std::thread::sleep(Duration::from_millis(20));
    // SP-08 (#15) has not passed, so the cold path holds: no pre-built stream.
    assert_eq!(
        r.mic.builds.load(Ordering::SeqCst),
        0,
        "no stream before key-down"
    );
    assert_eq!(r.mic.plays.load(Ordering::SeqCst), 0);
    assert!(
        !r.mic.feed(&tone(0.5, 480)),
        "no stream, so no frame exists"
    );
    let lease = r.core.start(ptt()).unwrap();
    assert_eq!(
        r.mic.builds.load(Ordering::SeqCst),
        1,
        "the stream is built at key-down"
    );
    assert_eq!(r.mic.plays.load(Ordering::SeqCst), 1);
    r.core.stop(lease.id).unwrap();
    assert!(
        !r.mic.feed(&tone(0.5, 480)),
        "the stream is gone after the stop"
    );
}

#[test]
fn cap_12c_cap_13a_end_to_end_a_ptt_hands_one_utterance_to_the_consumer() {
    let r = rig();
    let lease = r.core.start(ptt()).unwrap();
    assert!(r.buf.is_taking());
    for _ in 0..10 {
        assert!(r.mic.feed(&tone(0.2, 480)));
    }
    let st = r.core.discard(lease.id);
    assert!(st.is_ok());
    assert!(
        r.out.take().is_empty(),
        "a tap makes no utterance (CAP-44b)"
    );
    let lease = r.core.start(ptt()).unwrap();
    for _ in 0..5 {
        r.mic.feed(&tone(0.2, 480));
    }
    r.core.stop(lease.id).unwrap();
    assert!(!r.buf.is_taking());
    let got = r.out.take();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].lease_id(), lease.id);
    assert_eq!(got[0].reason(), StopReason::Manual);
    assert_eq!(
        got[0].samples().len(),
        2_400,
        "only the audio of this lease"
    );
    assert_eq!(r.buf.held_samples(), 0);
}
