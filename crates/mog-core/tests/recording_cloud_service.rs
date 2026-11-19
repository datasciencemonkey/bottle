//! The recording cloud service (ADR 0103, PR C) over a scripted HTTP double: storage probe,
//! volume create, warehouses, upload with temp delete, job run, warehouse fallback, list,
//! detail, retry and the Catalog Explorer link. Clocks are simulated.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::clock::{SimClock, SimWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::Pause;
use mog_core::recording_cloud::cloud::*;
use mog_core::recording_cloud::errors::{ErrorKind, RecordingError, Stage};
use mog_core::recording_cloud::http::ScriptedHttp;
use mog_core::recording_cloud::job::{Engine, JobManifest, PartEntry, Status};
use mog_core::recording_cloud::settings_store::Storage;
use serde_json::{json, Value};

const P: &str = "fake-home";
const ROOT: &str = "/Volumes/main/team/audio";
const NAME: &str = "2026-11-19_0905";
const FOLDER: &str = "/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905";
const WH: &str = "e6084ca031cda30a";

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
    let pause = Arc::new(SimPause::default());
    RecordingCloudService::new(CloudDeps {
        http: http.clone(),
        workspace: Arc::new(Ws),
        wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
        mono: Arc::new(pause.clock.clone()),
        pause,
    })
}

fn temp_parts(tag: &str, n: usize) -> (PathBuf, Vec<LocalPart>) {
    let dir = std::env::temp_dir().join(format!("bottle-rec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let parts = (1..=n)
        .map(|i| {
            let path = dir.join(format!("{NAME}.part{i:02}.flac"));
            std::fs::write(&path, format!("fLaC-fake-{i}")).unwrap();
            LocalPart {
                path,
                duration_s: 30.0,
            }
        })
        .collect();
    (dir, parts)
}

fn job(parts: Vec<LocalPart>, warehouse: Option<&str>) -> RecordingJob {
    RecordingJob {
        root: ROOT.into(),
        id: "r1".into(),
        name: NAME.into(),
        started_at: "2026-11-19T14:05:00Z".into(),
        instructions: "Write short notes.".into(),
        endpoint: "databricks-claude-sonnet-5-5".into(),
        warehouse_id: warehouse.map(str::to_owned),
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
    http.json("recording.me", 200, json!({"userName":"first.last@example.com"}));
    http.json("files.list_directory", 404, json!({"error_code":"NOT_FOUND","message":"did not resolve"}));
    http.json("files.list_directory", 200, json!({"contents":[]}));
    let got = service(&http).probe_storage(P, &Storage::MyFiles).unwrap();
    assert_eq!(
        got,
        StorageProbe::Ready {
            root: "/Volumes/databricks/home/dbc-fake/first.last@example.com/my_files".into()
        }
    );
    let probes: Vec<String> = http.seen_on("files.list_directory").iter().map(path_of).collect();
    assert_eq!(
        probes,
        vec![
            "/Volumes/databricks/home/1234567890/first.last@example.com/my_files/",
            "/Volumes/databricks/home/dbc-fake/first.last@example.com/my_files/",
        ]
    );
    // Read-only: no write route was sent.
    assert!(http.seen().iter().all(|c| c.route == "recording.me" || c.route == "files.list_directory"));
}

#[test]
fn rec_storage_my_files_unavailable_when_no_candidate_resolves() {
    let http = Arc::new(ScriptedHttp::new());
    http.json("recording.me", 200, json!({"userName":"first.last@example.com"}));
    http.json("files.list_directory", 404, not_found());
    http.json("files.list_directory", 404, not_found());
    let e = service(&http).probe_storage(P, &Storage::MyFiles).unwrap_err();
    assert_eq!(e, RecordingError::new(ErrorKind::MyFilesUnavailable));
}

#[test]
fn rec_storage_existing_volume_maps_403_and_404() {
    let vol = Storage::Volume { path: "/Volumes/c/s/v".into() };
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 403, denied());
    assert_eq!(service(&http).probe_storage(P, &vol).unwrap_err().kind, ErrorKind::VolumePermission);
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 404, json!({"error_code":"NOT_FOUND","message":"Volume 'c.s.v' does not exist."}));
    assert_eq!(service(&http).probe_storage(P, &vol).unwrap_err().kind, ErrorKind::VolumeNotFound);
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 200, json!({"contents":[]}));
    assert_eq!(
        service(&http).probe_storage(P, &vol).unwrap(),
        StorageProbe::Ready { root: "/Volumes/c/s/v".into() }
    );
    let bad = Storage::Volume { path: "/Volumes/c/s".into() };
    assert_eq!(service(&http).probe_storage(P, &bad).unwrap_err().kind, ErrorKind::BadSettings);
}

#[test]
fn rec_storage_catalog_schema_asks_before_create_and_create_posts_managed_volume() {
    let cs = Storage::CatalogSchema { catalog: "main".into(), schema: "team".into() };
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
    assert!(http.seen_on("volumes.create").is_empty(), "the probe never creates");

    let http = Arc::new(ScriptedHttp::new());
    http.json("volumes.create", 200, json!({"full_name":"main.team.bottle_audio"}));
    let root = service(&http).create_volume(P, "main", "team").unwrap();
    assert_eq!(root, "/Volumes/main/team/bottle_audio");
    assert_eq!(
        http.seen_on("volumes.create")[0].json,
        Some(json!({"catalog_name":"main","schema_name":"team","name":"bottle_audio","volume_type":"MANAGED"}))
    );
    let http = Arc::new(ScriptedHttp::new());
    http.json("volumes.create", 403, json!({"error_code":"PERMISSION_DENIED","message":"User does not have CREATE VOLUME"}));
    assert_eq!(service(&http).create_volume(P, "main", "team").unwrap_err().kind, ErrorKind::VolumePermission);
    let http = Arc::new(ScriptedHttp::new());
    http.json("volumes.create", 409, json!({"error_code":"RESOURCE_ALREADY_EXISTS","message":"exists"}));
    assert_eq!(service(&http).create_volume(P, "main", "team").unwrap(), "/Volumes/main/team/bottle_audio");
    assert_eq!(service(&http).create_volume(P, "main", "te;am").unwrap_err().kind, ErrorKind::BadSettings);
}

// ---------- warehouses ----------

#[test]
fn rec_warehouses_list_and_start_a_stopped_one() {
    let http = Arc::new(ScriptedHttp::new());
    http.json("warehouses.list", 200, json!({"warehouses":[
        {"id":WH,"name":"my-serverless-wh","state":"RUNNING","enable_serverless_compute":true},
        {"id":"66fba25b212c4d4c","name":"Serverless Starter Warehouse","state":"STOPPED","enable_serverless_compute":true}]}));
    let list = service(&http).warehouses(P).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], Warehouse { id: WH.into(), name: "my-serverless-wh".into(), state: "RUNNING".into(), serverless: true });

    let http = Arc::new(ScriptedHttp::new());
    http.json("warehouses.get", 200, json!({"id":WH,"state":"STOPPED"}));
    http.json("warehouses.start", 200, json!({}));
    http.json("warehouses.get", 200, json!({"id":WH,"state":"STARTING"}));
    http.json("warehouses.get", 200, json!({"id":WH,"state":"RUNNING"}));
    service(&http).ensure_warehouse(P, WH).unwrap();
    assert_eq!(http.seen_on("warehouses.start").len(), 1);

    let http = Arc::new(ScriptedHttp::new());
    http.json("warehouses.get", 404, json!({"error_code":"RESOURCE_DOES_NOT_EXIST","message":"SQL warehouse 0123456789abcdef does not exist."}));
    assert_eq!(service(&http).ensure_warehouse(P, WH).unwrap_err().kind, ErrorKind::NoWarehouse);
}

#[test]
fn rec_warehouses_start_wait_is_bounded() {
    let http = Arc::new(ScriptedHttp::new());
    http.json("warehouses.get", 200, json!({"id":WH,"state":"STOPPED"}));
    http.json("warehouses.start", 200, json!({}));
    for _ in 0..500 {
        http.json("warehouses.get", 200, json!({"id":WH,"state":"STARTING"}));
    }
    let e = service(&http).ensure_warehouse(P, WH).unwrap_err();
    assert_eq!(e.kind, ErrorKind::NoWarehouse);
    assert!(http.seen_on("warehouses.get").len() < 500);
}

// ---------- pipeline: job engine ----------

fn script_upload_ok(http: &ScriptedHttp, n_parts: usize) {
    // instructions.md, job.json, parts..., then job.json after parts, the script, job.json.
    for _ in 0..(n_parts * 2 + 8) {
        http.empty("files.upload", 204);
    }
    for _ in 0..4 {
        http.empty("files.create_directory", 200);
    }
}

#[test]
fn rec_pipeline_job_engine_uploads_deletes_temp_submits_and_reads_the_result() {
    let (dir, parts) = temp_parts("job", 2);
    let http = Arc::new(ScriptedHttp::new());
    script_upload_ok(&http, 2);
    http.json("jobs.runs_submit", 200, json!({"run_id": 42}));
    http.json("jobs.runs_get", 200, json!({"state":{"life_cycle_state":"RUNNING"}}));
    http.json("jobs.runs_get", 200, json!({"state":{"life_cycle_state":"TERMINATED","result_state":"SUCCESS"}}));
    let done = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":60.0,
        "parts":[{"file":"2026-11-19_0905.part01.flac","duration_s":30.0,"uploaded":true},
                 {"file":"2026-11-19_0905.part02.flac","duration_s":30.0,"uploaded":true}],
        "status":"done","endpoint":"databricks-claude-sonnet-5-5","engine":"job","run_id":42,
        "updated_at":"2026-11-19T15:30:00Z"});
    http.bytes("files.download", &serde_json::to_vec(&done).unwrap());

    let mut seen: Vec<Status> = Vec::new();
    let out = service(&http).run(P, &job(parts.clone(), None), &mut |j: &JobManifest| seen.push(j.status));
    assert_eq!(out.status, Status::Done, "{out:?}");
    assert_eq!(out.engine, Some(Engine::Job));
    assert_eq!(out.run_id, Some(42));
    assert!(seen.contains(&Status::Uploading) && seen.contains(&Status::Transcribing));

    let up = uploads(&http);
    let names: Vec<&str> = up.iter().map(|(p, _)| p.as_str()).collect();
    assert!(names.contains(&"/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905/2026-11-19_0905.instructions.md"));
    assert!(names.contains(&"/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905/2026-11-19_0905.part01.flac"));
    assert!(names.contains(&"/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905/2026-11-19_0905.part02.flac"));
    assert!(names.contains(&"/Volumes/main/team/audio/Bottle/.bottle/transcribe_v1.py"));
    let part1 = up.iter().find(|(p, _)| p.ends_with("part01.flac")).unwrap();
    assert_eq!(part1.1, b"fLaC-fake-1");
    // Parts never overwrite; the sidecars do.
    for c in http.seen_on("files.upload") {
        let p = path_of(&c);
        let ow = c.params.get("overwrite").and_then(Value::as_str).unwrap_or("");
        if p.ends_with(".flac") || p.ends_with(".py") {
            assert_eq!(ow, "false", "{p}");
        } else {
            assert_eq!(ow, "true", "{p}");
        }
    }
    let instr = up.iter().find(|(p, _)| p.ends_with(".instructions.md")).unwrap();
    assert_eq!(instr.1, b"Write short notes.");
    // Temp audio is gone only after upload succeeded.
    for p in &parts {
        assert!(!p.path.exists(), "{:?}", p.path);
    }
    let submit = http.seen_on("jobs.runs_submit")[0].json.clone().unwrap();
    assert_eq!(submit["tasks"][0]["spark_python_task"]["parameters"][1], FOLDER);
    let manifest = last_upload_json(&http, ".job.json");
    assert_eq!(manifest["run_id"], 42);
    assert_eq!(manifest["engine"], "job");
    assert_eq!(manifest["status"], "transcribing");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_upload_403_keeps_temp_audio_and_fails_at_uploading() {
    let (dir, parts) = temp_parts("deny", 1);
    let http = Arc::new(ScriptedHttp::new());
    http.empty("files.create_directory", 200);
    http.json("files.upload", 403, denied());
    http.json("files.upload", 403, denied());
    http.json("files.upload", 403, denied());
    let out = service(&http).run(P, &job(parts.clone(), None), &mut |_| {});
    assert_eq!(out.status, Status::Failed);
    assert_eq!(out.error_kind, Some(ErrorKind::VolumePermission));
    assert_eq!(out.error_stage, Some(Stage::Uploading));
    assert!(parts[0].path.exists(), "temp audio must stay for Retry");
    assert!(http.seen_on("jobs.runs_submit").is_empty());
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
    http.json("files.upload", 409, json!({"error_code":"ALREADY_EXISTS","message":"exists"}));
    for _ in 0..6 {
        http.empty("files.upload", 204);
    }
    http.on("jobs.runs_submit", Err(CoreError::Unavailable { component: "R7", reason: "offline" }));
    let out = service(&http).run(P, &job(parts.clone(), None), &mut |_| {});
    assert!(!parts[0].path.exists());
    assert!(out.parts[0].uploaded);
    assert_eq!(out.error_kind, Some(ErrorKind::Offline));
    assert_eq!(out.error_stage, Some(Stage::Transcribing));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_no_serverless_and_no_warehouse_fails_plainly() {
    let (dir, parts) = temp_parts("nosls", 1);
    let http = Arc::new(ScriptedHttp::new());
    script_upload_ok(&http, 1);
    http.json("jobs.runs_submit", 400, json!({"error_code":"INVALID_PARAMETER_VALUE","message":"Serverless compute is not enabled for this workspace."}));
    let out = service(&http).run(P, &job(parts, None), &mut |_| {});
    assert_eq!(out.error_kind, Some(ErrorKind::ServerlessUnavailable));
    assert_eq!(out.error_stage, Some(Stage::Transcribing));
    let _ = std::fs::remove_dir_all(dir);
}

// ---------- pipeline: warehouse fallback ----------

const CELL: &str = r#"{"error_message":null,"metadata":{},"response":{"duration_seconds":29.5,"segments":[{"end":4.64,"speaker_id":"1","start":0,"text":"Hello team."},{"end":9.0,"speaker_id":"2","start":5.0,"text":"Hi."}]}}"#;

fn ok_cell(id: &str, cell: &str) -> Value {
    json!({"statement_id":id,"status":{"state":"SUCCEEDED"},"result":{"row_count":1,"data_array":[[cell]]}})
}

#[test]
fn rec_pipeline_falls_back_to_the_warehouse_and_writes_transcript_and_notes() {
    let (dir, parts) = temp_parts("wh", 2);
    let http = Arc::new(ScriptedHttp::new());
    script_upload_ok(&http, 2);
    http.json("jobs.runs_submit", 400, json!({"error_code":"INVALID_PARAMETER_VALUE","message":"Serverless compute is not enabled for this workspace."}));
    http.json("warehouses.get", 200, json!({"id":WH,"state":"RUNNING"}));
    http.json("statements.execute", 200, json!({"statement_id":"s1","status":{"state":"RUNNING"}}));
    http.json("statements.get", 200, ok_cell("s1", CELL));
    http.json("statements.execute", 200, ok_cell("s2", CELL));
    http.json("statements.execute", 200, ok_cell("s3", "## Summary\nShort call."));
    let out = service(&http).run(P, &job(parts, Some(WH)), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    assert_eq!(out.engine, Some(Engine::Warehouse));

    let ex = http.seen_on("statements.execute");
    assert_eq!(ex.len(), 3);
    let p0 = ex[0].json.as_ref().unwrap()["parameters"][0]["value"].as_str().unwrap().to_owned();
    assert_eq!(p0, format!("{FOLDER}/2026-11-19_0905.part01.flac"));
    let notes_params = &ex[2].json.as_ref().unwrap()["parameters"];
    let text: Vec<&str> = notes_params.as_array().unwrap().iter().map(|p| p["value"].as_str().unwrap()).collect();
    assert!(text.contains(&"Write short notes."));
    assert!(text.iter().any(|t| t.contains("**Speaker 2.1** [00:00:30]")), "{text:?}");

    let tj = last_upload_json(&http, ".transcript.json");
    assert_eq!(tj["instructions"], "Write short notes.");
    assert_eq!(tj["parts"][1]["offset_s"], 30.0);
    assert_eq!(tj["segments"][2]["speaker"], "2.1");
    assert_eq!(tj["segments"][2]["start"], 30.0);
    let up = uploads(&http);
    let md = up.iter().find(|(p, _)| p.ends_with(".transcript.md")).unwrap();
    assert!(String::from_utf8_lossy(&md.1).contains("**Speaker 1** [00:00:00]"));
    let notes = up.iter().find(|(p, _)| p.ends_with(".notes.md")).unwrap();
    assert_eq!(notes.1, b"## Summary\nShort call.");
    let manifest = last_upload_json(&http, ".job.json");
    assert_eq!(manifest["status"], "done");
    assert_eq!(manifest["engine"], "warehouse");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_pipeline_warehouse_ai_transcribe_disabled_is_plain() {
    let (dir, parts) = temp_parts("off", 1);
    let http = Arc::new(ScriptedHttp::new());
    script_upload_ok(&http, 1);
    http.json("jobs.runs_submit", 400, json!({"error_code":"FEATURE_DISABLED","message":"Serverless jobs are disabled."}));
    http.json("warehouses.get", 200, json!({"id":WH,"state":"RUNNING"}));
    http.json("statements.execute", 200, json!({"statement_id":"s1","status":{"state":"FAILED","error":{"error_code":"BAD_REQUEST",
        "message":"[UNRESOLVED_ROUTINE] Cannot resolve routine `ai_transcribe` on search path [`system`.`builtin`]. SQLSTATE: 42883"}}}));
    let out = service(&http).run(P, &job(parts, Some(WH)), &mut |_| {});
    assert_eq!(out.error_kind, Some(ErrorKind::AiTranscribeDisabled));
    assert_eq!(out.error_stage, Some(Stage::Transcribing));
    let _ = std::fs::remove_dir_all(dir);
}

// ---------- list, detail, retry, link ----------

fn manifest(name: &str, started: &str, status: &str) -> Value {
    json!({"version":1,"id":name,"name":name,"started_at":started,"duration_s":1.0,"parts":[],
        "status":status,"endpoint":"e","updated_at":started})
}

#[test]
fn rec_list_reads_each_job_json_newest_first() {
    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 200, json!({"contents":[
        {"path":"/Volumes/main/team/audio/Bottle/Recordings/2026-11-18_0900","is_directory":true,"name":"2026-11-18_0900"},
        {"path":"/Volumes/main/team/audio/Bottle/Recordings/stray.txt","is_directory":false,"name":"stray.txt"},
        {"path":"/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905","is_directory":true,"name":"2026-11-19_0905"}]}));
    http.bytes("files.download", &serde_json::to_vec(&manifest("2026-11-18_0900", "2026-11-18T14:00:00Z", "done")).unwrap());
    http.bytes("files.download", &serde_json::to_vec(&manifest("2026-11-19_0905", "2026-11-19T14:05:00Z", "failed")).unwrap());
    let list = service(&http).list(P, ROOT).unwrap();
    let names: Vec<&str> = list.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names, vec!["2026-11-19_0905", "2026-11-18_0900"]);
    let reads: Vec<String> = http.seen_on("files.download").iter().map(path_of).collect();
    assert_eq!(reads[0], "/Volumes/main/team/audio/Bottle/Recordings/2026-11-18_0900/2026-11-18_0900.job.json");

    let http = Arc::new(ScriptedHttp::new());
    http.json("files.list_directory", 404, not_found());
    assert!(service(&http).list(P, ROOT).unwrap().is_empty());
}

#[test]
fn rec_detail_reads_transcript_and_notes() {
    let http = Arc::new(ScriptedHttp::new());
    http.bytes("files.download", &serde_json::to_vec(&manifest(NAME, "2026-11-19T14:05:00Z", "done")).unwrap());
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"x","parts":[{"file":"a","offset_s":0.0,"duration_s":9.0}],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    http.json("files.download", 404, not_found());
    let d = service(&http).detail(P, ROOT, NAME).unwrap();
    assert_eq!(d.job.name, NAME);
    assert_eq!(d.transcript.unwrap().segments[0].text, "Hi.");
    assert_eq!(d.notes, None);
    assert_eq!(
        d.link,
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath=%2FVolumes%2Fmain%2Fteam%2Faudio%2FBottle%2FRecordings%2F2026-11-19_0905%2F"
    );
    assert_eq!(service(&http).detail(P, ROOT, "../x").unwrap_err().kind, ErrorKind::BadSettings);
}

#[test]
fn rec_retry_from_writing_notes_runs_only_the_notes_on_the_warehouse() {
    let http = Arc::new(ScriptedHttp::new());
    let mut failed = manifest(NAME, "2026-11-19T14:05:00Z", "failed");
    failed["error_kind"] = json!("endpoint_missing");
    failed["error_stage"] = json!("writing_notes");
    failed["engine"] = json!("warehouse");
    failed["warehouse_id"] = json!(WH);
    http.bytes("files.download", &serde_json::to_vec(&failed).unwrap());
    let tj = json!({"version":1,"id":"r1","name":NAME,"started_at":"2026-11-19T14:05:00Z","duration_s":9.0,
        "instructions":"Write short notes.","parts":[],
        "segments":[{"start":0.0,"end":1.0,"speaker":"1","text":"Hi.","part":1}]});
    http.bytes("files.download", &serde_json::to_vec(&tj).unwrap());
    http.json("warehouses.get", 200, json!({"id":WH,"state":"RUNNING"}));
    http.json("statements.execute", 200, ok_cell("s9", "## Summary\nOK"));
    for _ in 0..4 {
        http.empty("files.upload", 204);
    }
    let out = service(&http).retry(P, &job(vec![], Some(WH)), &mut |_| {});
    assert_eq!(out.status, Status::Done, "{out:?}");
    assert_eq!(http.seen_on("statements.execute").len(), 1);
    assert!(http.seen_on("jobs.runs_submit").is_empty());
    assert!(uploads(&http).iter().any(|(p, b)| p.ends_with(".notes.md") && b == b"## Summary\nOK"));
}

#[test]
fn rec_retry_upload_with_temp_audio_gone_is_audio_missing() {
    let http = Arc::new(ScriptedHttp::new());
    let mut failed = manifest(NAME, "2026-11-19T14:05:00Z", "failed");
    failed["error_stage"] = json!("uploading");
    failed["parts"] = json!([{"file":"2026-11-19_0905.part01.flac","duration_s":30.0,"uploaded":false}]);
    http.bytes("files.download", &serde_json::to_vec(&failed).unwrap());
    for _ in 0..4 {
        http.empty("files.create_directory", 200);
        http.empty("files.upload", 204);
    }
    let gone = LocalPart { path: PathBuf::from("/nonexistent/bottle/x.part01.flac"), duration_s: 30.0 };
    let out = service(&http).retry(P, &job(vec![gone], None), &mut |_| {});
    assert_eq!(out.error_kind, Some(ErrorKind::AudioMissing));
}

#[test]
fn rec_link_uses_profile_host_and_workspace_id() {
    let http = Arc::new(ScriptedHttp::new());
    let url = service(&http).link(P, ROOT, NAME).unwrap();
    assert!(url.starts_with("https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath="));
    let _ = PartEntry::new("x", 1.0);
}
