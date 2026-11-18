//! AT-04 "Dictate offline" (#231), section 15. The Rust part.
//!
//! The AT script: turn off the network, dictate with local STT, ask a question, write a memory
//! item. Pass: local text returns marked "not cleaned" and the insertion works; no cloud request
//! occurs; the confirmed ask goes to the outbox and replays when online; the embedding request
//! waits in the outbox.
//!
//! What runs here is the contract part of CAP-72: a transcript without `provider`, `model` or
//! `runtime` is refused, so every transcript row that passes the gateway names them. The engine
//! that fills them (R4, #46) runs here with a fake recognizer. The brain parts (fallback chain, command grammar,
//! outbox, offline recall) are in `src/acceptance/at-04-offline.test.ts`. The network-off run is
//! a person's step (#175): docs/tests/AT-DICTATION-STT-OFFLINE.md.

use std::sync::Arc;

use mog_core::capture::StopReason;
use mog_core::stt::{SttEngine, LOCAL_STT_MODEL, LOCAL_STT_RUNTIME};
use mog_core::stt_engine::{LocalStt, NoTranscriptSink, Recognizer, RecognizerLoader, SttIdentity};
use mog_core::types::CaptureMode;
use mog_core::types::{Transcript, TranscriptProvider};
use serde_json::{json, Value};

fn row() -> Value {
    json!({
        "segmentId": "seg-1",
        "text": "cleaned",
        "raw": "raw",
        "provider": "local_parakeet",
        "model": "parakeet-tdt-0.6b-v3-int8",
        "runtime": "sherpa-onnx 1.12.0",
        "latencyMs": 420
    })
}

#[test]
fn at_04_cap_72_a_transcript_names_its_provider_model_and_runtime() {
    let t: Transcript = serde_json::from_value(row()).expect("a full transcript row");
    assert_eq!(t.provider, TranscriptProvider::LocalParakeet);
    assert!(!t.model.is_empty() && !t.runtime.is_empty());
    for field in ["provider", "model", "runtime"] {
        let mut v = row();
        v.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<Transcript>(v).is_err(),
            "a transcript without {field} must be refused"
        );
    }
}

#[test]
fn at_04_cap_72_local_stt_records_the_model_and_runtime_offline() {
    // #46: R4 with a fake recognizer (no model, no network). The Transcript that
    // `stt_transcribe` returns names provider local_parakeet, and model and runtime are the
    // local Parakeet identity, not empty strings. The real-model run is in tests/stt_local.rs
    // behind BOTTLE_STT_MODEL_DIR (TODO(#414)).
    struct Fake;
    impl Recognizer for Fake {
        fn transcribe(&mut self, _samples: &[f32]) -> String {
            "offline text".into()
        }
    }
    struct Loader;
    impl RecognizerLoader for Loader {
        fn load(&self) -> Result<Box<dyn Recognizer>, &'static str> {
            Ok(Box::new(Fake))
        }
        fn identity(&self) -> SttIdentity {
            SttIdentity::local_parakeet()
        }
    }
    let r4 = LocalStt::new(Arc::new(Loader), Arc::new(NoTranscriptSink), |_| {});
    r4.load_model()
        .expect("the local model loads with no network");
    let tone: Vec<f32> = (0..16_000)
        .map(|i| 0.3 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 16_000.0).sin())
        .collect();
    r4.engine().begin(1, "mog-a", CaptureMode::Ptt);
    r4.engine().frames(&tone, 16_000);
    r4.engine().commit(1, StopReason::Released);
    r4.engine().sync();
    let t = r4.transcribe("ptt-1").expect("a local transcript");
    assert_eq!(t.text, "offline text");
    assert_eq!(t.provider, TranscriptProvider::LocalParakeet);
    assert_eq!(t.model, LOCAL_STT_MODEL);
    assert_eq!(t.runtime, LOCAL_STT_RUNTIME);
}
