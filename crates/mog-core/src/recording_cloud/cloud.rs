//! The recording cloud service (ADR 0103, PR C): storage, and the pipeline
//! upload -> transcribe -> notes, all on the device with the home-profile token (UPDATE 4: no
//! warehouse, no job, no statement). The job manifest in the volume is kept up to date at each
//! stage. Every call goes through [`RecordingHttp`] (R7 with the R6 token broker on the profile
//! host). Blocking: run it on a worker thread.
//!
//! Local temp audio is deleted only after its part upload succeeded. A failed stage is stored
//! in job.json (`error_kind`, `error_stage`) and Retry starts there; a part that is no longer on
//! the device is read back from the volume.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::clock::WallClock;
use crate::error::CoreResult;
use crate::http_transport::{Pause, ProfileHosts};
use crate::ipc::JsonObject;
use crate::token_broker::TokenBroker;
use crate::types::{TagApp, TagOp, Tags};

use super::errors::{
    from_core, from_files_status, from_model_reply, ErrorKind, ModelCall, RecordingError, Stage,
};
use super::http::{RecordingHttp, Reply, ReplyBody, ReqBody};
use super::job::{iso_utc, JobManifest, PartEntry, Status};
use super::layout::{
    catalog_explorer_url, instructions_file, is_recording_name, job_file, managed_volume_path,
    my_files_candidates, notes_file, part_file, transcript_json_file, transcript_md_file,
    VolumeRoot, MANAGED_VOLUME,
};
use super::model::{notes_request, parse_transcript, reply_text, sniff, transcribe_request};
use super::settings_store::{is_endpoint_name, valid_instructions, Storage};
use super::stitch;
use super::transcript::{
    merge_parts, render_markdown, PartInfo, PartInput, PartTranscript, TranscriptDoc,
};

/// The model route (Unity Gateway chat, NFR-21a).
pub const MODEL_ROUTE: &str = "mlflow.chat_completions";
/// Model call attempts on a 429 or 503.
pub const MODEL_ATTEMPTS: u32 = 4;
/// The first wait after a 429 or 503 (doubled each time).
pub const MODEL_BACKOFF: Duration = Duration::from_secs(5);
/// The context lines of the part before, for a later part.
pub const CONTEXT_LINES: usize = 10;

/// The host and workspace ID of a profile.
pub trait WorkspaceInfo: Send + Sync {
    /// The workspace URL (`https://<host>`).
    fn host_url(&self, profile: &str) -> CoreResult<String>;
    /// The workspace ID (`x-databricks-org-id`, W-06a).
    fn workspace_id(&self, profile: &str) -> CoreResult<String>;
}

/// [`WorkspaceInfo`] over T7 profile hosts and R6 (the existing SCIM / org-id seam).
pub struct ProfileWorkspace {
    /// T7 hosts.
    pub hosts: Arc<dyn ProfileHosts>,
    /// R6.
    pub tokens: Arc<dyn TokenBroker>,
}

impl WorkspaceInfo for ProfileWorkspace {
    fn host_url(&self, profile: &str) -> CoreResult<String> {
        self.hosts.workspace_url(profile)
    }
    fn workspace_id(&self, profile: &str) -> CoreResult<String> {
        self.tokens.workspace_id(profile)
    }
}

/// The seams of [`RecordingCloudService`].
pub struct CloudDeps {
    /// R7 recording routes.
    pub http: Arc<dyn RecordingHttp>,
    /// Host and workspace ID.
    pub workspace: Arc<dyn WorkspaceInfo>,
    /// Wall time for job.json stamps.
    pub wall: Arc<dyn WallClock>,
    /// Waits between model retries.
    pub pause: Arc<dyn Pause>,
}

/// The result of a storage probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageProbe {
    /// The volume root to use.
    Ready {
        /// The root path, no trailing `/`.
        root: String,
    },
    /// `bottle_audio` does not exist in the `catalog.schema` yet; ask the user to create it.
    NeedsVolume {
        /// The catalog.
        catalog: String,
        /// The schema.
        schema: String,
        /// The volume path it would have.
        path: String,
    },
}

/// One local temp part.
#[derive(Debug, Clone, PartialEq)]
pub struct LocalPart {
    /// The temp file (`.opus` Ogg Opus; `.flac` or `.wav` also work).
    pub path: PathBuf,
    /// Its duration in seconds.
    pub duration_s: f64,
}

/// One finished capture to process.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingJob {
    /// The volume root.
    pub root: String,
    /// The recording ID.
    pub id: String,
    /// The recording name (folder name).
    pub name: String,
    /// The start time (UTC, RFC 3339).
    pub started_at: String,
    /// The instructions for the notes.
    pub instructions: String,
    /// The transcribe model endpoint.
    pub transcribe_endpoint: String,
    /// The notes model endpoint.
    pub notes_endpoint: String,
    /// The MOG ID for the request tags (SEC-06a).
    pub mog: String,
    /// The local temp parts, in order (normally one: the whole recording fits one request).
    pub parts: Vec<LocalPart>,
}

/// The detail of one recording.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordingDetail {
    /// The manifest.
    pub job: JobManifest,
    /// The transcript, when written.
    pub transcript: Option<TranscriptDoc>,
    /// The notes markdown, when written.
    pub notes: Option<String>,
    /// The transcript markdown, when written.
    pub transcript_md: Option<String>,
    /// The instructions sidecar, when written.
    pub instructions: Option<String>,
    /// The Catalog Explorer link of the folder.
    pub link: String,
}

/// The recording cloud service.
pub struct RecordingCloudService {
    d: CloudDeps,
}

impl std::fmt::Debug for RecordingCloudService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingCloudService")
            .finish_non_exhaustive()
    }
}

fn obj(v: Value) -> JsonObject {
    match v {
        Value::Object(m) => m.into_iter().collect(),
        _ => JsonObject::new(),
    }
}

type Kind<T> = Result<T, ErrorKind>;
type Staged<T> = Result<T, (ErrorKind, Stage)>;

/// `<name>.<ext>` or `<name>.partNN.<ext>`, `ext` one of `opus`, `ogg`, `flac`, `wav`.
pub fn is_audio_file_of(name: &str, file: &str) -> bool {
    let Some(rest) = file.strip_prefix(name).and_then(|r| r.strip_prefix('.')) else {
        return false;
    };
    let rest = match rest.strip_prefix("part") {
        Some(r)
            if r.len() > 3
                && r.as_bytes()[..2].iter().all(u8::is_ascii_digit)
                && r.as_bytes()[2] == b'.' =>
        {
            &r[3..]
        }
        Some(_) => return false,
        None => rest,
    };
    is_recording_name(name) && matches!(rest, "opus" | "ogg" | "flac" | "wav")
}

/// The file extension of a local temp file (`opus`, Ogg Opus, when it has none Bottle knows).
fn part_ext(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()) {
        Some("flac") => "flac",
        Some("wav") => "wav",
        _ => "opus",
    }
}

/// Add a manifest entry for each local part that the manifest does not list yet (a part that
/// the live upload never saw, or a recording that a crash left), and keep the entries in file
/// order. A part with an unknown length is measured from its Ogg Opus granule position.
pub fn merge_local_parts(m: &mut JobManifest, parts: &[LocalPart]) {
    for l in parts {
        let Some(file) = l.path.file_name().and_then(|f| f.to_str()) else {
            continue;
        };
        if !is_audio_file_of(&m.name, file) || m.parts.iter().any(|e| e.file == file) {
            continue;
        }
        let dur = if l.duration_s > 0.0 {
            l.duration_s
        } else {
            std::fs::read(&l.path)
                .ok()
                .and_then(|b| stitch::duration_s(&b))
                .unwrap_or(0.0)
        };
        m.parts.push(PartEntry::new(file, dur));
    }
    m.parts.sort_by(|a, b| a.file.cmp(&b.file));
    m.duration_s = m.parts.iter().map(|p| p.duration_s).sum();
}

impl RecordingCloudService {
    /// A service on `deps`.
    pub fn new(deps: CloudDeps) -> Self {
        Self { d: deps }
    }

    fn now(&self) -> String {
        iso_utc(self.d.wall.now_ms())
    }

    fn send(&self, p: &str, route: &str, params: Value, body: ReqBody<'_>) -> CoreResult<Reply> {
        self.d.http.send(p, route, &obj(params), body, None)
    }

    // ---------- Files API helpers ----------

    fn mkdir(&self, p: &str, path: &str, fallback: ErrorKind) -> Kind<()> {
        let r = self
            .send(
                p,
                "files.create_directory",
                json!({"path": path}),
                ReqBody::None,
            )
            .map_err(|e| from_core(&e, fallback))?;
        if r.ok() || r.status == 409 {
            Ok(())
        } else {
            Err(from_files_status(r.status, fallback))
        }
    }

    fn upload(
        &self,
        p: &str,
        path: &str,
        bytes: &[u8],
        overwrite: bool,
        fallback: ErrorKind,
    ) -> Kind<()> {
        let ow = if overwrite { "true" } else { "false" };
        let r = self
            .send(
                p,
                "files.upload",
                json!({"path": path, "overwrite": ow}),
                ReqBody::Bytes(bytes),
            )
            .map_err(|e| from_core(&e, fallback))?;
        // A part that is already there (409 with overwrite=false) was a whole upload: the Files
        // API writes a file in one request.
        if r.ok() || (!overwrite && r.status == 409) {
            Ok(())
        } else {
            Err(from_files_status(r.status, fallback))
        }
    }

    /// The bytes of a file, `None` when it does not exist.
    fn download(&self, p: &str, path: &str, fallback: ErrorKind) -> Kind<Option<Vec<u8>>> {
        let r = self
            .send(p, "files.download", json!({"path": path}), ReqBody::None)
            .map_err(|e| from_core(&e, fallback))?;
        match (r.ok(), r.status, r.body) {
            (true, _, ReplyBody::Bytes(b)) => Ok(Some(b)),
            (true, _, ReplyBody::Json(v)) => Ok(Some(serde_json::to_vec(&v).unwrap_or_default())),
            (true, _, ReplyBody::Empty) => Ok(Some(Vec::new())),
            (false, 404, _) => Ok(None),
            (false, s, _) => Err(from_files_status(s, fallback)),
        }
    }

    fn read_job(&self, p: &str, root: &VolumeRoot, name: &str) -> Kind<Option<JobManifest>> {
        let path = format!("{}/{}", root.folder(name), job_file(name));
        Ok(self
            .download(p, &path, ErrorKind::VolumeNotFound)?
            .and_then(|b| serde_json::from_slice(&b).ok()))
    }

    fn write_job(
        &self,
        p: &str,
        root: &VolumeRoot,
        m: &JobManifest,
        fallback: ErrorKind,
    ) -> Kind<()> {
        let path = format!("{}/{}", root.folder(&m.name), job_file(&m.name));
        let text = serde_json::to_vec_pretty(m).map_err(|_| fallback)?;
        self.upload(p, &path, &text, true, fallback)
    }

    // ---------- storage ----------

    fn probe_dir(&self, p: &str, path: &str) -> Kind<Reply> {
        self.send(
            p,
            "files.list_directory",
            json!({"path": path}),
            ReqBody::None,
        )
        .map_err(|e| from_core(&e, ErrorKind::VolumeNotFound))
    }

    /// Resolve the Settings storage to a volume root (read-only `GET fs/directories` probes).
    pub fn probe_storage(
        &self,
        p: &str,
        storage: &Storage,
    ) -> Result<StorageProbe, RecordingError> {
        let e = RecordingError::new;
        match storage {
            Storage::MyFiles => {
                let me = self
                    .send(p, "recording.me", json!({}), ReqBody::None)
                    .map_err(|x| e(from_core(&x, ErrorKind::MyFilesUnavailable)))?;
                let user = me
                    .json()
                    .get("userName")
                    .and_then(Value::as_str)
                    .filter(|_| me.ok())
                    .ok_or(e(ErrorKind::MyFilesUnavailable))?
                    .to_owned();
                let host = self
                    .d
                    .workspace
                    .host_url(p)
                    .map_err(|x| e(from_core(&x, ErrorKind::MyFilesUnavailable)))?;
                let ws = self.d.workspace.workspace_id(p).unwrap_or_default();
                for cand in my_files_candidates(&ws, &host, &user) {
                    if self.probe_dir(p, &cand).map_err(e)?.ok() {
                        return Ok(StorageProbe::Ready {
                            root: cand.trim_end_matches('/').to_owned(),
                        });
                    }
                }
                Err(e(ErrorKind::MyFilesUnavailable))
            }
            Storage::Volume { path } => {
                let root = VolumeRoot::parse(path).map_err(e)?;
                let r = self.probe_dir(p, &format!("{}/", root.path())).map_err(e)?;
                if r.ok() {
                    Ok(StorageProbe::Ready {
                        root: root.path().to_owned(),
                    })
                } else {
                    Err(e(from_files_status(r.status, ErrorKind::VolumeNotFound)))
                }
            }
            Storage::CatalogSchema { catalog, schema } => {
                let path = managed_volume_path(catalog, schema).map_err(e)?;
                let r = self.probe_dir(p, &format!("{path}/")).map_err(e)?;
                match r.status {
                    s if (200..300).contains(&s) => Ok(StorageProbe::Ready { root: path }),
                    404 => Ok(StorageProbe::NeedsVolume {
                        catalog: catalog.clone(),
                        schema: schema.clone(),
                        path,
                    }),
                    s => Err(e(from_files_status(s, ErrorKind::VolumeNotFound))),
                }
            }
        }
    }

    /// Create the managed volume `bottle_audio` in `catalog.schema`. Call it only after the user
    /// confirmed. An existing volume counts as created.
    pub fn create_volume(
        &self,
        p: &str,
        catalog: &str,
        schema: &str,
    ) -> Result<String, RecordingError> {
        let e = RecordingError::new;
        let path = managed_volume_path(catalog, schema).map_err(e)?;
        let body = json!({"catalog_name": catalog, "schema_name": schema,
            "name": MANAGED_VOLUME, "volume_type": "MANAGED"});
        let r = self
            .send(p, "volumes.create", json!({}), ReqBody::Json(&body))
            .map_err(|x| e(from_core(&x, ErrorKind::VolumeNotFound)))?;
        if r.ok() || r.status == 409 || r.error_code() == "RESOURCE_ALREADY_EXISTS" {
            Ok(path)
        } else {
            Err(e(from_files_status(r.status, ErrorKind::VolumeNotFound)))
        }
    }

    // ---------- model calls ----------

    /// One chat call with retries on a 429 (the R7 bucket holds the `Retry-After`) or a 503.
    fn model(&self, p: &str, body: &Value, tags: &Tags, call: ModelCall) -> Kind<String> {
        let fallback = match call {
            ModelCall::Transcribe => ErrorKind::TranscribeFailed,
            ModelCall::Notes => ErrorKind::NotesFailed,
        };
        let mut wait = MODEL_BACKOFF;
        for attempt in 1..=MODEL_ATTEMPTS {
            let last = attempt == MODEL_ATTEMPTS;
            match self.d.http.send(
                p,
                MODEL_ROUTE,
                &JsonObject::new(),
                ReqBody::Json(body),
                Some(tags),
            ) {
                Err(x) if x.code() == "rate_limited" && !last => {}
                Err(x) => return Err(from_core(&x, fallback)),
                Ok(r) if r.ok() => return reply_text(r.json()).ok_or(fallback),
                Ok(r) if r.status == 503 && !last => {}
                Ok(r) => {
                    return Err(from_model_reply(
                        call,
                        r.status,
                        r.error_code(),
                        r.message(),
                    ))
                }
            }
            self.d.pause.pause(wait);
            wait *= 2;
        }
        Err(fallback)
    }

    fn tags(mog: &str, op: TagOp) -> Tags {
        Tags {
            app: TagApp::Bottle,
            mog: mog.to_owned(),
            op,
        }
    }

    // ---------- pipeline ----------

    fn finish_fail(
        &self,
        p: &str,
        root: Option<&VolumeRoot>,
        m: &mut JobManifest,
        kind: ErrorKind,
        stage: Stage,
        on: &mut dyn FnMut(&JobManifest),
    ) {
        m.fail(kind, stage, &self.now());
        if let Some(root) = root {
            // Best effort: the volume may be the thing that failed.
            let _ = self.write_job(p, root, m, kind);
        }
        on(m);
    }

    fn update(
        &self,
        p: &str,
        root: &VolumeRoot,
        m: &mut JobManifest,
        s: Status,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Kind<()> {
        m.set_status(s, &self.now());
        let fallback = match s {
            Status::Uploading | Status::Recording => ErrorKind::UploadFailed,
            Status::Transcribing => ErrorKind::TranscribeFailed,
            _ => ErrorKind::NotesFailed,
        };
        self.write_job(p, root, m, fallback)?;
        on(m);
        Ok(())
    }

    fn check_job(&self, job: &RecordingJob) -> Kind<VolumeRoot> {
        let root = VolumeRoot::parse(&job.root)?;
        let ok = is_recording_name(&job.name)
            && valid_instructions(&job.instructions)
            && is_endpoint_name(&job.transcribe_endpoint)
            && is_endpoint_name(&job.notes_endpoint);
        ok.then_some(root).ok_or(ErrorKind::BadSettings)
    }

    /// Process a finished capture: upload, transcribe, notes. Returns the final manifest
    /// (`done` or `failed`); `on` sees each change.
    pub fn run(
        &self,
        p: &str,
        job: &RecordingJob,
        on: &mut dyn FnMut(&JobManifest),
    ) -> JobManifest {
        let parts = job
            .parts
            .iter()
            .enumerate()
            .map(|(i, l)| {
                let ext = part_ext(&l.path);
                // The local name when it fits (PR B writes `<name>.opus`); else one part is
                // `<name>.opus` and more are `<name>.partNN.opus`.
                let local = l.path.file_name().and_then(|f| f.to_str());
                let file = match local.filter(|f| is_audio_file_of(&job.name, f)) {
                    Some(f) => f.to_owned(),
                    None if job.parts.len() == 1 => format!("{}.{ext}", job.name),
                    None => part_file(&job.name, i as u32 + 1, ext),
                };
                // A part of unknown length is measured from its Ogg Opus granule position.
                let dur = if l.duration_s > 0.0 {
                    l.duration_s
                } else {
                    std::fs::read(&l.path)
                        .ok()
                        .and_then(|b| stitch::duration_s(&b))
                        .unwrap_or(0.0)
                };
                PartEntry::new(&file, dur)
            })
            .collect();
        let mut m = JobManifest::new(
            &job.id,
            &job.name,
            &job.started_at,
            parts,
            &job.transcribe_endpoint,
            &job.notes_endpoint,
            &self.now(),
        );
        match self.check_job(job) {
            Ok(root) => self.run_from_stage(p, &root, job, m, Stage::Uploading, on),
            Err(k) => {
                self.finish_fail(p, None, &mut m, k, Stage::Uploading, on);
                m
            }
        }
    }

    /// Retry a recording from its failed stage (job.json in the volume). With no job.json it
    /// runs the whole pipeline. A recording that is done is returned as it is.
    pub fn retry(
        &self,
        p: &str,
        job: &RecordingJob,
        on: &mut dyn FnMut(&JobManifest),
    ) -> JobManifest {
        let Ok(root) = self.check_job(job) else {
            return self.run(p, job, on);
        };
        let mut m = match self.read_job(p, &root, &job.name) {
            Ok(Some(m)) => m,
            _ => return self.run(p, job, on),
        };
        // Parts that the live upload never listed (a crash) join the manifest.
        merge_local_parts(&mut m, &job.parts);
        let stage = match (m.status, m.error_stage) {
            (Status::Done, _) => return m,
            (Status::Failed, Some(s)) => s,
            (Status::Transcribing, _) => Stage::Transcribing,
            (Status::WritingNotes, _) => Stage::WritingNotes,
            _ => Stage::Uploading,
        };
        self.run_from_stage(p, &root, job, m, stage, on)
    }

    fn run_from_stage(
        &self,
        p: &str,
        root: &VolumeRoot,
        job: &RecordingJob,
        mut m: JobManifest,
        stage: Stage,
        on: &mut dyn FnMut(&JobManifest),
    ) -> JobManifest {
        // Retry takes the endpoints of Settings now, so a model fix in Settings applies.
        m.transcribe_endpoint = job.transcribe_endpoint.clone();
        m.notes_endpoint = job.notes_endpoint.clone();
        let mut cache: HashMap<usize, Vec<u8>> = HashMap::new();
        if stage == Stage::Uploading {
            match self.upload_stage(p, root, job, &mut m, on) {
                Ok(c) => cache = c,
                Err(k) => {
                    self.finish_fail(p, Some(root), &mut m, k, Stage::Uploading, on);
                    return m;
                }
            }
        }
        let doc = if stage == Stage::WritingNotes {
            self.read_transcript(p, root, &m.name)
        } else {
            self.transcribe_stage(p, root, job, &mut m, &mut cache, on)
        };
        let r = doc.and_then(|doc| self.notes_stage(p, root, job, &mut m, &doc, on));
        if let Err((k, s)) = r {
            self.finish_fail(p, Some(root), &mut m, k, s, on);
        }
        m
    }

    /// Upload the instructions, the manifest and each part not uploaded yet. Returns the bytes
    /// of the parts uploaded now, so the transcribe stage does not read them back.
    fn upload_stage(
        &self,
        p: &str,
        root: &VolumeRoot,
        job: &RecordingJob,
        m: &mut JobManifest,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Kind<HashMap<usize, Vec<u8>>> {
        let f = ErrorKind::UploadFailed;
        let folder = root.folder(&job.name);
        self.mkdir(p, &folder, f)?;
        let instr = format!("{folder}/{}", instructions_file(&job.name));
        self.upload(p, &instr, job.instructions.as_bytes(), true, f)?;
        self.update(p, root, m, Status::Uploading, on)?;
        let mut cache = HashMap::new();
        for i in 0..m.parts.len() {
            if m.parts[i].uploaded {
                continue;
            }
            // Match the local file by name (Retry gets only the files still on disk); by
            // position only when the local list is the whole recording.
            let file = m.parts[i].file.clone();
            let local = job
                .parts
                .iter()
                .find(|l| l.path.file_name().and_then(|f| f.to_str()) == Some(file.as_str()))
                .or_else(|| (job.parts.len() == m.parts.len()).then(|| &job.parts[i]))
                .ok_or(ErrorKind::AudioMissing)?;
            let bytes = self.upload_one(p, root, &folder, m, i, local, on)?;
            cache.insert(i, bytes);
        }
        Ok(cache)
    }

    /// Upload part `i` of `m`, then write job.json, then delete the local file (ADR 0103 order).
    /// If the job.json write fails, the part counts as not uploaded, the local file stays, and a
    /// retry uploads it again (a 409 counts as uploaded). Returns the audio bytes.
    #[allow(clippy::too_many_arguments)]
    fn upload_one(
        &self,
        p: &str,
        root: &VolumeRoot,
        folder: &str,
        m: &mut JobManifest,
        i: usize,
        local: &LocalPart,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Kind<Vec<u8>> {
        let bytes = std::fs::read(&local.path).map_err(|_| ErrorKind::AudioMissing)?;
        if sniff(&bytes).is_none() {
            return Err(ErrorKind::AudioUnreadable);
        }
        let dest = format!("{folder}/{}", m.parts[i].file);
        self.upload(p, &dest, &bytes, false, ErrorKind::UploadFailed)?;
        m.parts[i].uploaded = true;
        if let Err(k) = self.touch(p, root, m, on) {
            m.parts[i].uploaded = false;
            return Err(k);
        }
        // A failed delete leaves a temp file that the launch scan shows again; it is never a
        // reason to fail the stage.
        let _ = std::fs::remove_file(&local.path);
        Ok(bytes)
    }

    /// Write job.json as it is (the status stays), with a fresh stamp, and tell `on`.
    fn touch(
        &self,
        p: &str,
        root: &VolumeRoot,
        m: &mut JobManifest,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Kind<()> {
        m.updated_at = self.now();
        self.write_job(p, root, m, ErrorKind::UploadFailed)?;
        on(m);
        Ok(())
    }

    // ---------- upload during the call ----------

    /// The folder, the instructions and a job.json in `recording`, at the start of a call (or
    /// at the resume of one after a crash: a job.json that is in the volume already is kept).
    /// Call it again after a failure: every step can run twice.
    pub fn live_begin(
        &self,
        p: &str,
        job: &RecordingJob,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Kind<JobManifest> {
        let root = self.check_job(job)?;
        let f = ErrorKind::UploadFailed;
        let folder = root.folder(&job.name);
        self.mkdir(p, &folder, f)?;
        let instr = format!("{folder}/{}", instructions_file(&job.name));
        self.upload(p, &instr, job.instructions.as_bytes(), true, f)?;
        let mut m = match self.read_job(p, &root, &job.name) {
            Ok(Some(m)) => m,
            _ => JobManifest::new(
                &job.id,
                &job.name,
                &job.started_at,
                Vec::new(),
                &job.transcribe_endpoint,
                &job.notes_endpoint,
                &self.now(),
            ),
        };
        m.set_status(Status::Recording, &self.now());
        self.write_job(p, &root, &m, f)?;
        on(&m);
        Ok(m)
    }

    /// Upload one finished part while the call goes on: upload, then job.json, then delete the
    /// local file. A part that is in the volume already (it is marked in `m`) is not sent
    /// again. `m` keeps its status (`recording`).
    pub fn live_upload_part(
        &self,
        p: &str,
        job: &RecordingJob,
        m: &mut JobManifest,
        local: &LocalPart,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Kind<()> {
        let root = self.check_job(job)?;
        let file = local
            .path
            .file_name()
            .and_then(|f| f.to_str())
            .filter(|f| is_audio_file_of(&job.name, f))
            .ok_or(ErrorKind::AudioUnreadable)?
            .to_owned();
        if m.parts.iter().any(|e| e.file == file && e.uploaded) {
            let _ = std::fs::remove_file(&local.path);
            return Ok(());
        }
        merge_local_parts(m, std::slice::from_ref(local));
        let i = m
            .parts
            .iter()
            .position(|e| e.file == file)
            .ok_or(ErrorKind::AudioMissing)?;
        let folder = root.folder(&job.name);
        self.upload_one(p, &root, &folder, m, i, local, on)
            .map(|_| ())
    }

    /// The call stopped: upload what is left, then run the one transcription over the whole call
    /// and write the notes. `m` is the manifest of the live upload, if there is one; `job.parts`
    /// holds the parts still on the device.
    pub fn live_finish(
        &self,
        p: &str,
        job: &RecordingJob,
        m: Option<JobManifest>,
        on: &mut dyn FnMut(&JobManifest),
    ) -> JobManifest {
        let root = match self.check_job(job) {
            Ok(r) => r,
            Err(k) => {
                let mut m = m.unwrap_or_else(|| self.fresh_manifest(job));
                self.finish_fail(p, None, &mut m, k, Stage::Uploading, on);
                return m;
            }
        };
        let mut m = match m {
            Some(m) => m,
            None => match self.read_job(p, &root, &job.name) {
                Ok(Some(m)) => m,
                _ => self.fresh_manifest(job),
            },
        };
        merge_local_parts(&mut m, &job.parts);
        m.started_at = job.started_at.clone();
        self.run_from_stage(p, &root, job, m, Stage::Uploading, on)
    }

    fn fresh_manifest(&self, job: &RecordingJob) -> JobManifest {
        JobManifest::new(
            &job.id,
            &job.name,
            &job.started_at,
            Vec::new(),
            &job.transcribe_endpoint,
            &job.notes_endpoint,
            &self.now(),
        )
    }

    fn transcribe_stage(
        &self,
        p: &str,
        root: &VolumeRoot,
        job: &RecordingJob,
        m: &mut JobManifest,
        cache: &mut HashMap<usize, Vec<u8>>,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Staged<TranscriptDoc> {
        let t = Stage::Transcribing;
        let at = |k: ErrorKind| (k, t);
        self.update(p, root, m, Status::Transcribing, on)
            .map_err(at)?;
        if m.parts.is_empty() {
            return Err(at(ErrorKind::AudioMissing));
        }
        let folder = root.folder(&m.name);
        let tags = Self::tags(&job.mog, TagOp::Asr);
        // Every part, in order: the bytes this run uploaded, else the volume copy.
        let mut audio: Vec<Vec<u8>> = Vec::with_capacity(m.parts.len());
        for (i, part) in m.parts.iter().enumerate() {
            audio.push(match cache.remove(&i) {
                Some(b) => b,
                None => self
                    .download(
                        p,
                        &format!("{folder}/{}", part.file),
                        ErrorKind::TranscribeFailed,
                    )
                    .map_err(at)?
                    .ok_or(at(ErrorKind::AudioMissing))?,
            });
        }
        // One pass over the whole call (ADR 0103 follow-up): the Ogg Opus parts join into one
        // stream, so the speaker labels stay the same from start to end. Anything else (FLAC,
        // WAV, a part that does not join) goes part by part, as before.
        let joined = if audio.len() > 1 {
            let refs: Vec<&[u8]> = audio.iter().map(Vec::as_slice).collect();
            stitch::join(&refs).ok()
        } else {
            None
        };
        let mut inputs = Vec::with_capacity(m.parts.len());
        let mut context: Option<String> = None;
        if let Some(bytes) = joined {
            drop(audio);
            let body = transcribe_request(&m.transcribe_endpoint, &bytes, None).map_err(at)?;
            drop(bytes);
            let text = self
                .model(p, &body, &tags, ModelCall::Transcribe)
                .map_err(at)?;
            let total: f64 = m.parts.iter().map(|x| x.duration_s).sum();
            let dur = (total > 0.0).then_some(total);
            inputs.push(PartInput {
                file: m.parts[0].file.clone(),
                duration_s: dur,
                transcript: PartTranscript {
                    duration_s: total,
                    segments: parse_transcript(&text, dur),
                },
            });
        } else {
            for (part, bytes) in m.parts.iter().zip(audio) {
                let body = transcribe_request(&m.transcribe_endpoint, &bytes, context.as_deref())
                    .map_err(at)?;
                drop(bytes);
                let text = self
                    .model(p, &body, &tags, ModelCall::Transcribe)
                    .map_err(at)?;
                let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
                context = Some(lines[lines.len().saturating_sub(CONTEXT_LINES)..].join("\n"));
                let dur = (part.duration_s > 0.0).then_some(part.duration_s);
                inputs.push(PartInput {
                    file: part.file.clone(),
                    duration_s: dur,
                    transcript: PartTranscript {
                        duration_s: part.duration_s,
                        segments: parse_transcript(&text, dur),
                    },
                });
            }
        }
        let mut merged = merge_parts(&inputs);
        if inputs.len() == 1 && m.parts.len() > 1 {
            // One transcript over the joined parts: list the real files with their offsets.
            let mut offset = 0.0;
            merged.parts = m
                .parts
                .iter()
                .map(|x| {
                    let info = PartInfo {
                        file: x.file.clone(),
                        offset_s: offset,
                        duration_s: x.duration_s,
                    };
                    offset += x.duration_s;
                    info
                })
                .collect();
        }
        let doc = TranscriptDoc {
            version: 1,
            id: m.id.clone(),
            name: m.name.clone(),
            started_at: m.started_at.clone(),
            duration_s: merged.duration_s,
            instructions: job.instructions.clone(),
            parts: merged.parts,
            segments: merged.segments,
        };
        let f = ErrorKind::TranscribeFailed;
        let tj = serde_json::to_vec_pretty(&doc).map_err(|_| at(f))?;
        self.upload(
            p,
            &format!("{folder}/{}", transcript_json_file(&m.name)),
            &tj,
            true,
            f,
        )
        .map_err(at)?;
        let md = render_markdown(&doc);
        self.upload(
            p,
            &format!("{folder}/{}", transcript_md_file(&m.name)),
            md.as_bytes(),
            true,
            f,
        )
        .map_err(at)?;
        Ok(doc)
    }

    fn read_transcript(&self, p: &str, root: &VolumeRoot, name: &str) -> Staged<TranscriptDoc> {
        let path = format!("{}/{}", root.folder(name), transcript_json_file(name));
        let b = self
            .download(p, &path, ErrorKind::NotesFailed)
            .map_err(|k| (k, Stage::WritingNotes))?
            .ok_or((ErrorKind::TranscribeFailed, Stage::Transcribing))?;
        serde_json::from_slice(&b).map_err(|_| (ErrorKind::TranscribeFailed, Stage::Transcribing))
    }

    fn notes_stage(
        &self,
        p: &str,
        root: &VolumeRoot,
        job: &RecordingJob,
        m: &mut JobManifest,
        doc: &TranscriptDoc,
        on: &mut dyn FnMut(&JobManifest),
    ) -> Staged<()> {
        let n = Stage::WritingNotes;
        let at = |k: ErrorKind| (k, n);
        self.update(p, root, m, Status::WritingNotes, on)
            .map_err(at)?;
        let body = notes_request(&m.notes_endpoint, &doc.instructions, &render_markdown(doc))
            .map_err(at)?;
        let notes = self
            .model(
                p,
                &body,
                &Self::tags(&job.mog, TagOp::Writer),
                ModelCall::Notes,
            )
            .map_err(at)?;
        let path = format!("{}/{}", root.folder(&m.name), notes_file(&m.name));
        self.upload(p, &path, notes.as_bytes(), true, ErrorKind::NotesFailed)
            .map_err(at)?;
        self.update(p, root, m, Status::Done, on).map_err(at)
    }

    // ---------- single steps (the contract `RecordingCloud` seam) ----------

    /// Upload one local audio file into the recording folder (`overwrite=false`; a 409 counts
    /// as uploaded). The file name must be `<name>.opus` or `<name>.partNN.<ext>`.
    pub fn upload_audio(
        &self,
        p: &str,
        root: &str,
        name: &str,
        local: &std::path::Path,
    ) -> Kind<String> {
        let root = VolumeRoot::parse(root)?;
        let file = local
            .file_name()
            .and_then(|f| f.to_str())
            .filter(|f| is_audio_file_of(name, f))
            .ok_or(ErrorKind::AudioUnreadable)?
            .to_owned();
        let bytes = std::fs::read(local).map_err(|_| ErrorKind::AudioMissing)?;
        if sniff(&bytes).is_none() {
            return Err(ErrorKind::AudioUnreadable);
        }
        let folder = root.folder(name);
        self.mkdir(p, &folder, ErrorKind::UploadFailed)?;
        self.upload(
            p,
            &format!("{folder}/{file}"),
            &bytes,
            false,
            ErrorKind::UploadFailed,
        )?;
        Ok(file)
    }

    /// Transcribe one audio file (part-relative times). `context`: the last lines before it.
    pub fn transcribe_audio(
        &self,
        p: &str,
        endpoint: &str,
        mog: &str,
        local: &std::path::Path,
        duration_s: f64,
        context: Option<&str>,
    ) -> Kind<Vec<super::transcript::RawSegment>> {
        let bytes = std::fs::read(local).map_err(|_| ErrorKind::AudioMissing)?;
        let body = transcribe_request(endpoint, &bytes, context)?;
        drop(bytes);
        let text = self.model(
            p,
            &body,
            &Self::tags(mog, TagOp::Asr),
            ModelCall::Transcribe,
        )?;
        Ok(parse_transcript(
            &text,
            (duration_s > 0.0).then_some(duration_s),
        ))
    }

    /// The notes of a transcript.
    pub fn notes(
        &self,
        p: &str,
        endpoint: &str,
        mog: &str,
        instructions: &str,
        transcript_md: &str,
    ) -> Kind<String> {
        let body = notes_request(endpoint, instructions, transcript_md)?;
        self.model(p, &body, &Self::tags(mog, TagOp::Writer), ModelCall::Notes)
    }

    /// Write one text file into the recording folder (`overwrite=true`).
    pub fn write_text(&self, p: &str, root: &str, name: &str, file: &str, text: &[u8]) -> Kind<()> {
        let root = VolumeRoot::parse(root)?;
        if !is_recording_name(name) || !file.starts_with(name) || file.contains('/') {
            return Err(ErrorKind::BadSettings);
        }
        let folder = root.folder(name);
        self.mkdir(p, &folder, ErrorKind::UploadFailed)?;
        self.upload(
            p,
            &format!("{folder}/{file}"),
            text,
            true,
            ErrorKind::UploadFailed,
        )
    }

    /// The job manifest of a recording, if it exists.
    pub fn job(&self, p: &str, root: &str, name: &str) -> Kind<Option<JobManifest>> {
        let root = VolumeRoot::parse(root)?;
        if !is_recording_name(name) {
            return Err(ErrorKind::BadSettings);
        }
        self.read_job(p, &root, name)
    }

    /// The instructions sidecar of a recording, if it exists.
    pub fn instructions(&self, p: &str, root: &str, name: &str) -> Kind<Option<String>> {
        let r = VolumeRoot::parse(root)?;
        if !is_recording_name(name) {
            return Err(ErrorKind::BadSettings);
        }
        let path = format!("{}/{}", r.folder(name), instructions_file(name));
        Ok(self
            .download(p, &path, ErrorKind::VolumeNotFound)?
            .and_then(|b| String::from_utf8(b).ok()))
    }

    // ---------- list, detail, link ----------

    /// The recordings in `<root>/Bottle/Recordings/`, newest first (from each job.json).
    pub fn list(&self, p: &str, root: &str) -> Result<Vec<JobManifest>, RecordingError> {
        let e = RecordingError::new;
        let root = VolumeRoot::parse(root).map_err(e)?;
        let mut names: Vec<String> = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut params = json!({"path": format!("{}/", root.recordings_dir())});
            if let Some(t) = &token {
                params["page_token"] = json!(t);
            }
            let r = self
                .send(p, "files.list_directory", params, ReqBody::None)
                .map_err(|x| e(from_core(&x, ErrorKind::VolumeNotFound)))?;
            if r.status == 404 {
                return Ok(Vec::new());
            }
            if !r.ok() {
                return Err(e(from_files_status(r.status, ErrorKind::VolumeNotFound)));
            }
            for c in r
                .json()
                .get("contents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let dir = c
                    .get("is_directory")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if let Some(n) = c
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| dir && is_recording_name(n))
                {
                    names.push(n.to_owned());
                }
            }
            token = r
                .json()
                .get("next_page_token")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if token.is_none() || names.len() > 10_000 {
                break;
            }
        }
        let mut out: Vec<JobManifest> = names
            .iter()
            .filter_map(|n| self.read_job(p, &root, n).ok().flatten())
            .collect();
        out.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(b.name.cmp(&a.name)));
        Ok(out)
    }

    /// One recording: manifest, transcript, notes and link.
    pub fn detail(
        &self,
        p: &str,
        root: &str,
        name: &str,
    ) -> Result<RecordingDetail, RecordingError> {
        let e = RecordingError::new;
        let root = VolumeRoot::parse(root).map_err(e)?;
        if !is_recording_name(name) {
            return Err(e(ErrorKind::BadSettings));
        }
        let job = self
            .read_job(p, &root, name)
            .map_err(e)?
            .ok_or(e(ErrorKind::VolumeNotFound))?;
        let folder = root.folder(name);
        let transcript = self
            .download(
                p,
                &format!("{folder}/{}", transcript_json_file(name)),
                ErrorKind::VolumeNotFound,
            )
            .map_err(e)?
            .and_then(|b| serde_json::from_slice(&b).ok());
        let notes = self
            .download(
                p,
                &format!("{folder}/{}", notes_file(name)),
                ErrorKind::VolumeNotFound,
            )
            .map_err(e)?
            .and_then(|b| String::from_utf8(b).ok());
        let text = |file: String| -> Result<Option<String>, RecordingError> {
            Ok(self
                .download(p, &format!("{folder}/{file}"), ErrorKind::VolumeNotFound)
                .map_err(e)?
                .and_then(|b| String::from_utf8(b).ok()))
        };
        let transcript_md = text(transcript_md_file(name))?;
        let instructions = text(instructions_file(name))?;
        let link = self.link_for(p, &root, name).map_err(e)?;
        Ok(RecordingDetail {
            job,
            transcript,
            notes,
            transcript_md,
            instructions,
            link,
        })
    }

    fn link_for(&self, p: &str, root: &VolumeRoot, name: &str) -> Kind<String> {
        let host = self
            .d
            .workspace
            .host_url(p)
            .map_err(|x| from_core(&x, ErrorKind::BadSettings))?;
        let ws = self
            .d
            .workspace
            .workspace_id(p)
            .map_err(|x| from_core(&x, ErrorKind::BadSettings))?;
        catalog_explorer_url(&host, &ws, root, name)
    }

    /// The Catalog Explorer link of a recording folder (host from the profile).
    pub fn link(&self, p: &str, root: &str, name: &str) -> Result<String, RecordingError> {
        let e = RecordingError::new;
        let root = VolumeRoot::parse(root).map_err(e)?;
        self.link_for(p, &root, name).map_err(e)
    }

    /// The profile host (for [`super::opener::open_link`]).
    pub fn host_url(&self, p: &str) -> CoreResult<String> {
        self.d.workspace.host_url(p)
    }
}
