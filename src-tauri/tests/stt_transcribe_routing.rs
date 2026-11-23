//! #514 part 1 (ADR 0082): the Tauri host sends `stt_transcribe` to R4 `LocalStt` through
//! `SttEngine::transcribe`. Only the brain may call it (SEC-02a). R4's reasons cross as the
//! content-free `host_failed` (SEC-03). The transcript text never shows in a `Debug` output.
//! Every value is plainly fake.

use bottle_lib::ipc::{route, runs_off_main_thread, BrainLink, HostComponents};
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::ChannelRef;
use mog_core::stt::{SttEngine, LOCAL_STT_MODEL, LOCAL_STT_RUNTIME};
use mog_core::stt_engine::{LocalStt, NoModelLoader, NoTranscriptSink};
use mog_core::types::{Transcript, TranscriptProvider};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

const MOG: &str = "00000000-0000-4000-8000-0000000000a1";
/// The fake transcript text. No output except the reply may hold it.
const MARK: &str = "test-transcript-not-real";

struct NoBrain;

impl BrainLink for NoBrain {
    fn relay(&self, _event: &str, _payload: Value) -> bool {
        false
    }
    fn subscribe(&self, _channel: &ChannelRef) -> bool {
        false
    }
    fn notify_hub(&self, _notice: Value) -> bool {
        false
    }
    fn subscribe_hub(&self, _channel: &ChannelRef) -> bool {
        false
    }
}

fn transcript(segment_id: &str) -> Transcript {
    Transcript {
        segment_id: segment_id.to_owned(),
        text: MARK.to_owned(),
        raw: format!("{MARK} raw"),
        lang: Some("en".to_owned()),
        provider: TranscriptProvider::LocalParakeet,
        model: LOCAL_STT_MODEL.to_owned(),
        runtime: LOCAL_STT_RUNTIME.to_owned(),
        latency_ms: 42,
        remember: None,
    }
}

/// An R4 double: it records each asked segment and answers `answer`.
struct FakeStt {
    seen: Mutex<Vec<String>>,
    answer: Result<(), &'static str>,
}

impl FakeStt {
    fn ok() -> Self {
        Self::answering(Ok(()))
    }
    fn answering(answer: Result<(), &'static str>) -> Self {
        Self {
            seen: Mutex::default(),
            answer,
        }
    }
    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl SttEngine for FakeStt {
    fn load_model(&self) -> CoreResult<()> {
        Ok(())
    }
    fn transcribe(&self, segment_id: &str) -> CoreResult<Transcript> {
        self.seen.lock().unwrap().push(segment_id.to_owned());
        match self.answer {
            Ok(()) => Ok(transcript(segment_id)),
            Err(reason) => Err(CoreError::Unavailable {
                component: "R4",
                reason,
            }),
        }
    }
}

fn call(from: &str, stt: Option<&dyn SttEngine>, args: Value) -> Result<Value, Value> {
    let components = HostComponents {
        stt,
        ..HostComponents::none(from, &NoBrain)
    };
    route(components, "stt_transcribe", args)
}

fn args(segment_id: &str, provider: &str) -> Value {
    json!({
        "segmentId": segment_id,
        "provider": provider,
        "tags": {"app": "bottle", "mog": MOG, "op": "asr"}
    })
}

#[test]
fn cap_72_brain_stt_transcribe_reaches_r4_and_returns_the_transcript() {
    let r4 = FakeStt::ok();
    let out = call("brain", Some(&r4), args("ptt-7", "local_parakeet")).unwrap();
    // The `Transcript` shape of `src/shared/interfaces.ts`.
    assert_eq!(
        out,
        json!({
            "segmentId": "ptt-7",
            "text": MARK,
            "raw": format!("{MARK} raw"),
            "lang": "en",
            "provider": "local_parakeet",
            "model": LOCAL_STT_MODEL,
            "runtime": LOCAL_STT_RUNTIME,
            "latencyMs": 42,
        })
    );
    assert_eq!(r4.seen(), vec!["ptt-7".to_owned()]);
}

#[test]
fn sec_02a_at_17_a_view_window_cannot_call_stt_transcribe() {
    let r4 = FakeStt::ok();
    for from in ["hub", "mog-a1", "card-a1"] {
        assert_eq!(
            call(from, Some(&r4), args("ptt-7", "local_parakeet")),
            Err(json!({"error": "window_not_allowed"})),
            "{from}"
        );
    }
    assert!(r4.seen().is_empty(), "R4 got a call from a view window");
}

#[test]
fn sec_03_each_r4_reason_crosses_as_content_free_host_failed() {
    for reason in [
        "stt_segment_unknown",
        "stt_model_missing",
        "stt_engine_failed",
    ] {
        let r4 = FakeStt::answering(Err(reason));
        let got = call("brain", Some(&r4), args("ptt-7", "local_parakeet"));
        assert_eq!(got, Err(json!({"error": "host_failed"})), "{reason}");
        let text = got.unwrap_err().to_string();
        assert!(
            !text.contains("ptt-7") && !text.contains(MARK),
            "{reason}: {text}"
        );
    }
}

#[test]
fn sec_03_the_real_r4_answers_an_unknown_segment_with_host_failed() {
    let r4 = LocalStt::new(Arc::new(NoModelLoader), Arc::new(NoTranscriptSink), |_| {});
    assert_eq!(
        call("brain", Some(&r4), args("ptt-1", "local_parakeet")),
        Err(json!({"error": "host_failed"}))
    );
}

#[test]
fn sec_03_no_r4_managed_answers_host_failed() {
    assert_eq!(
        call("brain", None, args("ptt-7", "local_parakeet")),
        Err(json!({"error": "host_failed"}))
    );
}

#[test]
fn cap_57c_a_databricks_asr_provider_is_not_routed_to_r4() {
    // Local STT is the PTT default (CAP-57c). Databricks ASR has no host route yet (#49).
    let r4 = FakeStt::ok();
    for provider in ["dbx_gemini", "dbx_whisper"] {
        assert_eq!(
            call("brain", Some(&r4), args("ptt-7", provider)),
            Err(json!({"error": "not_implemented"})),
            "{provider}"
        );
    }
    assert!(r4.seen().is_empty(), "R4 decoded for a cloud provider");
}

#[test]
fn nfr_01a_stt_transcribe_runs_off_the_main_thread() {
    assert!(runs_off_main_thread("stt_transcribe"));
}

#[test]
fn sec_03_sec_16b_a_transcript_debug_holds_no_text() {
    let t = transcript("ptt-7");
    for shown in [
        format!("{t:?}"),
        format!("{t:#?}"),
        format!("{:?}", Ok::<_, CoreError>(t.clone())),
        format!("{:#?}", Some(t.clone())),
    ] {
        assert!(!shown.contains(MARK), "{shown}");
        assert!(
            shown.contains("ptt-7"),
            "the content-free ID stays: {shown}"
        );
    }
}

#[test]
fn cap_72_route_from_reads_the_managed_r4() {
    let src = include_str!("../src/ipc.rs");
    let body: String = src[src.find("fn route_from").expect("route_from")..]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        body.contains("try_state::<mog_core::stt_engine::LocalStt>()"),
        "the host reads the R4 that install_capture manages"
    );
}

#[test]
fn cap_72_install_capture_manages_the_r4_that_route_from_reads() {
    // Without `handle.manage(stt)`, every `stt_transcribe` answers `host_failed`.
    let src = include_str!("../src/lib.rs");
    let start = src.find("fn install_capture").expect("install_capture");
    let rest = &src[start..];
    // Only the body of `install_capture`: it ends at the next top-level item.
    let end = rest.find("\n}\n").expect("the end of install_capture");
    let body: String = rest[..end].chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        body.contains("letstt=mog_core::stt_engine::LocalStt::new("),
        "install_capture builds R4 as a LocalStt"
    );
    assert!(
        body.contains("handle.manage(stt);"),
        "install_capture manages R4, so route_from finds it"
    );
}
