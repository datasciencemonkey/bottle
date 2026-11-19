//! Pure parts of the recording cloud pipeline (ADR 0103, PR C): layout and link builder,
//! settings file, SQL statements, ai_transcribe parsing, segment merge, markdown, job manifest.

use mog_core::recording_cloud::errors::{ErrorKind, Stage};
use mog_core::recording_cloud::job::*;
use mog_core::recording_cloud::layout::*;
use mog_core::recording_cloud::settings_store::*;
use mog_core::recording_cloud::statement::*;
use mog_core::recording_cloud::transcript::*;
use serde_json::{json, Value};

// ---------- layout ----------

#[test]
fn rec_layout_volume_paths_are_strict() {
    for ok in [
        "/Volumes/c/s/v",
        "/Volumes/c/s/v/",
        "/Volumes/databricks/home/7474645105283837/a.b@example.com/my_files/",
        "/Volumes/c/s/v/Bottle/.bottle/transcribe_v1.py",
    ] {
        assert!(is_volume_path(ok), "{ok}");
    }
    for bad in [
        "",
        "/Volumes/c/s",
        "/Volumes/c//v",
        "/Volumes/c/s/v/../x",
        "/Volumes/c/s/v/./x",
        "/Volumes/c/s/v/a b",
        "/Volumes/c/s/v/a?x=1",
        "/Volumes/c/s/v/a#f",
        "/Volumes/c/s/v/a'b",
        "Volumes/c/s/v",
        "/Workspace/Users/x",
        "https://evil.example/Volumes/c/s/v",
    ] {
        assert!(!is_volume_path(bad), "{bad:?}");
    }
}

#[test]
fn rec_layout_root_folder_and_file_names() {
    let root = VolumeRoot::parse("/Volumes/main/team/audio/").unwrap();
    assert_eq!(
        (root.catalog(), root.schema(), root.volume()),
        ("main", "team", "audio")
    );
    assert_eq!(root.path(), "/Volumes/main/team/audio");
    assert_eq!(root.recordings_dir(), "/Volumes/main/team/audio/Bottle/Recordings");
    assert_eq!(
        root.folder("2026-11-19_0905"),
        "/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905"
    );
    assert_eq!(
        root.script_path(),
        "/Volumes/main/team/audio/Bottle/.bottle/transcribe_v1.py"
    );
    assert!(VolumeRoot::parse("/Volumes/main/team").is_err());
    let n = "2026-11-19_0905";
    assert_eq!(part_file(n, 1), "2026-11-19_0905.part01.flac");
    assert_eq!(part_file(n, 12), "2026-11-19_0905.part12.flac");
    assert_eq!(instructions_file(n), "2026-11-19_0905.instructions.md");
    assert_eq!(job_file(n), "2026-11-19_0905.job.json");
    assert_eq!(transcript_json_file(n), "2026-11-19_0905.transcript.json");
    assert_eq!(transcript_md_file(n), "2026-11-19_0905.transcript.md");
    assert_eq!(notes_file(n), "2026-11-19_0905.notes.md");
}

#[test]
fn rec_layout_names_from_local_time_with_clash_suffix() {
    let base = recording_name(2026, 11, 9, 7, 5).unwrap();
    assert_eq!(base, "2026-11-09_0705");
    assert!(recording_name(2026, 13, 1, 0, 0).is_none());
    assert_eq!(unique_name(&base, &[]), "2026-11-09_0705");
    let taken = vec!["2026-11-09_0705".to_owned(), "2026-11-09_0705_2".to_owned()];
    assert_eq!(unique_name(&base, &taken), "2026-11-09_0705_3");
    assert!(is_recording_name("2026-11-09_0705"));
    assert!(is_recording_name("2026-11-09_0705_3"));
    assert!(!is_recording_name("../x"));
    assert!(!is_recording_name("2026-11-09_0705_"));
}

#[test]
fn rec_layout_my_files_candidates_workspace_id_then_deployment() {
    let c = my_files_candidates(
        "7474645105283837",
        "https://fevm-serverless-9cefok.cloud.databricks.com",
        "first.last@example.com",
    );
    assert_eq!(
        c,
        vec![
            "/Volumes/databricks/home/7474645105283837/first.last@example.com/my_files/",
            "/Volumes/databricks/home/fevm-serverless-9cefok/first.last@example.com/my_files/",
        ]
    );
    assert!(my_files_candidates("7", "https://h.example", "a b").is_empty());
    assert_eq!(
        managed_volume_path("main", "team").unwrap(),
        "/Volumes/main/team/bottle_audio"
    );
    assert!(managed_volume_path("main;drop", "team").is_err());
}

#[test]
fn rec_layout_catalog_explorer_link() {
    let root = VolumeRoot::parse("/Volumes/main/team/audio").unwrap();
    let url = catalog_explorer_url(
        "https://dbc-fake.cloud.databricks.com",
        "1234567890",
        &root,
        "2026-11-19_0905",
    )
    .unwrap();
    assert_eq!(
        url,
        "https://dbc-fake.cloud.databricks.com/explore/data/volumes/main/team/audio?o=1234567890&volumePath=%2FVolumes%2Fmain%2Fteam%2Faudio%2FBottle%2FRecordings%2F2026-11-19_0905%2F"
    );
    assert!(catalog_explorer_url("http://dbc-fake.cloud.databricks.com", "1", &root, "2026-11-19_0905").is_err());
    assert!(catalog_explorer_url("https://dbc-fake.cloud.databricks.com/x", "1", &root, "2026-11-19_0905").is_err());
    assert!(catalog_explorer_url("https://dbc-fake.cloud.databricks.com", "1x", &root, "2026-11-19_0905").is_err());
    assert!(catalog_explorer_url("https://dbc-fake.cloud.databricks.com", "1", &root, "../x").is_err());
}

// ---------- settings ----------

#[test]
fn rec_settings_defaults_match_the_brief() {
    let s = RecordingSettings::default();
    assert!(!s.enabled);
    assert!(!s.consent_ack);
    assert_eq!(s.storage, Storage::MyFiles);
    assert_eq!(s.warehouse_id, None);
    assert_eq!(s.notes_endpoint, "databricks-claude-sonnet-5-5");
    assert_eq!(s.length_min, 60);
    assert!(s.preset_instructions.starts_with("You get the transcript of a call."));
    assert!(s.preset_instructions.contains("## Action items"));
    assert!(s.preset_instructions.ends_with("Use only facts from the transcript.\n"));
    assert_eq!(s.validate(), Ok(()));
}

#[test]
fn rec_settings_validation() {
    let ok = RecordingSettings::default();
    let mut s = ok.clone();
    s.length_min = 4;
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
    s.length_min = 181;
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
    let mut s = ok.clone();
    s.notes_endpoint = "bad'name".into();
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
    let mut s = ok.clone();
    s.warehouse_id = Some("e6084ca031cda30a/../x".into());
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
    let mut s = ok.clone();
    s.storage = Storage::Volume {
        path: "/Volumes/c/s".into(),
    };
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
    let mut s = ok.clone();
    s.storage = Storage::CatalogSchema {
        catalog: "main".into(),
        schema: "te am".into(),
    };
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
    let mut s = ok;
    s.preset_instructions = String::new();
    assert_eq!(s.validate(), Err(ErrorKind::BadSettings));
}

#[test]
fn rec_settings_file_round_trip_and_bad_file_falls_back() {
    let dir = std::env::temp_dir().join(format!("bottle-rec-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let file = SettingsFile::new(dir.join(SETTINGS_FILE));
    assert_eq!(file.read(), RecordingSettings::default());
    let mut s = RecordingSettings::default();
    s.enabled = true;
    s.storage = Storage::Volume {
        path: "/Volumes/c/s/v".into(),
    };
    s.warehouse_id = Some("e6084ca031cda30a".into());
    file.write(&s).unwrap();
    assert_eq!(file.read(), s);
    let text = std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap();
    assert!(text.contains("\"kind\": \"volume\""), "{text}");
    let mut bad = s.clone();
    bad.length_min = 1;
    assert_eq!(file.write(&bad), Err(ErrorKind::BadSettings));
    assert_eq!(file.read(), s);
    std::fs::write(dir.join(SETTINGS_FILE), b"{not json").unwrap();
    assert_eq!(file.read(), RecordingSettings::default());
    std::fs::write(dir.join(SETTINGS_FILE), br#"{"enabled":true}"#).unwrap();
    let partial = file.read();
    assert!(partial.enabled);
    assert_eq!(partial.length_min, 60);
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- SQL statements ----------

fn param<'a>(body: &'a Value, name: &str) -> &'a str {
    body["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap()["value"]
        .as_str()
        .unwrap()
}

#[test]
fn rec_statement_transcribe_uses_a_named_path_parameter() {
    let b = transcribe_request("e6084ca031cda30a", "/Volumes/c/s/v/Bottle/Recordings/n/n.part01.flac").unwrap();
    assert_eq!(b["warehouse_id"], "e6084ca031cda30a");
    assert_eq!(b["format"], "JSON_ARRAY");
    assert_eq!(b["disposition"], "INLINE");
    assert_eq!(b["wait_timeout"], "30s");
    assert_eq!(b["on_wait_timeout"], "CONTINUE");
    let sql = b["statement"].as_str().unwrap();
    assert_eq!(sql, TRANSCRIBE_SQL);
    assert!(!sql.contains("/Volumes"));
    assert_eq!(param(&b, "path"), "/Volumes/c/s/v/Bottle/Recordings/n/n.part01.flac");
    assert_eq!(
        transcribe_request("e6084ca031cda30a", "/Volumes/c/s/v/x'; DROP TABLE t; --"),
        Err(ErrorKind::BadSettings)
    );
    assert_eq!(
        transcribe_request("bad id", "/Volumes/c/s/v/a.flac"),
        Err(ErrorKind::NoWarehouse)
    );
}

#[test]
fn rec_statement_notes_passes_text_and_endpoint_as_parameters() {
    let b = notes_request("e6084ca031cda30a", "databricks-claude-sonnet-5-5", "Write notes.", "**Speaker 1** [00:00:00]\nhi 'there'").unwrap();
    let sql = b["statement"].as_str().unwrap();
    assert_eq!(sql, NOTES_SQL);
    assert!(sql.contains("ai_query(:endpoint, concat(:instructions,"));
    assert_eq!(param(&b, "endpoint"), "databricks-claude-sonnet-5-5");
    assert_eq!(param(&b, "instructions"), "Write notes.");
    assert_eq!(param(&b, "transcript"), "**Speaker 1** [00:00:00]\nhi 'there'");
    assert_eq!(
        notes_request("e6084ca031cda30a", "x' OR 1=1", "a", "b"),
        Err(ErrorKind::EndpointMissing)
    );
}

#[test]
fn rec_statement_parse_states_and_errors() {
    let ok = json!({"statement_id":"s1","status":{"state":"SUCCEEDED"},
        "result":{"row_count":1,"data_array":[["{\"x\":1}"]]}});
    let p = parse_statement(&ok, ErrorKind::TranscribeFailed).unwrap();
    assert_eq!(p.id, "s1");
    assert_eq!(p.state, StatementState::Succeeded);
    assert_eq!(p.first_cell.as_deref(), Some("{\"x\":1}"));
    let run = json!({"statement_id":"s2","status":{"state":"PENDING"}});
    assert_eq!(parse_statement(&run, ErrorKind::TranscribeFailed).unwrap().state, StatementState::Running);
    let failed = json!({"statement_id":"s3","status":{"state":"FAILED","error":{"error_code":"BAD_REQUEST",
        "message":"[AI_FUNCTION_COMPILATION_ERROR] An error occurred while compiling the AI function: INVALID_ARGUMENT: 'no-such-endpoint-xyz' does not exist or is not accessible. SQLSTATE: 42883; line 1 pos 7"}}});
    assert_eq!(
        parse_statement(&failed, ErrorKind::NotesFailed).unwrap().state,
        StatementState::Failed(ErrorKind::EndpointMissing)
    );
    let canceled = json!({"statement_id":"s4","status":{"state":"CANCELED"}});
    assert_eq!(
        parse_statement(&canceled, ErrorKind::NotesFailed).unwrap().state,
        StatementState::Failed(ErrorKind::NotesFailed)
    );
    assert!(parse_statement(&json!({"x":1}), ErrorKind::NotesFailed).is_err());
}

// ---------- transcript ----------

/// The ai_transcribe VARIANT from a live run on DEFAULT (FLAC, macOS `say`), as JSON_ARRAY text.
const LIVE_CELL: &str = r#"{"error_message":null,"metadata":{"audio_stream_count":1,"degraded":false,"input_duration_seconds":4.656,"num_chunks":1,"version":"1.0"},"response":{"duration_seconds":4.669,"segments":[{"end":4.64,"speaker_id":"1","start":0,"text":"Hello team. The launch moves to Friday. Maria will send the notes."}]}}"#;

#[test]
fn rec_transcript_parses_the_live_variant() {
    let t = parse_ai_transcribe(LIVE_CELL).unwrap();
    assert!((t.duration_s - 4.669).abs() < 1e-9);
    assert_eq!(t.segments.len(), 1);
    assert_eq!(t.segments[0].speaker_id, "1");
    assert_eq!(t.segments[0].start, 0.0);
    assert!(t.segments[0].text.starts_with("Hello team."));
    let bad = r#"{"error_message":"Unsupported or unrecognized audio format.","metadata":{},"response":null}"#;
    assert_eq!(parse_ai_transcribe(bad).unwrap_err(), ErrorKind::TranscribeFailed);
    let long = r#"{"error_message":"Input audio exceeds the maximum duration.","response":null}"#;
    assert_eq!(parse_ai_transcribe(long).unwrap_err(), ErrorKind::TooLong);
    assert_eq!(parse_ai_transcribe("not json").unwrap_err(), ErrorKind::TranscribeFailed);
    // speaker_id may arrive as a number.
    let num = r#"{"error_message":null,"response":{"duration_seconds":2,"segments":[{"start":0,"end":1,"speaker_id":2,"text":"x"}]}}"#;
    assert_eq!(parse_ai_transcribe(num).unwrap().segments[0].speaker_id, "2");
}

fn seg(start: f64, end: f64, sp: &str, text: &str) -> RawSegment {
    RawSegment {
        start,
        end,
        speaker_id: sp.into(),
        text: text.into(),
    }
}

#[test]
fn rec_transcript_merge_gives_absolute_times_and_part_prefixed_speakers() {
    let parts = vec![
        PartInput {
            file: "n.part01.flac".into(),
            duration_s: Some(3300.0),
            transcript: PartTranscript {
                duration_s: 3299.5,
                segments: vec![seg(0.0, 2.0, "1", "Hi."), seg(2.0, 4.0, "2", "Hello.")],
            },
        },
        PartInput {
            file: "n.part02.flac".into(),
            duration_s: None,
            transcript: PartTranscript {
                duration_s: 60.0,
                segments: vec![seg(1.0, 3.0, "1", "Back.")],
            },
        },
    ];
    let m = merge_parts(&parts);
    assert_eq!(m.parts.len(), 2);
    assert_eq!(m.parts[0].offset_s, 0.0);
    assert_eq!(m.parts[1].offset_s, 3300.0);
    assert_eq!(m.parts[1].duration_s, 60.0);
    assert!((m.duration_s - 3360.0).abs() < 1e-9);
    let speakers: Vec<&str> = m.segments.iter().map(|s| s.speaker.as_str()).collect();
    assert_eq!(speakers, vec!["1", "2", "2.1"]);
    assert_eq!(m.segments[2].start, 3301.0);
    assert_eq!(m.segments[2].part, 2);
    // One part: plain labels.
    let one = merge_parts(&parts[..1]);
    assert_eq!(one.segments[1].speaker, "2");
}

#[test]
fn rec_transcript_markdown_merges_turns_with_timestamps() {
    let doc = TranscriptDoc {
        version: 1,
        id: "r1".into(),
        name: "2026-11-19_0905".into(),
        started_at: "2026-11-19T14:05:00Z".into(),
        duration_s: 3725.0,
        instructions: "x".into(),
        parts: vec![],
        segments: vec![
            Segment { start: 0.0, end: 1.0, speaker: "1".into(), text: "Hi.".into(), part: 1 },
            Segment { start: 1.0, end: 2.0, speaker: "1".into(), text: "Still me.".into(), part: 1 },
            Segment { start: 3723.4, end: 3725.0, speaker: "2".into(), text: "Bye.".into(), part: 1 },
        ],
    };
    let md = render_markdown(&doc);
    assert_eq!(
        md,
        "# Call 2026-11-19_0905\n\n**Speaker 1** [00:00:00]\nHi. Still me.\n\n**Speaker 2** [01:02:03]\nBye.\n"
    );
    assert_eq!(fmt_hms(83.9), "00:01:23");
    let v = serde_json::to_value(&doc).unwrap();
    assert_eq!(v["instructions"], "x");
    assert_eq!(v["segments"][2]["speaker"], "2");
}

// ---------- job manifest and runs ----------

#[test]
fn rec_job_manifest_shape_and_transitions() {
    let mut j = JobManifest::new(
        "r1",
        "2026-11-19_0905",
        "2026-11-19T14:05:00Z",
        vec![PartEntry::new("2026-11-19_0905.part01.flac", 61.5)],
        "databricks-claude-sonnet-5-5",
        Some("e6084ca031cda30a".into()),
        "2026-11-19T15:05:00Z",
    );
    assert_eq!(j.status, Status::Uploading);
    assert!((j.duration_s - 61.5).abs() < 1e-9);
    j.fail(ErrorKind::VolumePermission, Stage::Uploading, "2026-11-19T15:06:00Z");
    let v = serde_json::to_value(&j).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error_kind"], "volume_permission");
    assert_eq!(v["error_stage"], "uploading");
    assert_eq!(v["updated_at"], "2026-11-19T15:06:00Z");
    assert_eq!(v["parts"][0]["file"], "2026-11-19_0905.part01.flac");
    assert_eq!(v["parts"][0]["uploaded"], false);
    j.set_status(Status::Transcribing, "2026-11-19T15:07:00Z");
    let v = serde_json::to_value(&j).unwrap();
    assert!(v.get("error_kind").is_none(), "{v}");
    assert_eq!(v["status"], "transcribing");
    // The script writes the same shape; a manifest it wrote reads back.
    let from_script = json!({"version":1,"id":"r1","name":"n","started_at":"2026-11-19T14:05:00Z",
        "duration_s":1.0,"parts":[{"file":"n.part01.flac","duration_s":1.0,"uploaded":true}],
        "status":"done","endpoint":"e","engine":"job","run_id":42,"updated_at":"2026-11-19T15:00:00Z"});
    let back: JobManifest = serde_json::from_value(from_script).unwrap();
    assert_eq!(back.status, Status::Done);
    assert_eq!(back.engine, Some(Engine::Job));
    assert_eq!(back.run_id, Some(42));
}

#[test]
fn rec_job_iso_utc() {
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_utc(1_795_100_700_000), "2026-11-19T15:05:00Z");
}

#[test]
fn rec_job_runs_submit_body_is_serverless_with_fixed_script() {
    let b = runs_submit_body(&RunRequest {
        script: "/Volumes/c/s/v/Bottle/.bottle/transcribe_v1.py",
        folder: "/Volumes/c/s/v/Bottle/Recordings/n",
        name: "2026-11-19_0905",
        endpoint: "databricks-claude-sonnet-5-5",
        from_stage: Stage::Transcribing,
        idempotency_token: "r1-1",
    })
    .unwrap();
    assert_eq!(b["run_name"], "Bottle transcribe 2026-11-19_0905");
    assert_eq!(b["idempotency_token"], "r1-1");
    let task = &b["tasks"][0];
    assert_eq!(task["task_key"], "transcribe");
    assert_eq!(task["environment_key"], "bottle");
    assert!(task.get("new_cluster").is_none() && task.get("existing_cluster_id").is_none());
    assert_eq!(task["spark_python_task"]["python_file"], "/Volumes/c/s/v/Bottle/.bottle/transcribe_v1.py");
    assert_eq!(
        task["spark_python_task"]["parameters"],
        json!(["--folder","/Volumes/c/s/v/Bottle/Recordings/n","--name","2026-11-19_0905",
               "--endpoint","databricks-claude-sonnet-5-5","--from-stage","transcribing"])
    );
    assert_eq!(b["environments"][0]["environment_key"], "bottle");
    assert!(b["environments"][0]["spec"]["environment_version"].is_string());
    assert!(runs_submit_body(&RunRequest {
        script: "/Volumes/c/s/v/x.py",
        folder: "/Volumes/c/s/v/Bottle/Recordings/n",
        name: "bad name",
        endpoint: "e",
        from_stage: Stage::Transcribing,
        idempotency_token: "r1-1",
    })
    .is_err());
}

#[test]
fn rec_job_parse_run_states() {
    let r = |v: Value| parse_run(&v);
    assert_eq!(r(json!({"state":{"life_cycle_state":"PENDING"}})), RunState::Running);
    assert_eq!(r(json!({"state":{"life_cycle_state":"QUEUED"}})), RunState::Running);
    assert_eq!(r(json!({"state":{"life_cycle_state":"RUNNING"}})), RunState::Running);
    assert_eq!(
        r(json!({"state":{"life_cycle_state":"TERMINATED","result_state":"SUCCESS"}})),
        RunState::Succeeded
    );
    assert_eq!(
        r(json!({"state":{"life_cycle_state":"TERMINATED","result_state":"FAILED"}})),
        RunState::Failed
    );
    assert_eq!(r(json!({"state":{"life_cycle_state":"INTERNAL_ERROR"}})), RunState::Failed);
    assert_eq!(
        r(json!({"status":{"state":"TERMINATED","termination_details":{"code":"SUCCESS"}}})),
        RunState::Succeeded
    );
}

#[test]
fn rec_job_script_is_embedded_and_versioned() {
    assert_eq!(SCRIPT_FILE, "transcribe_v1.py");
    assert!(SCRIPT_V1.contains("ai_transcribe"));
    assert!(SCRIPT_V1.contains("ai_query"));
    assert!(SCRIPT_V1.contains("def main"));
}
