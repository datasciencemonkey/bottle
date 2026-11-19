//! The app side of PR C (ADR 0103): the knob seed at launch asks nothing, each finished
//! recording runs the pipeline once with its profile and instructions, an empty temp dir is
//! deleted, and Retry finds `<name>.opus` in any per-id temp folder.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bottle_lib::recording_cloud_host::{temp_names_of, temp_parts_of, PipelineHandoff};
use bottle_lib::recording_host::knob_hook;
use mog_core::recording::{MemorySettingsStore, PipelineRequest};
use mog_core::recording_capture::chunker::PartFile;
use mog_core::recording_capture::session::{
    FinishedRecording, PartsHandoff, StopCause, SystemAudioState,
};
use mog_core::types::RecordingSettings;

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("bottle-rec-host-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn rec(dir: &std::path::Path, id: &str, profile: Option<&str>, parts: usize) -> FinishedRecording {
    let name = "2026-11-19_0905";
    FinishedRecording {
        id: id.into(),
        name: name.into(),
        dir: dir.to_path_buf(),
        parts: (0..parts)
            .map(|i| {
                let path = dir.join(format!("{name}.opus"));
                std::fs::write(&path, b"OggS-fake").unwrap();
                PartFile {
                    index: i as u32 + 1,
                    file_name: format!("{name}.opus"),
                    path,
                    samples: 16_000 * 30,
                    offset_ms: 0,
                }
            })
            .collect(),
        started_at_ms: 1_795_097_100_000,
        duration_ms: 30_000,
        cause: StopCause::User,
        system_audio: SystemAudioState::Ok,
        profile: profile.map(str::to_owned),
        instructions: Some("Only action items.".into()),
    }
}

fn store(with_storage: bool) -> Arc<MemorySettingsStore> {
    let storage = with_storage.then(|| {
        serde_json::from_value(serde_json::json!({"kind":"volume","root":"/Volumes/c/s/v"}))
            .unwrap()
    });
    Arc::new(MemorySettingsStore::new(RecordingSettings {
        storage,
        ..RecordingSettings::default()
    }))
}

type Runs = Arc<Mutex<Vec<(PipelineRequest, u64)>>>;

fn handoff(with_storage: bool) -> (PipelineHandoff, Runs) {
    let runs: Runs = Arc::new(Mutex::new(Vec::new()));
    let r = runs.clone();
    let h = PipelineHandoff::new(
        Arc::new(move |req, started| r.lock().unwrap().push((req, started))),
        store(with_storage),
    );
    (h, runs)
}

fn wait_for(runs: &Runs, n: usize) {
    for _ in 0..200 {
        if runs.lock().unwrap().len() >= n {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn rec_knob_seed_at_launch_does_not_ask_and_later_writes_do() {
    let seeds = Arc::new(Mutex::new(Vec::new()));
    let sets = Arc::new(Mutex::new(Vec::new()));
    let (a, b) = (seeds.clone(), sets.clone());
    let hook = knob_hook(
        move |on| a.lock().unwrap().push(on),
        move |on| b.lock().unwrap().push(on),
    );
    hook(true);
    hook(false);
    hook(true);
    assert_eq!(*seeds.lock().unwrap(), vec![true]);
    assert_eq!(*sets.lock().unwrap(), vec![false, true]);
}

#[test]
fn rec_handoff_runs_the_pipeline_once_with_profile_and_instructions() {
    let dir = temp("once");
    let (h, runs) = handoff(true);
    h.finished(rec(&dir, "rec-1", Some("fake-home"), 1));
    h.finished(rec(&dir, "rec-1", Some("fake-home"), 1));
    wait_for(&runs, 1);
    std::thread::sleep(Duration::from_millis(50));
    let got = runs.lock().unwrap();
    assert_eq!(got.len(), 1, "exactly once per recording");
    let (req, started) = &got[0];
    assert_eq!(req.profile, "fake-home");
    assert_eq!(req.instructions.as_str(), "Only action items.");
    assert_eq!(req.storage.root.as_str(), "/Volumes/c/s/v");
    assert_eq!(req.parts.len(), 1);
    assert_eq!(req.parts[0].duration_s, 30.0);
    assert!(req.parts[0].path.ends_with("2026-11-19_0905.opus"));
    assert_eq!(*started, 1_795_097_100_000);
    drop(got);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_handoff_without_profile_or_storage_keeps_audio_for_retry() {
    let dir = temp("keep");
    let (h, runs) = handoff(true);
    h.finished(rec(&dir, "rec-2", None, 1));
    let (h2, runs2) = handoff(false);
    h2.finished(rec(&dir, "rec-3", Some("fake-home"), 1));
    std::thread::sleep(Duration::from_millis(50));
    assert!(runs.lock().unwrap().is_empty());
    assert!(runs2.lock().unwrap().is_empty());
    assert!(dir.join("2026-11-19_0905.opus").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn rec_handoff_deletes_an_empty_temp_dir() {
    let dir = temp("empty");
    let (h, runs) = handoff(true);
    h.finished(rec(&dir, "rec-4", Some("fake-home"), 0));
    assert!(!dir.exists(), "an empty temp dir is deleted");
    assert!(runs.lock().unwrap().is_empty());
}

#[test]
fn rec_retry_finds_the_opus_file_in_any_per_id_folder() {
    let root = temp("scan");
    let a = root.join("rec-abc");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("2026-11-19_0905.opus"), b"OggS").unwrap();
    std::fs::write(a.join("notes.txt"), b"x").unwrap();
    let b = root.join("rec-def");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("2026-11-18_0900.opus"), b"OggS").unwrap();
    let found = temp_parts_of(Some(&root), "2026-11-19_0905");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].path, a.join("2026-11-19_0905.opus"));
    assert!(temp_parts_of(None, "2026-11-19_0905").is_empty());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rec_temp_names_lists_every_recording_left_on_the_device() {
    // ADR 0103 decision 7 (review of #592, BLOCKING 2): the names with audio still on the
    // device, so the cloud side can list them as "upload failed" with Retry.
    let root = temp("names");
    let a = root.join("rec-abc");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("2026-11-19_0905.part01.opus"), b"OggS").unwrap();
    std::fs::write(a.join("2026-11-19_0905.part02.opus"), b"OggS").unwrap();
    std::fs::write(a.join("not-a-recording.txt"), b"x").unwrap();
    let b = root.join("rec-def");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("2026-11-18_0900.opus"), b"OggS").unwrap();

    let names = temp_names_of(Some(&root));
    let mut names: Vec<&str> = names.iter().map(|(n, _)| n.as_str()).collect();
    names.sort_unstable();
    assert_eq!(names, vec!["2026-11-18_0900", "2026-11-19_0905"]);
    assert!(temp_names_of(None).is_empty());
    let _ = std::fs::remove_dir_all(root);
}
