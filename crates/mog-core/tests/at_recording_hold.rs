//! The talk key during a call recording, core half (ADR 0118, UX slice 6, AC-6.4..6.12, 6.14).
//!
//! Fake devices and `SimClock` only: no test opens a real mic or system audio. The host
//! wiring (the key-down hook, the refusal in `capture_host`) is a later lane; these tests call
//! the core gate API that the hook will call.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::audio_device::level::{LevelCaptureEvents, LevelGate, LevelOut, TeeSink};
use mog_core::audio_device::ptt_buffer::{PttBuffer, PttCaptureEvents, Utterance, UtteranceSink};
use mog_core::audio_device::system::{RecordingDevice, SystemAudioDevice};
use mog_core::audio_device::{DataFn, FaultFn, FrameSink, Interleaved, MicFrame, MicHost};
use mog_core::capture::{
    AudioDevice, AudioSource, CaptureCore, CaptureEvents, CaptureRequest, CaptureService,
    DeviceError, Stopped,
};
use mog_core::clock::{SimClock, SimWallClock};
use mog_core::recording_capture::chunker::{PartFile, PartFormat};
use mog_core::recording_capture::session::{
    FinishedRecording, HeldRange, PartsHandoff, RecordingCaptureError, RecordingSession,
    SessionConfig, Snapshot, StartOptions, StopCause,
};
use mog_core::recording_capture::tap::{GateCaptureEvents, RecordingTap, GATE_TAIL_MS};
use mog_core::recording_cloud::cloud::{merge_local_parts, LocalPart};
use mog_core::recording_cloud::job::JobManifest;
use mog_core::recording_cloud::transcript::{
    bottle_gaps, render_markdown, BottleGap, Segment, TranscriptDoc,
};
use mog_core::types::{CaptureMode, CaptureState};

// ---------- fakes ----------

#[derive(Clone, Default)]
struct FakeMic {
    open: Arc<AtomicUsize>,
    opens: Arc<AtomicUsize>,
}
impl AudioDevice for FakeMic {
    fn open(&self, source: AudioSource) -> Result<(), DeviceError> {
        assert_eq!(source, AudioSource::Mic);
        self.opens.fetch_add(1, Ordering::SeqCst);
        self.open.store(1, Ordering::SeqCst);
        Ok(())
    }
    fn close(&self, source: AudioSource) {
        assert_eq!(source, AudioSource::Mic);
        self.open.store(0, Ordering::SeqCst);
    }
}

#[derive(Clone, Default)]
struct FakeHost {
    data: Arc<Mutex<Option<DataFn>>>,
}
struct FakeStream;
impl MicHost for FakeHost {
    type Stream = FakeStream;
    fn build(&mut self, on_data: DataFn, _on_fault: FaultFn) -> Result<FakeStream, DeviceError> {
        *self.data.lock().unwrap() = Some(on_data);
        Ok(FakeStream)
    }
    fn play(&mut self, _stream: &FakeStream) -> Result<(), DeviceError> {
        Ok(())
    }
}
impl FakeHost {
    fn play(&self, samples: &[f32], channels: u16, rate: u32) {
        if let Some(f) = self.data.lock().unwrap().as_mut() {
            f(Interleaved {
                samples,
                channels,
                sample_rate: rate,
            });
        }
    }
}

#[derive(Default)]
struct Handoff(Mutex<Vec<FinishedRecording>>, Mutex<Vec<PartFile>>);
impl PartsHandoff for Handoff {
    fn part_ready(&self, _id: &str, part: &PartFile) {
        self.1.lock().unwrap().push(part.clone());
    }
    fn finished(&self, rec: FinishedRecording) {
        self.0.lock().unwrap().push(rec);
    }
}

#[derive(Default)]
struct Utterances(Mutex<Vec<(u64, Vec<f32>)>>);
impl UtteranceSink for Utterances {
    fn utterance(&self, u: Utterance) {
        self.0
            .lock()
            .unwrap()
            .push((u.lease_id(), u.samples().to_vec()));
    }
}

/// Forwards `capture.stopped` to the recording session, as the host chain does.
#[derive(Default)]
struct Fwd(Mutex<Option<Arc<RecordingSession>>>);
impl CaptureEvents for Fwd {
    fn overlay_granted(&self, _lease: &mog_core::capture::Lease) {}
    fn state(&self, _state: &CaptureState) {}
    fn stopped(&self, stopped: &Stopped) {
        let s = self.0.lock().unwrap().clone();
        if let Some(s) = s {
            s.lease_stopped(stopped.lease_id, stopped.reason);
        }
    }
}

type Chain = GateCaptureEvents<LevelCaptureEvents<PttCaptureEvents<FwdRef>>>;

struct NoLevel;
impl LevelOut for NoLevel {
    fn level(&self, _dbfs: f32) {}
}

struct FwdRef(Arc<Fwd>);

impl CaptureEvents for FwdRef {
    fn overlay_granted(&self, lease: &mog_core::capture::Lease) {
        self.0.overlay_granted(lease);
    }
    fn state(&self, state: &CaptureState) {
        self.0.state(state);
    }
    fn stopped(&self, stopped: &Stopped) {
        self.0.stopped(stopped);
    }
}

const NAME: &str = "2026-11-19_0905";
const SYS_AMP: f32 = 0.2;
const MIC_AMP: f32 = 0.1;

struct Rig {
    session: Arc<RecordingSession>,
    core: Arc<CaptureCore>,
    clock: Arc<SimClock>,
    tap: RecordingTap,
    mic: FakeMic,
    sys: FakeHost,
    handoff: Arc<Handoff>,
    utterances: Arc<Utterances>,
    enabled: Arc<AtomicBool>,
    mic_in: Box<dyn FrameSink + Send>,
    root: PathBuf,
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn temp_dir() -> PathBuf {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).unwrap();
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    let d = std::env::temp_dir().join(format!("bottle-hold-test-{hex}"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rig(part_max_samples: Option<u64>) -> Rig {
    let clock = Arc::new(SimClock::new());
    let wall = SimWallClock::at(1_763_544_600_000);
    let tap = RecordingTap::with_queue_blocks(clock.clone(), 8_192);
    let mic = FakeMic::default();
    let sys = FakeHost::default();
    let h = sys.clone();
    let (sys_dev, status) = SystemAudioDevice::spawn(
        move || h,
        Box::new(tap.system_sink()),
        Duration::from_secs(5),
        Duration::from_secs(5),
    )
    .unwrap();
    let fwd = Arc::new(Fwd::default());
    let utterances = Arc::new(Utterances::default());
    let ptt = PttBuffer::new(utterances.clone());
    // The host order, with the gate outermost: Gate -> Level -> Ptt -> relay.
    let chain: Chain = GateCaptureEvents::new(
        LevelCaptureEvents::new(
            PttCaptureEvents::new(FwdRef(fwd.clone()), ptt.clone()),
            LevelGate::new(Arc::new(NoLevel)),
        ),
        tap.clone(),
    );
    let core = Arc::new(CaptureCore::new(
        Box::new(RecordingDevice::new(
            Box::new(mic.clone()),
            Box::new(sys_dev),
        )),
        clock.clone(),
        Arc::new(chain),
    ));
    let handoff = Arc::new(Handoff::default());
    let enabled = Arc::new(AtomicBool::new(true));
    let en = Arc::clone(&enabled);
    let root = temp_dir();
    let session = RecordingSession::new(
        core.clone(),
        tap.clone(),
        status,
        clock.clone(),
        Arc::new(wall),
        Arc::new(move || en.load(Ordering::SeqCst)),
        handoff.clone(),
        SessionConfig {
            temp_root: root.clone(),
            part_max_samples,
            part_format: PartFormat::Flac,
            ..SessionConfig::default()
        },
    );
    *fwd.0.lock().unwrap() = Some(Arc::clone(&session));
    // The host tee: the PTT buffer and the recording tap read one mic stream.
    let mic_in: Box<dyn FrameSink + Send> = Box::new(TeeSink(ptt.sink(), tap.mic_sink()));
    Rig {
        session,
        core,
        clock,
        tap,
        mic,
        sys,
        handoff,
        utterances,
        enabled,
        mic_in,
        root,
    }
}

fn opts() -> StartOptions {
    StartOptions {
        name: NAME.into(),
        length_min: 60,
    }
}

impl Rig {
    /// `ms` of audio: the mic through the host tee, system audio through the fake host.
    fn play(&mut self, ms: u64) {
        for _ in 0..(ms / 10) {
            let m = vec![MIC_AMP; 480];
            self.mic_in.frame(MicFrame {
                samples: &m,
                sample_rate: 48_000,
            });
            self.sys.play(&vec![SYS_AMP; 960], 2, 48_000);
            self.clock.advance(Duration::from_millis(10));
        }
    }

    fn ptt(&self) -> u64 {
        self.core
            .start(CaptureRequest {
                mog_id: "mog-a".into(),
                mode: CaptureMode::Ptt,
                others_track: false,
            })
            .unwrap()
            .id
    }

    fn finish(&self) -> FinishedRecording {
        assert_eq!(self.tap.dropped(), 0, "no block dropped");
        self.session.stop(StopCause::User).unwrap()
    }
}

fn decode(path: &Path) -> Vec<i32> {
    let mut r = claxon::FlacReader::open(path).expect("flac");
    r.samples().map(|s| s.unwrap()).collect()
}

fn level(v: f32) -> i32 {
    (v * 32_767.0) as i32
}

/// The mean of `samples` between two times in seconds.
fn mean(samples: &[i32], from_s: f64, to_s: f64) -> i32 {
    let a = (from_s * 16_000.0) as usize;
    let b = (to_s * 16_000.0) as usize;
    (samples[a..b].iter().map(|s| i64::from(*s)).sum::<i64>() / (b - a) as i64) as i32
}

// ---------- AC-6.4, 6.5, 6.12 ----------

#[test]
fn ac_6_4_6_5_6_12_one_mic_open_and_the_recording_never_pauses() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    assert_eq!(r.mic.opens.load(Ordering::SeqCst), 1);
    r.play(500);
    r.tap.talk_key_down();
    let ptt = r.ptt();
    // The hold reads the open mic: no second open.
    assert_eq!(r.mic.opens.load(Ordering::SeqCst), 1, "AC-6.4");
    r.play(1_000);
    assert!(
        matches!(r.session.snapshot(), Snapshot::Recording(_)),
        "AC-6.12"
    );
    r.core.stop(ptt).unwrap();
    r.tap.talk_key_up();
    assert_eq!(
        r.mic.open.load(Ordering::SeqCst),
        1,
        "the recording keeps the mic"
    );
    assert!(
        matches!(r.session.snapshot(), Snapshot::Recording(_)),
        "AC-6.12"
    );
    r.play(500);
    r.finish();
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 0, "AC-6.5");
    assert_eq!(r.mic.opens.load(Ordering::SeqCst), 1, "AC-6.4");
}

#[test]
fn ac_6_5_the_mic_stays_open_after_the_recording_ends_during_a_hold_until_the_hold_ends() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.tap.talk_key_down();
    let ptt = r.ptt();
    r.play(300);
    // A user stop during the hold: the hold still owns the mic.
    r.finish();
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 1);
    r.core.stop(ptt).unwrap();
    r.tap.talk_key_up();
    assert_eq!(r.mic.open.load(Ordering::SeqCst), 0);
}

// ---------- AC-6.6, 6.7 ----------

#[test]
fn ac_6_6_6_7_gate_zeroes_mic_blocks_from_key_down_to_250_ms_after_the_hold() {
    let clock = Arc::new(SimClock::new());
    let tap = RecordingTap::new(clock.clone());
    let rx = tap.arm();
    let mut mic = tap.mic_sink();
    let mut sys = tap.system_sink();
    let mut feed = |tap_ms: u64| {
        for _ in 0..(tap_ms / 10) {
            mic.frame(MicFrame {
                samples: &[0.3; 160],
                sample_rate: 16_000,
            });
            sys.frame(MicFrame {
                samples: &[0.5; 160],
                sample_rate: 16_000,
            });
            clock.advance(Duration::from_millis(10));
        }
    };
    feed(100);
    tap.talk_key_down();
    feed(200);
    tap.talk_key_up();
    // The tail: 250 ms.
    feed(GATE_TAIL_MS);
    feed(100);
    tap.disarm();
    let blocks: Vec<_> = rx.iter().collect();
    let mics: Vec<_> = blocks
        .iter()
        .filter(|b| b.track == mog_core::recording_capture::mixer::Track::Mic)
        .collect();
    assert_eq!(mics.len(), 100 / 10 + 20 + 25 + 10);
    for (i, b) in mics.iter().enumerate() {
        let gated = (10..10 + 20 + 25).contains(&i);
        assert_eq!(b.gated, gated, "block {i}");
        assert_eq!(b.samples.len(), 160, "same length, block {i}");
        let want = if gated { 0.0 } else { 0.3 };
        assert!(b.samples.iter().all(|s| *s == want), "block {i}");
    }
    // AC-6.7: system blocks are unchanged and never gated.
    let syss: Vec<_> = blocks
        .iter()
        .filter(|b| b.track == mog_core::recording_capture::mixer::Track::System)
        .collect();
    assert_eq!(syss.len(), mics.len());
    assert!(syss
        .iter()
        .all(|b| !b.gated && b.samples.iter().all(|s| *s == 0.5)));
}

#[test]
fn ac_6_6_the_gate_stays_closed_while_the_key_is_up_but_the_lease_lives() {
    let clock = Arc::new(SimClock::new());
    let tap = RecordingTap::new(clock.clone());
    let _rx = tap.arm();
    tap.talk_key_down();
    tap.overlay_lease(true);
    tap.talk_key_up();
    clock.advance(Duration::from_secs(5));
    assert!(tap.mic_gated(), "a live lease keeps it closed");
    tap.overlay_lease(false);
    assert!(tap.mic_gated(), "tail after the lease ends");
    clock.advance(Duration::from_millis(GATE_TAIL_MS));
    assert!(!tap.mic_gated());
}

#[test]
fn ac_6_6_end_to_end_the_recorded_mic_is_silent_in_the_hold_and_the_call_goes_on() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.play(2_000);
    r.tap.talk_key_down();
    let ptt = r.ptt();
    r.play(2_000);
    r.core.stop(ptt).unwrap();
    r.tap.talk_key_up();
    r.play(2_000);
    let done = r.finish();
    let s = decode(&done.parts[0].path);
    // Blocks are 10 ms, so the edges are exact: key-down at sample 32 000, the hold ends at
    // 64 000, and the tail ends 250 ms later at 68 000.
    let both = level(MIC_AMP + SYS_AMP);
    let sys = level(SYS_AMP);
    let near = |a: i32, b: i32| (a - b).abs() <= 1;
    assert!(s[1_000..32_000].iter().all(|v| near(*v, both)), "before");
    assert!(
        s[32_000..68_000].iter().all(|v| near(*v, sys)),
        "AC-6.6, AC-6.7: every sample from key-down to the end of the tail"
    );
    assert!(
        s[68_000..s.len() - 400].iter().all(|v| near(*v, both)),
        "after"
    );
}

// ---------- AC-6.8 ----------

#[test]
fn ac_6_8_the_ptt_buffer_makes_one_utterance_with_the_overlay_lease_id() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.play(500);
    r.tap.talk_key_down();
    let ptt = r.ptt();
    r.play(1_000);
    r.core.stop(ptt).unwrap();
    r.tap.talk_key_up();
    r.play(300);
    let u = r.utterances.0.lock().unwrap();
    assert_eq!(u.len(), 1, "one utterance");
    assert_eq!(u[0].0, ptt, "the overlay lease ID, not the session lease");
    // The buffer got the real frames, not the gated zeros.
    assert!(u[0].1.iter().any(|s| s.abs() > 0.1));
    assert!(
        u[0].1.len() >= 47_000 && u[0].1.len() <= 49_000,
        "{}",
        u[0].1.len()
    );
}

// ---------- AC-6.9, 6.10, 6.11 ----------

fn hold(r: &mut Rig, ms: u64) {
    r.tap.talk_key_down();
    let ptt = r.ptt();
    r.play(ms);
    r.core.stop(ptt).unwrap();
    r.tap.talk_key_up();
    r.play(500);
}

fn doc(gaps: Vec<BottleGap>, segs: &[(f64, &str, &str)]) -> TranscriptDoc {
    TranscriptDoc {
        version: 1,
        id: "r".into(),
        name: "call".into(),
        started_at: "2026-11-19T14:05:00Z".into(),
        duration_s: 20.0,
        instructions: String::new(),
        parts: vec![],
        segments: segs
            .iter()
            .map(|(start, sp, text)| Segment {
                start: *start,
                end: start + 1.0,
                speaker: (*sp).into(),
                text: (*text).into(),
                part: 1,
            })
            .collect(),
        bottle_gaps: gaps,
    }
}

fn manifest_of(done: &FinishedRecording) -> JobManifest {
    let mut m = JobManifest::new("r", NAME, "2026-11-19T14:05:00Z", vec![], "t", "n", "now");
    let parts: Vec<LocalPart> = done
        .parts
        .iter()
        .map(|p| LocalPart {
            path: p.path.clone(),
            duration_s: p.samples as f64 / 16_000.0,
            held: p.held.clone(),
        })
        .collect();
    merge_local_parts(&mut m, &parts);
    m
}

#[test]
fn ac_6_9_a_long_hold_gives_one_marker_at_its_start_time() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.play(4_000);
    hold(&mut r, 8_000);
    r.play(1_000);
    let done = r.finish();
    let m = manifest_of(&done);
    let gaps = bottle_gaps(&m.parts);
    assert_eq!(gaps.len(), 1);
    // 8 s hold plus the 250 ms tail, from t = 4 s.
    assert!((gaps[0].start_s - 4.0).abs() < 0.05, "{:?}", gaps[0]);
    assert!((gaps[0].dur_s - 8.25).abs() < 0.05, "{:?}", gaps[0]);
    let md = render_markdown(&doc(gaps, &[(1.0, "1", "Before."), (14.0, "2", "After.")]));
    assert_eq!(md.matches("You spoke to Bottle").count(), 1);
    assert!(
        md.contains("*You spoke to Bottle (0:08)* [00:00:04]"),
        "{md}"
    );
    let (before, marker, after) = (
        md.find("Before.").unwrap(),
        md.find("You spoke").unwrap(),
        md.find("After.").unwrap(),
    );
    assert!(before < marker && marker < after);
}

#[test]
fn ac_6_10_a_short_hold_gives_no_marker_but_its_mic_is_zero() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.play(2_000);
    hold(&mut r, 400);
    r.play(2_000);
    let done = r.finish();
    let m = manifest_of(&done);
    let gaps = bottle_gaps(&m.parts);
    assert_eq!(gaps.len(), 1, "the range is recorded");
    assert!(gaps[0].dur_s < 1.0);
    let md = render_markdown(&doc(gaps, &[(0.5, "1", "Hi.")]));
    assert!(!md.contains("You spoke"), "{md}");
    let s = decode(&done.parts[0].path);
    assert!((mean(&s, 2.1, 2.6) - level(SYS_AMP)).abs() < 60, "mic zero");
}

#[test]
fn ac_6_9_the_marker_carries_no_text_and_no_audio_in_the_transcript_json() {
    let d = doc(
        vec![BottleGap {
            start_s: 4.0,
            dur_s: 8.25,
        }],
        &[(1.0, "1", "Hi.")],
    );
    let json = serde_json::to_string(&d).unwrap();
    assert!(
        json.contains(r#""bottleGaps":[{"startS":4.0,"durS":8.25}]"#),
        "{json}"
    );
    let none = serde_json::to_string(&doc(vec![], &[])).unwrap();
    assert!(!none.contains("bottleGaps"));
}

#[test]
fn ac_6_11_each_part_has_its_held_ranges_in_job_json() {
    // 2 s parts; a hold from 1 s to 4.25 s crosses two borders.
    let mut r = rig(Some(32_000));
    r.session.start(opts()).unwrap();
    r.play(1_000);
    r.tap.talk_key_down();
    let ptt = r.ptt();
    r.play(3_000);
    r.core.stop(ptt).unwrap();
    r.tap.talk_key_up();
    r.play(3_000);
    let done = r.finish();
    assert!(done.parts.len() >= 3, "{}", done.parts.len());
    let per_part: Vec<Vec<HeldRange>> = done.parts.iter().map(|p| p.held.clone()).collect();
    let one = |v: &Vec<HeldRange>| {
        assert_eq!(v.len(), 1, "{v:?}");
        v[0]
    };
    let p1 = one(&per_part[0]);
    assert!((p1.start_samples as i64 - 16_000).abs() < 800, "{p1:?}");
    assert_eq!(
        p1.start_samples + p1.len_samples,
        32_000,
        "cut at the border"
    );
    let p2 = one(&per_part[1]);
    assert_eq!((p2.start_samples, p2.len_samples), (0, 32_000));
    let p3 = one(&per_part[2]);
    assert_eq!(p3.start_samples, 0);
    assert!((p3.len_samples as i64 - 4_000).abs() < 800, "{p3:?}");
    // The manifest has them, per part, under `heldRanges`.
    let m = manifest_of(&done);
    assert_eq!(m.parts[1].held_ranges, per_part[1]);
    // Each part that closed while the call ran carried its own ranges at `part_ready`.
    let live = r.handoff.1.lock().unwrap();
    assert!(live.len() >= 2, "{}", live.len());
    for (l, f) in live.iter().zip(&done.parts) {
        assert_eq!(l.held, f.held, "part {}", l.index);
    }
    let json = serde_json::to_string(&m).unwrap();
    assert!(
        json.contains("heldRanges") && json.contains("startSamples"),
        "{json}"
    );
    // And the gaps merge into absolute times.
    let g = bottle_gaps(&m.parts);
    assert_eq!(g.len(), 3);
    assert!((g[0].start_s - 1.0).abs() < 0.1);
}

#[test]
fn ac_6_11_a_recording_with_no_hold_has_no_held_ranges() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.play(1_000);
    let done = r.finish();
    assert!(done.parts.iter().all(|p| p.held.is_empty()));
    assert!(std::fs::read_dir(&done.dir).unwrap().all(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .ends_with(".flac")));
    let json = serde_json::to_string(&manifest_of(&done)).unwrap();
    assert!(!json.contains("heldRanges"));
    assert_eq!(r.handoff.0.lock().unwrap().len(), 1);
}

// ---------- AC-6.14 ----------

#[test]
fn ac_6_14_with_the_knob_off_nothing_in_this_slice_opens_the_mic() {
    let mut r = rig(None);
    r.enabled.store(false, Ordering::SeqCst);
    assert_eq!(
        r.session.start(opts()).unwrap_err(),
        RecordingCaptureError::Disabled
    );
    // The gate API is flags only.
    r.tap.talk_key_down();
    r.tap.overlay_lease(true);
    r.tap.overlay_lease(false);
    r.tap.talk_key_up();
    r.play(200);
    assert_eq!(r.mic.opens.load(Ordering::SeqCst), 0, "no mic open");
    assert_eq!(r.session.snapshot(), Snapshot::Idle);
    assert!(r.utterances.0.lock().unwrap().is_empty());
    // Frames reach no queue while the tap is disarmed: nothing is kept.
    assert_eq!(r.tap.dropped(), 0);
}

#[test]
fn a_live_overlay_at_arm_keeps_the_gate_closed_until_it_stops_and_the_tail_ends() {
    let clock = Arc::new(SimClock::new());
    let tap = RecordingTap::new(clock.clone());
    // The overlay was granted between the session lease and the arm; the key is down.
    tap.talk_key_down();
    tap.overlay_lease(true);
    let _rx = tap.arm();
    assert!(tap.mic_gated(), "arm does not clear live state");
    // The host syncs the physical key at arm: it is up now.
    tap.resync_key(false);
    clock.advance(Duration::from_secs(10));
    assert!(tap.mic_gated(), "the overlay lease still lives");
    tap.overlay_lease(false);
    assert!(tap.mic_gated(), "the tail");
    clock.advance(Duration::from_millis(GATE_TAIL_MS));
    assert!(!tap.mic_gated());
    // resync_key(true) closes it again.
    tap.resync_key(true);
    assert!(tap.mic_gated());
}

#[test]
fn a_late_overlay_grant_after_its_stop_does_not_stick_the_gate() {
    let clock = Arc::new(SimClock::new());
    let tap = RecordingTap::new(clock.clone());
    let _rx = tap.arm();
    let events = GateCaptureEvents::new(FwdRef(Arc::new(Fwd::default())), tap.clone());
    let lease = mog_core::capture::Lease {
        id: 9,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Ptt,
    };
    // Out of order: the stop first, the grant late.
    events.stopped(&Stopped {
        lease_id: 9,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Ptt,
        reason: mog_core::capture::StopReason::Released,
    });
    events.overlay_granted(&lease);
    assert!(!tap.mic_gated(), "no live lease, so no gate");
    // A normal order closes it and the stop starts the tail.
    let lease = mog_core::capture::Lease { id: 10, ..lease };
    events.overlay_granted(&lease);
    assert!(tap.mic_gated());
    events.stopped(&Stopped {
        lease_id: 10,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Ptt,
        reason: mog_core::capture::StopReason::Released,
    });
    clock.advance(Duration::from_millis(GATE_TAIL_MS));
    assert!(!tap.mic_gated());
}

#[allow(dead_code)]
fn _types(_: &PartFile) {}

#[test]
fn the_one_second_marker_limit_counts_the_hold_without_the_tail() {
    let mut r = rig(None);
    r.session.start(opts()).unwrap();
    r.play(1_000);
    hold(&mut r, 800);
    r.play(1_000);
    hold(&mut r, 1_000);
    r.play(500);
    let done = r.finish();
    let gaps = bottle_gaps(&manifest_of(&done).parts);
    assert_eq!(gaps.len(), 2);
    // 0.8 s hold: the range is 1.05 s with the tail, but there is no marker.
    let md = render_markdown(&doc(gaps, &[(0.2, "1", "Hi.")]));
    assert_eq!(md.matches("You spoke to Bottle").count(), 1, "{md}");
    assert!(md.contains("(0:01)"), "{md}");
}
