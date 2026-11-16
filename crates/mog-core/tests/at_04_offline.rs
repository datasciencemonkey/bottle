//! AT-04 "Dictate offline" (#231), section 15. The Rust part.
//!
//! The AT script: turn off the network, dictate with local STT, ask a question, write a memory
//! item. Pass: local text returns marked "not cleaned" and the insertion works; no cloud request
//! occurs; the confirmed ask goes to the outbox and replays when online; the embedding request
//! waits in the outbox.
//!
//! What runs here is the contract part of CAP-72: a transcript without `provider`, `model` or
//! `runtime` is refused, so every transcript row that passes the gateway names them. The engine
//! that fills them (R4, #46) is pending. The brain parts (fallback chain, command grammar,
//! outbox, offline recall) are in `src/acceptance/at-04-offline.test.ts`. The network-off run is
//! a person's step (#175): docs/tests/AT-DICTATION-STT-OFFLINE.md.

use mog_core::stt::{StubSttEngine, SttEngine};
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
#[ignore = "pending TODO(#46): R4 SttEngine (model residency, transcript identity) is not built"]
fn at_04_cap_72_local_stt_records_the_model_and_runtime_offline() {
    // TODO(#46): with no network, load the local model and transcribe a fixture segment; the
    // Transcript of stt_transcribe has provider local_parakeet and a non-empty model and runtime.
    let r4 = StubSttEngine;
    r4.load_model().expect("TODO(#46): the local model loads with no network");
    let text = r4.transcribe("seg-1").expect("TODO(#46): a local transcript");
    assert!(!text.is_empty());
    todo!("TODO(#46): assert the stt_transcribe Transcript names provider, model and runtime");
}
