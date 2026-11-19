//! The contract seam of PR C (ADR 0103): [`CloudAdapter`] implements
//! [`crate::recording::RecordingCloud`] over [`RecordingCloudService`], and maps the service's
//! kinds, manifests and transcripts to the contract wire types. The recording ID on the wire is
//! the folder name (`YYYY-MM-DD_HHMM`, with `_N` on a clash), which fits `RecordingId`.

use std::sync::Arc;

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
    temp_parts: Arc<dyn Fn(&str) -> Vec<LocalPart> + Send + Sync>,
    on_job: Arc<dyn Fn(&RecordingSummary) + Send + Sync>,
}

impl CloudAdapter {
    /// An adapter. `temp_parts(name)`: the local temp audio of a recording still on the device.
    /// `on_job`: each manifest change (the host sends `recording.job`).
    pub fn new(
        service: Arc<RecordingCloudService>,
        settings: Arc<dyn RecordingSettingsStore>,
        temp_parts: Arc<dyn Fn(&str) -> Vec<LocalPart> + Send + Sync>,
        on_job: Arc<dyn Fn(&RecordingSummary) + Send + Sync>,
    ) -> Self {
        Self {
            service,
            settings,
            temp_parts,
            on_job,
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
        Ok(rows.iter().filter_map(summary_of).collect())
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
        let settings = self
            .settings
            .load()
            .map_err(|_| RecordingErrorKind::Internal)?;
        let instructions = self
            .service
            .instructions(p, root, name)
            .map_err(wire_kind)?
            .unwrap_or_else(|| settings.preset_instructions.as_str().to_owned());
        let started_at = self
            .service
            .job(p, root, name)
            .map_err(wire_kind)?
            .map(|m| m.started_at)
            .unwrap_or_else(|| iso_utc(0));
        let job = RecordingJob {
            root: root.to_owned(),
            id: name.to_owned(),
            name: name.to_owned(),
            started_at,
            instructions,
            transcribe_endpoint: settings.transcribe_endpoint.as_str().to_owned(),
            notes_endpoint: settings.notes_endpoint.as_str().to_owned(),
            mog: RECORDING_MOG_TAG.to_owned(),
            parts: (self.temp_parts)(name),
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
