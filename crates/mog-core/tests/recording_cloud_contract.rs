//! PR C on the contract seam (ADR 0103): `CloudAdapter` implements `RecordingCloud`, and
//! `RecordingComponent` routes the cloud-bound commands to it with the brain's home profile.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::clock::SimWallClock;
use mog_core::error::CoreResult;
use mog_core::http_transport::Pause;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::recording::{MemorySettingsStore, RecordingComponent};
use mog_core::recording_cloud::cloud::{CloudDeps, RecordingCloudService, WorkspaceInfo};
use mog_core::recording_cloud::contract::{summary_of, wire_kind, CloudAdapter};
use mog_core::recording_cloud::errors::ErrorKind;
use mog_core::recording_cloud::http::ScriptedHttp;
use mog_core::recording_cloud::job::{iso_utc, parse_iso_utc_ms, JobManifest, PartEntry};
use mog_core::types::{RecordingErrorKind, RecordingSettings, RecordingSummary};
use serde_json::{json, Value};

struct Ws;
impl WorkspaceInfo for Ws {
    fn host_url(&self, _p: &str) -> CoreResult<String> {
        Ok("https://dbc-fake.cloud.databricks.com".into())
    }
    fn workspace_id(&self, _p: &str) -> CoreResult<String> {
        Ok("1234567890".into())
    }
}

struct NoPause;
impl Pause for NoPause {
    fn pause(&self, _d: Duration) {}
}

type Opened = Arc<Mutex<Vec<(String, String)>>>;

fn component(
    http: &Arc<ScriptedHttp>,
    storage: Option<Value>,
) -> (
    RecordingComponent,
    Opened,
    Arc<Mutex<Vec<RecordingSummary>>>,
) {
    let mut s = RecordingSettings::default();
    s.storage = storage.map(|v| serde_json::from_value(v).unwrap());
    let store = Arc::new(MemorySettingsStore::new(s));
    let service = Arc::new(RecordingCloudService::new(CloudDeps {
        http: http.clone(),
        workspace: Arc::new(Ws),
        wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
        pause: Arc::new(NoPause),
    }));
    let jobs = Arc::new(Mutex::new(Vec::new()));
    let j = jobs.clone();
    let adapter = Arc::new(CloudAdapter::new(
        service,
        store.clone(),
        Arc::new(|_: &str| Vec::new()),
        Arc::new(move |s: &RecordingSummary| j.lock().unwrap().push(s.clone())),
    ));
    let opened: Opened = Arc::new(Mutex::new(Vec::new()));
    let o = opened.clone();
    let c = RecordingComponent::new(store);
    c.set_cloud(
        adapter,
        Arc::new(move |p: &String, url: &str| {
            o.lock().unwrap().push((p.clone(), url.to_owned()));
            Ok(())
        }),
    );
    (c, opened, jobs)
}

const ROOT: &str = "/Volumes/main/team/audio";
const NAME: &str = "2026-11-19_0905";

fn manifest(status: &str) -> Value {
    json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":61.6,
        "parts":[{"file":"2026-11-19_0905.opus","duration_s":61.6,"uploaded":true}],
        "status":status,"error_kind":"notes_endpoint_missing","error_stage":"writing_notes",
        "transcribe_endpoint":"databricks-gemini-3-8-flash","notes_endpoint":"databricks-claude-sonnet-5-5",
        "updated_at":"2026-11-19T14:07:00Z"})
}

fn vol() -> Option<Value> {
    Some(json!({"kind":"volume","root":ROOT}))
}

#[test]
fn rec_component_list_needs_storage_and_the_profile() {
    let http = Arc::new(ScriptedHttp::new());
    let (c, _, _) = component(&http, None);
    let e = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap_err();
    assert_eq!(e.code(), "no_storage");
    let e = c.call("recording_list", json!({})).unwrap_err();
    assert_eq!(e.code(), "invalid_args");

    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "files.list_directory",
        200,
        json!({"contents":[
        {"path":"x","is_directory":true,"name":NAME}]}),
    );
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("failed")).unwrap(),
    );
    let (c, _, _) = component(&http, vol());
    let v = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    assert_eq!(v[0]["id"], NAME);
    assert_eq!(v[0]["status"], "failed");
    assert_eq!(v[0]["errorKind"], "notes_endpoint_missing");
    assert_eq!(v[0]["errorStage"], "writing_notes");
    assert_eq!(v[0]["durationS"], 62);
    assert_eq!(v[0]["startedAtMs"], 1_795_097_100_000u64);
}

#[test]
fn rec_component_detail_and_open_folder() {
    let http = Arc::new(ScriptedHttp::new());
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("done")).unwrap(),
    );
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"Write notes.","parts":[],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    http.bytes("files.download", b"## Summary");
    http.bytes("files.download", b"**Speaker 1** [00:00:00]\nHi.\n");
    http.bytes("files.download", b"Write notes.");
    let (c, opened, _) = component(&http, vol());
    let d = c
        .call("recording_detail", json!({"profile":"fake-home","id":NAME}))
        .unwrap();
    assert_eq!(d["summary"]["id"], NAME);
    assert_eq!(d["instructions"], "Write notes.");
    assert_eq!(d["segments"][0]["text"], "Hi.");
    assert_eq!(d["notesMd"], "## Summary");
    assert!(d["transcriptMd"]
        .as_str()
        .unwrap()
        .contains("**Speaker 1**"));

    let v = c
        .call(
            "recording_open",
            json!({"profile":"fake-home","id":NAME,"target":"folder"}),
        )
        .unwrap();
    assert_eq!(v, json!({}));
    let o = opened.lock().unwrap().clone();
    assert_eq!(o.len(), 1);
    assert_eq!(o[0].0, "fake-home");
    assert!(o[0].1.starts_with("https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath="));
}

#[test]
fn rec_component_probe_and_volume_create() {
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "recording.me",
        200,
        json!({"userName":"first.last@example.com"}),
    );
    http.json(
        "files.list_directory",
        404,
        json!({"error_code":"NOT_FOUND"}),
    );
    http.json(
        "files.list_directory",
        404,
        json!({"error_code":"NOT_FOUND"}),
    );
    let (c, _, _) = component(&http, None);
    let v = c
        .call("recording_storage_probe", json!({"profile":"fake-home"}))
        .unwrap();
    assert_eq!(v, json!({"myFilesRoot": null}));

    let http = Arc::new(ScriptedHttp::new());
    http.json("volumes.create", 200, json!({}));
    let (c, _, _) = component(&http, None);
    let v = c
        .call(
            "recording_volume_create",
            json!({"profile":"fake-home","catalog":"main","schema":"team"}),
        )
        .unwrap();
    assert_eq!(
        v,
        json!({"kind":"volume","root":"/Volumes/main/team/bottle_audio"})
    );
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "volumes.create",
        403,
        json!({"error_code":"PERMISSION_DENIED"}),
    );
    let (c, _, _) = component(&http, None);
    let e = c
        .call(
            "recording_volume_create",
            json!({"profile":"fake-home","catalog":"main","schema":"team"}),
        )
        .unwrap_err();
    assert_eq!(e.code(), "volume_permission");
}

#[test]
fn rec_component_retry_runs_from_the_failed_stage_and_emits_jobs() {
    let http = Arc::new(ScriptedHttp::new());
    http.bytes("files.download", b"Write notes."); // instructions.md
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("failed")).unwrap(),
    ); // started_at
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("failed")).unwrap(),
    ); // retry reads it
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"Write notes.","parts":[],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    http.json(
        "mlflow.chat_completions",
        200,
        json!({"choices":[{"message":{"content":"## Notes"}}]}),
    );
    for _ in 0..6 {
        http.empty("files.upload", 204);
    }
    let (c, _, jobs) = component(&http, vol());
    let v = c
        .call("recording_retry", json!({"profile":"fake-home","id":NAME}))
        .unwrap();
    assert_eq!(v["status"], "done", "{v}");
    let seen: Vec<_> = jobs.lock().unwrap().iter().map(|s| s.status).collect();
    assert!(seen.len() >= 2, "{seen:?}");
    assert_eq!(http.seen_on("mlflow.chat_completions").len(), 1);
}

#[test]
fn rec_contract_kind_and_time_mapping() {
    assert_eq!(
        wire_kind(ErrorKind::MyFilesUnavailable),
        RecordingErrorKind::NoStorage
    );
    assert_eq!(wire_kind(ErrorKind::Offline), RecordingErrorKind::Network);
    assert_eq!(wire_kind(ErrorKind::SignedOut), RecordingErrorKind::Auth);
    assert_eq!(
        wire_kind(ErrorKind::AudioMissing),
        RecordingErrorKind::AudioUnreadable
    );
    assert_eq!(
        wire_kind(ErrorKind::NotesFailed),
        RecordingErrorKind::Internal
    );
    for ms in [0i64, 951_782_400_000, 1_795_100_700_000] {
        assert_eq!(parse_iso_utc_ms(&iso_utc(ms)), Some(ms));
    }
    assert_eq!(parse_iso_utc_ms("2026-13-01T00:00:00Z"), None);
    assert_eq!(parse_iso_utc_ms("bad"), None);
    let m = JobManifest::new(
        "r",
        "bad name",
        "2026-11-19T14:05:00Z",
        vec![PartEntry::new("x", 1.0)],
        "a",
        "b",
        "t",
    );
    assert!(summary_of(&m).is_none());
}

#[test]
fn rec_component_without_cloud_answers_not_implemented() {
    let c = RecordingComponent::new(Arc::new(MemorySettingsStore::default()));
    let e = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap_err();
    assert_eq!(e.code(), "not_implemented");
}
