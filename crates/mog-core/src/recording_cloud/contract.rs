//! The contract seam of PR C (ADR 0103): [`CloudAdapter`] implements
//! [`crate::recording::RecordingCloud`] over [`RecordingCloudService`], and maps the service's
//! kinds, manifests and transcripts to the contract wire types. The recording ID on the wire is
//! the folder name (`YYYY-MM-DD_HHMM`, with `_N` on a clash), which fits `RecordingId`.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::recording::{
    FinishedPart, PipelineRequest, RecordingCloud, RecordingResult, RecordingSettingsStore,
};
use crate::types::{
    EndpointName, Instructions, ProfileName, RecordingDetail, RecordingErrorKind, RecordingId,
    RecordingSegment, RecordingStatus, RecordingStorage, RecordingStorageKind, RecordingSummary,
    StorageProbe, UcName, VolumePath,
};

use super::cloud::{LocalPart, RecordingCloudService, RecordingJob, StorageProbe as Probe};
use super::errors::{ErrorKind, RecordingError, Stage};
use super::job::{iso_utc, parse_iso_utc_ms, JobManifest, Status};
use super::layout::{
    instructions_file, job_file, notes_file, transcript_json_file, transcript_md_file,
};
use super::settings_store::Storage;
use super::sidecar::Sidecar;
use super::transcript::{render_markdown, Segment, TranscriptDoc};

/// The local temp audio still on the device (PR B's per-ID folders under the app data dir).
pub trait LocalAudio: Send + Sync {
    /// The local audio files of recording `name`, in part order.
    fn parts(&self, name: &str) -> Vec<LocalPart>;
    /// Every recording with audio on the device: its name and its start (epoch ms, from the
    /// file time). The launch scan of ADR 0103 decision 7.
    fn names(&self) -> Vec<(String, u64)>;
    /// The names capture still owns (a recording in progress writes its open part under its
    /// final name, in the same folder the scan reads), or that finished but are not yet handed
    /// off to the pipeline. `list` and `retry` both leave these out (review of #592, BLOCKING
    /// B3): the scan's "upload failed" is for audio left over from an earlier run, never audio
    /// from the one running now.
    fn active(&self) -> HashSet<String>;
    /// The sidecar of the recording `name` (its profile and instructions), when the temp dir
    /// has one. The default has none.
    fn sidecar(&self, _name: &str) -> Option<Sidecar> {
        None
    }
    /// Delete what is left of `name` on the device after its pipeline ended with `done` (the
    /// sidecar and the empty temp dir). The default does nothing.
    fn tidy(&self, _name: &str) {}
    /// Delete every file of `name` on the device: audio parts, temp files and the sidecar.
    /// `true` when nothing of it is left. The default deletes nothing.
    fn delete(&self, _name: &str) -> bool {
        false
    }
    /// Does the device prove that no part of `name` was ever uploaded? The default cannot.
    fn never_uploaded(&self, _name: &str) -> bool {
        false
    }
}

/// No local audio (tests, and builds with no app data dir).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoLocalAudio;

impl LocalAudio for NoLocalAudio {
    fn parts(&self, _name: &str) -> Vec<LocalPart> {
        Vec::new()
    }
    fn names(&self) -> Vec<(String, u64)> {
        Vec::new()
    }
    fn active(&self) -> HashSet<String> {
        HashSet::new()
    }
}

/// A job.json that is not `done` or `failed` and has not changed for this long belongs to a
/// run that died (a crash or a quit): Retry may start again.
pub const STALE_AFTER_MS: i64 = 30 * 60 * 1000;
/// The key of a failed end: (profile, volume root, name).
type EndKey = (String, String, String);

/// The most failed ends the adapter keeps.
const MAX_ENDS: usize = 64;

fn end_key(p: &str, root: &str, name: &str) -> EndKey {
    (p.to_owned(), root.to_owned(), name.to_owned())
}

/// Gets each manifest change (the host sends `recording.job`).
pub type JobSink = Arc<dyn Fn(&RecordingSummary) + Send + Sync>;

/// The MOG tag of recording model calls (SEC-06a): recordings belong to the app, not one MOG.
pub const RECORDING_MOG_TAG: &str = "recording";

/// A service kind as the contract kind.
pub fn wire_kind(k: ErrorKind) -> RecordingErrorKind {
    use RecordingErrorKind as W;
    match k {
        ErrorKind::VolumePermission => W::VolumePermission,
        ErrorKind::VolumeNotFound | ErrorKind::MyFilesUnavailable => W::NoStorage,
        ErrorKind::TooLong => W::TooLong,
        ErrorKind::TranscribeEndpointMissing => W::TranscribeEndpointMissing,
        ErrorKind::NotesEndpointMissing => W::NotesEndpointMissing,
        ErrorKind::AudioUnreadable | ErrorKind::AudioMissing => W::AudioUnreadable,
        ErrorKind::SignedOut => W::Auth,
        ErrorKind::Offline => W::Network,
        ErrorKind::RateLimited => W::RateLimited,
        ErrorKind::UploadFailed => W::UploadFailed,
        ErrorKind::TranscribeFailed | ErrorKind::NotesFailed | ErrorKind::BadSettings => {
            W::Internal
        }
    }
}

fn err(e: RecordingError) -> RecordingErrorKind {
    wire_kind(e.kind)
}

/// A service status as the contract status.
pub fn wire_status(s: Status) -> RecordingStatus {
    match s {
        Status::Recording => RecordingStatus::Recording,
        Status::Uploading => RecordingStatus::Uploading,
        Status::Transcribing => RecordingStatus::Transcribing,
        Status::WritingNotes => RecordingStatus::WritingNotes,
        Status::Done => RecordingStatus::Done,
        Status::Failed => RecordingStatus::Failed,
    }
}

fn wire_stage(s: Stage) -> RecordingStatus {
    match s {
        Stage::Uploading => RecordingStatus::Uploading,
        Stage::Transcribing => RecordingStatus::Transcribing,
        Stage::WritingNotes => RecordingStatus::WritingNotes,
    }
}

/// The list row of a manifest. `None` when its name is not a valid `RecordingId`.
pub fn summary_of(m: &JobManifest) -> Option<RecordingSummary> {
    Some(RecordingSummary {
        id: RecordingId::try_from(m.name.clone()).ok()?,
        name: m.name.clone(),
        started_at_ms: parse_iso_utc_ms(&m.started_at).unwrap_or(0).max(0) as u64,
        duration_s: m.duration_s.max(0.0).round() as u32,
        status: wire_status(m.status),
        error_kind: m.error_kind.map(wire_kind),
        error_stage: m.error_stage.map(wire_stage),
        parts_uploaded: Some(m.parts.iter().filter(|p| p.uploaded).count() as u32),
        parts_total: Some(m.parts.len() as u32),
        on_device_only: false,
    })
}

fn wire_segment(s: &Segment) -> RecordingSegment {
    RecordingSegment {
        start: s.start,
        end: s.end,
        speaker: s.speaker.clone(),
        text: s.text.clone(),
        part: s.part,
    }
}

/// [`RecordingCloud`] over the service. Retry reads the endpoints and the preset from the
/// Settings store, and the local temp audio from `temp_dir(name)` (PR B's folder).
pub struct CloudAdapter {
    service: Arc<RecordingCloudService>,
    settings: Arc<dyn RecordingSettingsStore>,
    local: Arc<dyn LocalAudio>,
    on_job: JobSink,
    /// The recording names with a pipeline run in flight (the hand-off and Retry share it), so
    /// two runs never race on one recording.
    inflight: Mutex<HashSet<String>>,
    /// The recordings that upload during the call or resume after a crash, by name.
    live: Mutex<HashMap<String, LiveStatus>>,
    /// The last failed end of a run in this process, by (profile, volume root, name). The
    /// job.json write of a failure is best effort: when it does not land, the volume still says
    /// `writing_notes` (or another step), and the list and Retry read this end instead of a step
    /// that no run owns any more. Keyed by root, so a storage change never mixes them up.
    ends: Mutex<HashMap<EndKey, JobManifest>>,
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
}

/// Where a live recording is, for the list row. Content-free.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LiveStatus {
    id: String,
    started_at_ms: u64,
    /// The call still runs (the open part counts in the total).
    recording: bool,
    announced: BTreeSet<String>,
    uploaded: u32,
    /// The kind of the last failed attempt, until a try works.
    error: Option<ErrorKind>,
}

impl LiveStatus {
    fn total(&self) -> u32 {
        self.announced.len() as u32 + u32::from(self.recording)
    }
}

/// A message to the live worker of one recording.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveMsg {
    /// A part closed while the call goes on.
    Part(LocalPart),
    /// The call stopped. Every part that is still on the device, in order.
    Finished(Vec<LocalPart>),
}

/// What the live worker got while it waited.
#[derive(Debug, Clone, PartialEq)]
pub enum Recv {
    /// A message.
    Msg(LiveMsg),
    /// The wait ran out (time to try an upload again).
    Timeout,
    /// The sender is gone: the app ends. The parts stay on the device.
    Closed,
}

/// The wait between two upload tries after a failure: 5 s, doubling, at most 60 s. A try that
/// works resets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff(Duration);

impl Backoff {
    /// The first wait.
    pub const FIRST: Duration = Duration::from_secs(5);
    /// The longest wait.
    pub const MAX: Duration = Duration::from_secs(60);
    /// A new backoff.
    pub fn new() -> Self {
        Self(Self::FIRST)
    }
    /// The wait now, and double it for the next failure.
    pub fn advance(&mut self) -> Duration {
        let d = self.0;
        self.0 = (self.0 * 2).min(Self::MAX);
        d
    }
    /// A try worked.
    pub fn reset(&mut self) {
        self.0 = Self::FIRST;
    }
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

/// The job of a live recording: the profile and the job with no parts yet.
#[derive(Debug, Clone)]
pub struct LiveContext {
    /// The home profile of `recording_start`.
    pub profile: String,
    /// The job. `parts` stays empty: parts come as messages.
    pub job: RecordingJob,
    /// Epoch ms of the start.
    pub started_at_ms: u64,
    /// The call still runs when the worker starts (false: a resume after a crash).
    pub recording: bool,
}

/// A failure that a later try can fix: the network, a gateway fault, a rate limit.
pub fn is_transient(k: ErrorKind) -> bool {
    matches!(
        k,
        ErrorKind::Offline | ErrorKind::UploadFailed | ErrorKind::RateLimited
    )
}

/// Holds a recording name in the in-flight set until it drops.
struct InFlight<'a> {
    set: &'a Mutex<HashSet<String>>,
    name: String,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.name);
    }
}

/// Removes a live recording from the registry when its worker ends.
struct LiveGuard<'a> {
    map: &'a Mutex<HashMap<String, LiveStatus>>,
    name: String,
}

impl Drop for LiveGuard<'_> {
    fn drop(&mut self) {
        self.map
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.name);
    }
}

impl CloudAdapter {
    /// An adapter. `local`: the temp audio still on the device. `on_job`: each manifest change
    /// (the host sends `recording.job`). `now_ms`: wall time, for the stale check.
    pub fn new(
        service: Arc<RecordingCloudService>,
        settings: Arc<dyn RecordingSettingsStore>,
        local: Arc<dyn LocalAudio>,
        on_job: JobSink,
        now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        Self {
            service,
            settings,
            local,
            on_job,
            inflight: Mutex::new(HashSet::new()),
            live: Mutex::new(HashMap::new()),
            ends: Mutex::new(HashMap::new()),
            now_ms,
        }
    }

    /// `m` from the volume, or the failed end this process saw for it when `m` is still at a
    /// step (not done, not failed) that no run, worker or capture here owns now.
    fn settled(
        &self,
        p: &str,
        root: &str,
        m: JobManifest,
        active: &HashSet<String>,
    ) -> JobManifest {
        if self.in_flight(&m.name) || active.contains(&m.name) {
            return m;
        }
        self.end_of(p, root, m)
    }

    /// As [`Self::settled`] for a caller that holds the run slot of `m` itself (Retry, Delete).
    fn end_of(&self, p: &str, root: &str, m: JobManifest) -> JobManifest {
        if matches!(m.status, Status::Done | Status::Failed) || self.is_live(&m.name) {
            return m;
        }
        self.ends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&end_key(p, root, &m.name))
            .cloned()
            .unwrap_or(m)
    }

    /// Take `name` for one run, or `None` when a run on it is in flight.
    fn begin(&self, name: &str) -> Option<InFlight<'_>> {
        let fresh = self
            .inflight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_owned());
        fresh.then(|| InFlight {
            set: &self.inflight,
            name: name.to_owned(),
        })
    }

    /// Is a run on `name` in flight now?
    pub fn in_flight(&self, name: &str) -> bool {
        self.inflight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(name)
    }

    /// The list row of a recording whose audio is only on the device (the upload never wrote a
    /// job.json): failed, `upload_failed`, at the upload step, so Retry runs the whole pipeline.
    pub fn local_only_summary(name: &str, started_at_ms: u64) -> Option<RecordingSummary> {
        Some(RecordingSummary {
            id: RecordingId::try_from(name.to_owned()).ok()?,
            name: name.to_owned(),
            started_at_ms,
            duration_s: 0,
            status: RecordingStatus::Failed,
            error_kind: Some(RecordingErrorKind::UploadFailed),
            error_stage: Some(RecordingStatus::Uploading),
            parts_uploaded: Some(0),
            parts_total: None,
            on_device_only: true,
        })
    }

    /// The row of a recording that capture owns and no worker uploads.
    fn recording_row(name: &str, started_at_ms: u64) -> Option<RecordingSummary> {
        Some(RecordingSummary {
            id: RecordingId::try_from(name.to_owned()).ok()?,
            name: name.to_owned(),
            started_at_ms,
            duration_s: 0,
            status: RecordingStatus::Recording,
            error_kind: None,
            error_stage: None,
            parts_uploaded: None,
            parts_total: None,
            on_device_only: false,
        })
    }

    /// May Retry start a run on `m`? Only a failed run, or one whose job.json went stale.
    fn retry_allowed(&self, m: &JobManifest) -> bool {
        match m.status {
            // A job.json still in `recording` has no live worker here: `retry` checked the
            // capture, and the in-flight guard comes before this call.
            Status::Failed | Status::Recording => true,
            Status::Done => false,
            _ => parse_iso_utc_ms(&m.updated_at)
                .is_none_or(|t| (self.now_ms)() - t >= STALE_AFTER_MS),
        }
    }

    /// The service (for the pipeline hand-off).
    pub fn service(&self) -> &Arc<RecordingCloudService> {
        &self.service
    }

    /// Tell the sink about `m`, a manifest of a run on `p` and volume `root`.
    fn emit(&self, p: &str, root: &str, m: &JobManifest) {
        {
            let key = end_key(p, root, &m.name);
            let mut ends = self.ends.lock().unwrap_or_else(PoisonError::into_inner);
            if m.status == Status::Failed {
                if !ends.contains_key(&key) && ends.len() >= MAX_ENDS {
                    // Keep the map small: the oldest end goes.
                    let oldest = ends
                        .iter()
                        .min_by_key(|(_, e)| parse_iso_utc_ms(&e.updated_at).unwrap_or(0))
                        .map(|(k, _)| k.clone());
                    if let Some(k) = oldest {
                        ends.remove(&k);
                    }
                }
                ends.insert(key, m.clone());
            } else {
                // A new run works on it, or it is done: no failed end counts any more.
                ends.remove(&key);
            }
        }
        if let Some(s) = summary_of(m) {
            let s = self.overlay(s);
            (self.on_job)(&s);
        }
    }

    /// The summary of a recording with the live state on it: the counts of the open call and
    /// the "waiting for network" mark (`errorKind: network` on a status that is not failed).
    fn overlay(&self, mut s: RecordingSummary) -> RecordingSummary {
        let live = self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&s.name)
            .cloned();
        let Some(l) = live else { return s };
        if matches!(
            s.status,
            RecordingStatus::Recording | RecordingStatus::Uploading
        ) {
            s.parts_uploaded = Some(l.uploaded.max(s.parts_uploaded.unwrap_or(0)));
            s.parts_total = Some(l.total().max(s.parts_total.unwrap_or(0)));
            s.error_kind = l.error.map(|k| {
                if is_transient(k) {
                    RecordingErrorKind::Network
                } else {
                    wire_kind(k)
                }
            });
        }
        s
    }

    /// The row of a live recording that has no job.json yet (the volume was not reachable).
    fn live_row(&self, name: &str, l: &LiveStatus) -> Option<RecordingSummary> {
        let now = (self.now_ms)().max(0) as u64;
        let s = RecordingSummary {
            id: RecordingId::try_from(name.to_owned()).ok()?,
            name: name.to_owned(),
            started_at_ms: l.started_at_ms,
            duration_s: if l.recording {
                (now.saturating_sub(l.started_at_ms) / 1000) as u32
            } else {
                0
            },
            status: if l.recording {
                RecordingStatus::Recording
            } else {
                RecordingStatus::Uploading
            },
            error_kind: None,
            error_stage: None,
            parts_uploaded: Some(l.uploaded),
            parts_total: Some(l.total()),
            on_device_only: false,
        };
        Some(self.overlay(s))
    }

    fn live_edit(&self, name: &str, f: impl FnOnce(&mut LiveStatus)) {
        if let Some(l) = self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(name)
        {
            f(l);
        }
    }

    /// Tell the sink about a live change that no manifest write carries (an error that begins
    /// or ends, a part that closes).
    fn emit_live(&self, name: &str) {
        let l = self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned();
        if let Some(s) = l.and_then(|l| self.live_row(name, &l)) {
            (self.on_job)(&s);
        }
    }

    /// Is `name` a live recording now?
    pub fn is_live(&self, name: &str) -> bool {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(name)
    }

    /// The job of a recording for the live worker and the pipeline.
    #[allow(clippy::too_many_arguments)]
    pub fn job_for(
        &self,
        profile: &str,
        storage: &RecordingStorage,
        id: &RecordingId,
        name: &str,
        started_at_ms: u64,
        instructions: &Instructions,
        settings: &crate::types::RecordingSettings,
        recording: bool,
    ) -> LiveContext {
        LiveContext {
            profile: profile.to_owned(),
            job: RecordingJob {
                root: storage.root.as_str().to_owned(),
                id: id.as_str().to_owned(),
                name: name.to_owned(),
                started_at: iso_utc(started_at_ms as i64),
                instructions: instructions.as_str().to_owned(),
                transcribe_endpoint: settings.transcribe_endpoint.as_str().to_owned(),
                notes_endpoint: settings.notes_endpoint.as_str().to_owned(),
                mog: RECORDING_MOG_TAG.to_owned(),
                parts: Vec::new(),
            },
            started_at_ms,
            recording,
        }
    }

    /// Upload the parts of one recording while it runs, then finish it. The worker of one
    /// recording: it takes `recv` messages (a part closed, the stop), uploads every part in
    /// order (upload, job.json, delete), and on any failure keeps the part on the device and
    /// tries again after a [`Backoff`] wait, for the whole call. Recording never depends on it.
    /// After the stop it uploads the last part, then runs ONE transcription over the whole call
    /// and writes the notes. Returns the final summary, or `None` when the sender went away
    /// (the app ends: the parts stay for the launch scan) or a run on this name is in flight.
    pub fn run_live(
        &self,
        ctx: &LiveContext,
        recv: &mut dyn FnMut(Option<Duration>) -> Recv,
    ) -> Option<RecordingSummary> {
        let name = ctx.job.name.clone();
        let p = ctx.profile.as_str();
        let _guard = self.begin(&name)?;
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                name.clone(),
                LiveStatus {
                    id: ctx.job.id.clone(),
                    started_at_ms: ctx.started_at_ms,
                    recording: ctx.recording,
                    announced: BTreeSet::new(),
                    uploaded: 0,
                    error: None,
                },
            );
        let _live = LiveGuard {
            map: &self.live,
            name: name.clone(),
        };
        let mut manifest: Option<JobManifest> = None;
        let mut queue: Vec<LocalPart> = Vec::new();
        let mut backoff = Backoff::new();
        let mut finished: Option<Vec<LocalPart>> = None;
        loop {
            let tried = self.live_pump(p, ctx, &mut manifest, &mut queue);
            let err = tried.err();
            self.live_edit(&name, |l| {
                l.error = err;
                if let Some(m) = &manifest {
                    l.uploaded = m.parts.iter().filter(|e| e.uploaded).count() as u32;
                }
            });
            self.emit_live(&name);
            if err.is_none() {
                backoff.reset();
            }
            if finished.is_some() {
                // After the stop: leave the loop when nothing is left to send, or when the
                // failure is not one that waiting fixes (the job then fails with its kind).
                if err.is_none() || err.is_some_and(|k| !is_transient(k)) {
                    break;
                }
            }
            let wait = err.is_some().then(|| backoff.advance());
            match recv(wait) {
                Recv::Msg(LiveMsg::Part(part)) => {
                    self.live_announce(&name, &part);
                    queue.push(part);
                }
                Recv::Msg(LiveMsg::Finished(parts))
                    if ctx.recording && parts.is_empty() && queue.is_empty() =>
                {
                    // The call stopped and no audio ever came in: nothing to upload or
                    // transcribe, so no wait for the network. The job ends failed at once.
                    self.live_edit(&name, |l| l.recording = false);
                    let m = self.service.live_no_audio(p, &ctx.job, manifest, &mut |m| {
                        self.emit(p, &ctx.job.root, m)
                    });
                    return summary_of(&m).map(|s| self.overlay(s));
                }
                Recv::Msg(LiveMsg::Finished(parts)) => {
                    for part in &parts {
                        self.live_announce(&name, part);
                        let file = part.path.file_name();
                        let known = queue.iter().any(|q| q.path.file_name() == file);
                        let sent = manifest.as_ref().is_some_and(|m| {
                            m.parts.iter().any(|e| {
                                e.uploaded && Some(e.file.as_str()) == file.and_then(|f| f.to_str())
                            })
                        });
                        if !known && !sent && part.path.exists() {
                            queue.push(part.clone());
                        }
                    }
                    self.live_edit(&name, |l| l.recording = false);
                    finished = Some(parts);
                }
                Recv::Timeout => {}
                Recv::Closed => return None,
            }
        }
        let mut job = ctx.job.clone();
        job.parts = finished
            .unwrap_or_default()
            .into_iter()
            .filter(|l| l.path.exists())
            .collect();
        // Parts that failed in the pump and are still queued stay in `job.parts` (they are on
        // the device), so the upload stage of the finish tries them once more.
        let m = self
            .service
            .live_finish(p, &job, manifest, &mut |m| self.emit(p, &ctx.job.root, m));
        summary_of(&m).map(|s| self.overlay(s))
    }

    fn live_announce(&self, name: &str, part: &LocalPart) {
        if let Some(f) = part.path.file_name().and_then(|f| f.to_str()) {
            self.live_edit(name, |l| {
                l.announced.insert(f.to_owned());
            });
        }
    }

    /// One try to send what waits: the folder and job.json first, then each queued part in
    /// order. A part that is gone, or is not audio, is dropped (it can never be sent).
    fn live_pump(
        &self,
        p: &str,
        ctx: &LiveContext,
        manifest: &mut Option<JobManifest>,
        queue: &mut Vec<LocalPart>,
    ) -> Result<(), ErrorKind> {
        let mut on = |m: &JobManifest| self.emit(p, &ctx.job.root, m);
        if manifest.is_none() {
            *manifest = Some(self.service.live_begin(p, &ctx.job, &mut on)?);
        }
        let Some(m) = manifest.as_mut() else {
            return Ok(());
        };
        while let Some(part) = queue.first().cloned() {
            match self
                .service
                .live_upload_part(p, &ctx.job, m, &part, &mut on)
            {
                Ok(()) => {}
                Err(ErrorKind::AudioMissing | ErrorKind::AudioUnreadable) => {}
                Err(k) => return Err(k),
            }
            queue.remove(0);
        }
        Ok(())
    }

    /// Run the whole pipeline for one finished capture, once. Returns the final summary.
    pub fn run_pipeline(
        &self,
        req: &PipelineRequest,
        started_at_ms: u64,
    ) -> RecordingResult<RecordingSummary> {
        let job = RecordingJob {
            root: req.storage.root.as_str().to_owned(),
            id: req.id.as_str().to_owned(),
            name: req.name.clone(),
            started_at: iso_utc(started_at_ms as i64),
            instructions: req.instructions.as_str().to_owned(),
            transcribe_endpoint: req.settings.transcribe_endpoint.as_str().to_owned(),
            notes_endpoint: req.settings.notes_endpoint.as_str().to_owned(),
            mog: RECORDING_MOG_TAG.to_owned(),
            parts: req
                .parts
                .iter()
                .map(|p| LocalPart {
                    path: p.path.clone(),
                    duration_s: p.duration_s,
                    held: Vec::new(),
                })
                .collect(),
        };
        let Some(_guard) = self.begin(&req.name) else {
            return Err(RecordingErrorKind::RecordingBusy);
        };
        let m = self.service.run(req.profile.as_str(), &job, &mut |m| {
            self.emit(req.profile.as_str(), req.storage.root.as_str(), m)
        });
        summary_of(&m).ok_or(RecordingErrorKind::Internal)
    }

    fn root(storage: &RecordingStorage) -> &str {
        storage.root.as_str()
    }
}

impl RecordingCloud for CloudAdapter {
    fn probe_storage(&self, profile: &ProfileName) -> RecordingResult<StorageProbe> {
        match self
            .service
            .probe_storage(profile.as_str(), &Storage::MyFiles)
        {
            Ok(Probe::Ready { root }) => Ok(StorageProbe {
                my_files_root: VolumePath::try_from(root).ok(),
            }),
            Ok(Probe::NeedsVolume { .. }) => Ok(StorageProbe {
                my_files_root: None,
            }),
            Err(e) if e.kind == ErrorKind::MyFilesUnavailable => Ok(StorageProbe {
                my_files_root: None,
            }),
            Err(e) => Err(err(e)),
        }
    }

    fn create_volume(
        &self,
        profile: &ProfileName,
        catalog: &UcName,
        schema: &UcName,
    ) -> RecordingResult<RecordingStorage> {
        let root = self
            .service
            .create_volume(profile.as_str(), catalog.as_str(), schema.as_str())
            .map_err(err)?;
        Ok(RecordingStorage {
            kind: RecordingStorageKind::Volume,
            root: VolumePath::try_from(root).map_err(|_| RecordingErrorKind::Internal)?,
        })
    }

    fn upload_part(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        name: &str,
        part: &FinishedPart,
    ) -> RecordingResult<()> {
        self.service
            .upload_audio(profile.as_str(), Self::root(storage), name, &part.path)
            .map(|_| ())
            .map_err(wire_kind)
    }

    fn transcribe_part(
        &self,
        profile: &ProfileName,
        endpoint: &EndpointName,
        part: &FinishedPart,
        context: &[RecordingSegment],
    ) -> RecordingResult<Vec<RecordingSegment>> {
        let ctx: Vec<String> = context
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(|s| {
                format!(
                    "[{}] {}: {}",
                    super::transcript::fmt_hms(s.start - part.offset_s),
                    super::transcript::speaker_name(&s.speaker),
                    s.text
                )
            })
            .collect();
        let ctx = (!ctx.is_empty()).then(|| ctx.join("\n"));
        let raw = self
            .service
            .transcribe_audio(
                profile.as_str(),
                endpoint.as_str(),
                RECORDING_MOG_TAG,
                &part.path,
                part.duration_s,
                ctx.as_deref(),
            )
            .map_err(wire_kind)?;
        Ok(raw
            .into_iter()
            .map(|s| RecordingSegment {
                start: part.offset_s + s.start,
                end: part.offset_s + s.end,
                speaker: s.speaker_id,
                text: s.text,
                part: part.index,
            })
            .collect())
    }

    fn write_job(
        &self,
        profile: &ProfileName,
        request: &PipelineRequest,
        status: RecordingStatus,
        detail: Option<&RecordingDetail>,
    ) -> RecordingResult<RecordingSummary> {
        let p = profile.as_str();
        let root = Self::root(&request.storage);
        let name = request.name.as_str();
        let w = |file: String, text: &[u8]| {
            self.service
                .write_text(p, root, name, &file, text)
                .map_err(wire_kind)
        };
        w(
            instructions_file(name),
            request.instructions.as_str().as_bytes(),
        )?;
        let mut m = self
            .service
            .job(p, root, name)
            .map_err(wire_kind)?
            .unwrap_or_else(|| {
                JobManifest::new(
                    request.id.as_str(),
                    name,
                    &iso_utc(0),
                    request
                        .parts
                        .iter()
                        .map(|fp| {
                            let file = fp
                                .path
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or_default();
                            super::job::PartEntry::new(file, fp.duration_s)
                        })
                        .collect(),
                    request.settings.transcribe_endpoint.as_str(),
                    request.settings.notes_endpoint.as_str(),
                    &iso_utc(0),
                )
            });
        if let Some(d) = detail {
            let doc = TranscriptDoc {
                version: 1,
                id: request.id.as_str().to_owned(),
                name: name.to_owned(),
                started_at: iso_utc(d.summary.started_at_ms as i64),
                duration_s: f64::from(d.summary.duration_s),
                instructions: d.instructions.clone(),
                parts: Vec::new(),
                segments: d
                    .segments
                    .iter()
                    .map(|s| Segment {
                        start: s.start,
                        end: s.end,
                        speaker: s.speaker.clone(),
                        text: s.text.clone(),
                        part: s.part,
                    })
                    .collect(),
                bottle_gaps: Vec::new(),
            };
            let tj = serde_json::to_vec_pretty(&doc).map_err(|_| RecordingErrorKind::Internal)?;
            w(transcript_json_file(name), &tj)?;
            let md = d
                .transcript_md
                .clone()
                .unwrap_or_else(|| render_markdown(&doc));
            w(transcript_md_file(name), md.as_bytes())?;
            if let Some(n) = &d.notes_md {
                w(notes_file(name), n.as_bytes())?;
            }
        }
        m.status = match status {
            RecordingStatus::Recording => Status::Recording,
            RecordingStatus::Uploading => Status::Uploading,
            RecordingStatus::Transcribing => Status::Transcribing,
            RecordingStatus::WritingNotes => Status::WritingNotes,
            RecordingStatus::Done => Status::Done,
            RecordingStatus::Failed => Status::Failed,
        };
        let text = serde_json::to_vec_pretty(&m).map_err(|_| RecordingErrorKind::Internal)?;
        w(job_file(name), &text)?;
        summary_of(&m).ok_or(RecordingErrorKind::Internal)
    }

    fn list(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
    ) -> RecordingResult<Vec<RecordingSummary>> {
        // A transport fault on the volume listing itself (offline, a token hiccup, a slow
        // network right after launch) must never blank the whole list (review of #592, "A",
        // 2026-11-25/26): every recording exists locally first, whether or not it has ever
        // reached the volume, so the local and live rows below still have to show. A 404
        // (nothing in the volume yet) already reads as zero rows here (`cloud::list`); a
        // genuine fault now reads the same way instead of refusing the whole call.
        let rows = self
            .service
            .list(profile.as_str(), Self::root(storage))
            .unwrap_or_default();
        let active = self.local.active();
        let now = (self.now_ms)();
        // A job.json that is neither done nor failed and stopped changing belongs to a run that
        // died (a crash, a quit). No worker, no run and no capture owns it: it reads as failed
        // at its step, so Retry shows.
        let rows: Vec<JobManifest> = rows
            .into_iter()
            .map(|m| self.settled(profile.as_str(), Self::root(storage), m, &active))
            .map(|mut m| {
                let stuck = !self.in_flight(&m.name)
                    && !active.contains(&m.name)
                    && !self.is_live(&m.name)
                    && parse_iso_utc_ms(&m.updated_at).is_some_and(|t| now - t >= STALE_AFTER_MS);
                let step = match m.status {
                    Status::Uploading => Some((ErrorKind::UploadFailed, Stage::Uploading)),
                    Status::Transcribing => {
                        Some((ErrorKind::TranscribeFailed, Stage::Transcribing))
                    }
                    Status::WritingNotes => Some((ErrorKind::NotesFailed, Stage::WritingNotes)),
                    _ => None,
                };
                if let (true, Some((k, st))) = (stuck, step) {
                    m.fail(k, st, &m.updated_at.clone());
                }
                m
            })
            .collect();
        let mut out: Vec<RecordingSummary> = rows.iter().filter_map(summary_of).collect();
        let live: HashMap<String, LiveStatus> = self
            .live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let local: HashMap<String, u64> = self.local.names().into_iter().collect();
        for row in &mut out {
            if live.contains_key(&row.name) {
                // A worker owns it: show its live counts and its network mark.
                *row = self.overlay(row.clone());
            } else if row.status == RecordingStatus::Recording
                && !self.in_flight(&row.name)
                && !active.contains(&row.name)
            {
                // job.json says "recording", but no capture and no worker has it: a crash.
                // With the audio still here the row is a failed upload that Retry fixes.
                row.status = RecordingStatus::Failed;
                row.error_kind = Some(RecordingErrorKind::UploadFailed);
                row.error_stage = Some(RecordingStatus::Uploading);
            }
        }
        // A recording that runs or uploads now, with no job.json in the volume yet.
        for (name, l) in &live {
            if !out.iter().any(|r| &r.name == name) {
                out.extend(self.live_row(name, l));
            }
        }
        // A run here that failed with no job.json in the volume (a call with no audio whose
        // start never reached the volume): its failed end is the row.
        let ends: Vec<JobManifest> = self
            .ends
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(|((p, r, _), _)| p == profile.as_str() && r == Self::root(storage))
            .map(|(_, m)| m.clone())
            .collect();
        for m in &ends {
            if !out.iter().any(|r| r.name == m.name) && !local.contains_key(&m.name) {
                out.extend(summary_of(m));
            }
        }
        // ADR 0103 decision 7: audio left on the device with no job.json in the volume. Never
        // the recording capture owns right now, or one that finished but has not yet reached
        // the pipeline (review of #592, BLOCKING B3). A recording that capture owns, with no
        // worker (the live upload could not start), shows as recording.
        for (name, started) in &local {
            if out.iter().any(|r| &r.name == name)
                || self.in_flight(name)
                || live.contains_key(name)
            {
                continue;
            }
            if active.contains(name) {
                out.extend(Self::recording_row(name, *started));
            } else if let Some(s) = Self::local_only_summary(name, *started) {
                out.push(s);
            }
        }
        // A failed upload with no part in the volume and no audio left here was deleted on this
        // device (`recording_delete` never touches the volume): nothing to retry, so no row.
        out.retain(|r| {
            let dead = r.status == RecordingStatus::Failed
                && r.error_stage == Some(RecordingStatus::Uploading)
                && r.parts_uploaded.unwrap_or(0) == 0;
            !dead || local.contains_key(&r.name) || live.contains_key(&r.name)
        });
        out.sort_by(|a, b| {
            b.started_at_ms
                .cmp(&a.started_at_ms)
                .then(b.name.cmp(&a.name))
        });
        Ok(out)
    }

    fn detail(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
    ) -> RecordingResult<RecordingDetail> {
        let d = self
            .service
            .detail(profile.as_str(), Self::root(storage), id.as_str())
            .map_err(err)?;
        let job = self.settled(
            profile.as_str(),
            Self::root(storage),
            d.job,
            &self.local.active(),
        );
        let summary = summary_of(&job).ok_or(RecordingErrorKind::Internal)?;
        let instructions = d
            .transcript
            .as_ref()
            .map(|t| t.instructions.clone())
            .or(d.instructions)
            .unwrap_or_default();
        Ok(RecordingDetail {
            summary,
            instructions,
            segments: d
                .transcript
                .as_ref()
                .map(|t| t.segments.iter().map(wire_segment).collect())
                .unwrap_or_default(),
            transcript_md: d.transcript_md,
            notes_md: d.notes,
        })
    }

    fn retry(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
    ) -> RecordingResult<RecordingSummary> {
        let p = profile.as_str();
        let root = Self::root(storage);
        let name = id.as_str();
        if self.local.active().contains(name) {
            // Capture still owns this name, or it is waiting for the hand-off to the pipeline:
            // Retry must never touch it (review of #592, BLOCKING B3).
            return Err(RecordingErrorKind::RecordingBusy);
        }
        let local_started = || {
            self.local
                .names()
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t)
        };
        let now = |m: &Option<JobManifest>| -> RecordingResult<RecordingSummary> {
            match m {
                Some(m) => summary_of(m).ok_or(RecordingErrorKind::Internal),
                None => Self::local_only_summary(name, local_started().unwrap_or(0))
                    .ok_or(RecordingErrorKind::Internal),
            }
        };
        // The guard comes before the job.json read (review of #592, non-blocking item 5): a
        // read before the guard leaves a gap where a concurrent run can reach `done` before
        // this call takes the slot, and this call would restart it.
        let Some(_guard) = self.begin(name) else {
            let current = self.service.job(p, root, name).map_err(wire_kind)?;
            let active = self.local.active();
            return now(&current.map(|m| self.settled(p, root, m, &active)));
        };
        // This call holds the run slot now: a failed end of this process counts.
        let current = self
            .service
            .job(p, root, name)
            .map_err(wire_kind)?
            .map(|m| self.end_of(p, root, m));
        if current.as_ref().is_some_and(|m| !self.retry_allowed(m)) {
            return now(&current);
        }
        let settings = self
            .settings
            .load()
            .map_err(|_| RecordingErrorKind::Internal)?;
        let instructions = self
            .service
            .instructions(p, root, name)
            .map_err(wire_kind)?
            .unwrap_or_else(|| settings.preset_instructions.as_str().to_owned());
        let started_at = current
            .as_ref()
            .map(|m| m.started_at.clone())
            .unwrap_or_else(|| iso_utc(local_started().unwrap_or(0) as i64));
        let job = RecordingJob {
            root: root.to_owned(),
            id: name.to_owned(),
            name: name.to_owned(),
            started_at,
            instructions,
            transcribe_endpoint: settings.transcribe_endpoint.as_str().to_owned(),
            notes_endpoint: settings.notes_endpoint.as_str().to_owned(),
            mog: RECORDING_MOG_TAG.to_owned(),
            parts: self.local.parts(name),
        };
        let m = self.service.retry(p, &job, &mut |m| self.emit(p, root, m));
        if m.status == Status::Done {
            self.local.tidy(name);
        }
        summary_of(&m).ok_or(RecordingErrorKind::Internal)
    }

    fn folder_link(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
    ) -> RecordingResult<String> {
        self.service
            .link(profile.as_str(), Self::root(storage), id.as_str())
            .map_err(err)
    }

    fn target_link(
        &self,
        profile: &ProfileName,
        storage: &RecordingStorage,
        id: &RecordingId,
        target: crate::ipc::RecordingOpenTarget,
    ) -> RecordingResult<String> {
        use super::layout::RecordingFile;
        use crate::ipc::RecordingOpenTarget as T;
        let file = match target {
            T::Folder => return self.folder_link(profile, storage, id),
            T::Transcript => RecordingFile::Transcript,
            T::Notes => RecordingFile::Notes,
        };
        self.service
            .file_link(profile.as_str(), Self::root(storage), id.as_str(), file)
            .map(|l| l.url)
            .map_err(err)
    }

    fn delete(
        &self,
        profile: &ProfileName,
        storage: Option<&RecordingStorage>,
        id: &RecordingId,
    ) -> RecordingResult<bool> {
        let name = id.as_str();
        // The id must name audio that Bottle's own temp store holds. No path crosses IPC.
        if !self.local.names().iter().any(|(n, _)| n == name) {
            return Ok(false);
        }
        if self.local.active().contains(name) || self.is_live(name) {
            return Err(RecordingErrorKind::RecordingBusy);
        }
        // The in-flight slot also keeps a Retry out while the files go.
        let Some(_guard) = self.begin(name) else {
            return Err(RecordingErrorKind::RecordingBusy);
        };
        if storage.is_none() && !self.local.never_uploaded(name) {
            // Settings has no storage now, but a part may have gone up before it was cleared.
            return Err(RecordingErrorKind::RecordingBusy);
        }
        if let Some(storage) = storage {
            // Read only. A recording with a part in the volume, or past the upload, is not
            // "not uploaded": the volume copy is the one that counts.
            let job = self
                .service
                .job(profile.as_str(), Self::root(storage), name)
                .map_err(wire_kind)?;
            if let Some(m) = job.map(|m| self.end_of(profile.as_str(), Self::root(storage), m)) {
                if !self.retry_allowed(&m) || m.parts.iter().any(|p| p.uploaded) {
                    return Err(RecordingErrorKind::RecordingBusy);
                }
            }
        }
        if self.local.delete(name) {
            Ok(true)
        } else {
            Err(RecordingErrorKind::Internal)
        }
    }

    fn list_local(&self) -> RecordingResult<Vec<RecordingSummary>> {
        let active = self.local.active();
        let mut out: Vec<RecordingSummary> = self
            .local
            .names()
            .into_iter()
            .filter(|(name, _)| !self.in_flight(name))
            .filter_map(|(name, started)| {
                // A call that records now shows as recording, not as a failed upload.
                if active.contains(&name) {
                    Self::recording_row(&name, started)
                } else {
                    Self::local_only_summary(&name, started)
                }
            })
            .collect();
        out.sort_by(|a, b| {
            b.started_at_ms
                .cmp(&a.started_at_ms)
                .then(b.name.cmp(&a.name))
        });
        Ok(out)
    }
}

// ---------- the in-flight guard (review of #592, BLOCKING 1) ----------
//
// `begin`/`InFlight` are private, so the race is tested here, in the crate, rather than through
// the public `RecordingCloud` seam: a deterministic check of the mutex itself (no thread timing,
// NFR-23a) stands in for "a double-click on Retry" and "a Retry while the hand-off still runs",
// which both reduce to two callers racing `begin` on the same name.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SimWallClock;
    use crate::error::CoreResult;
    use crate::http_transport::Pause;
    use crate::recording::MemorySettingsStore;
    use crate::recording_cloud::cloud::{CloudDeps, WorkspaceInfo};
    use crate::recording_cloud::http::ScriptedHttp;
    use crate::types::RecordingSettings;
    use std::time::Duration;

    struct Ws;
    impl WorkspaceInfo for Ws {
        fn host_url(&self, _p: &str) -> CoreResult<String> {
            Ok("https://dbc-fake.cloud.databricks.com".into())
        }
        fn workspace_id(&self, _p: &str) -> CoreResult<String> {
            Ok("1".into())
        }
    }

    struct NoPause;
    impl Pause for NoPause {
        fn pause(&self, _d: Duration) {}
    }

    fn adapter() -> CloudAdapter {
        let service = Arc::new(RecordingCloudService::new(CloudDeps {
            http: Arc::new(ScriptedHttp::new()),
            workspace: Arc::new(Ws),
            wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
            pause: Arc::new(NoPause),
        }));
        CloudAdapter::new(
            service,
            Arc::new(MemorySettingsStore::new(RecordingSettings::default())),
            Arc::new(NoLocalAudio),
            Arc::new(|_: &RecordingSummary| {}),
            Arc::new(|| 1_795_100_700_000i64),
        )
    }

    /// Local audio with one name that counts its delete calls.
    struct OneLocal(std::sync::atomic::AtomicU32);
    impl LocalAudio for OneLocal {
        fn parts(&self, _name: &str) -> Vec<LocalPart> {
            Vec::new()
        }
        fn names(&self) -> Vec<(String, u64)> {
            vec![("2026-11-19_0905".to_owned(), 1)]
        }
        fn active(&self) -> HashSet<String> {
            HashSet::new()
        }
        fn delete(&self, _name: &str) -> bool {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            true
        }
        fn never_uploaded(&self, _name: &str) -> bool {
            true
        }
    }

    fn adapter_with(local: Arc<OneLocal>) -> CloudAdapter {
        let service = Arc::new(RecordingCloudService::new(CloudDeps {
            http: Arc::new(ScriptedHttp::new()),
            workspace: Arc::new(Ws),
            wall: Arc::new(SimWallClock::at(1_795_100_700_000)),
            pause: Arc::new(NoPause),
        }));
        CloudAdapter::new(
            service,
            Arc::new(MemorySettingsStore::new(RecordingSettings::default())),
            local,
            Arc::new(|_: &RecordingSummary| {}),
            Arc::new(|| 1_795_100_700_000i64),
        )
    }

    #[test]
    fn delete_is_busy_while_a_run_holds_the_slot_or_a_live_worker_owns_the_name() {
        let local = Arc::new(OneLocal(0.into()));
        let a = adapter_with(local.clone());
        let id = RecordingId::try_from("2026-11-19_0905".to_owned()).unwrap();
        let p = ProfileName::try_from("fake-home".to_owned()).unwrap();
        let guard = a.begin("2026-11-19_0905").unwrap();
        assert_eq!(
            RecordingCloud::delete(&a, &p, None, &id),
            Err(RecordingErrorKind::RecordingBusy)
        );
        drop(guard);
        a.live.lock().unwrap().insert(
            "2026-11-19_0905".to_owned(),
            LiveStatus {
                id: "r".to_owned(),
                started_at_ms: 1,
                recording: false,
                announced: BTreeSet::new(),
                uploaded: 0,
                error: None,
            },
        );
        assert_eq!(
            RecordingCloud::delete(&a, &p, None, &id),
            Err(RecordingErrorKind::RecordingBusy)
        );
        assert_eq!(local.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn begin_refuses_a_second_run_on_the_same_name_until_the_first_drops() {
        let a = adapter();
        let first = a.begin("2026-11-19_0905").expect("the first run starts");
        assert!(
            a.begin("2026-11-19_0905").is_none(),
            "a double-click on Retry, or a Retry while the hand-off still runs, must not start \
             a second concurrent run"
        );
        assert!(a.in_flight("2026-11-19_0905"));
        assert!(
            a.begin("2026-11-19_0901").is_some(),
            "a different recording is unaffected"
        );
        drop(first);
        assert!(!a.in_flight("2026-11-19_0905"));
        assert!(
            a.begin("2026-11-19_0905").is_some(),
            "a run may start once the first one finishes (drops its guard)"
        );
    }

    #[test]
    fn run_pipeline_refuses_while_a_run_on_the_same_name_is_in_flight() {
        let a = adapter();
        let name = "2026-11-19_0905";
        let _guard = a.begin(name).expect("the first run starts");
        let req = PipelineRequest {
            profile: "fake-home".to_owned(),
            storage: RecordingStorage {
                kind: RecordingStorageKind::Volume,
                root: VolumePath::try_from("/Volumes/main/team/audio".to_owned()).unwrap(),
            },
            id: RecordingId::try_from(name.to_owned()).unwrap(),
            name: name.to_owned(),
            parts: Vec::new(),
            instructions: Instructions::try_from("Write notes.".to_owned()).unwrap(),
            settings: RecordingSettings::default(),
        };
        let e = a.run_pipeline(&req, 0).unwrap_err();
        assert_eq!(e, RecordingErrorKind::RecordingBusy);
    }

    #[test]
    fn retry_allowed_only_for_failed_or_a_stale_job() {
        let a = adapter();
        let fresh = |status| JobManifest {
            updated_at: iso_utc(1_795_100_700_000),
            ..manifest_of(status)
        };
        assert!(a.retry_allowed(&fresh(Status::Failed)));
        assert!(!a.retry_allowed(&fresh(Status::Done)));
        assert!(
            !a.retry_allowed(&fresh(Status::Transcribing)),
            "a fresh, running job is not stale"
        );
        let stale = JobManifest {
            updated_at: iso_utc(1_795_100_700_000 - STALE_AFTER_MS - 1),
            ..manifest_of(Status::Transcribing)
        };
        assert!(
            a.retry_allowed(&stale),
            "a job that stopped updating (a crash or a quit) may be retried"
        );
    }

    fn manifest_of(status: Status) -> JobManifest {
        let mut m = JobManifest::new(
            "r1",
            "2026-11-19_0905",
            &iso_utc(0),
            vec![super::super::job::PartEntry::new(
                "2026-11-19_0905.opus",
                1.0,
            )],
            "system.ai.gemini-3-8-flash",
            "system.ai.claude-sonnet-5-5",
            &iso_utc(0),
        );
        m.status = status;
        m
    }
}
