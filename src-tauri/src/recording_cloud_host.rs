//! The cloud half of call recording in the app (ADR 0103, PR C): one
//! [`RecordingCloudService`] on its own R7 [`ReqwestHttpTransport`], over the one R6 of
//! [`crate::mcp_host`] (the token never leaves R6), the managed T7 profile hosts and the one
//! [`SharedRateBucket`] (G-09a). It points the settings store of the `recording_*` component at
//! `<app config dir>/recording-settings.json` (no secret). Links open with the OS opener, only
//! for a Catalog Explorer URL on the profile host.
//!
//! The service is blocking: the host runs it on a worker thread. The `recording_*` commands of
//! the contract PR (`issue/rec-0-contract`) route here once it lands. The caller passes the home
//! profile the app already uses; this module never picks one.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use mog_core::clock::SystemWallClock;
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::{ProfileHosts, ReqwestHttpTransport, ThreadPause};
use mog_core::ipc::events;
use mog_core::recording::{
    FinishedPart, PipelineRequest, RecordingComponent, RecordingSettingsStore,
};
use mog_core::recording_capture::chunker::PartFile;
use mog_core::recording_capture::session::{
    FinishedRecording, PartsHandoff, Snapshot, StartedRecording,
};
use mog_core::recording_cloud::cloud::{
    is_audio_file_of, CloudDeps, LocalPart, ProfileWorkspace, RecordingCloudService,
};
use mog_core::recording_cloud::contract::{CloudAdapter, LiveMsg, LocalAudio, Recv};
use mog_core::recording_cloud::layout::is_recording_name;
use mog_core::recording_cloud::opener::{open_link, SystemOpener, UrlOpener};
use mog_core::recording_cloud::settings_store::{JsonSettingsStore, SETTINGS_FILE};
use mog_core::recording_cloud::sidecar::{self, Sidecar};
use mog_core::types::{Instructions, RecordingId};
use tauri::Manager;

use crate::ipc::BrainChannel;
use crate::mcp_host::McpHost;
use crate::profiles_host::ProfilesHost;
use crate::rate_bucket_host::SharedRateBucket;
use crate::recording_host::{HandoffSlot, RecordingHost};

const ID: &str = "recording_cloud_host";

fn missing(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// The managed recording cloud parts.
pub struct RecordingCloudHost {
    /// The pipeline, storage and list service.
    pub service: Arc<RecordingCloudService>,
    /// The contract seam over the service (the `recording_*` component and the hand-off).
    pub adapter: Arc<CloudAdapter>,
    opener: Arc<dyn UrlOpener>,
    /// R7 of the service: the host keeps it alive.
    _transport: Arc<ReqwestHttpTransport>,
}

impl std::fmt::Debug for RecordingCloudHost {
    /// Shows nothing: no host, profile or path.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingCloudHost").finish_non_exhaustive()
    }
}

impl RecordingCloudHost {
    /// Open the Catalog Explorer folder of recording `name` in `root` for `profile`.
    pub fn open_folder(&self, profile: &str, root: &str, name: &str) -> CoreResult<()> {
        let url = self
            .service
            .link(profile, root, name)
            .map_err(|_| missing("link_failed"))?;
        let host = self.service.host_url(profile)?;
        open_link(&*self.opener, &host, &url)
    }
}

/// The temp audio folder under the app data dir (not the cache dir, so the OS does not purge a
/// recording that waits for Retry): `<app data>/recordings/<id>/<name>.opus`.
pub const TEMP_DIR: &str = "recordings";

/// The local temp audio files of recording `name` still on the device, in part order. PR B
/// keeps each recording in `<root>/<id>/`, so every folder of `root` is searched.
pub fn temp_parts_of(root: Option<&std::path::Path>, name: &str) -> Vec<LocalPart> {
    temp_parts_in(
        &root
            .map(std::path::Path::to_path_buf)
            .into_iter()
            .collect::<Vec<_>>(),
        name,
    )
}

/// As [`temp_parts_of`] over several roots (the app data dir, and the cache dir of builds
/// before #592).
pub fn temp_parts_in(roots: &[std::path::PathBuf], name: &str) -> Vec<LocalPart> {
    let mut files: Vec<std::path::PathBuf> = roots
        .iter()
        .flat_map(|root| {
            std::fs::read_dir(root)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|d| d.is_dir())
                .flat_map(|d| {
                    std::fs::read_dir(d)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                })
        })
        .filter(|p| {
            p.file_name()
                .and_then(|f| f.to_str())
                .is_some_and(|f| is_audio_file_of(name, f))
        })
        .collect();
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));
    files
        .into_iter()
        .map(|path| LocalPart {
            path,
            duration_s: 0.0,
            held: Vec::new(),
        })
        .collect()
}

/// The recording name encoded by a local temp audio file name (`<name>.<ext>` or
/// `<name>.partNN.<ext>`), or `None` when the file is not one (ADR 0103 decision 7).
fn recording_name_of_file(file: &str) -> Option<&str> {
    let (base, ext) = file.rsplit_once('.')?;
    if !matches!(ext, "opus" | "ogg" | "flac" | "wav") {
        return None;
    }
    let name = match base.rsplit_once(".part") {
        Some((n, digits)) if digits.len() == 2 && digits.bytes().all(|c| c.is_ascii_digit()) => n,
        _ => base,
    };
    is_recording_name(name).then_some(name)
}

/// The modified time of `path`, in epoch ms (`0` when it cannot be read).
fn modified_ms(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Every recording name with temp audio still on the device, and its earliest file's modified
/// time (epoch ms): the launch scan of ADR 0103 decision 7, run lazily on every `recording_list`
/// (PR B's per-ID folders under `root`, searched the same way as [`temp_parts_of`]).
pub fn temp_names_of(root: Option<&std::path::Path>) -> Vec<(String, u64)> {
    temp_names_in(
        &root
            .map(std::path::Path::to_path_buf)
            .into_iter()
            .collect::<Vec<_>>(),
    )
}

/// As [`temp_names_of`] over several roots. A name in two roots keeps the earliest time.
pub fn temp_names_in(roots: &[std::path::PathBuf]) -> Vec<(String, u64)> {
    let mut names: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    let dirs = roots
        .iter()
        .flat_map(|root| std::fs::read_dir(root).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|d| d.is_dir());
    for dir in dirs {
        let files = std::fs::read_dir(&dir).into_iter().flatten().flatten();
        for entry in files {
            let path = entry.path();
            let Some(name) = path
                .file_name()
                .and_then(|f| f.to_str())
                .and_then(recording_name_of_file)
            else {
                continue;
            };
            let t = modified_ms(&path);
            names
                .entry(name.to_owned())
                .and_modify(|cur| *cur = (*cur).min(t))
                .or_insert(t);
        }
    }
    names.into_iter().collect()
}

/// The temp dirs of `roots` that hold a sidecar, with the audio parts in each: the recordings
/// that a crash left and that can resume with no click.
pub fn resumable(
    roots: &[std::path::PathBuf],
) -> Vec<(std::path::PathBuf, Sidecar, Vec<LocalPart>)> {
    let mut out = Vec::new();
    let dirs = roots
        .iter()
        .flat_map(|root| std::fs::read_dir(root).into_iter().flatten().flatten())
        .map(|e| e.path())
        .filter(|d| d.is_dir());
    for dir in dirs {
        let Some(sc) = sidecar::read(&dir) else {
            continue;
        };
        let mut parts: Vec<LocalPart> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|f| f.to_str())
                    .is_some_and(|f| is_audio_file_of(&sc.name, f))
            })
            .map(|path| LocalPart {
                path,
                duration_s: 0.0,
                held: Vec::new(),
            })
            .collect();
        parts.sort_by(|a, b| a.path.file_name().cmp(&b.path.file_name()));
        if parts.is_empty() {
            continue;
        }
        out.push((dir, sc, parts));
    }
    out
}

/// Delete the sidecar of `dir` when no audio is left in it, then the dir if it is empty.
pub fn tidy_dir(dir: &std::path::Path) {
    let has_audio = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|f| recording_name_of_file(f).is_some())
        });
    if !has_audio {
        sidecar::remove(dir);
    }
}

/// The [`LocalAudio`] seam over the temp recordings folders: `<app data>/recordings/<id>/`, and
/// `<app cache>/recordings/<id>/` where builds before #592 left their audio.
pub struct TempAudio {
    roots: Vec<std::path::PathBuf>,
    /// The names capture still owns, or that finished but are not yet handed off (review of
    /// #592, BLOCKING B3): the scan and Retry both skip these.
    busy: Arc<dyn Fn() -> HashSet<String> + Send + Sync>,
}

impl TempAudio {
    /// The seam over `roots`; `busy` names the recordings capture or a worker owns now.
    pub fn new(
        roots: Vec<std::path::PathBuf>,
        busy: Arc<dyn Fn() -> HashSet<String> + Send + Sync>,
    ) -> Self {
        Self { roots, busy }
    }
}

impl LocalAudio for TempAudio {
    fn parts(&self, name: &str) -> Vec<LocalPart> {
        temp_parts_in(&self.roots, name)
    }
    fn names(&self) -> Vec<(String, u64)> {
        temp_names_in(&self.roots)
    }
    fn active(&self) -> HashSet<String> {
        (self.busy)()
    }
    fn sidecar(&self, name: &str) -> Option<Sidecar> {
        resumable(&self.roots)
            .into_iter()
            .find(|(_, s, _)| s.name == name)
            .map(|(_, s, _)| s)
    }
    fn tidy(&self, name: &str) {
        for root in &self.roots {
            for dir in std::fs::read_dir(root).into_iter().flatten().flatten() {
                let dir = dir.path();
                if sidecar::read(&dir).is_some_and(|s| s.name == name) {
                    tidy_dir(&dir);
                    let _ = std::fs::remove_dir(&dir);
                }
            }
        }
    }
}

/// The recording names a hand-off has taken from PR B but has not yet reached the pipeline's
/// own in-flight guard (the gap between [`PipelineHandoff::finished`] and
/// `CloudAdapter::run_pipeline`'s `begin`). Review of #592, BLOCKING B3.
#[derive(Default)]
pub struct ActiveNames {
    names: Mutex<HashSet<String>>,
}

impl ActiveNames {
    fn mark(&self, name: &str) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned());
    }
    fn release(&self, name: &str) {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }
    fn get(&self) -> HashSet<String> {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    /// Is `name` marked active now? For tests.
    pub fn contains(&self, name: &str) -> bool {
        self.names
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(name)
    }
}

/// The cloud end of PR B's hand-off: each finished recording runs the pipeline exactly once,
/// on its own thread, with the profile and the instructions of its `recording_start`.
pub struct PipelineHandoff {
    run: Arc<dyn Fn(PipelineRequest, u64) + Send + Sync>,
    settings: Arc<dyn RecordingSettingsStore>,
    seen: Mutex<HashSet<String>>,
    /// Marked for a recording's name from the moment a run is dispatched until `run` returns,
    /// so the cloud-side scan and Retry never touch it in between (review of #592, BLOCKING
    /// B3).
    active: Arc<ActiveNames>,
}

impl PipelineHandoff {
    /// A hand-off that calls `run(request, started_at_ms)` on a worker thread.
    pub fn new(
        run: Arc<dyn Fn(PipelineRequest, u64) + Send + Sync>,
        settings: Arc<dyn RecordingSettingsStore>,
        active: Arc<ActiveNames>,
    ) -> Self {
        Self {
            run,
            settings,
            seen: Mutex::new(HashSet::new()),
            active,
        }
    }

    /// The pipeline request of `rec`, or `None` when it cannot run now: no audio (the empty
    /// temp dir is deleted), no home profile, or no storage in Settings. Then the audio stays
    /// in the temp dir for Retry.
    pub fn request(&self, rec: &FinishedRecording) -> Option<PipelineRequest> {
        if rec.parts.is_empty() {
            remove_dir_if_empty(&rec.dir);
            return None;
        }
        let settings = self.settings.load().ok()?;
        let profile = rec.profile.clone().filter(|p| !p.trim().is_empty())?;
        let storage = settings.storage.clone()?;
        let id = RecordingId::try_from(rec.id.clone()).ok()?;
        let instructions = rec
            .instructions
            .clone()
            .and_then(|i| Instructions::try_from(i).ok())
            .unwrap_or_else(|| settings.preset_instructions.clone());
        Some(PipelineRequest {
            profile,
            storage,
            id,
            name: rec.name.clone(),
            parts: rec
                .parts
                .iter()
                .map(|p| FinishedPart {
                    index: p.index,
                    path: p.path.clone(),
                    offset_s: p.offset_ms as f64 / 1000.0,
                    duration_s: p.samples as f64 / f64::from(SAMPLE_RATE),
                })
                .collect(),
            instructions,
            settings,
        })
    }
}

/// The capture sample rate of the parts (16 kHz mono).
const SAMPLE_RATE: u32 = 16_000;

/// Delete `dir` only when it is empty (after every part was uploaded and deleted).
pub fn remove_dir_if_empty(dir: &std::path::Path) {
    tidy_dir(dir);
    let _ = std::fs::remove_dir(dir);
}

/// Releases `name` from `active` when dropped: a panic inside the dispatched run still
/// unwinds through this guard, so the name is never left marked active forever (review of
/// #592, LOW).
struct ActiveGuard {
    active: Arc<ActiveNames>,
    name: String,
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.active.release(&self.name);
    }
}

impl PartsHandoff for PipelineHandoff {
    fn finished(&self, rec: FinishedRecording) {
        let first = self
            .seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(rec.id.clone());
        if !first {
            return;
        }
        let Some(req) = self.request(&rec) else {
            return;
        };
        self.active.mark(&rec.name);
        let run = self.run.clone();
        let dir = rec.dir.clone();
        let started = rec.started_at_ms.max(0) as u64;
        let guard = ActiveGuard {
            active: self.active.clone(),
            name: rec.name.clone(),
        };
        let spawned = std::thread::Builder::new()
            .name("bottle-rec-pipeline".into())
            .spawn(move || {
                let _guard = guard;
                run(req, started);
                remove_dir_if_empty(&dir);
            });
        if spawned.is_err() {
            // The thread never started, so its guard never ran: release the mark here (a
            // failed spawn must not leave the name stuck active forever).
            self.active.release(&rec.name);
        }
    }
}

/// The channel side of one live worker: parts go in, the worker uploads them.
type LiveTx = std::sync::mpsc::Sender<LiveMsg>;

/// The local part of a part file (its length is known from the samples).
fn local_part(p: &PartFile) -> LocalPart {
    LocalPart {
        path: p.path.clone(),
        duration_s: p.samples as f64 / f64::from(SAMPLE_RATE),
        held: p.held.clone(),
    }
}

/// Drives the worker of one recording off a channel: `None` waits for a message with no limit;
/// `Some(d)` waits at most `d` (the retry wait after a failed upload).
fn recv_from(rx: &std::sync::mpsc::Receiver<LiveMsg>, wait: Option<std::time::Duration>) -> Recv {
    use std::sync::mpsc::RecvTimeoutError;
    match wait {
        None => rx.recv().map_or(Recv::Closed, Recv::Msg),
        Some(d) => match rx.recv_timeout(d) {
            Ok(m) => Recv::Msg(m),
            Err(RecvTimeoutError::Timeout) => Recv::Timeout,
            // The stop message came and the sender is gone: nothing more arrives, but the
            // worker still retries a failed upload. Wait the time, then say it ran out.
            Err(RecvTimeoutError::Disconnected) => {
                std::thread::sleep(d);
                Recv::Timeout
            }
        },
    }
}

/// Upload in parts during the call (ADR 0103 follow-up). `started` opens one worker thread for
/// the recording (only for a recording that `recording_start` let through: the knob is on and
/// the consent notice is acknowledged), `part_ready` sends each closed part to it, and
/// `finished` tells it to stop. The worker uploads, retries in the background, and at the stop
/// runs the one transcription. A recording that has no worker (no storage in Settings, no home
/// profile) goes the old way: the pipeline runs at the stop ([`PipelineHandoff`]).
pub struct LiveHandoff {
    adapter: Arc<CloudAdapter>,
    settings: Arc<dyn RecordingSettingsStore>,
    fallback: PipelineHandoff,
    active: Arc<ActiveNames>,
    workers: Mutex<std::collections::HashMap<String, LiveTx>>,
}

impl LiveHandoff {
    /// A hand-off over `adapter`, with `fallback` for a recording that has no worker.
    pub fn new(
        adapter: Arc<CloudAdapter>,
        settings: Arc<dyn RecordingSettingsStore>,
        fallback: PipelineHandoff,
        active: Arc<ActiveNames>,
    ) -> Self {
        Self {
            adapter,
            settings,
            fallback,
            active,
            workers: Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Run one worker on its own thread. `dir` is the temp dir of the recording: on a `done`
    /// job the sidecar and the dir go; on any other end the sidecar goes too (the audio stays
    /// for Retry, and the launch scan does not run the same failure again).
    fn spawn_worker(
        &self,
        ctx: mog_core::recording_cloud::contract::LiveContext,
        dir: std::path::PathBuf,
        rx: std::sync::mpsc::Receiver<LiveMsg>,
    ) -> bool {
        let name = ctx.job.name.clone();
        self.active.mark(&name);
        let guard = ActiveGuard {
            active: self.active.clone(),
            name: name.clone(),
        };
        let adapter = self.adapter.clone();
        let spawned = std::thread::Builder::new()
            .name("bottle-rec-live".into())
            .spawn(move || {
                let _guard = guard;
                // A result (done or failed) ends the recording's live state: the sidecar goes,
                // and the dir if no audio is left. A failed job keeps its audio for Retry.
                if adapter
                    .run_live(&ctx, &mut |wait| recv_from(&rx, wait))
                    .is_some()
                {
                    sidecar::remove(&dir);
                }
            });
        if spawned.is_err() {
            self.active.release(&name);
        }
        spawned.is_ok()
    }
}

impl PartsHandoff for LiveHandoff {
    fn started(&self, rec: &StartedRecording) {
        let Ok(settings) = self.settings.load() else {
            return;
        };
        let Some(profile) = rec.profile.clone().filter(|p| !p.trim().is_empty()) else {
            return;
        };
        // The knob gates everything, also the upload during the call.
        if !(settings.enabled && settings.consent_ack) {
            return;
        }
        let Some(storage) = settings.storage.clone() else {
            return;
        };
        let (Ok(id), true) = (
            RecordingId::try_from(rec.id.clone()),
            is_recording_name(&rec.name),
        ) else {
            return;
        };
        let instructions = rec
            .instructions
            .clone()
            .and_then(|i| Instructions::try_from(i).ok())
            .unwrap_or_else(|| settings.preset_instructions.clone());
        // The sidecar lets a crash resume with no click. No token, no audio.
        let _ = sidecar::write(
            &rec.dir,
            &Sidecar::new(
                &rec.id,
                &rec.name,
                rec.started_at_ms,
                &profile,
                rec.instructions.as_deref(),
            ),
        );
        let ctx = self.adapter.job_for(
            &profile,
            &storage,
            &id,
            &rec.name,
            rec.started_at_ms.max(0) as u64,
            &instructions,
            &settings,
            true,
        );
        let (tx, rx) = std::sync::mpsc::channel();
        if self.spawn_worker(ctx, rec.dir.clone(), rx) {
            self.workers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(rec.id.clone(), tx);
        }
    }

    fn part_ready(&self, id: &str, part: &PartFile) {
        let tx = self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(id)
            .cloned();
        if let Some(tx) = tx {
            let _ = tx.send(LiveMsg::Part(local_part(part)));
        }
    }

    fn finished(&self, rec: FinishedRecording) {
        let tx = self
            .workers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&rec.id);
        match tx {
            Some(tx) => {
                if rec.parts.is_empty() {
                    // No audio came in: nothing to upload. Dropping the sender ends the worker
                    // with no result; the empty dir and the sidecar go.
                    drop(tx);
                    remove_dir_if_empty(&rec.dir);
                    sidecar::remove(&rec.dir);
                    return;
                }
                let parts = rec.parts.iter().map(local_part).collect();
                let _ = tx.send(LiveMsg::Finished(parts));
            }
            None => self.fallback.finished(rec),
        }
    }
}

/// Resume the recordings that a crash left (they have a sidecar): each one gets a worker that
/// sends the parts on disk and runs the one transcription. It runs only with the knob on, the
/// consent acknowledged and a storage chosen. A recording with no sidecar (an old build)
/// stays on the device as "Not uploaded" until the user clicks Upload.
pub fn resume_left(
    live: &LiveHandoff,
    roots: &[std::path::PathBuf],
    busy: &HashSet<String>,
) -> usize {
    let Ok(settings) = live.settings.load() else {
        return 0;
    };
    let Some(storage) = settings.storage.clone() else {
        return 0;
    };
    if !(settings.enabled && settings.consent_ack) {
        return 0;
    }
    let mut n = 0;
    for (dir, sc, parts) in resumable(roots) {
        if busy.contains(&sc.name) || live.adapter.is_live(&sc.name) {
            continue;
        }
        let Ok(id) = RecordingId::try_from(sc.id.clone()) else {
            continue;
        };
        let instructions = sc
            .instructions
            .clone()
            .and_then(|i| Instructions::try_from(i).ok())
            .unwrap_or_else(|| settings.preset_instructions.clone());
        let ctx = live.adapter.job_for(
            &sc.profile,
            &storage,
            &id,
            &sc.name,
            sc.started_at_ms.max(0) as u64,
            &instructions,
            &settings,
            false,
        );
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(LiveMsg::Finished(parts));
        drop(tx);
        if live.spawn_worker(ctx, dir, rx) {
            n += 1;
        }
    }
    n
}

/// The names the cloud-side scan and Retry must leave alone right now, from every source
/// (review of #592, BLOCKING B3 and the LOW ordering item): the recording running now, a stop
/// still finishing one, a hand-off waiting for the cloud side to connect, and a pipeline run in
/// flight. Call sites must read `active` last, after `recording` and `finishing`: see
/// `install`'s `busy` closure.
fn merge_busy_names(
    recording: Option<String>,
    finishing: Option<String>,
    pending: Vec<String>,
    active: HashSet<String>,
) -> HashSet<String> {
    let mut names: HashSet<String> = recording.into_iter().chain(finishing).collect();
    names.extend(pending);
    names.extend(active);
    names
}

/// Build the recording cloud host. [`crate::mcp_host::install`] and
/// [`crate::profiles_host::install`] run first.
pub fn install(handle: &tauri::AppHandle<tauri::Wry>) -> CoreResult<()> {
    let bucket = handle
        .try_state::<SharedRateBucket>()
        .ok_or(missing("rate_bucket_missing"))?
        .0
        .clone();
    let hosts: Arc<dyn ProfileHosts> = handle
        .try_state::<ProfilesHost>()
        .ok_or(missing("profiles_missing"))?
        .0
        .clone();
    let tokens = handle
        .try_state::<McpHost>()
        .ok_or(missing("token_broker_missing"))?
        .token_broker();
    let transport = Arc::new(ReqwestHttpTransport::new(
        hosts.clone(),
        tokens.clone(),
        bucket,
    )?);
    let service = Arc::new(RecordingCloudService::new(CloudDeps {
        http: transport.clone(),
        workspace: Arc::new(ProfileWorkspace { hosts, tokens }),
        wall: Arc::new(SystemWallClock),
        pause: Arc::new(ThreadPause),
    }));
    // The settings file of the `recording_*` component (lib.rs builds it before the config
    // dir is known). No config dir: the defaults stay (knob off) and writes fail.
    if let (Some(store), Ok(dir)) = (
        handle.try_state::<Arc<JsonSettingsStore>>(),
        handle.path().app_config_dir(),
    ) {
        store.set_path(dir.join(SETTINGS_FILE));
    }
    let store: Arc<dyn RecordingSettingsStore> = match handle.try_state::<Arc<JsonSettingsStore>>()
    {
        Some(s) => s.inner().clone(),
        None => Arc::new(JsonSettingsStore::deferred()),
    };
    // Where the temp audio lives: the app data dir, and the cache dir where builds before #592
    // left it. Both are scanned, so those recordings show as "Not uploaded".
    let temp_roots: Vec<std::path::PathBuf> = [
        handle.path().app_data_dir().ok(),
        handle.path().app_cache_dir().ok(),
    ]
    .into_iter()
    .flatten()
    .map(|d| d.join(TEMP_DIR))
    .collect();
    let active_names = Arc::new(ActiveNames::default());
    let busy: Arc<dyn Fn() -> HashSet<String> + Send + Sync> = {
        let active_names = active_names.clone();
        let app = handle.clone();
        Arc::new(move || {
            // Capture owns the name of the recording running right now (review of #592,
            // BLOCKING B3), or a stop still flushing the writer and running the hand-off
            // (`snapshot()` alone would read `Idle` there already).
            let (recording, finishing) = match app.try_state::<Arc<RecordingHost>>() {
                Some(host) => {
                    let recording = match host.session().snapshot() {
                        Snapshot::Recording(info) => Some(info.name),
                        Snapshot::Idle => None,
                    };
                    (recording, host.session().finishing_name())
                }
                None => (None, None),
            };
            // A recording that finished before the cloud side connected still waits in the
            // slot; it is not active capture, but it is not abandoned either.
            let pending = match app.try_state::<Arc<HandoffSlot>>() {
                Some(slot) => slot.pending().into_iter().map(|rec| rec.name).collect(),
                None => Vec::new(),
            };
            // Read `active_names` last (review of #592, LOW): it is the one source that
            // overlaps in time with `finishing`, and these reads are not atomic together, so
            // reading it last catches a hand-off that marks a name while this whole check is
            // still running, instead of risking a stale "not busy" from an earlier read.
            merge_busy_names(recording, finishing, pending, active_names.get())
        })
    };
    let local: Arc<dyn LocalAudio> = Arc::new(TempAudio {
        roots: temp_roots.clone(),
        busy,
    });
    let app = handle.clone();
    let on_job = Arc::new(move |s: &mog_core::types::RecordingSummary| {
        if let (Some(ch), Ok(v)) = (app.try_state::<BrainChannel>(), serde_json::to_value(s)) {
            ch.send(events::RECORDING_JOB, v);
        }
    });
    let adapter = Arc::new(CloudAdapter::new(
        service.clone(),
        store.clone(),
        local,
        on_job,
        Arc::new(|| {
            use mog_core::clock::WallClock;
            mog_core::clock::SystemWallClock.now_ms()
        }),
    ));
    // PR B's finished recordings run the pipeline once each (waiting ones go out now).
    if let Some(slot) = handle.try_state::<Arc<HandoffSlot>>() {
        let a = adapter.clone();
        let fallback = PipelineHandoff::new(
            Arc::new(move |req, started| {
                let _ = a.run_pipeline(&req, started);
            }),
            store.clone(),
            active_names.clone(),
        );
        let live = Arc::new(LiveHandoff::new(
            adapter.clone(),
            store.clone(),
            fallback,
            active_names.clone(),
        ));
        slot.connect(live.clone());
        // A crash left parts and sidecars: resume them in the background, off the setup path.
        let roots = temp_roots.clone();
        let _ = std::thread::Builder::new()
            .name("bottle-rec-resume".into())
            .spawn(move || {
                let _ = resume_left(&live, &roots, &HashSet::new());
            });
    }
    let opener: Arc<dyn UrlOpener> = Arc::new(SystemOpener);
    if let Some(component) = handle.try_state::<Arc<RecordingComponent>>() {
        let (svc, op) = (service.clone(), opener.clone());
        component.set_cloud(
            adapter.clone(),
            Arc::new(move |profile: &String, url: &str| {
                let host = svc.host_url(profile)?;
                open_link(&*op, &host, url)
            }),
        );
    }
    handle.manage(RecordingCloudHost {
        service,
        adapter,
        opener,
        _transport: transport,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_busy_names_unions_every_source() {
        let active: HashSet<String> = ["d".to_owned()].into_iter().collect();
        let names = merge_busy_names(
            Some("a".to_owned()),
            Some("b".to_owned()),
            vec!["c".to_owned()],
            active,
        );
        let expected: HashSet<String> =
            ["a", "b", "c", "d"].into_iter().map(String::from).collect();
        assert_eq!(names, expected);
    }

    #[test]
    fn merge_busy_names_with_only_the_active_mark() {
        // The case the ordering fix guards: nothing from capture or the slot, only a name the
        // pipeline's own in-flight guard marked.
        let active: HashSet<String> = ["2026-11-19_0905".to_owned()].into_iter().collect();
        let names = merge_busy_names(None, None, Vec::new(), active.clone());
        assert_eq!(names, active);
    }
}
