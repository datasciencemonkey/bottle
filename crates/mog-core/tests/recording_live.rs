//! Upload in parts during the call, and one transcription at the end (ADR 0103 follow-up).
//!
//! The volume is an in-memory fake with an offline switch. Parts are real Ogg Opus files from
//! the app's own encoder. Waits are not real: the worker's `recv` is a script, and the wait it
//! asks for is recorded, so the test sees the backoff without any sleep.

use std::collections::{BTreeMap, VecDeque};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::clock::SimWallClock;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::Pause;
use mog_core::ipc::JsonObject;
use mog_core::recording::MemorySettingsStore;
use mog_core::recording::RecordingCloud;
use mog_core::recording_capture::opus::OpusWriter;
use mog_core::recording_cloud::cloud::{
    CloudDeps, LocalPart, RecordingCloudService, RecordingJob, WorkspaceInfo,
};
use mog_core::recording_cloud::contract::{
    Backoff, CloudAdapter, LiveContext, LiveMsg, LocalAudio, Recv,
};
use mog_core::recording_cloud::http::{RecordingHttp, Reply, ReplyBody, ReqBody};
use mog_core::recording_cloud::job::{JobManifest, Status};
use mog_core::recording_cloud::model::base64;
use mog_core::recording_cloud::stitch;
use mog_core::types::{
    ProfileName, RecordingErrorKind, RecordingSettings, RecordingStatus, RecordingStorage,
    RecordingStorageKind, RecordingSummary, Tags, VolumePath,
};
use serde_json::{json, Value};

const P: &str = "fake-home";
const ROOT: &str = "/Volumes/main/team/audio";
const NAME: &str = "2026-11-20_0900";
const FOLDER: &str = "/Volumes/main/team/audio/Bottle/Recordings/2026-11-20_0900";
const MODEL: &str = "mlflow.chat_completions";

// ---------- the fake volume ----------

#[derive(Default)]
struct Vol {
    files: BTreeMap<String, Vec<u8>>,
    /// What the volume saw, in order: `upload:<file>`, `job:<status>:<uploaded>`.
    log: Vec<String>,
    model_calls: Vec<Value>,
    /// For each upload of a part: did its local file still exist?
    local_at_upload: Vec<bool>,
    /// For each write of job.json: did the part uploaded last still exist locally?
    local_at_job: Vec<bool>,
    watch: Vec<PathBuf>,
    last_uploaded: String,
}

#[derive(Default)]
struct FakeVolume {
    vol: Mutex<Vol>,
    offline: AtomicBool,
    deny: AtomicBool,
    fail_job_writes: AtomicUsize,
}

fn ok(status: u16) -> CoreResult<Reply> {
    Ok(Reply {
        status,
        body: ReplyBody::Empty,
    })
}

fn json_reply(status: u16, v: Value) -> CoreResult<Reply> {
    Ok(Reply {
        status,
        body: ReplyBody::Json(v),
    })
}

impl RecordingHttp for FakeVolume {
    fn send(
        &self,
        _profile: &str,
        route: &str,
        params: &JsonObject,
        body: ReqBody<'_>,
        _tags: Option<&Tags>,
    ) -> CoreResult<Reply> {
        if self.offline.load(Ordering::SeqCst) {
            return Err(CoreError::Unavailable {
                component: "test",
                reason: "offline",
            });
        }
        let path = params
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mut v = self.vol.lock().unwrap();
        match route {
            "files.create_directory" => ok(200),
            "files.upload" => {
                if self.deny.load(Ordering::SeqCst) {
                    return json_reply(403, json!({"error_code":"PERMISSION_DENIED"}));
                }
                let ReqBody::Bytes(b) = body else {
                    return ok(400);
                };
                let overwrite = params.get("overwrite").and_then(Value::as_str) == Some("true");
                let file = path.rsplit('/').next().unwrap_or("").to_owned();
                if file.ends_with(".job.json") {
                    let left = self.fail_job_writes.load(Ordering::SeqCst);
                    if left > 0 {
                        self.fail_job_writes.store(left - 1, Ordering::SeqCst);
                        return Err(CoreError::Unavailable {
                            component: "test",
                            reason: "offline",
                        });
                    }
                    let m: JobManifest = serde_json::from_slice(b).unwrap();
                    let n = m.parts.iter().filter(|p| p.uploaded).count();
                    v.log.push(format!("job:{:?}:{n}", m.status));
                    let last = v.last_uploaded.clone();
                    let seen = v.watch.iter().any(|p| {
                        p.file_name().and_then(|f| f.to_str()) == Some(last.as_str()) && p.exists()
                    });
                    v.local_at_job.push(seen);
                } else if file.contains(".part") {
                    v.log.push(format!("upload:{file}"));
                    let seen = v.watch.iter().any(|p| {
                        p.file_name().and_then(|f| f.to_str()) == Some(file.as_str()) && p.exists()
                    });
                    v.local_at_upload.push(seen);
                    v.last_uploaded = file.clone();
                }
                if !overwrite && v.files.contains_key(&path) {
                    return json_reply(409, json!({"error_code":"RESOURCE_ALREADY_EXISTS"}));
                }
                v.files.insert(path, b.to_vec());
                ok(204)
            }
            "files.download" => match v.files.get(&path) {
                Some(b) => Ok(Reply {
                    status: 200,
                    body: ReplyBody::Bytes(b.clone()),
                }),
                None => json_reply(404, json!({"error_code":"NOT_FOUND"})),
            },
            "files.list_directory" => {
                let prefix = path.clone();
                let mut names: Vec<String> = v
                    .files
                    .keys()
                    .filter_map(|k| k.strip_prefix(&prefix))
                    .filter_map(|r| r.split('/').next().map(str::to_owned))
                    .collect();
                names.dedup();
                if names.is_empty() {
                    return json_reply(404, json!({"error_code":"NOT_FOUND"}));
                }
                let contents: Vec<Value> = names
                    .iter()
                    .map(|n| json!({"name": n, "is_directory": true}))
                    .collect();
                json_reply(200, json!({ "contents": contents }))
            }
            MODEL => {
                let b = match body {
                    ReqBody::Json(b) => b.clone(),
                    _ => Value::Null,
                };
                let model = b["model"].as_str().unwrap_or("").to_owned();
                v.model_calls.push(b);
                if model.contains("gemini") {
                    json_reply(
                        200,
                        json!({"choices":[{"message":{"role":"assistant","content":
                            "[00:00] Speaker 1: Hello.\n[01:00] Speaker 2: Hi."}}]}),
                    )
                } else {
                    json_reply(
                        200,
                        json!({"choices":[{"message":{"role":"assistant","content":"## Summary\nOk."}}]}),
                    )
                }
            }
            _ => ok(404),
        }
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

struct LocalFake {
    dir: PathBuf,
    active: Vec<String>,
}
impl LocalAudio for LocalFake {
    fn parts(&self, name: &str) -> Vec<LocalPart> {
        let mut v: Vec<LocalPart> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| f.starts_with(name))
            })
            .map(|path| LocalPart {
                path,
                duration_s: 0.0,
                held: Vec::new(),
            })
            .collect();
        v.sort_by(|a, b| a.path.cmp(&b.path));
        v
    }
    fn names(&self) -> Vec<(String, u64)> {
        let mut out: Vec<(String, u64)> = Vec::new();
        for e in std::fs::read_dir(&self.dir).into_iter().flatten().flatten() {
            let f = e.file_name().to_string_lossy().into_owned();
            let name = f.split('.').next().unwrap_or("").to_owned();
            if mog_core::recording_cloud::layout::is_recording_name(&name)
                && !out.iter().any(|(n, _)| *n == name)
            {
                out.push((name, 1_000));
            }
        }
        out
    }
    fn active(&self) -> std::collections::HashSet<String> {
        self.active.iter().cloned().collect()
    }
}

// ---------- helpers ----------

fn real_part(seconds: usize, hz: f32) -> Vec<u8> {
    let mut w = OpusWriter::new(Cursor::new(Vec::new())).unwrap();
    let n = 16_000 * seconds;
    let s: Vec<f32> = (0..n)
        .map(|i| 0.3 * (i as f32 * hz * std::f32::consts::TAU / 16_000.0).sin())
        .collect();
    w.write(&s).unwrap();
    w.finish().unwrap().into_inner()
}

struct Rig {
    vol: Arc<FakeVolume>,
    adapter: Arc<CloudAdapter>,
    summaries: Arc<Mutex<Vec<RecordingSummary>>>,
    dir: PathBuf,
    bytes: Vec<Vec<u8>>,
}

fn rig(tag: &str, parts: usize, active: Vec<String>) -> Rig {
    let dir = std::env::temp_dir().join(format!("bottle-live-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let vol = Arc::new(FakeVolume::default());
    let service = Arc::new(RecordingCloudService::new(CloudDeps {
        http: vol.clone(),
        workspace: Arc::new(Ws),
        wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
        pause: Arc::new(NoPause),
    }));
    let summaries: Arc<Mutex<Vec<RecordingSummary>>> = Arc::default();
    let sink = summaries.clone();
    let adapter = Arc::new(CloudAdapter::new(
        service,
        Arc::new(MemorySettingsStore::new(RecordingSettings::default())),
        Arc::new(LocalFake {
            dir: dir.clone(),
            active,
        }),
        Arc::new(move |s: &RecordingSummary| sink.lock().unwrap().push(s.clone())),
        Arc::new(|| 1_795_100_700_000i64),
    ));
    let bytes = (0..parts)
        .map(|i| real_part(2 + i, 200.0 + 50.0 * i as f32))
        .collect();
    Rig {
        vol,
        adapter,
        summaries,
        dir,
        bytes,
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Rig {
    /// Write part `i` (0-based) to the temp dir as the capture would.
    fn write_part(&self, i: usize) -> LocalPart {
        let path = self.dir.join(format!("{NAME}.part{:02}.opus", i + 1));
        std::fs::write(&path, &self.bytes[i]).unwrap();
        LocalPart {
            path,
            duration_s: stitch::duration_s(&self.bytes[i]).unwrap(),
            held: Vec::new(),
        }
    }

    fn ctx(&self, recording: bool) -> LiveContext {
        LiveContext {
            profile: P.into(),
            job: RecordingJob {
                root: ROOT.into(),
                id: "rec-1".into(),
                name: NAME.into(),
                started_at: "2026-11-20T09:00:00Z".into(),
                instructions: "Write short notes.".into(),
                transcribe_endpoint: "system.ai.gemini-3-8-flash".into(),
                notes_endpoint: "system.ai.claude-sonnet-5-5".into(),
                mog: "recording".into(),
                parts: Vec::new(),
            },
            started_at_ms: 1_795_100_400_000,
            recording,
        }
    }

    fn job_json(&self) -> JobManifest {
        let v = self.vol.vol.lock().unwrap();
        serde_json::from_slice(&v.files[&format!("{FOLDER}/{NAME}.job.json")]).unwrap()
    }
}

/// Run the worker with a script. `step(i, waited)` runs before the i-th receive and may change
/// the world (go offline, inspect the volume); it returns what the worker receives.
fn run(
    r: &Rig,
    ctx: &LiveContext,
    mut step: impl FnMut(usize, Option<Duration>) -> Recv,
) -> Option<RecordingSummary> {
    let mut i = 0;
    r.adapter.run_live(ctx, &mut |wait| {
        let out = step(i, wait);
        i += 1;
        out
    })
}

// ---------- tests ----------

#[test]
fn rec_parts_each_part_uploads_while_the_call_goes_on_and_the_last_at_the_stop() {
    let r = rig("order", 3, vec![]);
    let p1 = r.write_part(0);
    let p2 = r.write_part(1);
    let p3 = r.write_part(2);
    r.vol.vol.lock().unwrap().watch = vec![p1.path.clone(), p2.path.clone(), p3.path.clone()];
    let mut queue: VecDeque<LiveMsg> = VecDeque::from([
        LiveMsg::Part(p1.clone()),
        LiveMsg::Part(p2.clone()),
        LiveMsg::Finished(vec![p3.clone()]),
    ]);
    let (a1, a2) = (p1.path.clone(), p2.path.clone());
    let a3 = p3.path.clone();
    let rig_ref = &r;
    let out = run(&r, &r.ctx(true), |i, wait| {
        assert_eq!(wait, None, "no failure: the worker waits for a message");
        // While the call goes on, part 1 is already in the volume and gone from the disk.
        if i == 1 {
            assert!(!a1.exists(), "part 1 deleted after its upload");
            assert!(a3.exists());
            let m = rig_ref.job_json();
            assert_eq!(m.status, Status::Recording, "job.json says recording");
            assert_eq!(m.parts.len(), 1);
            assert!(m.parts[0].uploaded);
        }
        if i == 2 {
            assert!(!a2.exists());
        }
        Recv::Msg(queue.pop_front().unwrap())
    })
    .unwrap();
    assert_eq!(out.status, RecordingStatus::Done, "{out:?}");
    assert_eq!(out.parts_uploaded, Some(3));
    assert_eq!(out.parts_total, Some(3));

    let v = r.vol.vol.lock().unwrap();
    // Order: for every part, upload, then job.json with that part marked, then delete.
    let parts_log: Vec<&String> = v
        .log
        .iter()
        .filter(|l| l.starts_with("upload:") || l.starts_with("job:Recording"))
        .collect();
    let ups: Vec<&str> = parts_log.iter().map(|s| s.as_str()).collect();
    let first_up = ups.iter().position(|l| l.ends_with("part01.opus")).unwrap();
    assert_eq!(ups[first_up + 1], "job:Recording:1");
    let second_up = ups.iter().position(|l| l.ends_with("part02.opus")).unwrap();
    assert_eq!(ups[second_up + 1], "job:Recording:2");
    assert!(first_up < second_up);
    assert!(
        v.local_at_upload.iter().all(|e| *e),
        "the local part exists at its upload"
    );
    assert!(
        v.local_at_job.iter().any(|e| *e),
        "the local part still exists when job.json is written (delete comes last)"
    );
    assert!(v.local_at_job.len() >= 3);

    // ONE transcription over the whole call: one request with the joined audio.
    let transcribe: Vec<&Value> = v
        .model_calls
        .iter()
        .filter(|c| c["model"].as_str().unwrap().contains("gemini"))
        .collect();
    assert_eq!(transcribe.len(), 1, "exactly one transcribe request");
    let url = transcribe[0]["messages"][0]["content"][1]["audio_url"]["url"]
        .as_str()
        .unwrap();
    let refs: Vec<&[u8]> = r.bytes.iter().map(Vec::as_slice).collect();
    let joined = stitch::join(&refs).unwrap();
    assert_eq!(
        url,
        format!("data:audio/ogg;base64,{}", base64(&joined)),
        "the request holds the parts joined in order"
    );
    assert!(!transcribe[0].to_string().contains("continues a call"));
    assert_eq!(v.model_calls.len(), 2, "one transcribe, one notes");
    // The result files are in the volume; the audio parts stay as parts.
    assert!(v.files.contains_key(&format!("{FOLDER}/{NAME}.notes.md")));
    assert!(v
        .files
        .contains_key(&format!("{FOLDER}/{NAME}.part03.opus")));
    drop(v);
    let m = r.job_json();
    assert_eq!(m.status, Status::Done);
    assert_eq!(m.parts.len(), 3);
    assert!(!a3.exists(), "the last part is deleted after its upload");
}

#[test]
fn rec_parts_a_network_failure_during_the_call_keeps_parts_and_recovers() {
    let r = rig("net", 3, vec![]);
    let p1 = r.write_part(0);
    let p2 = r.write_part(1);
    let p3 = r.write_part(2);
    let mut waits: Vec<Option<Duration>> = Vec::new();
    let vol = r.vol.clone();
    let (a1, a2) = (p1.path.clone(), p2.path.clone());
    let sums = r.summaries.clone();
    let out = run(&r, &r.ctx(true), |i, wait| {
        waits.push(wait);
        match i {
            0 => {
                vol.offline.store(true, Ordering::SeqCst);
                Recv::Msg(LiveMsg::Part(p1.clone()))
            }
            // Offline: the worker asked for a short wait. The part stays on the disk.
            1 => {
                assert!(a1.exists(), "an upload that failed keeps the part");
                Recv::Msg(LiveMsg::Part(p2.clone()))
            }
            2 | 3 => Recv::Timeout,
            4 => {
                assert!(a1.exists() && a2.exists(), "still offline: parts stay");
                // The live row shows "waiting for network": status recording + network.
                let last = sums.lock().unwrap().last().cloned().unwrap();
                assert_eq!(last.status, RecordingStatus::Recording);
                assert_eq!(last.error_kind, Some(RecordingErrorKind::Network));
                assert_eq!(last.parts_total, Some(3), "2 closed parts and the open one");
                assert_eq!(last.parts_uploaded, Some(0));
                vol.offline.store(false, Ordering::SeqCst);
                Recv::Timeout
            }
            // Back online: both parts went out in order, with no new message needed.
            5 => {
                assert!(!a1.exists() && !a2.exists());
                let last = sums.lock().unwrap().last().cloned().unwrap();
                assert_eq!(last.error_kind, None, "the network mark is gone");
                Recv::Msg(LiveMsg::Finished(vec![p3.clone()]))
            }
            _ => panic!("no more messages"),
        }
    })
    .unwrap();
    assert_eq!(out.status, RecordingStatus::Done, "{out:?}");
    // The waits: none while it was fine, then 5 s, 10 s, 20 s (a doubling backoff).
    assert_eq!(waits[0], None);
    assert_eq!(waits[1], Some(Duration::from_secs(5)));
    assert_eq!(waits[2], Some(Duration::from_secs(10)));
    assert_eq!(waits[3], Some(Duration::from_secs(20)));
    let v = r.vol.vol.lock().unwrap();
    let ups: Vec<&String> = v.log.iter().filter(|l| l.starts_with("upload:")).collect();
    assert_eq!(ups.len(), 3, "each part was uploaded once");
}

#[test]
fn rec_parts_backoff_doubles_to_a_cap_and_resets() {
    let mut b = Backoff::new();
    let got: Vec<u64> = (0..6).map(|_| b.advance().as_secs()).collect();
    assert_eq!(got, [5, 10, 20, 40, 60, 60]);
    b.reset();
    assert_eq!(b.advance(), Duration::from_secs(5));
}

#[test]
fn rec_parts_the_stop_waits_for_the_network_and_then_finishes() {
    let r = rig("stopnet", 2, vec![]);
    let p1 = r.write_part(0);
    let p2 = r.write_part(1);
    let vol = r.vol.clone();
    let out = run(&r, &r.ctx(true), |i, wait| match i {
        0 => Recv::Msg(LiveMsg::Part(p1.clone())),
        1 => {
            vol.offline.store(true, Ordering::SeqCst);
            Recv::Msg(LiveMsg::Finished(vec![p2.clone()]))
        }
        2 => {
            assert!(wait.is_some(), "after the stop it still retries");
            Recv::Timeout
        }
        3 => {
            vol.offline.store(false, Ordering::SeqCst);
            Recv::Timeout
        }
        _ => panic!("done by now"),
    })
    .unwrap();
    assert_eq!(out.status, RecordingStatus::Done, "{out:?}");
}

#[test]
fn rec_parts_a_volume_permission_error_at_the_stop_fails_the_job_and_keeps_the_audio() {
    let r = rig("deny", 1, vec![]);
    let p1 = r.write_part(0);
    let vol = r.vol.clone();
    vol.deny.store(true, Ordering::SeqCst);
    let out = run(&r, &r.ctx(true), |i, _| match i {
        0 => Recv::Msg(LiveMsg::Finished(vec![p1.clone()])),
        _ => panic!("no wait for a permanent error"),
    })
    .unwrap();
    assert_eq!(out.status, RecordingStatus::Failed);
    assert_eq!(out.error_kind, Some(RecordingErrorKind::VolumePermission));
    assert!(p1.path.exists(), "the local audio stays for Retry");
}

#[test]
fn rec_parts_a_failed_job_json_write_keeps_the_part_and_a_retry_sends_it_again() {
    // Service level: the order upload -> job.json -> delete. If job.json fails, no delete.
    let r = rig("jobfail3", 1, vec![]);
    let p1 = r.write_part(0);
    let svc = r.adapter.service();
    let ctx = r.ctx(true);
    let mut m = svc.live_begin(P, &ctx.job, &mut |_| {}).unwrap();
    r.vol.fail_job_writes.store(1, Ordering::SeqCst);
    let e = svc.live_upload_part(P, &ctx.job, &mut m, &p1, &mut |_| {});
    assert!(e.is_err());
    assert!(p1.path.exists(), "job.json was not written: the part stays");
    assert!(
        !m.parts[0].uploaded,
        "the part does not count as uploaded until job.json says so"
    );
    // The retry: the part is in the volume already (409 counts), job.json now works.
    svc.live_upload_part(P, &ctx.job, &mut m, &p1, &mut |_| {})
        .unwrap();
    assert!(!p1.path.exists());
    assert!(m.parts[0].uploaded);
    assert!(r.job_json().parts[0].uploaded);
}

#[test]
fn rec_parts_crash_recovery_resumes_the_parts_left_on_disk() {
    // A crash after part 1 went out: the volume has part 1 and job.json (status recording).
    // The disk has parts 2 and 3 (3 was open: no end of stream). At launch the worker resumes.
    let r = rig("crash", 3, vec![]);
    let p1 = r.write_part(0);
    {
        let svc = r.adapter.service();
        let ctx = r.ctx(true);
        let mut m = svc.live_begin(P, &ctx.job, &mut |_| {}).unwrap();
        svc.live_upload_part(P, &ctx.job, &mut m, &p1, &mut |_| {})
            .unwrap();
    }
    let p2 = r.write_part(1);
    // The open part at the crash: long enough to hold complete pages, with a torn end.
    let mut r = r;
    r.bytes[2] = real_part(14, 330.0);
    let torn_len = r.bytes[2].len() - 30;
    let p3path = r.dir.join(format!("{NAME}.part03.opus"));
    std::fs::write(&p3path, &r.bytes[2][..torn_len]).unwrap();
    let p3 = LocalPart {
        path: p3path.clone(),
        duration_s: 0.0,
        held: Vec::new(),
    };
    let uploads_before = r
        .vol
        .vol
        .lock()
        .unwrap()
        .log
        .iter()
        .filter(|l| l.starts_with("upload:"))
        .count();
    assert_eq!(uploads_before, 1);

    let mut ctx = r.ctx(false);
    ctx.job.id = "rec-1".into();
    let out = run(&r, &ctx, |i, _| match i {
        0 => Recv::Msg(LiveMsg::Finished(vec![p2.clone(), p3.clone()])),
        _ => panic!("done"),
    })
    .unwrap();
    assert_eq!(out.status, RecordingStatus::Done, "{out:?}");
    let v = r.vol.vol.lock().unwrap();
    let ups: Vec<&String> = v.log.iter().filter(|l| l.starts_with("upload:")).collect();
    assert_eq!(ups.len(), 3, "part 1 is not sent twice: {ups:?}");
    assert!(!p2.path.exists() && !p3path.exists());
    let transcribe = v
        .model_calls
        .iter()
        .filter(|c| c["model"].as_str().unwrap().contains("gemini"))
        .count();
    assert_eq!(transcribe, 1, "one transcription over parts 1, 2 and 3");
    drop(v);
    let m = r.job_json();
    assert_eq!(m.parts.len(), 3);
    assert!(m.parts.iter().all(|p| p.uploaded));
    assert!(m.parts[2].duration_s > 0.0, "the torn part was measured");
}

#[test]
fn rec_parts_a_closed_sender_leaves_the_parts_for_the_launch_scan() {
    let r = rig("closed", 1, vec![]);
    let p1 = r.write_part(0);
    let vol = r.vol.clone();
    vol.offline.store(true, Ordering::SeqCst);
    let out = run(&r, &r.ctx(true), |i, _| match i {
        0 => Recv::Msg(LiveMsg::Part(p1.clone())),
        _ => Recv::Closed,
    });
    assert!(out.is_none());
    assert!(p1.path.exists());
    assert!(!r.adapter.is_live(NAME), "the registry is clean");
}

fn storage() -> RecordingStorage {
    RecordingStorage {
        kind: RecordingStorageKind::Volume,
        root: VolumePath::try_from(ROOT.to_owned()).unwrap(),
    }
}

fn put_job(r: &Rig, name: &str, status: Status, parts: &[(&str, bool)], started: &str) {
    let mut m = JobManifest::new(
        name,
        name,
        started,
        parts
            .iter()
            .map(|(f, up)| {
                let mut e = mog_core::recording_cloud::job::PartEntry::new(f, 300.0);
                e.uploaded = *up;
                e
            })
            .collect(),
        "system.ai.gemini-3-8-flash",
        "system.ai.claude-sonnet-5-5",
        started,
    );
    m.status = status;
    // The adapter's clock is 1_795_100_700_000: a job.json from just before is fresh.
    m.updated_at = mog_core::recording_cloud::job::iso_utc(1_795_100_700_000 - 1_000);
    if status == Status::Failed {
        m.error_kind = Some(mog_core::recording_cloud::errors::ErrorKind::Offline);
        m.error_stage = Some(mog_core::recording_cloud::errors::Stage::Transcribing);
    }
    let path = format!("{ROOT}/Bottle/Recordings/{name}/{name}.job.json");
    r.vol
        .vol
        .lock()
        .unwrap()
        .files
        .insert(path, serde_json::to_vec(&m).unwrap());
}

#[test]
fn rec_parts_the_list_merges_volume_local_and_live_rows_newest_first() {
    // Volume: done, failed, uploading 2/5 with no worker, and a stuck "recording".
    // Device: an old-build recording with no job.json, and one that capture owns now.
    let r = rig("list", 1, vec!["2026-11-20_1200".into()]);
    put_job(
        &r,
        "2026-11-18_0900",
        Status::Done,
        &[("a.part01.opus", true)],
        "2026-11-18T09:00:00Z",
    );
    put_job(
        &r,
        "2026-11-19_0900",
        Status::Failed,
        &[("b.part01.opus", true)],
        "2026-11-19T09:00:00Z",
    );
    put_job(
        &r,
        "2026-11-19_1000",
        Status::Uploading,
        &[
            ("c.part01.opus", true),
            ("c.part02.opus", true),
            ("c.part03.opus", false),
            ("c.part04.opus", false),
            ("c.part05.opus", false),
        ],
        "2026-11-19T10:00:00Z",
    );
    put_job(
        &r,
        "2026-11-19_1100",
        Status::Recording,
        &[("d.part01.opus", true)],
        "2026-11-19T11:00:00Z",
    );
    // A run that died in the transcribe step long ago: stuck, so Retry shows.
    put_job(
        &r,
        "2026-11-18_1300",
        Status::Transcribing,
        &[("e.part01.opus", true)],
        "2026-11-18T13:00:00Z",
    );
    {
        let mut v = r.vol.vol.lock().unwrap();
        let k = format!("{ROOT}/Bottle/Recordings/2026-11-18_1300/2026-11-18_1300.job.json");
        let mut m: JobManifest = serde_json::from_slice(&v.files[&k]).unwrap();
        m.updated_at = "2026-11-18T13:01:00Z".into();
        v.files.insert(k, serde_json::to_vec(&m).unwrap());
    }
    // Device only: no job.json.
    std::fs::write(r.dir.join("2026-11-17_0800.opus"), b"OggS").unwrap();
    std::fs::write(r.dir.join("2026-11-20_1200.part01.opus"), b"OggS").unwrap();

    let rows = r
        .adapter
        .list(&ProfileName::try_from(P.to_owned()).unwrap(), &storage())
        .unwrap();
    let by = |n: &str| {
        rows.iter()
            .find(|x| x.name == n)
            .unwrap_or_else(|| panic!("{n}"))
    };
    assert_eq!(by("2026-11-18_0900").status, RecordingStatus::Done);
    assert_eq!(by("2026-11-19_0900").status, RecordingStatus::Failed);
    let up = by("2026-11-19_1000");
    assert_eq!(up.status, RecordingStatus::Uploading);
    assert_eq!((up.parts_uploaded, up.parts_total), (Some(2), Some(5)));
    let dead = by("2026-11-18_1300");
    assert_eq!(dead.status, RecordingStatus::Failed);
    assert_eq!(dead.error_stage, Some(RecordingStatus::Transcribing));
    // A job.json that says "recording" with no capture and no worker is a crash: Retry.
    let stuck = by("2026-11-19_1100");
    assert_eq!(stuck.status, RecordingStatus::Failed);
    assert_eq!(stuck.error_kind, Some(RecordingErrorKind::UploadFailed));
    // The old build: on the device only, with Upload.
    let old = by("2026-11-17_0800");
    assert!(old.on_device_only);
    assert_eq!(old.status, RecordingStatus::Failed);
    // Capture owns it, no worker: recording.
    assert_eq!(by("2026-11-20_1200").status, RecordingStatus::Recording);
    assert!(!by("2026-11-20_1200").on_device_only);
    // Newest first (by start time, the device-only ones by file time).
    let times: Vec<u64> = rows.iter().map(|x| x.started_at_ms).collect();
    let mut sorted = times.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(times, sorted);
}

#[test]
fn rec_parts_the_list_shows_a_live_recording_with_counts_and_the_network_mark() {
    let r = rig("listlive", 2, vec![]);
    let p1 = r.write_part(0);
    let p2 = r.write_part(1);
    let adapter = r.adapter.clone();
    let vol = r.vol.clone();
    let mut seen: Vec<RecordingSummary> = Vec::new();
    let out = run(&r, &r.ctx(true), |i, _| match i {
        0 => Recv::Msg(LiveMsg::Part(p1.clone())),
        1 => {
            let rows = adapter
                .list(&ProfileName::try_from(P.to_owned()).unwrap(), &storage())
                .unwrap();
            seen.extend(rows);
            vol.offline.store(true, Ordering::SeqCst);
            Recv::Msg(LiveMsg::Part(p2.clone()))
        }
        2 => {
            vol.offline.store(false, Ordering::SeqCst);
            Recv::Msg(LiveMsg::Finished(vec![]))
        }
        _ => panic!("done"),
    });
    assert!(out.is_some());
    let live = seen.iter().find(|x| x.name == NAME).unwrap();
    assert_eq!(live.status, RecordingStatus::Recording);
    assert_eq!(live.parts_uploaded, Some(1));
    assert_eq!(
        live.parts_total,
        Some(2),
        "one closed part and the open one"
    );
}

#[test]
fn rec_parts_retry_of_a_device_only_recording_uploads_it() {
    // "Not uploaded" -> Upload is `recording_retry`: it runs the whole pipeline for parts that
    // have no job.json in the volume.
    let r = rig("upload", 2, vec![]);
    r.write_part(0);
    r.write_part(1);
    let id = mog_core::types::RecordingId::try_from(NAME.to_owned()).unwrap();
    let settings = RecordingSettings {
        storage: Some(storage()),
        ..RecordingSettings::default()
    };
    let adapter = Arc::new(CloudAdapter::new(
        r.adapter.service().clone(),
        Arc::new(MemorySettingsStore::new(settings)),
        Arc::new(LocalFake {
            dir: r.dir.clone(),
            active: vec![],
        }),
        Arc::new(|_: &RecordingSummary| {}),
        Arc::new(|| 1_795_100_700_000i64),
    ));
    let out = adapter
        .retry(
            &ProfileName::try_from(P.to_owned()).unwrap(),
            &storage(),
            &id,
        )
        .unwrap();
    assert_eq!(out.status, RecordingStatus::Done, "{out:?}");
    assert_eq!(out.parts_total, Some(2));
    let v = r.vol.vol.lock().unwrap();
    let transcribe = v
        .model_calls
        .iter()
        .filter(|c| c["model"].as_str().unwrap().contains("gemini"))
        .count();
    assert_eq!(transcribe, 1, "one transcription over both parts");
}
