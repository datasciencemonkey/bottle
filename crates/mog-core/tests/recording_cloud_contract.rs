//! PR C on the contract seam (ADR 0103): `CloudAdapter` implements `RecordingCloud`, and
//! `RecordingComponent` routes the cloud-bound commands to it with the brain's home profile.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::clock::SimWallClock;
use mog_core::error::CoreResult;
use mog_core::http_transport::Pause;
use mog_core::ipc_gateway::GatewayComponents;
use mog_core::recording::{MemorySettingsStore, RecordingComponent};
use mog_core::recording_cloud::cloud::{
    CloudDeps, LocalPart, RecordingCloudService, WorkspaceInfo,
};
use mog_core::recording_cloud::contract::{
    summary_of, wire_kind, CloudAdapter, LocalAudio, NoLocalAudio,
};
use mog_core::recording_cloud::errors::ErrorKind;
use mog_core::recording_cloud::http::ScriptedHttp;
use mog_core::recording_cloud::job::{iso_utc, parse_iso_utc_ms, JobManifest, PartEntry};
use mog_core::types::{RecordingErrorKind, RecordingSettings, RecordingSummary};
use serde_json::{json, Value};

/// A fixed set of names with temp audio on the device (ADR 0103 decision 7), and a fixed set
/// of names that are active (capture still owns them, or they are not yet handed off; review
/// of #592, BLOCKING B3).
struct FakeLocal(Vec<(String, u64)>, HashSet<String>);

impl FakeLocal {
    fn new(names: Vec<(String, u64)>) -> Self {
        Self(names, HashSet::new())
    }
    fn with_active(names: Vec<(String, u64)>, active: &[&str]) -> Self {
        Self(names, active.iter().map(|s| (*s).to_owned()).collect())
    }
}

impl LocalAudio for FakeLocal {
    fn parts(&self, _name: &str) -> Vec<LocalPart> {
        Vec::new()
    }
    fn names(&self) -> Vec<(String, u64)> {
        self.0.clone()
    }
    fn active(&self) -> HashSet<String> {
        self.1.clone()
    }
}

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
    component_with(
        http,
        storage,
        Arc::new(NoLocalAudio),
        Arc::new(|| 1_795_100_700_000i64),
    )
}

/// Like [`component`], with the local temp audio and the wall clock of the in-flight guard
/// under the test's control (the review of #592: the launch scan of decision 7, and the
/// stale-job check of `retry_allowed`).
fn component_with(
    http: &Arc<ScriptedHttp>,
    storage: Option<Value>,
    local: Arc<dyn LocalAudio>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
) -> (
    RecordingComponent,
    Opened,
    Arc<Mutex<Vec<RecordingSummary>>>,
) {
    let s = RecordingSettings {
        storage: storage.map(|v| serde_json::from_value(v).unwrap()),
        ..Default::default()
    };
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
        local,
        Arc::new(move |s: &RecordingSummary| j.lock().unwrap().push(s.clone())),
        now_ms,
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
    // No storage: no volume call, and the list is the audio that waits on the device.
    let v = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    assert_eq!(v, json!([]));
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
    // The review of #592 (BLOCKING 1) reads job.json once more, up front, to gate the retry
    // (`retry_allowed`) before anything else runs.
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("failed")).unwrap(),
    ); // the gate's own read (current)
    http.bytes("files.download", b"Write notes."); // instructions.md
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("failed")).unwrap(),
    ); // the service's own read, inside retry()
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
fn rec_component_retry_refuses_a_done_recording() {
    // Review of #592 (BLOCKING 1): Retry may run only a failed, or a stale, job. A `done`
    // recording never restarts, so no upload or model call is scripted; any attempt to run one
    // would fail with `no_script` and the test would fail for the right reason.
    let http = Arc::new(ScriptedHttp::new());
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("done")).unwrap(),
    );
    let (c, _, jobs) = component(&http, vol());
    let v = c
        .call("recording_retry", json!({"profile":"fake-home","id":NAME}))
        .unwrap();
    assert_eq!(v["status"], "done", "{v}");
    assert_eq!(http.seen().len(), 1, "a done recording starts no new run");
    assert!(jobs.lock().unwrap().is_empty(), "no job update is emitted");
}

#[test]
fn rec_component_list_shows_local_only_audio_as_upload_failed() {
    // ADR 0103 decision 7: audio left on the device with no job.json in the volume (the first
    // `mkdir` or `job.json` write never ran: offline, 403, signed out, or no profile/storage at
    // the hand-off) still shows in Recordings, as upload failed, so Retry can pick it up. The
    // review of #592 (BLOCKING 2).
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "files.list_directory",
        404,
        json!({"error_code":"NOT_FOUND"}),
    );
    let local: Arc<dyn LocalAudio> =
        Arc::new(FakeLocal::new(vec![(NAME.to_owned(), 1_795_097_100_000)]));
    let (c, _, _) = component_with(&http, vol(), local, Arc::new(|| 1_795_100_700_000i64));
    let v = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v[0]["id"], NAME);
    assert_eq!(v[0]["status"], "failed");
    assert_eq!(v[0]["errorKind"], "upload_failed");
    assert_eq!(v[0]["errorStage"], "uploading");
    assert_eq!(v[0]["startedAtMs"], 1_795_097_100_000u64);
}

#[test]
fn rec_component_list_does_not_duplicate_audio_already_in_a_job() {
    // When the volume already has a job.json for a name, the device-only row of decision 7
    // must not duplicate it (a 409 can leave the local file, mid-upload, alongside a real row).
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "files.list_directory",
        200,
        json!({"contents":[{"path":"x","is_directory":true,"name":NAME}]}),
    );
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("failed")).unwrap(),
    );
    let local: Arc<dyn LocalAudio> =
        Arc::new(FakeLocal::new(vec![(NAME.to_owned(), 1_795_097_100_000)]));
    let (c, _, _) = component_with(&http, vol(), local, Arc::new(|| 1_795_100_700_000i64));
    let v = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    assert_eq!(v.as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v[0]["status"], "failed");
    assert_eq!(
        v[0]["errorKind"], "notes_endpoint_missing",
        "the real job.json wins"
    );
}

#[test]
fn rec_component_list_leaves_out_the_recording_running_now() {
    // Review of #592 (BLOCKING B3): the chunker writes the open part under its final name, in
    // the same folder the scan reads, so a call in progress must never show as "upload
    // failed" just because it has no job.json yet.
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "files.list_directory",
        404,
        json!({"error_code":"NOT_FOUND"}),
    );
    let local: Arc<dyn LocalAudio> = Arc::new(FakeLocal::with_active(
        vec![(NAME.to_owned(), 1_795_097_100_000)],
        &[NAME],
    ));
    let (c, _, _) = component_with(&http, vol(), local, Arc::new(|| 1_795_100_700_000i64));
    let v = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    // Upload in parts: the hub list shows the call in progress as "recording", never as a failed
    // upload.
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 1, "{v}");
    assert_eq!(rows[0]["status"], "recording");
    assert!(rows[0].get("errorKind").is_none(), "{v}");
}

#[test]
fn rec_component_retry_refuses_the_recording_running_now() {
    // Same review item: Retry on that name must never upload or delete the part the encoder
    // still writes.
    let http = Arc::new(ScriptedHttp::new());
    let local: Arc<dyn LocalAudio> = Arc::new(FakeLocal::with_active(Vec::new(), &[NAME]));
    let (c, _, jobs) = component_with(&http, vol(), local, Arc::new(|| 1_795_100_700_000i64));
    let e = c
        .call("recording_retry", json!({"profile":"fake-home","id":NAME}))
        .unwrap_err();
    assert_eq!(e.code(), "recording_busy");
    assert_eq!(
        http.seen().len(),
        0,
        "no call reads or touches the open part"
    );
    assert!(jobs.lock().unwrap().is_empty());
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

#[test]
fn rec_component_list_without_storage_shows_the_device_only_recordings_newest_first() {
    let http = Arc::new(ScriptedHttp::new());
    let local = Arc::new(FakeLocal::new(vec![
        ("2026-11-19_0905".to_owned(), 1_000),
        ("2026-11-20_1000".to_owned(), 2_000),
    ]));
    let (c, _, _) = component_with(&http, None, local, Arc::new(|| 1_795_100_700_000i64));
    let v = c
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    let rows = v.as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "2026-11-20_1000");
    assert_eq!(rows[0]["onDeviceOnly"], true);
    assert_eq!(rows[0]["errorKind"], "upload_failed");
    // A call that records now shows as recording, not as a failed upload.
    let local = Arc::new(FakeLocal::with_active(
        vec![("2026-11-20_1000".to_owned(), 2_000)],
        &["2026-11-20_1000"],
    ));
    let (c2, _, _) = component_with(&http, None, local, Arc::new(|| 1_795_100_700_000i64));
    let v2 = c2
        .call("recording_list", json!({"profile":"fake-home"}))
        .unwrap();
    assert_eq!(v2[0]["status"], "recording");
    // Retry still needs a storage: the brain shows why.
    let e = c
        .call(
            "recording_retry",
            json!({"profile":"fake-home","id":"2026-11-20_1000"}),
        )
        .unwrap_err();
    assert_eq!(e.code(), "no_storage");
}

#[test]
fn rec_component_open_transcript_and_notes_open_the_folder_link_only_for_a_good_name() {
    let http = Arc::new(ScriptedHttp::new());
    let (c, opened, _) = component(&http, vol());
    for target in ["transcript", "notes", "folder"] {
        let v = c
            .call(
                "recording_open",
                json!({"profile":"fake-home","id":NAME,"target":target}),
            )
            .unwrap();
        assert_eq!(v, json!({}));
    }
    let o = opened.lock().unwrap().clone();
    assert_eq!(o.len(), 3);
    assert!(o.iter().all(|(_, u)| u == &o[0].1));
    assert!(o[0].1.starts_with(
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath=%2FVolumes%2Fmain%2Fteam%2Faudio%2FBottle%2FRecordings%2F2026-11-19_0905%2F"
    ));
    // An unknown target and a bad name never reach the opener.
    assert!(c
        .call(
            "recording_open",
            json!({"profile":"fake-home","id":NAME,"target":"audio"})
        )
        .is_err());
    assert!(c
        .call(
            "recording_open",
            json!({"profile":"fake-home","id":"..-x","target":"notes"})
        )
        .is_err());
    assert_eq!(opened.lock().unwrap().len(), 3);
}
