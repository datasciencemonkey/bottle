//! The mic level stream (CAP-16a, CAP-16b) and the -3 dBFS peak normalisation (CAP-20) of R2
//! (#23, ADR 0067). No real mic: frames are synthetic tones.

use std::sync::{Arc, Mutex};

use mog_core::audio_device::level::{
    rms_dbfs, window_samples, LevelCaptureEvents, LevelGate, LevelOut, LevelSink, TeeSink,
    LEVEL_FLOOR_DBFS, LEVEL_MAX_HZ,
};
use mog_core::audio_device::normalize::{normalize_peak, peak_dbfs, TARGET_PEAK_DBFS};
use mog_core::audio_device::{FrameSink, MicFrame, SinkEvent};
use mog_core::capture::{CaptureEvents, StopReason, Stopped};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack};

const RATE: u32 = 48_000;
const MOG_A: &str = "00000000-0000-4000-8000-0000000000a1";

/// The levels that reached the `onLevel` Channel, in order.
#[derive(Default)]
struct Levels(Mutex<Vec<f32>>);

impl LevelOut for Levels {
    fn level(&self, dbfs: f32) {
        self.0.lock().unwrap().push(dbfs);
    }
}

impl Levels {
    fn take(&self) -> Vec<f32> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// A sine with the peak `amp`, `n` samples long.
fn tone(amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin())
        .collect()
}

fn db_to_amp(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn feed(sink: &mut impl FrameSink, samples: &[f32], block: usize) {
    for chunk in samples.chunks(block) {
        sink.frame(MicFrame {
            samples: chunk,
            sample_rate: RATE,
        });
    }
}

fn open_sink() -> (Arc<Levels>, LevelGate, LevelSink) {
    let out = Arc::new(Levels::default());
    let gate = LevelGate::new(out.clone());
    let sink = LevelSink::new(gate.clone());
    (out, gate, sink)
}

fn recording(owner: &str) -> CaptureState {
    CaptureState {
        owner: Some(owner.to_string()),
        mode: Some(CaptureMode::Ptt),
        lease_id: Some("1".to_string()),
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

#[derive(Default)]
struct Inner(Mutex<Vec<String>>);

impl CaptureEvents for Inner {
    fn state(&self, state: &CaptureState) {
        self.0
            .lock()
            .unwrap()
            .push(format!("state recording={}", state.recording));
    }
    fn stopped(&self, stopped: &Stopped) {
        self.0
            .lock()
            .unwrap()
            .push(format!("stopped {}", stopped.reason.code()));
    }
}

// ---------- CAP-16a: RMS level at 30 Hz or less ----------

#[test]
fn cap_16a_rms_dbfs_of_a_full_scale_sine_is_minus_3() {
    let db = rms_dbfs(&tone(1.0, RATE as usize));
    assert!((db - -3.01).abs() < 0.05, "{db}");
    let quiet = rms_dbfs(&tone(db_to_amp(-30.0), RATE as usize));
    assert!((quiet - -33.01).abs() < 0.05, "{quiet}");
}

#[test]
fn cap_16a_silence_gives_the_finite_floor() {
    assert_eq!(rms_dbfs(&[0.0; 480]), LEVEL_FLOOR_DBFS);
    assert_eq!(rms_dbfs(&[]), LEVEL_FLOOR_DBFS);
    assert!(LEVEL_FLOOR_DBFS.is_finite());
}

#[test]
fn cap_16a_window_is_at_least_one_thirtieth_of_a_second() {
    assert_eq!(LEVEL_MAX_HZ, 30);
    assert_eq!(window_samples(48_000), 1_600);
    assert_eq!(window_samples(44_100), 1_470);
    // 16 kHz / 30 = 533.3: round up so the rate stays at 30 Hz or less.
    assert_eq!(window_samples(16_000), 534);
    assert_eq!(window_samples(0), 1);
}

#[test]
fn cap_16a_one_second_of_audio_gives_at_most_30_levels() {
    let (out, gate, mut sink) = open_sink();
    gate.open();
    // Small blocks (like a 10 ms callback) and odd blocks both stay at 30 per second.
    feed(&mut sink, &tone(0.5, RATE as usize), 480);
    let small = out.take();
    assert_eq!(small.len(), 30);
    feed(&mut sink, &tone(0.5, RATE as usize), 1_013);
    let odd = out.take();
    assert!(odd.len() <= 30 && odd.len() >= 29, "{}", odd.len());
    // One large block still gives one level per window, never a burst above 30 Hz.
    feed(&mut sink, &tone(0.5, RATE as usize), RATE as usize);
    assert!(out.take().len() <= 30);
}

#[test]
fn cap_16a_level_is_the_rms_of_the_window() {
    let (out, gate, mut sink) = open_sink();
    gate.open();
    feed(&mut sink, &tone(db_to_amp(-20.0), 1_600 * 3), 160);
    let levels = out.take();
    assert_eq!(levels.len(), 3);
    for db in levels {
        assert!((db - -23.01).abs() < 0.1, "{db}");
    }
}

#[test]
fn cap_16a_no_level_while_the_gate_is_closed() {
    let (out, _gate, mut sink) = open_sink();
    feed(&mut sink, &tone(0.5, RATE as usize), 480);
    assert!(out.take().is_empty());
}

#[test]
fn cap_16a_capture_events_open_the_level_stream_while_recording() {
    let out = Arc::new(Levels::default());
    let gate = LevelGate::new(out.clone());
    let events = LevelCaptureEvents::new(Inner::default(), gate.clone());
    assert!(!gate.is_open());
    events.state(&recording(MOG_A));
    assert!(gate.is_open());
    // The inner sink still gets every event.
    assert_eq!(
        events.inner().0.lock().unwrap().as_slice(),
        ["state recording=true"]
    );
}

#[test]
fn cap_16a_tee_sink_feeds_both_sinks() {
    let (out_a, gate_a, sink_a) = open_sink();
    let (out_b, gate_b, sink_b) = open_sink();
    gate_a.open();
    gate_b.open();
    let mut tee = TeeSink(sink_a, sink_b);
    feed(&mut tee, &tone(0.5, 1_600), 160);
    assert_eq!(out_a.take().len(), 1);
    assert_eq!(out_b.take().len(), 1);
}

// ---------- CAP-16b: the stream stops when capture stops ----------

#[test]
fn cap_16b_no_level_after_close() {
    let (out, gate, mut sink) = open_sink();
    gate.open();
    feed(&mut sink, &tone(0.5, 1_600 * 2), 160);
    assert_eq!(out.take().len(), 2);
    gate.close();
    feed(&mut sink, &tone(0.5, RATE as usize), 160);
    assert!(out.take().is_empty());
}

#[test]
fn cap_16b_a_new_capture_starts_a_fresh_window() {
    let (out, gate, mut sink) = open_sink();
    gate.open();
    // Half a window of loud audio, then the capture stops.
    feed(&mut sink, &tone(1.0, 800), 160);
    gate.close();
    gate.open();
    // The next capture is quiet: its first level holds none of the old loud samples.
    feed(&mut sink, &tone(db_to_amp(-40.0), 1_600), 160);
    let levels = out.take();
    assert_eq!(levels.len(), 1);
    assert!((levels[0] - -43.01).abs() < 0.1, "{}", levels[0]);
}

#[test]
fn cap_16b_discard_and_gap_reset_the_window() {
    let (out, gate, mut sink) = open_sink();
    gate.open();
    feed(&mut sink, &tone(1.0, 800), 160);
    sink.event(SinkEvent::Discard);
    feed(&mut sink, &tone(db_to_amp(-40.0), 1_600), 160);
    let levels = out.take();
    assert_eq!(levels.len(), 1);
    assert!((levels[0] - -43.01).abs() < 0.1, "{}", levels[0]);
    feed(&mut sink, &tone(1.0, 800), 160);
    sink.event(SinkEvent::Gap { gap_ms: 0 });
    feed(&mut sink, &tone(db_to_amp(-40.0), 1_600), 160);
    let levels = out.take();
    assert_eq!(levels.len(), 1);
    assert!((levels[0] - -43.01).abs() < 0.1, "{}", levels[0]);
}

#[test]
fn cap_16b_capture_events_close_the_stream_on_stop_and_pause() {
    let out = Arc::new(Levels::default());
    let gate = LevelGate::new(out.clone());
    let events = LevelCaptureEvents::new(Inner::default(), gate.clone());
    events.state(&recording(MOG_A));
    assert!(gate.is_open());
    events.stopped(&Stopped {
        lease_id: 1,
        mog_id: MOG_A.to_string(),
        mode: CaptureMode::Ptt,
        reason: StopReason::Released,
    });
    events.state(&idle());
    assert!(!gate.is_open());
    events.state(&recording(MOG_A));
    let mut paused = recording(MOG_A);
    paused.paused = true;
    events.state(&paused);
    assert!(!gate.is_open());
    assert_eq!(
        events.inner().0.lock().unwrap().as_slice(),
        [
            "state recording=true",
            "stopped released",
            "state recording=false",
            "state recording=true",
            "state recording=true",
        ]
    );
}

#[test]
fn cap_16b_close_waits_for_a_level_in_flight() {
    // A level send that started before `close` ends before `close` returns, so the brain
    // never sees a level after the stop.
    struct Slow {
        started: std::sync::mpsc::Sender<()>,
        sent: Mutex<Vec<f32>>,
    }
    impl LevelOut for Slow {
        fn level(&self, dbfs: f32) {
            let _ = self.started.send(());
            std::thread::sleep(std::time::Duration::from_millis(50));
            self.sent.lock().unwrap().push(dbfs);
        }
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let out = Arc::new(Slow {
        started: tx,
        sent: Mutex::new(Vec::new()),
    });
    let gate = LevelGate::new(out.clone());
    gate.open();
    let mut sink = LevelSink::new(gate.clone());
    let audio = std::thread::spawn(move || feed(&mut sink, &tone(0.5, 1_600), 1_600));
    rx.recv().unwrap();
    gate.close();
    assert_eq!(out.sent.lock().unwrap().len(), 1);
    audio.join().unwrap();
}

// ---------- CAP-20: the input peak is -3 dBFS before STT ----------

#[test]
fn cap_20_quiet_input_reaches_minus_3_dbfs() {
    assert_eq!(TARGET_PEAK_DBFS, -3.0);
    let mut seg = tone(db_to_amp(-30.0), RATE as usize);
    assert!((peak_dbfs(&seg) - -30.0).abs() < 0.05);
    normalize_peak(&mut seg);
    assert!((peak_dbfs(&seg) - -3.0).abs() < 0.05, "{}", peak_dbfs(&seg));
}

#[test]
fn cap_20_loud_input_comes_down_to_minus_3_dbfs() {
    let mut seg = tone(1.0, RATE as usize);
    normalize_peak(&mut seg);
    assert!((peak_dbfs(&seg) - -3.0).abs() < 0.05, "{}", peak_dbfs(&seg));
    assert!(seg.iter().all(|s| s.abs() <= 1.0));
}

#[test]
fn cap_20_gain_keeps_the_shape() {
    let mut seg = vec![0.01, -0.02, 0.005, 0.0];
    let gain = normalize_peak(&mut seg);
    let want = db_to_amp(-3.0) / 0.02;
    assert!((gain - want).abs() < 1e-3, "{gain}");
    assert!((seg[0] - 0.01 * want).abs() < 1e-5);
    assert!((seg[1] + db_to_amp(-3.0)).abs() < 1e-5);
    assert_eq!(seg[3], 0.0);
}

#[test]
fn cap_20_digital_silence_stays_silence() {
    let mut seg = vec![0.0; 480];
    assert_eq!(normalize_peak(&mut seg), 1.0);
    assert!(seg.iter().all(|s| *s == 0.0));
    let mut empty: Vec<f32> = Vec::new();
    assert_eq!(normalize_peak(&mut empty), 1.0);
    assert_eq!(peak_dbfs(&[0.0; 4]), LEVEL_FLOOR_DBFS);
}

#[test]
fn cap_20_non_finite_samples_do_not_poison_the_segment() {
    let mut seg = vec![0.1, f32::NAN, -0.05, f32::INFINITY];
    normalize_peak(&mut seg);
    assert!(seg.iter().all(|s| s.is_finite()));
    assert!((peak_dbfs(&seg) - -3.0).abs() < 0.05, "{}", peak_dbfs(&seg));
}
