//! The app side of PR C (ADR 0103): the knob seed at launch asks nothing, each finished
//! recording runs the pipeline once with its profile and instructions, an empty temp dir is
//! deleted, and Retry finds `<name>.opus` in any per-id temp folder.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bottle_lib::recording_cloud_host::{
    resumable, resume_left, temp_names_in, temp_names_of, temp_parts_in, temp_parts_of, tidy_dir,
    ActiveNames, LiveHandoff, PipelineHandoff, TempAudio,
};
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
                    held: Vec::new(),
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
    let (h, runs, _) = handoff_with_run(with_storage, |_, _| {});
    (h, runs)
}

/// Like [`handoff`], with `on_run` called from inside the dispatched run (the review of #592,
/// BLOCKING B3: the window the active-names set must cover), and the active-names set itself
/// for the test to inspect.
fn handoff_with_run(
    with_storage: bool,
    on_run: impl Fn(&PipelineRequest, u64) + Send + Sync + 'static,
) -> (PipelineHandoff, Runs, Arc<ActiveNames>) {
    let runs: Runs = Arc::new(Mutex::new(Vec::new()));
    let active = Arc::new(ActiveNames::default());
    let r = runs.clone();
    let h = PipelineHandoff::new(
        Arc::new(move |req, started| {
            on_run(&req, started);
            r.lock().unwrap().push((req, started));
        }),
        store(with_storage),
        active.clone(),
    );
    (h, runs, active)
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
fn rec_handoff_marks_the_name_active_for_the_run_and_releases_it_after() {
    // Review of #592 (BLOCKING B3): the cloud scan and Retry must leave this name out for the
    // whole dispatched run, not only once `CloudAdapter::run_pipeline`'s own guard takes it.
    let dir = temp("active");
    let seen_active_mid_run = Arc::new(Mutex::new(false));
    let s = seen_active_mid_run.clone();
    let (h, runs, active) = handoff_with_run(true, move |_, _| {
        *s.lock().unwrap() = true;
        std::thread::sleep(Duration::from_millis(30));
    });
    assert!(
        !active.contains("2026-11-19_0905"),
        "not active before it starts"
    );
    h.finished(rec(&dir, "rec-5", Some("fake-home"), 1));
    // The run starts on its own thread; give it a moment to reach `on_run` and mark busy.
    for _ in 0..100 {
        if active.contains("2026-11-19_0905") {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        active.contains("2026-11-19_0905"),
        "active for the whole dispatched run"
    );
    wait_for(&runs, 1);
    for _ in 0..100 {
        if !active.contains("2026-11-19_0905") {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        !active.contains("2026-11-19_0905"),
        "released once the run returns"
    );
    assert!(*seen_active_mid_run.lock().unwrap());
    let _ = std::fs::remove_dir_all(dir);
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

// ---------- upload in parts: the old location, the sidecar, the resume ----------

/// The pre-#592 layout: `<app cache>/recordings/<id>/<name>.opus`.
#[test]
fn rec_parts_old_cache_location_is_scanned_with_the_data_location() {
    let data = temp("scan-data");
    let cache = temp("scan-cache");
    let new = data.join("rec-new");
    std::fs::create_dir_all(&new).unwrap();
    std::fs::write(new.join("2026-11-20_0900.part01.opus"), b"OggS").unwrap();
    let old1 = cache.join("rec-aaa");
    let old2 = cache.join("rec-bbb");
    std::fs::create_dir_all(&old1).unwrap();
    std::fs::create_dir_all(&old2).unwrap();
    std::fs::write(old1.join("2026-11-19_0905.opus"), b"OggS").unwrap();
    std::fs::write(old2.join("2026-11-19_1030.opus"), b"OggS").unwrap();

    let roots = vec![data.clone(), cache.clone()];
    let mut names: Vec<String> = temp_names_in(&roots).into_iter().map(|(n, _)| n).collect();
    names.sort();
    assert_eq!(
        names,
        ["2026-11-19_0905", "2026-11-19_1030", "2026-11-20_0900"]
    );
    assert_eq!(temp_parts_in(&roots, "2026-11-19_1030").len(), 1);
    // Old builds wrote no sidecar: nothing resumes by itself.
    assert!(resumable(&roots).is_empty());
    // Through the seam: the cloud side sees them as device-only recordings.
    use mog_core::recording_cloud::contract::LocalAudio;
    let seam = TempAudio::new(roots, Arc::new(Default::default));
    assert_eq!(seam.names().len(), 3);
    assert!(seam.sidecar("2026-11-19_0905").is_none());
    let _ = std::fs::remove_dir_all(data);
    let _ = std::fs::remove_dir_all(cache);
}

#[test]
fn rec_parts_a_sidecar_marks_a_dir_as_resumable_and_tidy_removes_it() {
    use mog_core::recording_cloud::sidecar::{self, Sidecar};
    let root = temp("sidecar");
    let d = root.join("rec-1");
    std::fs::create_dir_all(&d).unwrap();
    sidecar::write(
        &d,
        &Sidecar::new(
            "rec-1",
            "2026-11-20_0900",
            1_000,
            "fake-home",
            Some("Notes."),
        ),
    )
    .unwrap();
    // No audio yet: not resumable.
    assert!(resumable(std::slice::from_ref(&root)).is_empty());
    std::fs::write(d.join("2026-11-20_0900.part01.opus"), b"OggS").unwrap();
    std::fs::write(d.join("2026-11-20_0900.part02.opus"), b"OggS").unwrap();
    let found = resumable(std::slice::from_ref(&root));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].1.profile, "fake-home");
    assert_eq!(found[0].1.instructions.as_deref(), Some("Notes."));
    assert_eq!(found[0].2.len(), 2, "both parts, in order");
    // The sidecar holds no token and no audio.
    let text = std::fs::read_to_string(d.join("job.local.json")).unwrap();
    assert!(!text.contains("token") && text.len() < 400, "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.join("job.local.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // Audio left: tidy keeps the sidecar. Audio gone: it removes it and the dir.
    tidy_dir(&d);
    assert!(d.join("job.local.json").exists());
    std::fs::remove_file(d.join("2026-11-20_0900.part01.opus")).unwrap();
    std::fs::remove_file(d.join("2026-11-20_0900.part02.opus")).unwrap();
    tidy_dir(&d);
    assert!(!d.join("job.local.json").exists());
    let _ = std::fs::remove_dir_all(root);
}

fn live_handoff(enabled: bool, consent: bool, with_storage: bool) -> LiveHandoff {
    use mog_core::clock::SimWallClock;
    use mog_core::recording_cloud::cloud::{CloudDeps, ProfileWorkspace, RecordingCloudService};
    struct NoWait;
    impl mog_core::http_transport::Pause for NoWait {
        fn pause(&self, _d: Duration) {}
    }
    struct Ws;
    impl mog_core::recording_cloud::cloud::WorkspaceInfo for Ws {
        fn host_url(&self, _p: &str) -> mog_core::error::CoreResult<String> {
            Ok("https://dbc-fake.cloud.databricks.com".into())
        }
        fn workspace_id(&self, _p: &str) -> mog_core::error::CoreResult<String> {
            Ok("1".into())
        }
    }
    let _ = std::any::type_name::<ProfileWorkspace>();
    let service = Arc::new(RecordingCloudService::new(CloudDeps {
        http: Arc::new(mog_core::recording_cloud::http::ScriptedHttp::new()),
        workspace: Arc::new(Ws),
        wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
        pause: Arc::new(NoWait),
    }));
    let storage = with_storage.then(|| {
        serde_json::from_value(serde_json::json!({"kind":"volume","root":"/Volumes/c/s/v"}))
            .unwrap()
    });
    let settings = Arc::new(MemorySettingsStore::new(RecordingSettings {
        enabled,
        consent_ack: consent,
        storage,
        ..RecordingSettings::default()
    }));
    let adapter = Arc::new(mog_core::recording_cloud::contract::CloudAdapter::new(
        service,
        settings.clone(),
        Arc::new(mog_core::recording_cloud::contract::NoLocalAudio),
        Arc::new(|_| {}),
        Arc::new(|| 1_795_100_700_000i64),
    ));
    let active = Arc::new(ActiveNames::default());
    let fallback = PipelineHandoff::new(Arc::new(|_, _| {}), settings.clone(), active.clone());
    LiveHandoff::new(adapter, settings, fallback, active)
}

fn started(dir: &std::path::Path) -> mog_core::recording_capture::session::StartedRecording {
    mog_core::recording_capture::session::StartedRecording {
        id: "rec-1234".into(),
        name: "2026-11-20_0900".into(),
        dir: dir.to_path_buf(),
        started_at_ms: 1_795_100_400_000,
        profile: Some("fake-home".into()),
        instructions: None,
    }
}

#[test]
fn rec_parts_the_knob_and_consent_gate_the_upload_during_the_call() {
    // Knob off, no consent, no storage, no profile: no worker, no sidecar, nothing leaves.
    for (enabled, consent, storage) in [
        (false, true, true),
        (true, false, true),
        (true, true, false),
    ] {
        let d = temp("gate");
        let h = live_handoff(enabled, consent, storage);
        h.started(&started(&d));
        assert!(
            !d.join("job.local.json").exists(),
            "knob={enabled} consent={consent} storage={storage}"
        );
        let _ = std::fs::remove_dir_all(d);
    }
    let d = temp("gate-noprofile");
    let h = live_handoff(true, true, true);
    let mut s = started(&d);
    s.profile = None;
    h.started(&s);
    assert!(!d.join("job.local.json").exists());
    // All set: the sidecar is written and a worker takes the recording.
    let d2 = temp("gate-on");
    h.started(&started(&d2));
    assert!(d2.join("job.local.json").exists());
    let _ = std::fs::remove_dir_all(d);
    let _ = std::fs::remove_dir_all(d2);
}

#[test]
fn rec_parts_resume_at_launch_needs_the_knob_consent_and_storage() {
    let root = temp("resume");
    let d = root.join("rec-1");
    std::fs::create_dir_all(&d).unwrap();
    mog_core::recording_cloud::sidecar::write(
        &d,
        &mog_core::recording_cloud::sidecar::Sidecar::new(
            "rec-1",
            "2026-11-20_0900",
            1_000,
            "fake-home",
            None,
        ),
    )
    .unwrap();
    std::fs::write(d.join("2026-11-20_0900.part01.opus"), b"OggS").unwrap();
    let none = std::collections::HashSet::new();
    assert_eq!(
        resume_left(
            &live_handoff(false, true, true),
            std::slice::from_ref(&root),
            &none
        ),
        0
    );
    assert_eq!(
        resume_left(
            &live_handoff(true, false, true),
            std::slice::from_ref(&root),
            &none
        ),
        0
    );
    assert_eq!(
        resume_left(
            &live_handoff(true, true, false),
            std::slice::from_ref(&root),
            &none
        ),
        0
    );
    let busy: std::collections::HashSet<String> = ["2026-11-20_0900".to_owned()].into();
    assert_eq!(
        resume_left(
            &live_handoff(true, true, true),
            std::slice::from_ref(&root),
            &busy
        ),
        0
    );
    assert_eq!(
        resume_left(
            &live_handoff(true, true, true),
            std::slice::from_ref(&root),
            &none
        ),
        1
    );
    assert!(
        d.join("2026-11-20_0900.part01.opus").exists(),
        "audio stays until uploaded"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rec_delete_removes_the_audio_chunks_and_sidecar_of_one_recording_only() {
    use mog_core::recording_cloud::contract::LocalAudio;
    use mog_core::recording_cloud::sidecar::{self, Sidecar};
    let root = temp("delete");
    let d = root.join("rec-1");
    std::fs::create_dir_all(&d).unwrap();
    sidecar::write(
        &d,
        &Sidecar::new("rec-1", "2026-11-20_0900", 1_000, "fake-home", None),
    )
    .unwrap();
    std::fs::write(d.join("2026-11-20_0900.part01.opus"), b"OggS").unwrap();
    std::fs::write(d.join("2026-11-20_0900.part02.opus"), b"OggS").unwrap();
    std::fs::write(d.join("2026-11-20_0900.opus.tmp"), b"x").unwrap();
    // A second recording whose name starts the same way keeps its files (and its own dir).
    let other = root.join("rec-2");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("2026-11-20_0900_2.opus"), b"OggS").unwrap();
    let seam = TempAudio::new(vec![root.clone()], Arc::new(Default::default));
    seam.delete("2026-11-20_0900");
    assert!(
        !d.exists(),
        "the files, the sidecar and the empty dir are gone"
    );
    assert!(other.join("2026-11-20_0900_2.opus").exists());
    let names: Vec<String> = seam.names().into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, ["2026-11-20_0900_2"]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rec_delete_spares_empty_sibling_dirs_and_other_sidecars() {
    use mog_core::recording_cloud::contract::LocalAudio;
    use mog_core::recording_cloud::sidecar::{self, Sidecar};
    let root = temp("delete-safe");
    let d = root.join("rec-1");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("2026-11-20_0900.opus"), b"OggS").unwrap();
    // An empty dir: a capture that just started owns it.
    let fresh = root.join("rec-fresh");
    std::fs::create_dir_all(&fresh).unwrap();
    // A dir that holds only another recording's sidecar.
    let side = root.join("rec-side");
    std::fs::create_dir_all(&side).unwrap();
    sidecar::write(
        &side,
        &Sidecar::new("rec-side", "2026-11-21_1000", 1, "fake-home", None),
    )
    .unwrap();
    let seam = TempAudio::new(vec![root.clone()], Arc::new(Default::default));
    assert!(seam.delete("2026-11-20_0900"));
    assert!(!d.exists());
    assert!(fresh.exists(), "an empty sibling dir survives");
    assert!(
        side.join("job.local.json").exists(),
        "another sidecar survives"
    );
    // A busy name is never deleted.
    let busy: std::collections::HashSet<String> = ["2026-11-21_1000".to_owned()].into();
    let seam = TempAudio::new(vec![root.clone()], Arc::new(move || busy.clone()));
    assert!(!seam.delete("2026-11-21_1000"));
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn rec_delete_does_not_follow_a_symlinked_dir() {
    use mog_core::recording_cloud::contract::LocalAudio;
    let root = temp("delete-link");
    let outside = temp("delete-outside");
    std::fs::write(outside.join("2026-11-20_0900.opus"), b"OggS").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("rec-link")).unwrap();
    let seam = TempAudio::new(vec![root.clone()], Arc::new(Default::default));
    // The scan still lists the symlinked copy, so the call reports that files remain.
    assert!(!seam.delete("2026-11-20_0900"));
    assert!(
        outside.join("2026-11-20_0900.opus").exists(),
        "no symlink is followed"
    );
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(outside);
}

#[test]
fn rec_delete_proves_nothing_uploaded_only_from_the_first_part() {
    use mog_core::recording_cloud::contract::LocalAudio;
    let root = temp("delete-proof");
    let a = root.join("a");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("2026-11-20_0900.part01.opus"), b"x").unwrap();
    std::fs::write(a.join("2026-11-20_0900.part02.opus"), b"x").unwrap();
    let b = root.join("b");
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(b.join("2026-11-21_0900.part02.opus"), b"x").unwrap();
    let seam = TempAudio::new(vec![root.clone()], Arc::new(Default::default));
    assert!(seam.never_uploaded("2026-11-20_0900"));
    assert!(
        !seam.never_uploaded("2026-11-21_0900"),
        "part 01 went up and left"
    );
    assert!(!seam.never_uploaded("2026-12-01_0900"));
    let _ = std::fs::remove_dir_all(root);
}

/// A live hand-off over a volume that takes every write, with the job events it sends.
fn live_handoff_ok() -> (
    LiveHandoff,
    Arc<Mutex<Vec<mog_core::types::RecordingSummary>>>,
) {
    use mog_core::clock::SimWallClock;
    use mog_core::recording_cloud::cloud::{CloudDeps, RecordingCloudService};
    use mog_core::recording_cloud::http::ScriptedHttp;
    struct NoWait;
    impl mog_core::http_transport::Pause for NoWait {
        fn pause(&self, _d: Duration) {}
    }
    struct Ws;
    impl mog_core::recording_cloud::cloud::WorkspaceInfo for Ws {
        fn host_url(&self, _p: &str) -> mog_core::error::CoreResult<String> {
            Ok("https://dbc-fake.cloud.databricks.com".into())
        }
        fn workspace_id(&self, _p: &str) -> mog_core::error::CoreResult<String> {
            Ok("1".into())
        }
    }
    let http = ScriptedHttp::new();
    for _ in 0..10 {
        http.empty("files.create_directory", 200);
        http.empty("files.upload", 204);
    }
    http.empty("files.download", 404);
    let service = Arc::new(RecordingCloudService::new(CloudDeps {
        http: Arc::new(http),
        workspace: Arc::new(Ws),
        wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
        pause: Arc::new(NoWait),
    }));
    let settings = Arc::new(MemorySettingsStore::new(RecordingSettings {
        enabled: true,
        consent_ack: true,
        storage: Some(
            serde_json::from_value(serde_json::json!({"kind":"volume","root":"/Volumes/c/s/v"}))
                .unwrap(),
        ),
        ..RecordingSettings::default()
    }));
    let jobs: Arc<Mutex<Vec<mog_core::types::RecordingSummary>>> = Arc::default();
    let sink = jobs.clone();
    let adapter = Arc::new(mog_core::recording_cloud::contract::CloudAdapter::new(
        service,
        settings.clone(),
        Arc::new(mog_core::recording_cloud::contract::NoLocalAudio),
        Arc::new(move |s: &mog_core::types::RecordingSummary| sink.lock().unwrap().push(s.clone())),
        Arc::new(|| 1_795_100_700_000i64),
    ));
    let active = Arc::new(ActiveNames::default());
    let fallback = PipelineHandoff::new(Arc::new(|_, _| {}), settings.clone(), active.clone());
    (LiveHandoff::new(adapter, settings, fallback, active), jobs)
}

#[test]
fn rec_parts_a_stop_with_no_audio_still_ends_the_job_so_the_row_never_vanishes() {
    let d = temp("live-noaudio");
    let (h, jobs) = live_handoff_ok();
    h.started(&started(&d));
    let mut done = rec(&d, "rec-1234", Some("fake-home"), 0);
    done.name = "2026-11-20_0900".into();
    h.finished(done);
    assert!(!d.exists(), "the empty temp dir goes at once");
    let mut last = None;
    for _ in 0..300 {
        last = jobs.lock().unwrap().last().cloned();
        if last
            .as_ref()
            .is_some_and(|s| s.status == mog_core::types::RecordingStatus::Failed)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let last = last.expect("job events");
    assert_eq!(
        last.status,
        mog_core::types::RecordingStatus::Failed,
        "{last:?}"
    );
    assert_eq!(
        last.error_kind,
        Some(mog_core::types::RecordingErrorKind::AudioUnreadable)
    );
}
