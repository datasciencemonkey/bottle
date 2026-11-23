//! Pure parts of the recording cloud pipeline (ADR 0103, PR C): layout and link builder,
//! settings file, model requests and replies, transcript lines, merge, markdown, job manifest.

use mog_core::recording_cloud::errors::{ErrorKind, Stage};
use mog_core::recording_cloud::job::*;
use mog_core::recording_cloud::layout::*;
use mog_core::recording_cloud::model::*;
use mog_core::recording_cloud::settings_store::*;
use mog_core::recording_cloud::transcript::*;
use serde_json::json;

// ---------- layout ----------

#[test]
fn rec_layout_volume_paths_are_strict() {
    for ok in [
        "/Volumes/c/s/v",
        "/Volumes/c/s/v/",
        "/Volumes/databricks/home/7474645105283837/a.b@example.com/my_files/",
        "/Volumes/c/s/v/Bottle/Recordings/n/n.opus",
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
    assert_eq!(
        root.recordings_dir(),
        "/Volumes/main/team/audio/Bottle/Recordings"
    );
    assert_eq!(
        root.folder("2026-11-19_0905"),
        "/Volumes/main/team/audio/Bottle/Recordings/2026-11-19_0905"
    );
    assert!(VolumeRoot::parse("/Volumes/main/team").is_err());
    let n = "2026-11-19_0905";
    assert_eq!(part_file(n, 1, "opus"), "2026-11-19_0905.part01.opus");
    assert_eq!(part_file(n, 12, "flac"), "2026-11-19_0905.part12.flac");
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
    assert!(catalog_explorer_url(
        "http://dbc-fake.cloud.databricks.com",
        "1",
        &root,
        "2026-11-19_0905"
    )
    .is_err());
    assert!(catalog_explorer_url(
        "https://dbc-fake.cloud.databricks.com/x",
        "1",
        &root,
        "2026-11-19_0905"
    )
    .is_err());
    assert!(catalog_explorer_url(
        "https://dbc-fake.cloud.databricks.com",
        "1x",
        &root,
        "2026-11-19_0905"
    )
    .is_err());
    assert!(
        catalog_explorer_url("https://dbc-fake.cloud.databricks.com", "1", &root, "../x").is_err()
    );
}

// ---------- settings ----------

#[test]
fn rec_settings_json_file_round_trip_and_bad_file_falls_back() {
    use mog_core::recording::RecordingSettingsStore;
    use mog_core::types::{RecordingSettings, RecordingSettingsPatch};
    let dir = std::env::temp_dir().join(format!("bottle-rec-settings-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let deferred = JsonSettingsStore::deferred();
    assert_eq!(deferred.load().unwrap(), RecordingSettings::default());
    assert!(deferred.save(&RecordingSettings::default()).is_err());
    let file = JsonSettingsStore::new(dir.join(SETTINGS_FILE));
    let d = file.load().unwrap();
    assert!(!d.enabled && !d.consent_ack && d.storage.is_none());
    assert_eq!(
        d.transcribe_endpoint.as_str(),
        "databricks-gemini-3-8-flash"
    );
    assert_eq!(d.notes_endpoint.as_str(), "databricks-claude-sonnet-5-5");
    assert_eq!(d.length_min.get(), 60);
    let patch: RecordingSettingsPatch = serde_json::from_value(json!({
        "enabled": true,
        "storage": {"kind": "volume", "root": "/Volumes/c/s/v"},
        "lengthMin": 90
    }))
    .unwrap();
    let s = d.apply(patch);
    file.save(&s).unwrap();
    assert_eq!(file.load().unwrap(), s);
    let text = std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap();
    assert!(text.contains("\"lengthMin\": 90"), "{text}");
    assert!(!text.to_ascii_lowercase().contains("token"), "{text}");
    std::fs::write(dir.join(SETTINGS_FILE), b"{not json").unwrap();
    assert_eq!(file.load().unwrap(), RecordingSettings::default());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rec_settings_validators() {
    assert!(is_endpoint_name("databricks-gemini-3-8-flash"));
    assert!(!is_endpoint_name("bad'name"));
    assert!(!is_endpoint_name("a/../b"));
    assert!(valid_instructions("Write notes."));
    assert!(!valid_instructions("   "));
    assert!(valid_catalog_schema("main", "team"));
    assert!(!valid_catalog_schema("main", "te am"));
    assert_eq!(Storage::default(), Storage::MyFiles);
}

// ---------- model requests and replies ----------

const OGG: &[u8] = b"OggS\x00\x02fake-opus-bytes";

#[test]
fn rec_model_transcribe_request_is_an_inline_audio_url_on_the_gateway_shape() {
    let b = transcribe_request("databricks-gemini-3-8-flash", OGG, None).unwrap();
    assert_eq!(b["model"], "databricks-gemini-3-8-flash");
    assert_eq!(b["max_tokens"], 64000);
    let content = b["messages"][0]["content"].as_array().unwrap();
    assert_eq!(b["messages"][0]["role"], "user");
    assert_eq!(content[0]["type"], "text");
    let prompt = content[0]["text"].as_str().unwrap();
    assert!(prompt.contains("[mm:ss] Speaker N: text"), "{prompt}");
    assert!(!prompt.contains("continues a call"));
    assert_eq!(content[1]["type"], "audio_url");
    let url = content[1]["audio_url"]["url"].as_str().unwrap();
    assert_eq!(url, format!("data:audio/ogg;base64,{}", base64(OGG)));
    let ctx = transcribe_request("e", OGG, Some("[04:58] Speaker 2: See you.")).unwrap();
    let p = ctx["messages"][0]["content"][0]["text"].as_str().unwrap();
    assert!(
        p.contains("[04:58] Speaker 2: See you.") && p.contains("Keep the same speaker labels")
    );
}

#[test]
fn rec_model_only_known_audio_magic_and_size_budget() {
    assert_eq!(sniff(b"OggS...."), Some(AudioFormat::Ogg));
    assert_eq!(sniff(b"fLaC...."), Some(AudioFormat::Flac));
    assert_eq!(
        sniff(b"RIFF\x10\x00\x00\x00WAVEfmt "),
        Some(AudioFormat::Wav)
    );
    assert_eq!(sniff(b"hello"), None);
    assert_eq!(sniff(b"<html>"), None);
    assert_eq!(
        transcribe_request("e", b"hello, not audio", None),
        Err(ErrorKind::AudioUnreadable)
    );
    assert_eq!(
        transcribe_request("bad name", OGG, None),
        Err(ErrorKind::TranscribeEndpointMissing)
    );
    let mut big = b"OggS".to_vec();
    big.resize(REQUEST_BUDGET / 4 * 3, 0);
    assert_eq!(transcribe_request("e", &big, None), Err(ErrorKind::TooLong));
    let mut ok = b"OggS".to_vec();
    ok.resize(1024 * 1024, 0);
    assert!(transcribe_request("e", &ok, None).is_ok());
}

#[test]
fn rec_model_base64_is_standard() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
}

#[test]
fn rec_model_notes_request_and_reply_text() {
    let b = notes_request(
        "databricks-claude-sonnet-5-5",
        "Write notes.",
        "**Speaker 1** [00:00:00]\nHi.",
    )
    .unwrap();
    assert_eq!(b["model"], "databricks-claude-sonnet-5-5");
    assert_eq!(
        b["messages"][0]["content"],
        "Write notes.\n\nTranscript:\n**Speaker 1** [00:00:00]\nHi."
    );
    assert_eq!(
        notes_request("x' y", "a", "b"),
        Err(ErrorKind::NotesEndpointMissing)
    );
    // Claude: a plain string (live shape).
    let claude = json!({"choices":[{"message":{"role":"assistant","content":"**Notes:** Friday."},"index":0}]});
    assert_eq!(reply_text(&claude).as_deref(), Some("**Notes:** Friday."));
    // Gemini: a list of parts with a thoughtSignature (live shape).
    let gemini = json!({"choices":[{"message":{"role":"assistant","content":[
        {"type":"text","text":"[00:00] Speaker 1: Hello team.","thoughtSignature":"AY89a1-fake"},
        {"type":"text","text":"\n[00:05] Speaker 2: Hi."}]}}]});
    assert_eq!(
        reply_text(&gemini).as_deref(),
        Some("[00:00] Speaker 1: Hello team.\n[00:05] Speaker 2: Hi.")
    );
    assert_eq!(reply_text(&json!({"choices":[]})), None);
    assert_eq!(
        reply_text(&json!({"choices":[{"message":{"content":"  "}}]})),
        None
    );
}

#[test]
fn rec_model_parse_transcript_lines_tolerantly() {
    // The live 2-speaker reply, then the tolerant cases.
    let text = "[00:00] Speaker 1: Good morning, everyone.\n\
[00:05] Speaker 2: Thanks. The back-end work is done.\n\
and the script needs a review.\n\
- **[00:10] Speaker 1:** Okay. Thursday?\n\
```\n\
[1:02:03] You: Bye.\n\
[bad] Speaker 3: skipped as a continuation\n";
    let s = parse_transcript(text, Some(3730.0));
    assert_eq!(s.len(), 4, "{s:?}");
    assert_eq!(
        (s[0].start, s[0].end, s[0].speaker_id.as_str()),
        (0.0, 5.0, "1")
    );
    assert_eq!(
        s[1].text,
        "Thanks. The back-end work is done. and the script needs a review."
    );
    assert_eq!(
        (s[2].start, s[2].speaker_id.as_str(), s[2].text.as_str()),
        (10.0, "1", "Okay. Thursday?")
    );
    assert_eq!((s[3].start, s[3].speaker_id.as_str()), (3723.0, "You"));
    assert_eq!(s[3].end, 3730.0);
    assert!(s[3].text.starts_with("Bye."));
    assert!(parse_transcript("", None).is_empty());
    assert_eq!(parse_transcript("[00:61] Speaker 1: x", None).len(), 0);
}

// ---------- transcript ----------

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
    assert_eq!(speakers, vec!["1", "2", "1"]);
    assert_eq!(m.segments[2].start, 3301.0);
    assert_eq!(m.segments[2].part, 2);
    let one = merge_parts(&parts[..1]);
    assert_eq!(one.segments[1].speaker, "2");
    assert_eq!(one.duration_s, 3300.0);
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
            Segment {
                start: 0.0,
                end: 1.0,
                speaker: "1".into(),
                text: "Hi.".into(),
                part: 1,
            },
            Segment {
                start: 1.0,
                end: 2.0,
                speaker: "1".into(),
                text: "Still me.".into(),
                part: 1,
            },
            Segment {
                start: 3723.4,
                end: 3725.0,
                speaker: "2".into(),
                text: "Bye.".into(),
                part: 1,
            },
            Segment {
                start: 3725.0,
                end: 3726.0,
                speaker: "You".into(),
                text: "Thanks.".into(),
                part: 1,
            },
        ],
        bottle_gaps: vec![],
    };
    let md = render_markdown(&doc);
    assert_eq!(
        md,
        "# Call 2026-11-19_0905\n\n**Speaker 1** [00:00:00]\nHi. Still me.\n\n**Speaker 2** [01:02:03]\nBye.\n\n**You** [01:02:05]\nThanks.\n"
    );
    assert_eq!(fmt_hms(83.9), "00:01:23");
    let v = serde_json::to_value(&doc).unwrap();
    assert_eq!(v["instructions"], "x");
    assert_eq!(v["segments"][2]["speaker"], "2");
}

// ---------- job manifest ----------

#[test]
fn rec_job_manifest_shape_and_transitions() {
    let mut j = JobManifest::new(
        "r1",
        "2026-11-19_0905",
        "2026-11-19T14:05:00Z",
        vec![PartEntry::new("2026-11-19_0905.opus", 61.5)],
        "databricks-gemini-3-8-flash",
        "databricks-claude-sonnet-5-5",
        "2026-11-19T15:05:00Z",
    );
    assert_eq!(j.status, Status::Uploading);
    assert!((j.duration_s - 61.5).abs() < 1e-9);
    j.fail(
        ErrorKind::VolumePermission,
        Stage::Uploading,
        "2026-11-19T15:06:00Z",
    );
    let v = serde_json::to_value(&j).unwrap();
    assert_eq!(v["version"], 1);
    assert_eq!(v["status"], "failed");
    assert_eq!(v["error_kind"], "volume_permission");
    assert_eq!(v["error_stage"], "uploading");
    assert_eq!(v["updated_at"], "2026-11-19T15:06:00Z");
    assert_eq!(v["parts"][0]["file"], "2026-11-19_0905.opus");
    assert_eq!(v["parts"][0]["uploaded"], false);
    assert_eq!(v["transcribe_endpoint"], "databricks-gemini-3-8-flash");
    assert_eq!(v["notes_endpoint"], "databricks-claude-sonnet-5-5");
    assert!(
        v.get("engine").is_none() && v.get("warehouse_id").is_none(),
        "{v}"
    );
    j.set_status(Status::Transcribing, "2026-11-19T15:07:00Z");
    let v = serde_json::to_value(&j).unwrap();
    assert!(v.get("error_kind").is_none(), "{v}");
    assert_eq!(v["status"], "transcribing");
    let back: JobManifest = serde_json::from_value(v).unwrap();
    assert_eq!(back, j);
}

#[test]
fn rec_job_iso_utc() {
    assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
    assert_eq!(iso_utc(1_795_100_700_000), "2026-11-19T15:05:00Z");
    assert_eq!(iso_utc(951_782_400_000), "2000-02-29T00:00:00Z");
}

#[test]
fn rec_layout_file_link_names_the_file_and_opens_the_strict_folder_link() {
    use mog_core::recording_cloud::layout::{catalog_explorer_file_link, RecordingFile};
    let root = VolumeRoot::parse("/Volumes/main/team/audio").unwrap();
    let host = "https://dbc-fake.cloud.databricks.com";
    let folder = catalog_explorer_url(host, "1234567890", &root, "2026-11-19_0905").unwrap();
    for (file, want) in [
        (RecordingFile::Transcript, "2026-11-19_0905.transcript.md"),
        (RecordingFile::Notes, "2026-11-19_0905.notes.md"),
    ] {
        let l =
            catalog_explorer_file_link(host, "1234567890", &root, "2026-11-19_0905", file).unwrap();
        assert_eq!(l.file_name, want);
        // Databricks documents no file deep link: the URL is the folder URL.
        assert_eq!(l.url, folder);
    }
    for (h, ws, name) in [
        (
            "http://dbc-fake.cloud.databricks.com",
            "1",
            "2026-11-19_0905",
        ),
        (
            "https://dbc-fake.cloud.databricks.com/x",
            "1",
            "2026-11-19_0905",
        ),
        (
            "https://dbc-fake.cloud.databricks.com",
            "1x",
            "2026-11-19_0905",
        ),
        ("https://dbc-fake.cloud.databricks.com", "1", "../x"),
        (
            "https://dbc-fake.cloud.databricks.com",
            "1",
            "2026-11-19_0905.notes.md",
        ),
        (
            "https://dbc-fake.cloud.databricks.com",
            "1",
            "2026-11-19_0905/../x",
        ),
    ] {
        assert!(
            catalog_explorer_file_link(h, ws, &root, name, RecordingFile::Notes).is_err(),
            "{h} {ws} {name}"
        );
    }
}

#[test]
fn adr_0118_a_settings_file_from_before_held_note_loads_and_saves_with_the_flag_off() {
    use mog_core::recording::RecordingSettingsStore;
    let dir = std::env::temp_dir().join(format!("bottle-rec-held-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(SETTINGS_FILE);
    let old = json!({
        "enabled": true,
        "consentAck": true,
        "storage": {"kind": "volume", "root": "/Volumes/c/s/v"},
        "transcribeEndpoint": "databricks-gemini-3-8-flash",
        "notesEndpoint": "databricks-claude-sonnet-5-5",
        "presetInstructions": "Write short notes in Markdown.",
        "lengthMin": 45
    });
    std::fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();
    let store = JsonSettingsStore::new(&path);
    let s = store.load().unwrap();
    assert!(
        s.enabled && s.consent_ack && !s.held_note_shown,
        "the old file keeps its choices"
    );
    store.save(&s).unwrap();
    let on_disk: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(on_disk["heldNoteShown"], json!(false));
    assert_eq!(on_disk["lengthMin"], json!(45));
    let _ = std::fs::remove_dir_all(&dir);
}
