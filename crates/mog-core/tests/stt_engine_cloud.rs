//! Cloud dictation in R4 (ADR 0123): with the Databricks engine on, a PTT take also goes to the
//! cloud, and the cloud text replaces the local text. Any cloud error gives the local text, and
//! a cancel gives no result.
//!
//! Fakes only: no model, no device, no network.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mog_core::capture::StopReason;
use mog_core::cloud_dictation::{self, CloudStt};
use mog_core::stt_engine::{
    EngineParts, LocalSttEngine, Recognizer, RecognizerLoader, SttDone, SttIdentity, TranscriptSink,
};
use mog_core::types::{CaptureMode, TranscriptProvider};

struct Local;

impl Recognizer for Local {
    fn transcribe(&mut self, _samples: &[f32]) -> String {
        "local words".to_owned()
    }
}

struct Loader;

impl RecognizerLoader for Loader {
    fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
        Ok(Box::new(Local))
    }
    fn identity(&self) -> SttIdentity {
        SttIdentity::local_parakeet()
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
    fn wait(&self, n: usize) -> Vec<SttDone> {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            let all = self.0.lock().unwrap().clone();
            if all.len() >= n {
                return all;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.0.lock().unwrap().clone()
    }
    fn now(&self) -> Vec<SttDone> {
        self.0.lock().unwrap().clone()
    }
}

/// What the fake cloud does.
#[derive(Clone, Copy)]
enum Mode {
    Text,
    Fail(&'static str),
    /// Wait for the cancel flag (at most 5 s), then say `cancelled`.
    UntilCancel,
}

struct FakeCloud {
    on: AtomicBool,
    mode: Mode,
    calls: AtomicUsize,
    /// The length of the audio of the last call.
    len: AtomicUsize,
    cancelled: AtomicBool,
}

impl FakeCloud {
    fn new(on: bool, mode: Mode) -> Arc<Self> {
        Arc::new(Self {
            on: AtomicBool::new(on),
            mode,
            calls: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
            cancelled: AtomicBool::new(false),
        })
    }
}

impl CloudStt for FakeCloud {
    fn enabled(&self) -> bool {
        self.on.load(Ordering::SeqCst)
    }
    fn transcribe(&self, samples: &mut [f32], cancel: &AtomicBool) -> Result<String, &'static str> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.len.store(samples.len(), Ordering::SeqCst);
        match self.mode {
            Mode::Text => Ok("cloud words".to_owned()),
            Mode::Fail(code) => Err(code),
            Mode::UntilCancel => {
                let end = Instant::now() + Duration::from_secs(5);
                while Instant::now() < end && !cancel.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(2));
                }
                self.cancelled.store(true, Ordering::SeqCst);
                Err("cancelled")
            }
        }
    }
}

fn rig(cloud: &Arc<FakeCloud>) -> (LocalSttEngine, Arc<Out>) {
    let out = Arc::new(Out::default());
    let mut parts = EngineParts::new(
        Arc::new(Loader),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    parts.cloud = Some(Arc::clone(cloud) as Arc<dyn CloudStt>);
    let engine = LocalSttEngine::spawn(parts);
    engine.sync();
    (engine, out)
}

const RATE: u32 = 48_000;

fn tone(ms: u64) -> Vec<f32> {
    let n = (u64::from(RATE) * ms / 1_000) as usize;
    (0..n)
        .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / RATE as f32).sin())
        .collect()
}

fn feed(engine: &LocalSttEngine, samples: &[f32]) {
    for block in samples.chunks((RATE / 100) as usize) {
        engine.frames(block, RATE);
    }
}

fn hold(engine: &LocalSttEngine, mode: CaptureMode, samples: &[f32]) {
    engine.begin(7, "mog-a", mode);
    feed(engine, samples);
    engine.commit(7, StopReason::Released);
    engine.sync();
}

#[test]
fn adr_0123_the_cloud_text_replaces_the_local_text_and_names_its_provider() {
    let cloud = FakeCloud::new(true, Mode::Text);
    let (engine, out) = rig(&cloud);
    hold(&engine, CaptureMode::Ptt, &tone(1_600));
    let done = out.wait(1);
    assert_eq!(done.len(), 1);
    let t = done[0].result.as_ref().expect("a transcript");
    assert_eq!(
        (t.text.as_str(), t.raw.as_str()),
        ("cloud words", "cloud words")
    );
    assert_eq!(t.provider, TranscriptProvider::DbxGemini);
    assert_eq!(t.model, cloud_dictation::MODEL);
    assert_eq!(t.runtime, cloud_dictation::RUNTIME);
    assert_eq!(t.segment_id, "ptt-7");
    // The whole hold (1.6 s at 16 kHz) went to the cloud, once.
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 1);
    assert!((25_000..=26_000).contains(&cloud.len.load(Ordering::SeqCst)));
}

#[test]
fn adr_0123_every_cloud_error_falls_back_to_the_local_text_of_the_same_audio() {
    for code in [
        "timeout",
        "offline",
        "refused",
        "signed_out",
        "no_home",
        "bad_reply",
    ] {
        let cloud = FakeCloud::new(true, Mode::Fail(code));
        let (engine, out) = rig(&cloud);
        hold(&engine, CaptureMode::Ptt, &tone(1_600));
        let done = out.wait(1);
        assert_eq!(done.len(), 1, "{code}");
        let t = done[0].result.as_ref().expect("the local transcript");
        assert_eq!(t.text, "local words", "{code}");
        assert_eq!(t.provider, TranscriptProvider::LocalParakeet, "{code}");
        assert_eq!(cloud.calls.load(Ordering::SeqCst), 1, "{code}: no retry");
    }
}

#[test]
fn adr_0123_the_default_engine_never_calls_the_cloud() {
    let cloud = FakeCloud::new(false, Mode::Text);
    let (engine, out) = rig(&cloud);
    hold(&engine, CaptureMode::Ptt, &tone(1_600));
    let done = out.wait(1);
    assert_eq!(done[0].result.as_ref().unwrap().text, "local words");
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn adr_0123_a_hands_free_take_goes_to_the_cloud_too_and_a_session_never_does() {
    let cloud = FakeCloud::new(true, Mode::Text);
    let (engine, out) = rig(&cloud);
    hold(&engine, CaptureMode::HandsFree, &tone(1_600));
    let done = out.wait(1);
    assert_eq!(done[0].result.as_ref().unwrap().text, "cloud words");
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 1);
}

#[test]
fn adr_0123_silence_never_goes_to_the_cloud() {
    let cloud = FakeCloud::new(true, Mode::Text);
    let (engine, out) = rig(&cloud);
    hold(&engine, CaptureMode::Ptt, &vec![0.0; (RATE as usize) * 2]);
    let done = out.wait(1);
    assert_eq!(done[0].result.as_ref().unwrap().text, "");
    assert_eq!(cloud.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn adr_0123_a_wipe_cancels_the_call_and_gives_no_result() {
    let cloud = FakeCloud::new(true, Mode::UntilCancel);
    let (engine, out) = rig(&cloud);
    hold(&engine, CaptureMode::Ptt, &tone(1_600));
    let end = Instant::now() + Duration::from_secs(5);
    while cloud.calls.load(Ordering::SeqCst) == 0 && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(2));
    }
    engine.discard_all();
    engine.sync();
    while !cloud.cancelled.load(Ordering::SeqCst) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        cloud.cancelled.load(Ordering::SeqCst),
        "the call saw the cancel"
    );
    std::thread::sleep(Duration::from_millis(50));
    assert!(out.now().is_empty(), "no transcript after a cancel");
}

#[test]
fn adr_0123_the_cloud_deadline_is_six_seconds() {
    assert_eq!(cloud_dictation::DEADLINE, Duration::from_secs(6));
}
