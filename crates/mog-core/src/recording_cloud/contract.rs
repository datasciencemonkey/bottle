//! The contract seam of PR C (ADR 0103): [`CloudAdapter`] implements
//! [`crate::recording::RecordingCloud`] over [`RecordingCloudService`], and maps the service's
//! kinds, manifests and transcripts to the contract wire types. The recording ID on the wire is
//! the folder name (`YYYY-MM-DD_HHMM`, with `_N` on a clash), which fits `RecordingId`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

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
    now_ms: Arc<dyn Fn() -> i64 + Send + Sync>,
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
            now_ms,
        }
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
        })
    }

    /// May Retry start a run on `m`? Only a failed run, or one whose job.json went stale.
    fn retry_allowed(&self, m: &JobManifest) -> bool {
        match m.status {
            Status::Failed => true,
            Status::Done => false,
            _ => parse_iso_utc_ms(&m.updated_at)
                .is_none_or(|t| (self.now_ms)() - t >= STALE_AFTER_MS),
        }
    }

    /// The service (for the pipeline hand-off).
    pub fn service(&self) -> &Arc<RecordingCloudService> {
        &self.service
    }

    fn emit(&self, m: &JobManifest) {
        if let Some(s) = summary_of(m) {
            (self.on_job)(&s);
        }
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
                })
                .collect(),
        };
        let Some(_guard) = self.begin(&req.name) else {
            return Err(RecordingErrorKind::RecordingBusy);
        };
        let m = self
            .service
            .run(req.profile.as_str(), &job, &mut |m| self.emit(m));
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

    fn write_notes(
        &self,
        profile: &ProfileName,
        endpoint: &EndpointName,
        instructions: &Instructions,
        transcript_md: &str,
    ) -> RecordingResult<String> {
        self.service
            .notes(
                profile.as_str(),
                endpoint.as_str(),
                RECORDING_MOG_TAG,
                instructions.as_str(),
                transcript_md,
            )
            .map_err(wire_kind)
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
        let rows = self
            .service
            .list(profile.as_str(), Self::root(storage))
            .map_err(err)?;
        let mut out: Vec<RecordingSummary> = rows.iter().filter_map(summary_of).collect();
        // ADR 0103 decision 7: audio left on the device with no job.json in the volume shows
        // as "Upload failed - Retry". Never the recording capture owns right now, or one that
        // finished but has not yet reached the pipeline (review of #592, BLOCKING B3).
        let active = self.local.active();
        for (name, started) in self.local.names() {
            if out.iter().any(|r| r.name == name) || self.in_flight(&name) || active.contains(&name)
            {
                continue;
            }
            if let Some(s) = Self::local_only_summary(&name, started) {
                out.push(s);
            }
        }
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
        let summary = summary_of(&d.job).ok_or(RecordingErrorKind::Internal)?;
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
            return now(&current);
        };
        let current = self.service.job(p, root, name).map_err(wire_kind)?;
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
        let m = self.service.retry(p, &job, &mut |m| self.emit(m));
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
            "databricks-gemini-3-8-flash",
            "databricks-claude-sonnet-5-5",
            &iso_utc(0),
        );
        m.status = status;
        m
    }
}
