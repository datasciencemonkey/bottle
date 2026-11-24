//! The recording cloud service (ADR 0103, PR C) over a scripted HTTP double: storage probe,
//! volume create, upload with temp delete, the transcribe and notes model calls, list, detail,
//! retry and the Catalog Explorer link. Waits are simulated.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::clock::{SimClock, SimWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::Pause;
use mog_core::recording_cloud::cloud::*;
use mog_core::recording_cloud::errors::{ErrorKind, RecordingError, Stage};
use mog_core::recording_cloud::http::ScriptedHttp;
use mog_core::recording_cloud::job::{JobManifest, Status};
use mog_core::recording_cloud::settings_store::Storage;
use mog_core::types::TagOp;
use serde_json::{json, Value};

const P: &str = "fake-home";
const ROOT: &str = "/Volumes/main/team/audio";
const NAME: &str = "2026-11-19_0905";
const FOLDER: &str = "/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905";

struct Ws;
impl WorkspaceInfo for Ws {
    fn host_url(&self, _p: &str) -> CoreResult<String> {
        Ok("https://dbc-fake.cloud.databricks.com".into())
    }
    fn workspace_id(&self, _p: &str) -> CoreResult<String> {
        Ok("1234567890".into())
    }
}

#[derive(Debug, Default)]
struct SimPause {
    clock: SimClock,
    pauses: Mutex<Vec<Duration>>,
}
impl Pause for SimPause {
    fn pause(&self, d: Duration) {
        self.pauses.lock().unwrap().push(d);
        self.clock.advance(d);
    }
}

fn service(http: &Arc<ScriptedHttp>) -> RecordingCloudService {
    service_with(http, Arc::new(SimPause::default()))
}

fn service_with(http: &Arc<ScriptedHttp>, pause: Arc<SimPause>) -> RecordingCloudService {
    RecordingCloudService::new(CloudDeps {
        http: http.clone(),
        workspace: Arc::new(Ws),
        wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
        pause,
    })
}

fn temp_parts(tag: &str, n: usize) -> (PathBuf, Vec<LocalPart>) {
    let dir = std::env::temp_dir().join(format!("bottle-rec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let parts = (1..=n)
        .map(|i| {
            let path = if n == 1 {
                dir.join(format!("{NAME}.opus"))
            } else {
                dir.join(format!("{NAME}.part{i:02}.opus"))
            };
            std::fs::write(&path, format!("OggS-fake-{i}")).unwrap();
            LocalPart {
                path,
                duration_s: 30.0,
                held: Vec::new(),
            }
        })
        .collect();
    (dir, parts)
}

fn job(parts: Vec<LocalPart>) -> RecordingJob {
    RecordingJob {
        root: ROOT.into(),
        id: "r1".into(),
        name: NAME.into(),
        started_at: "2026-11-19T14:05:00Z".into(),
        instructions: "Write short notes.".into(),
        transcribe_endpoint: "system.ai.gemini-3-8-flash".into(),
        notes_endpoint: "system.ai.claude-sonnet-5-5".into(),
        mog: "m1".into(),
        parts,
    }
}

fn path_of(c: &mog_core::recording_cloud::http::SeenCall) -> String {
    c.params["path"].as_str().unwrap().to_owned()
}

fn uploads(http: &ScriptedHttp) -> Vec<(String, Vec<u8>)> {
    http.seen_on("files.upload")
        .into_iter()
        .map(|c| (path_of(&c), c.bytes.unwrap_or_default()))
        .collect()
}

fn last_upload_json(http: &ScriptedHttp, suffix: &str) -> Value {
    let (_, b) = uploads(http)
        .into_iter()
        .rev()
        .find(|(p, _)| p.ends_with(suffix))
        .unwrap_or_else(|| panic!("no upload of {suffix}"));
    serde_json::from_slice(&b).unwrap()
}

fn not_found() -> Value {
    json!({"error_code":"NOT_FOUND","message":"The file being accessed is not found."})
}

fn denied() -> Value {
    json!({"error_code":"PERMISSION_DENIED","message":"User does not have READ VOLUME privilege on VOLUME 'c.s.v'."})
}

// ---------- storage ----------

#[test]
fn rec_storage_my_files_probes_workspace_id_then_deployment_name() {
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "recording.me",
        200,
        json!({"userName":"first.last@example.com"}),
    );
    http.json(
        "files.list_directory",
        404,
        json!({"error_code":"NOT_FOUND","message":"did not resolve"}),
    );
    http.json("files.list_directory", 200, json!({"contents":[]}));
    let got = service(&http).probe_storage(P, &Storage::MyFiles).unwrap();
    assert_eq!(
        got,
        StorageProbe::Ready {
            root: "/Volumes/databricks/home/dbc-fake/first.last@example.com/my_files".into()
        }
    );
    let probes: Vec<String> = http
        .seen_on("files.list_directory")
        .iter()
        .map(path_of)
        .collect();
    assert_eq!(
        probes,
        vec![
            "/Volumes/databricks/home/1234567890/first.last@example.com/my_files/",
            "/Volumes/databricks/home/dbc-fake/first.last@example.com/my_files/",
        ]
    );
    // Read-only: no write route was sent.
    assert!(http
        .seen()
        .iter()
        .all(|c| c.route == "recording.me" || c.route == "files.list_directory"));
}

#[test]
fn rec_storage_my_files_unavailable_when_no_candidate_resolves() {
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "recording.me",
        200,
        json!({"userName":"first.last@example.com"}),
    );
    http.json("files.list_directory", 404, not_found());
    http.json("files.list_directory", 404, not_found());
    let e = service(&http)
        .probe_storage(P, &Storage::MyFiles)
        .unwrap_err();
    assert_eq!(e, RecordingError::new(ErrorKind::MyFilesUnavailable));
}

#[test]
fn rec_storage_existing_volume_maps_403_and_404() {
    let vol = Storage::Volume {
        path: "/Volumes/c/s/v".into(),
    };
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 403, denied());
    assert_eq!(
        service(&http).probe_storage(P, &vol).unwrap_err().kind,
        ErrorKind::VolumePermission
    );
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "files.list_directory",
        404,
        json!({"error_code":"NOT_FOUND","message":"Volume 'c.s.v' does not exist."}),
    );
    assert_eq!(
        service(&http).probe_storage(P, &vol).unwrap_err().kind,
        ErrorKind::VolumeNotFound
    );
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 200, json!({"contents":[]}));
    assert_eq!(
        service(&http).probe_storage(P, &vol).unwrap(),
        StorageProbe::Ready {
            root: "/Volumes/c/s/v".into()
        }
    );
    let bad = Storage::Volume {
        path: "/Volumes/c/s".into(),
    };
    assert_eq!(
        service(&http).probe_storage(P, &bad).unwrap_err().kind,
        ErrorKind::BadSettings
    );
}

#[test]
fn rec_storage_catalog_schema_asks_before_create_and_create_posts_managed_volume() {
    let cs = Storage::CatalogSchema {
        catalog: "main".into(),
        schema: "team".into(),
    };
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 404, json!({"error_code":"NOT_FOUND","message":"Volume 'main.team.bottle_audio' does not exist."}));
    assert_eq!(
        service(&http).probe_storage(P, &cs).unwrap(),
        StorageProbe::NeedsVolume {
            catalog: "main".into(),
            schema: "team".into(),
            path: "/Volumes/main/team/bottle_audio".into()
        }
    );
    assert!(
        http.seen_on("volumes.create").is_empty(),
        "the probe never creates"
    );

    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "volumes.create",
        200,
        json!({"full_name":"main.team.bottle_audio"}),
    );
    let root = service(&http).create_volume(P, "main", "team").unwrap();
    assert_eq!(root, "/Volumes/main/team/bottle_audio");
    assert_eq!(
        http.seen_on("volumes.create")[0].json,
        Some(
            json!({"catalog_name":"main","schema_name":"team","name":"bottle_audio","volume_type":"MANAGED"})
        )
    );
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "volumes.create",
        403,
        json!({"error_code":"PERMISSION_DENIED","message":"User does not have CREATE VOLUME"}),
    );
    assert_eq!(
        service(&http)
            .create_volume(P, "main", "team")
            .unwrap_err()
            .kind,
        ErrorKind::VolumePermission
    );
    let http = Arc::new(ScriptedHttp::new());
    http.json(
        "volumes.create",
        409,
        json!({"error_code":"RESOURCE_ALREADY_EXISTS","message":"exists"}),
    );
    assert_eq!(
        service(&http).create_volume(P, "main", "team").unwrap(),
        "/Volumes/main/team/bottle_audio"
    );
    assert_eq!(
        service(&http)
            .create_volume(P, "main", "te;am")
            .unwrap_err()
            .kind,
        ErrorKind::BadSettings
    );
}

// ---------- pipeline ----------

const MODEL: &str = "mlflow.chat_completions";

fn files_ok(http: &ScriptedHttp) {
    for _ in 0..20 {
        http.empty("files.upload", 204);
    }
    for _ in 0..4 {
        http.empty("files.create_directory", 200);
    }
}

fn gemini(text: &str) -> Value {
    json!({"model":"system.ai.gemini-3-8-flash","choices":[{"message":{"role":"assistant",
        "content":[{"type":"text","text":text,"thoughtSignature":"AY89a1-fake"}]},"index":0,"finish_reason":"stop"}]})
}

fn claude(text: &str) -> Value {
    json!({"model":"system.ai.claude-sonnet-5-5","choices":[{"message":{"role":"assistant","content":text},"index":0}]})
}

const LINES: &str = "[00:00] Speaker 1: Hello team.\n[00:05] Speaker 2: Hi.";

#[test]
fn rec_pipeline_uploads_deletes_temp_transcribes_and_writes_notes() {
    let (dir, parts) = temp_parts("one", 1);
    let http = Arc::new(ScriptedHttp::new());
    files_ok(&http);
    http.json(MODEL, 200, gemini(LINES));
    http.json(MODEL, 200, claude("## Summary\nShort call."));
    let mut seen: Vec<Status> = Vec::new();
    let out = service(&http).run(P, &job(parts.clone()), &mut |j: &JobManifest| {
        seen.push(j.status)
    });
    assert_eq!(out.status, Status::Done, "{out:?}");
    for s in [
        Status::Uploading,
        Status::Transcribing,
        Status::WritingNotes,
        Status::Done,
    ] {
        assert!(seen.contains(&s), "{s:?} in {seen:?}");
    }
    let up = uploads(&http);
    let part = up.iter().find(|(p, _)| p.ends_with(".opus")).unwrap();
    assert_eq!(part.0, format!("{FOLDER}/2026-11-19_0905.opus"));
    assert_eq!(part.1, b"OggS-fake-1");
    let instr = up
        .iter()
        .find(|(p, _)| p.ends_with(".instructions.md"))
        .unwrap();
    assert_eq!(instr.1, b"Write short notes.");
    for c in http.seen_on("files.upload") {
        let p = path_of(&c);
        let ow = c
            .params
            .get("overwrite")
            .and_then(Value::as_str)
            .unwrap_or("");
        assert_eq!(
            ow,
            if p.ends_with(".opus") {
                "false"
            } else {
                "true"
            },
            "{p}"
        );
    }
    // Temp audio is gone only after the upload succeeded.
    assert!(!parts[0].path.exists());

    let calls = http.seen_on(MODEL);
    assert_eq!(calls.len(), 2);
    let t = calls[0].json.as_ref().unwrap();
    assert_eq!(t["model"], "system.ai.gemini-3-8-flash");
    let url = t["messages"][0]["content"][1]["audio_url"]["url"]
        .as_str()
        .unwrap();
    assert!(url.starts_with("data:audio/ogg;base64,"), "{url}");
    assert_eq!(calls[0].tags.as_ref().unwrap().op, TagOp::Asr);
    assert_eq!(calls[0].tags.as_ref().unwrap().mog, "m1");
    let n = calls[1].json.as_ref().unwrap();
    assert_eq!(n["model"], "system.ai.claude-sonnet-5-5");
    let prompt = n["messages"][0]["content"].as_str().unwrap();
    assert!(
        prompt.starts_with("Write short notes.\n\nTranscript:\n"),
        "{prompt}"
    );
    assert!(prompt.contains("**Speaker 2** [00:00:05]\nHi."), "{prompt}");
    assert_eq!(calls[1].tags.as_ref().unwrap().op, TagOp::Writer);

    let tj = last_upload_json(&http, ".transcript.json");
    assert_eq!(tj["instructions"], "Write short notes.");
    assert_eq!(tj["segments"][1]["speaker"], "2");
    assert_eq!(tj["segments"][1]["start"], 5.0);
    assert_eq!(tj["segments"][1]["end"], 30.0);
    let md = up
        .iter()
        .find(|(p, _)| p.ends_with(".transcript.md"))
        .unwrap();
    assert!(String::from_utf8_lossy(&md.1).contains("**Speaker 1** [00:00:00]\nHello team."));
    let notes = up.iter().find(|(p, _)| p.ends_with(".notes.md")).unwrap();
    assert_eq!(notes.1, b"## Summary\nShort call.");
    let manifest = last_upload_json(&http, ".job.json");
    assert_eq!(manifest["status"], "done");
    // No compute route is ever sent (UPDATE 4).
    assert!(http
        .seen()
        .iter()
        .all(|c| !c.route.starts_with("warehouses")
            && !c.route.starts_with("statements")
            && !c.route.starts_with("jobs")));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_two_parts_offset_times_and_pass_context() {
    let (dir, parts) = temp_parts("two", 2);
    let http = Arc::new(ScriptedHttp::new());
    files_ok(&http);
    http.json(MODEL, 200, gemini(LINES));
    http.json(MODEL, 200, gemini("[00:02] Speaker 2: Back again."));
    http.json(MODEL, 200, claude("notes"));
    let out = service(&http).run(P, &job(parts), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    let calls = http.seen_on(MODEL);
    let p2 = calls[1].json.as_ref().unwrap()["messages"][0]["content"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(p2.contains("[00:05] Speaker 2: Hi."), "{p2}");
    let tj = last_upload_json(&http, ".transcript.json");
    assert_eq!(tj["parts"][1]["offset_s"], 30.0);
    assert_eq!(tj["segments"][2]["start"], 32.0);
    assert_eq!(tj["segments"][2]["speaker"], "2");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_upload_403_keeps_temp_audio_and_fails_at_uploading() {
    let (dir, parts) = temp_parts("deny", 1);
    let http = Arc::new(ScriptedHttp::new());
    http.empty("files.create_directory", 200);
    for _ in 0..3 {
        http.json("files.upload", 403, denied());
    }
    let out = service(&http).run(P, &job(parts.clone()), &mut |_| {});
    assert_eq!(out.status, Status::Failed);
    assert_eq!(out.error_kind, Some(ErrorKind::VolumePermission));
    assert_eq!(out.error_stage, Some(Stage::Uploading));
    assert!(parts[0].path.exists(), "temp audio must stay for Retry");
    assert!(http.seen_on(MODEL).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_refuses_non_audio_before_any_upload_of_it() {
    let (dir, parts) = temp_parts("text", 1);
    std::fs::write(&parts[0].path, b"hello, this is text").unwrap();
    let http = Arc::new(ScriptedHttp::new());
    files_ok(&http);
    let out = service(&http).run(P, &job(parts.clone()), &mut |_| {});
    assert_eq!(out.error_kind, Some(ErrorKind::AudioUnreadable));
    assert!(uploads(&http).iter().all(|(p, _)| !p.ends_with(".opus")));
    assert!(http.seen_on(MODEL).is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_existing_part_409_counts_as_uploaded() {
    let (dir, parts) = temp_parts("409", 1);
    let http = Arc::new(ScriptedHttp::new());
    for _ in 0..4 {
        http.empty("files.create_directory", 200);
    }
    http.empty("files.upload", 204); // instructions
    http.empty("files.upload", 204); // job.json
    http.json(
        "files.upload",
        409,
        json!({"error_code":"ALREADY_EXISTS","message":"exists"}),
    );
    for _ in 0..10 {
        http.empty("files.upload", 204);
    }
    http.on(
        MODEL,
        Err(CoreError::Unavailable {
            component: "R7",
            reason: "offline",
        }),
    );
    let out = service(&http).run(P, &job(parts.clone()), &mut |_| {});
    assert!(!parts[0].path.exists());
    assert!(out.parts[0].uploaded);
    assert_eq!(out.error_kind, Some(ErrorKind::Offline));
    assert_eq!(out.error_stage, Some(Stage::Transcribing));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_missing_models_and_bad_audio_map_plainly() {
    let cases = [
        (
            404,
            json!({"error_code":"NOT_FOUND","message":"'x' does not exist."}),
            false,
            ErrorKind::TranscribeEndpointMissing,
            Stage::Transcribing,
        ),
        (
            400,
            json!({"error_code":"BAD_REQUEST","message":"The provided file is malformed or corrupted and could not be decoded."}),
            false,
            ErrorKind::AudioUnreadable,
            Stage::Transcribing,
        ),
        (
            404,
            json!({"error_code":"NOT_FOUND","message":"'x' does not exist."}),
            true,
            ErrorKind::NotesEndpointMissing,
            Stage::WritingNotes,
        ),
    ];
    for (i, (status, body, notes, kind, stage)) in cases.into_iter().enumerate() {
        let (dir, parts) = temp_parts(&format!("model{i}"), 1);
        let http = Arc::new(ScriptedHttp::new());
        files_ok(&http);
        if notes {
            http.json(MODEL, 200, gemini(LINES));
        }
        http.json(MODEL, status, body);
        let out = service(&http).run(P, &job(parts), &mut |_| {});
        assert_eq!(
            (out.error_kind, out.error_stage),
            (Some(kind), Some(stage)),
            "case {i}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn rec_pipeline_retries_a_429_with_backoff() {
    let (dir, parts) = temp_parts("429", 1);
    let http = Arc::new(ScriptedHttp::new());
    files_ok(&http);
    let busy = || {
        Err(CoreError::Unavailable {
            component: "R7",
            reason: "rate_limited",
        })
    };
    http.on(MODEL, busy());
    http.on(MODEL, busy());
    http.json(MODEL, 200, gemini(LINES));
    http.json(MODEL, 200, claude("n"));
    let pause = Arc::new(SimPause::default());
    let out = service_with(&http, pause.clone()).run(P, &job(parts), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    assert_eq!(
        *pause.pauses.lock().unwrap(),
        vec![Duration::from_secs(5), Duration::from_secs(10)]
    );

    let (dir2, parts) = temp_parts("429b", 1);
    let http = Arc::new(ScriptedHttp::new());
    files_ok(&http);
    for _ in 0..4 {
        http.on(MODEL, busy());
    }
    let out = service(&http).run(P, &job(parts), &mut |_| {});
    assert_eq!(out.error_kind, Some(ErrorKind::RateLimited));
    assert_eq!(http.seen_on(MODEL).len(), 4);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(dir2);
}

// ---------- list, detail, retry, link ----------

fn manifest(name: &str, started: &str, status: &str) -> Value {
    json!({"version":1,"id":name,"name":name,"started_at":started,"duration_s":30.0,
        "parts":[{"file":format!("{name}.opus"),"duration_s":30.0,"uploaded":true}],
        "status":status,"transcribe_endpoint":"system.ai.gemini-3-8-flash",
        "notes_endpoint":"system.ai.claude-sonnet-5-5","updated_at":started})
}

#[test]
fn rec_list_reads_each_job_json_newest_first() {
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 200, json!({"contents":[
        {"path":"/Volumes/main/team/audio/Bottle/Recordings/2026-11-18_0900","is_directory":true,"name":"2026-11-18_0900"},
        {"path":"/Volumes/main/team/audio/Bottle/Recordings/stray.txt","is_directory":false,"name":"stray.txt"},
        {"path":"/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905","is_directory":true,"name":"2026-11-19_0905"}]}));
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest("2026-11-18_0900", "2026-11-18T14:00:00Z", "done")).unwrap(),
    );
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest(
            "2026-11-19_0905",
            "2026-11-19T14:05:00Z",
            "failed",
        ))
        .unwrap(),
    );
    let list = service(&http).list(P, ROOT).unwrap();
    let names: Vec<&str> = list.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, vec!["2026-11-19_0905", "2026-11-18_0900"]);
    let reads: Vec<String> = http.seen_on("files.download").iter().map(path_of).collect();
    assert_eq!(
        reads[0],
        "/Volumes/main/team/audio/Bottle/Recordings/2026-11-18_0900/2026-11-18_0900.job.json"
    );

    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 404, not_found());
    assert!(service(&http).list(P, ROOT).unwrap().is_empty());
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 403, denied());
    assert_eq!(
        service(&http).list(P, ROOT).unwrap_err().kind,
        ErrorKind::VolumePermission
    );
}

#[test]
fn rec_detail_reads_transcript_and_notes() {
    let http = Arc::new(ScriptedHttp::new());
    http.bytes(
        "files.download",
        &serde_json::to_vec(&manifest(NAME, "2026-11-19T14:05:00Z", "done")).unwrap(),
    );
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"x","parts":[{"file":"a","offset_s":0.0,"duration_s":9.0}],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    for _ in 0..3 {
        http.json("files.download", 404, not_found());
    }
    let d = service(&http).detail(P, ROOT, NAME).unwrap();
    assert_eq!(d.job.name, NAME);
    assert_eq!(d.transcript.unwrap().segments[0].text, "Hi.");
    assert_eq!(d.notes, None);
    assert_eq!(
        d.link,
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath=%2FVolumes%2Fmain%2Fteam%2Faudio%2FBottle%2FRecordings%2F2026-11-19_0905%2F"
    );
    assert_eq!(
        service(&http).detail(P, ROOT, "../x").unwrap_err().kind,
        ErrorKind::BadSettings
    );
}

#[test]
fn rec_retry_from_writing_notes_runs_only_the_notes() {
    let http = Arc::new(ScriptedHttp::new());
    let mut failed = manifest(NAME, "2026-11-19T14:05:00Z", "failed");
    failed["error_kind"] = json!("notes_endpoint_missing");
    failed["error_stage"] = json!("writing_notes");
    http.bytes("files.download", &serde_json::to_vec(&failed).unwrap());
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"Write short notes.","parts":[],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    http.json(MODEL, 200, claude("## Summary\nOK"));
    files_ok(&http);
    let mut j = job(vec![]);
    j.notes_endpoint = "my-fixed-model".into();
    let out = service(&http).retry(P, &j, &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    let calls = http.seen_on(MODEL);
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].json.as_ref().unwrap()["model"], "my-fixed-model");
    assert!(uploads(&http)
        .iter()
        .any(|(p, b)| p.ends_with(".notes.md") && b == b"## Summary\nOK"));
}

#[test]
fn rec_retry_transcribe_reads_the_part_back_from_the_volume() {
    let http = Arc::new(ScriptedHttp::new());
    let mut failed = manifest(NAME, "2026-11-19T14:05:00Z", "failed");
    failed["error_kind"] = json!("offline");
    failed["error_stage"] = json!("transcribing");
    http.bytes("files.download", &serde_json::to_vec(&failed).unwrap());
    http.bytes("files.download", b"OggS-from-volume");
    http.json(MODEL, 200, gemini(LINES));
    http.json(MODEL, 200, claude("n"));
    files_ok(&http);
    let out = service(&http).retry(P, &job(vec![]), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    let reads: Vec<String> = http.seen_on("files.download").iter().map(path_of).collect();
    assert_eq!(reads[1], format!("{FOLDER}/2026-11-19_0905.opus"));
    assert!(http
        .seen_on("files.upload")
        .iter()
        .all(|c| !path_of(c).ends_with(".opus")));
}

#[test]
fn rec_retry_upload_with_temp_audio_gone_is_audio_missing() {
    let http = Arc::new(ScriptedHttp::new());
    let mut failed = manifest(NAME, "2026-11-19T14:05:00Z", "failed");
    failed["error_stage"] = json!("uploading");
    failed["parts"] = json!([{"file":"2026-11-19_0905.opus","duration_s":30.0,"uploaded":false}]);
    http.bytes("files.download", &serde_json::to_vec(&failed).unwrap());
    files_ok(&http);
    let gone = LocalPart {
        path: PathBuf::from("/nonexistent/bottle/x.opus"),
        duration_s: 30.0,
        held: Vec::new(),
    };
    let out = service(&http).retry(P, &job(vec![gone]), &mut |_| {});
    assert_eq!(out.error_kind, Some(ErrorKind::AudioMissing));
    assert_eq!(out.error_stage, Some(Stage::Uploading));
}

#[test]
fn rec_link_uses_profile_host_and_workspace_id() {
    let http = Arc::new(ScriptedHttp::new());
    let url = service(&http).link(P, ROOT, NAME).unwrap();
    assert!(url.starts_with("https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath="));
    assert!(http.seen().is_empty(), "the link needs no call");
}

// ---------- ADR 0118: the talk key during a recording ----------

const MARKER: &str = "You spoke to Bottle";

#[test]
fn adr_0118_the_marker_is_in_the_user_files_but_never_in_the_notes_prompt() {
    let (dir, mut parts) = temp_parts("held", 1);
    parts[0].held = vec![mog_core::recording_capture::session::HeldRange {
        start_samples: 16_000,
        len_samples: 32_000,
    }];
    let http = Arc::new(ScriptedHttp::new());
    files_ok(&http);
    http.json(MODEL, 200, gemini(LINES));
    http.json(MODEL, 200, claude("## Summary\nShort call."));
    let out = service(&http).run(P, &job(parts), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    // The job manifest carries the ranges of its part.
    let jj = last_upload_json(&http, ".job.json");
    assert_eq!(jj["parts"][0]["heldRanges"][0]["startSamples"], 16_000);
    // The user's transcript has the marker and the gap.
    let tj = last_upload_json(&http, ".transcript.json");
    assert_eq!(tj["bottleGaps"][0]["startS"], 1.0);
    assert_eq!(tj["bottleGaps"][0]["durS"], 2.0);
    let md = uploads(&http)
        .into_iter()
        .rev()
        .find(|(p, _)| p.ends_with(".transcript.md"))
        .unwrap();
    assert!(String::from_utf8(md.1).unwrap().contains(MARKER));
    // The models never see it: not the transcribe call, not the notes call.
    for c in http.seen_on(MODEL) {
        assert!(!c.json.as_ref().unwrap().to_string().contains(MARKER));
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn adr_0118_a_retry_from_writing_notes_keeps_the_marker_out_of_the_prompt() {
    let http = Arc::new(ScriptedHttp::new());
    let mut failed = manifest(NAME, "2026-11-19T14:05:00Z", "failed");
    failed["error_stage"] = json!("writing_notes");
    http.bytes("files.download", &serde_json::to_vec(&failed).unwrap());
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"Write short notes.","parts":[],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1},
                    {"start":6.0,"end":7.0,"speaker":"2","text":"Bye.","part":1}],
        "bottleGaps":[{"startS":3.0,"durS":2.0}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    http.json(MODEL, 200, claude("## Summary\nOK"));
    files_ok(&http);
    let out = service(&http).retry(P, &job(vec![]), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    let calls = http.seen_on(MODEL);
    assert_eq!(calls.len(), 1);
    let body = calls[0].json.as_ref().unwrap().to_string();
    assert!(body.contains("Bye."));
    assert!(!body.contains(MARKER), "{body}");
}
