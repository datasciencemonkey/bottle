//! #46 CAP-34 (ADR 0082): R4 is the PTT buffer's consumer and keeps no second copy of the
//! audio. The test counts the live `Utterance` values, so it is alone in its own test binary
//! (no other test makes an utterance at the same time).

use std::sync::{Arc, Mutex};

use mog_core::audio_device::level::TeeSink;
use mog_core::audio_device::ptt_buffer::{live_utterances, PttBuffer, PttCaptureEvents};
use mog_core::audio_device::sink::{FrameSink, MicFrame};
use mog_core::capture::{CaptureEvents, StopReason, Stopped};
use mog_core::stt_engine::{
    EngineParts, LocalSttEngine, PowerProbe, PowerSource, Recognizer, RecognizerLoader,
    SttCaptureEvents, SttDone, SttIdentity, TranscriptSink,
};
use mog_core::types::{CaptureMode, CaptureState, OthersTrack};

struct Words;
impl Recognizer for Words {
    fn transcribe(&mut self, _s: &[f32]) -> String {
        "w".into()
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
    fn done(&self, d: SttDone) {
        self.0.lock().unwrap().push(d);
    }
}
struct NoEvents;
impl CaptureEvents for NoEvents {
    fn state(&self, _s: &CaptureState) {}
    fn stopped(&self, _s: &Stopped) {}
}

#[test]
fn cap_34_r4_drops_the_ptt_buffer_utterance_so_no_second_copy_lives() {
    let out = Arc::new(Out::default());
    let mut parts = EngineParts::new(
        Arc::new(Loader),
        Arc::clone(&out) as Arc<dyn TranscriptSink>,
    );
    parts.power = Arc::new(Ac);
    let r4 = LocalSttEngine::spawn(parts);
    let buf = PttBuffer::new(Arc::new(r4.clone()));
    let mut mic = TeeSink(buf.sink(), r4.frame_sink());
    let events = PttCaptureEvents::new(SttCaptureEvents::new(NoEvents, r4.clone()), buf.clone());
    let before = live_utterances();
    events.state(&CaptureState {
        owner: Some("mog-a".into()),
        mode: Some(CaptureMode::Ptt),
        lease_id: Some("1".into()),
        recording: true,
        paused: false,
        session_id: None,
        started_at: None,
        directed_to: None,
        others_track: OthersTrack::Off,
        mic: None,
    });
    let block: Vec<f32> = (0..480).map(|i| 0.3 * (i as f32 * 0.03).sin()).collect();
    for _ in 0..50 {
        mic.frame(MicFrame {
            samples: &block,
            sample_rate: 48_000,
        });
    }
    events.stopped(&Stopped {
        lease_id: 1,
        mog_id: "mog-a".into(),
        mode: CaptureMode::Ptt,
        reason: StopReason::Released,
    });
    r4.sync();
    assert_eq!(out.0.lock().unwrap().len(), 1, "R4 made the transcript");
    assert_eq!(
        live_utterances(),
        before,
        "the PTT buffer's utterance must not outlive the hand-off to R4 (CAP-34)"
    );
}
