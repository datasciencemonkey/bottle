//! Serde mirror of the IPC-facing types of `src/shared/interfaces.ts` (section 4.7 of the
//! requirements, and `ProfileInfo` / `TokenStatus` of 4.5) (#222).
//!
//! The JSON shape is the TS shape: camelCase fields where TS uses camelCase, snake_case
//! where TS uses snake_case (`RouterDecision`, Appendix D). Every struct rejects unknown
//! fields, and optional fields are absent (not `null`) when they are `None`.
//! `crates/mog-core/tests/ipc_types.rs` round-trips the shared fixtures in
//! `tests/fixtures/ipc-types`.
//!
//! ARC-6: none of these types holds a token or another credential, so `Debug` output is
//! safe to log.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A MOG id (uuid v4).
pub type MogId = String;
/// A Databricks CLI profile name.
pub type ProfileName = String;

/// A capture lease ID from R2 (CAP-02a, ADR 0007). Opaque on the wire; R2 never reuses one.
pub type LeaseId = String;

/// The 7 moods of the mascot (4.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Mood {
    Idle,
    Listening,
    Processing,
    Answering,
    Success,
    Error,
    Sleeping,
}

/// The audio track of a segment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Track {
    You,
    Others,
}

/// `Tags.app`: always `"bottle"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TagApp {
    Bottle,
}

/// `Tags.op`: the operation a call belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TagOp {
    Asr,
    Cleanup,
    Router,
    Writer,
    Judge,
    Embed,
    Genie,
    Merge,
    Agent,
    Page,
}

/// Request tags.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Tags {
    pub app: TagApp,
    pub mog: MogId,
    pub op: TagOp,
}

/// The capture mode (D53, D54, D63).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    Ptt,
    HandsFree,
    Session,
}

/// Session limits. In TS every field except `hard_cap_min` is a fixed literal
/// (55, 60, 30, 5, 15, 10, 120, 5); `hard_cap_min` is 60..480, default 240.
///
/// Deserializing rejects any other value (#259), so a `SessionPolicy` read from JSON is
/// always valid. A value built in Rust can be checked with [`SessionPolicy::validate`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    rename_all = "camelCase",
    deny_unknown_fields,
    try_from = "SessionPolicyWire"
)]
pub struct SessionPolicy {
    pub check_in_at_min: u32,
    pub soft_cap_min: u32,
    pub extend_min: u32,
    pub silence_nudge_min: u32,
    pub silence_stop_min: u32,
    pub hard_cap_min: u32,
    pub lock_stop_min: u32,
    pub hold_max_sec: u32,
    pub hands_free_max_min: u32,
}

/// The unvalidated wire form of [`SessionPolicy`]. schemars follows `try_from`, so it
/// derives `JsonSchema` too.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SessionPolicyWire {
    check_in_at_min: u32,
    soft_cap_min: u32,
    extend_min: u32,
    silence_nudge_min: u32,
    silence_stop_min: u32,
    hard_cap_min: u32,
    lock_stop_min: u32,
    hold_max_sec: u32,
    hands_free_max_min: u32,
}

/// The `hardCapMin` range in minutes (section 7.1: ceiling 4 h, admin range 1-8 h).
const HARD_CAP_MIN_RANGE: (u32, u32) = (60, 480);

/// A `SessionPolicy` value outside the spec. The field name is the TS (camelCase) name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SessionPolicyError {
    /// A field with a fixed value has another value.
    NotFixed {
        /// The field name.
        field: &'static str,
        /// The value the spec requires.
        expected: u32,
        /// The value found.
        actual: u32,
    },
    /// A field is outside its allowed range (inclusive).
    OutOfRange {
        /// The field name.
        field: &'static str,
        /// The smallest allowed value.
        min: u32,
        /// The largest allowed value.
        max: u32,
        /// The value found.
        actual: u32,
    },
}

impl std::fmt::Display for SessionPolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionPolicyError::NotFixed {
                field,
                expected,
                actual,
            } => write!(f, "{field} must be {expected}, found {actual}"),
            SessionPolicyError::OutOfRange {
                field,
                min,
                max,
                actual,
            } => write!(f, "{field} must be {min}..={max}, found {actual}"),
        }
    }
}

impl std::error::Error for SessionPolicyError {}

impl SessionPolicy {
    /// Check every field against the stop policy of section 7.1.
    ///
    /// The fixed values (55 < 60) and the `hard_cap_min` range (60..=480) also give the
    /// ordering check-in < soft cap <= ceiling.
    pub fn validate(&self) -> Result<(), SessionPolicyError> {
        // Fixed to match the literal types in `interfaces.ts` and the D27 defaults. Section 7.1
        // and Appendix C also list admin keys `SessionSoftCapMinutes`, `SessionExtendMinutes`,
        // `SilenceNudgeMinutes` and `SilenceStopMinutes` (the check-in follows the soft cap).
        // Making them configurable must change the TS and Rust mirrors together (#368).
        let fixed = [
            ("checkInAtMin", self.check_in_at_min, 55),
            ("softCapMin", self.soft_cap_min, 60),
            ("extendMin", self.extend_min, 30),
            ("silenceNudgeMin", self.silence_nudge_min, 5),
            ("silenceStopMin", self.silence_stop_min, 15),
            ("lockStopMin", self.lock_stop_min, 10),
            ("holdMaxSec", self.hold_max_sec, 120),
            ("handsFreeMaxMin", self.hands_free_max_min, 5),
        ];
        for (field, actual, expected) in fixed {
            if actual != expected {
                return Err(SessionPolicyError::NotFixed {
                    field,
                    expected,
                    actual,
                });
            }
        }
        let (min, max) = HARD_CAP_MIN_RANGE;
        if !(min..=max).contains(&self.hard_cap_min) {
            return Err(SessionPolicyError::OutOfRange {
                field: "hardCapMin",
                min,
                max,
                actual: self.hard_cap_min,
            });
        }
        Ok(())
    }
}

impl TryFrom<SessionPolicyWire> for SessionPolicy {
    type Error = SessionPolicyError;

    fn try_from(w: SessionPolicyWire) -> Result<Self, Self::Error> {
        let p = SessionPolicy {
            check_in_at_min: w.check_in_at_min,
            soft_cap_min: w.soft_cap_min,
            extend_min: w.extend_min,
            silence_nudge_min: w.silence_nudge_min,
            silence_stop_min: w.silence_stop_min,
            hard_cap_min: w.hard_cap_min,
            lock_stop_min: w.lock_stop_min,
            hold_max_sec: w.hold_max_sec,
            hands_free_max_min: w.hands_free_max_min,
        };
        p.validate()?;
        Ok(p)
    }
}

/// `CaptureState.othersTrack`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OthersTrack {
    Off,
    Ok,
    ZeroBuffers,
}

/// `CaptureState.mic` (#453, CAP-17b): the health of the You track (the mic), for the mic
/// fix-it card. A fixed code only: no device name, no device ID (CAP-01b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MicHealth {
    Ok,
    /// 2 s of zero buffers or no buffers (CAP-17b), for example after a revoked permission.
    ZeroBuffers,
    /// The OS refused mic access (for example WASAPI `E_ACCESSDENIED`).
    PermissionDenied,
    /// No input device is present.
    NoDevice,
}

/// The `capture.state` event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<MogId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<CaptureMode>,
    /// CAP-02a: the lease that owns the mic now (ADR 0007). A suspended lease (ARC-F9) is not
    /// shown here until it resumes. Absent when no capture is live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<LeaseId>,
    pub recording: bool,
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directed_to: Option<MogId>,
    pub others_track: OthersTrack,
    /// CAP-17b (#453): the mic health for the fix-it card. The core sets `ok` at each granted
    /// lease, `zero_buffers` or `no_device` on a mic fault, and the code of a refused mic
    /// open. Absent before the first reading and in payloads from before #453.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mic: Option<MicHealth>,
}

/// The key of an app-wide setting (Appendix F, scope App; #453).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum AppSettingKey {
    /// CAP-22: "Prefer the built-in mic when a Bluetooth headset connects". Default off.
    #[serde(rename = "app.capture.preferBuiltInMic")]
    CapturePreferBuiltInMic,
}

/// One app-wide setting and its value: the `app_setting_write` args and the reply of
/// `app_setting_read` and `app_setting_write` (#453). Every app setting is a boolean now
/// (CAP-22). A later setting with another type makes `value` a typed union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSetting {
    pub key: AppSettingKey,
    pub value: bool,
}

/// Why a capture lease stopped (`CaptureStopped.reason`, #425, ADR 0055).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaptureStopReason {
    /// `capture_stop` with the lease ID.
    Manual,
    /// The 120 s hold safety stop (CAP-04a).
    HoldCap,
    /// The 5 min hands-free cap (CAP-04b).
    HandsFreeCap,
    /// A higher-priority capture took the mic (ARC-F9).
    Preempted,
    /// `capture_stop_all`, for example from the panic key (MB-05).
    Panic,
    /// The device reported a failed stream.
    DeviceLost,
    /// The talk key came up and the post-roll ended (CAP-12b).
    Released,
    /// A tap or a cancelled PTT: its audio is dropped (CAP-44b).
    Discarded,
    /// A tap stopped hands-free (CAP-14b).
    Tap,
    /// Esc stopped hands-free; its audio is dropped (CAP-14b).
    Escape,
    /// The hands-free VAD end (CAP-14c).
    VadEnd,
    /// The mic gave only digital silence (CAP-17b, #22); its audio is dropped.
    ZeroBuffers,
}

/// The `capture.stopped` event payload (#425, ADR 0055): one lease stopped. Content-free
/// (CAP-01b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CaptureStopped {
    pub lease_id: LeaseId,
    pub mog_id: MogId,
    pub mode: CaptureMode,
    pub reason: CaptureStopReason,
}

/// The `capture.segment` event payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SegmentRef {
    pub segment_id: String,
    pub mog_id: MogId,
    /// CAP-02a / AT-17: the lease of the capture that made this segment (ADR 0007).
    pub lease_id: LeaseId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub track: Track,
    pub t_start_ms: u64,
    pub t_end_ms: u64,
    #[serde(rename = "final")]
    pub is_final: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directed_to: Option<MogId>,
}

/// `Transcript.provider`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptProvider {
    LocalParakeet,
    DbxGemini,
    DbxWhisper,
}

/// The result of `stt_transcribe`. Its `Debug` holds no text (#514; #505 is the general fix).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Transcript {
    pub segment_id: String,
    pub text: String,
    pub raw: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    pub provider: TranscriptProvider,
    pub model: String,
    pub runtime: String,
    pub latency_ms: u64,
    /// Local memory slice 1 (ADR 0113): Rust found "remember this: ..." in its own transcript.
    /// The brain stores `text` as a pinned memory with `ticket` and does not paste.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remember: Option<RememberHit>,
}

/// The voice "remember" of one transcript (ADR 0113). Its `Debug` holds no text.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RememberHit {
    /// A host-minted Pin ticket bound to `text`.
    pub ticket: String,
    /// The body to remember, as Rust parsed it.
    pub text: String,
}

impl std::fmt::Debug for RememberHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RememberHit")
            .field("text_chars", &self.text.chars().count())
            .finish_non_exhaustive()
    }
}

/// The outcome of a card or voice "remember" (`memory:saved`, ADR 0113). Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MemorySaved {
    pub mog_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub outcome: MemorySavedOutcome,
    /// How long the Undo shows, in ms.
    pub undo_ms: u64,
}

/// What a "remember" did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemorySavedOutcome {
    Saved,
    Quarantined,
    SecretRefused,
    Failed,
}

impl std::fmt::Debug for Transcript {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // No dictation text in a log, a panic message or an error (ADR 0082, section 9.2).
        f.debug_struct("Transcript")
            .field("segment_id", &self.segment_id)
            .field("text_chars", &self.text.chars().count())
            .field("raw_chars", &self.raw.chars().count())
            .field("lang", &self.lang)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("runtime", &self.runtime)
            .field("latency_ms", &self.latency_ms)
            .field("remember", &self.remember.is_some())
            .finish()
    }
}

/// A Databricks CLI profile (D10, D39).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub name: ProfileName,
    pub host: String,
    pub auth_type: String,
    pub supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}

/// One entry of `profiles_list` (4.5).
pub type ProfileInfo = Profile;

/// `TokenStatus.state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TokenState {
    Valid,
    Expiring,
    LoginRequired,
    Unknown,
}

/// The result of `profile_status` (4.5). It never holds a token (ARC-6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TokenStatus {
    pub profile: ProfileName,
    pub state: TokenState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
}

/// The router intent (`dictation` = UC1, B5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Intent {
    Question,
    MemoryQuestion,
    Context,
    Command,
    Dictation,
    Noise,
}

/// `RouterDecision.command`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RouterCommand {
    None,
    SwitchWorkspaceOnce,
    SwitchWorkspaceSession,
    ResetWorkspace,
    NewTopic,
    Cancel,
    StopRecording,
    Pause,
    Resume,
    ForgetSession,
    Other,
}

/// The router output (D25, D30, D63; schema in Appendix D). All fields are required;
/// `""` or `[]` means not applicable. Field names are snake_case, as in TS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouterDecision {
    pub intent: Intent,
    pub cleaned_text: String,
    pub genie_question: String,
    pub target_profile_hint: String,
    pub confidence: f64,
    pub follow_up: bool,
    pub topic_label: String,
    pub part_questions: Vec<String>,
    pub part_profile_hints: Vec<String>,
    pub command: RouterCommand,
    pub command_arg: String,
    pub context_note: String,
    pub injection_suspected: bool,
    /// `""` = none (D63).
    pub directed_mog_id: String,
}

// ---------- brain-to-view payloads (#285, ADR 0040) ----------
// The brain (TS) sends these as Tauri events; Rust never sends them. The mirror keeps the
// shared fixtures honest on both sides.

/// `MoodMapper` overlays (4.7) as `mog.view_state` carries them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MoodOverlays {
    pub attention: bool,
    pub recording: bool,
    pub offline: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hop: Option<String>,
    pub local_only: bool,
}

/// One MOG badge (MOG-07, UX-38, UX-39a, ADR 0029).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MogBadgeView {
    pub kind: String,
    pub text: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub hover_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

/// Frame-probe times of one `mog.view_state` (#13, ADR 0031). Numbers only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ViewTrace {
    pub seq: u64,
    pub event_at: f64,
    pub sent_at: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_down_at: Option<f64>,
}

/// The `mog.view_state` payload: what the brain computed for one MOG.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MogViewState {
    pub mog_id: MogId,
    pub mood: Mood,
    pub overlays: MoodOverlays,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badges: Option<Vec<MogBadgeView>>,
    /// The home domain picker of this MOG (PG-18, PG-20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_domain: Option<DomainPickerView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<ViewTrace>,
}

/// One part of a split answer (UX-11b).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlurbPart {
    pub id: String,
    pub text: String,
}

/// One card slot (ADR 0028).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CardBlurbView {
    pub id: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<BlurbPart>>,
}

/// One card chip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CardChip {
    pub id: String,
    pub label: String,
}

/// The scope of a domain picker (PG-18 home domain, PG-19 one draft).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DomainScope {
    Home,
    Draft,
}

/// The current domain of a picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// One domain the picker offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainOption {
    pub id: String,
    pub name: String,
}

/// The domain picker (PG-20): the list, or `paste` (the reason text) when it cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DomainPickerView {
    pub scope: DomainScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<DomainRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<DomainOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paste: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// The admin checklist for a failed Pages preflight (PG-11, PG-12).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PagesChecklistView {
    pub text: String,
    pub items: Vec<String>,
}

/// A copy format of the expanded card (UX-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CopyFormat {
    Markdown,
    Csv,
}

/// One expanded card section in the UX-EXP order (UX-18), tagged by `id` (#89, ADR 0040).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "id",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CardSection {
    Answer {
        md: String,
    },
    KeyNumbers {
        items: Vec<String>,
    },
    /// The SQL section is always collapsed first (`collapsed: true`).
    Sql {
        sql: String,
        collapsed: bool,
    },
    TablePreview {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
        more_rows: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        more_columns: Option<u32>,
    },
    SentToGenie {
        text: String,
    },
    Resend,
    Copy {
        formats: Vec<CopyFormat>,
    },
    Footer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stt_badge: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        deep_link: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace_host: Option<String>,
    },
    DataFlow {
        text: String,
    },
}

/// The `card.update` payload: what the card of one MOG shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CardUpdate {
    pub mog_id: MogId,
    pub blurbs: Vec<CardBlurbView>,
    /// Slice 2a chip ids of an agent turn: `agent-stop:<turnId>`, `agent-more:<opId>` and
    /// `agent-resend:<turnId>` (the host mints an Ask ticket for the last two) and
    /// `agent-open:<turnId>` (opens the hub Agent area).
    pub chips: Vec<CardChip>,
    /// Chips after the 4th (UX-20a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overflow_chips: Option<Vec<CardChip>>,
    /// The expanded card sections (UX-18).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<Vec<CardSection>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_md: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stt_badge: Option<String>,
    /// The domain of the Page draft in review (PG-19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_domain: Option<DomainPickerView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pages_checklist: Option<PagesChecklistView>,
    /// Slice 2a: the live agent turn in compact form, with "Open in Agent".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_turn: Option<AgentTurnView>,
}

/// The `mog.level` payload: one mic level sample (UX-06a). `owner` is the MOG that owns the
/// open mic, or `null` when the mic closed (UX-06b). It is always present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MicLevelFrame {
    /// Required and nullable: a frame with no `owner` key fails to parse.
    #[serde(deserialize_with = "nullable")]
    pub owner: Option<MogId>,
    pub dbfs: f64,
    /// Milliseconds on a monotonic clock.
    pub t: f64,
}

/// The talk key now (`hotkey.status`, CAP-39c, CAP-42, ADR 0039).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TalkKeyState {
    /// The talk key works.
    Active,
    /// The OS hides the talk key for now (the pill or the tooltip shows).
    Paused,
    /// The talk key cannot work (the hook or the host failed).
    Unavailable,
}

/// Why the talk key is not active. A fixed value: never a window title, a process name or a
/// key (SEC rules).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TalkKeyReason {
    /// macOS Secure Event Input is on (CAP-39c).
    SecureInput,
    /// Windows: an elevated window has the foreground (CAP-42).
    ElevatedForeground,
    /// Secure Event Input is on and the OS refused the fallback chord (CAP-39b).
    FallbackChordUnavailable,
    /// The hook or the host could not start the talk key.
    HookFailed,
}

/// The `hotkey.status` payload. `reason` is required and `null` when the talk key is active.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TalkKeyStatus {
    pub talk_key: TalkKeyState,
    /// Required and nullable, like `MicLevelFrame.owner`.
    #[serde(deserialize_with = "nullable")]
    pub reason: Option<TalkKeyReason>,
}

/// The `net.status` payload (ARC-F7, #466). Two failed connects mark the host offline. Sticky:
/// the host sends the last one again when the brain subscribes. No host name, URL or error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetStatus {
    pub online: bool,
}

// ---------- consent records (#430, ADR 0060) ----------

/// What a consent record is for: the first-run notice, or the attestation choice of a
/// meeting session (9.3, SEC-32a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConsentRecordKind {
    Notice,
    OnlyMe,
    OthersAgreed,
}

/// One consent record as `consent_records_list` returns it. It is content-free: no user, no
/// device and no MOG ID crosses IPC (SEC-32a, ADR 0060).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsentRecordView {
    pub record_id: String,
    pub kind: ConsentRecordKind,
    pub notice_version: u32,
    /// Required and nullable: `null` for a notice record.
    #[serde(deserialize_with = "nullable")]
    pub session_id: Option<String>,
    /// SEC-32a `accepted_at`, Unix ms.
    pub accepted_at: i64,
}

/// A required field that may be `null`. Plain `Option` would also accept a missing key.
fn nullable<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d)
}

// ---------- workspaces (ADR 0117) ----------

/// The sign-in state of one workspace profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSignIn {
    SignedIn,
    Expired,
    Unknown,
    Unsupported,
    Missing,
}

/// One Databricks CLI profile as a workspace of the one MOG.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceRow {
    pub profile: ProfileName,
    /// The workspace host. "" when `sign_in` is `Missing`.
    pub host: String,
    /// The old MOG configs of this profile. `[0]` owns the name and colour. Empty for a new
    /// connection.
    pub mog_ids: Vec<MogId>,
    /// "Available to your MOG". Always true for home.
    pub available: bool,
    pub is_home: bool,
    pub sign_in: WorkspaceSignIn,
}

/// D2 "Genie calls per question" (ADR 0117): 5, 10 or 20. Rust checks it when it parses, and
/// the JSON Schema has the same rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct GenieCallBudget(u8);

impl GenieCallBudget {
    /// The allowed values.
    pub const VALUES: [u8; 3] = [5, 10, 20];
    /// The owner's default (D8).
    pub const DEFAULT: GenieCallBudget = GenieCallBudget(10);
    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for GenieCallBudget {
    type Error = &'static str;
    fn try_from(v: u8) -> Result<Self, Self::Error> {
        if Self::VALUES.contains(&v) {
            Ok(Self(v))
        } else {
            Err("not a valid GenieCallBudget")
        }
    }
}

impl From<GenieCallBudget> for u8 {
    fn from(v: GenieCallBudget) -> u8 {
        v.0
    }
}

impl JsonSchema for GenieCallBudget {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GenieCallBudget".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "integer", "enum": [5, 10, 20] })
    }
}

/// The agent settings (ADR 0117 D1, D2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSettings {
    /// D1 "Combine across workspaces". Default false.
    pub cross_workspace_data: bool,
    /// SEC-28b: an admin lock forces D1 off. Always false until slice 3.
    pub cross_workspace_locked: bool,
    /// D2 "Genie calls per question": 5, 10 or 20. Default 10.
    pub genie_call_budget: GenieCallBudget,
    /// D9, ADR 0122: the orchestrator model, a `system.ai.<name>` id. Default
    /// `model_gateway::DEFAULT_ORCHESTRATOR_MODEL`. The field keeps its old name `endpoint`.
    pub endpoint: OrchestratorModel,
    /// ADR 0122: "Clean up dictation". Default true. A file from before ADR 0122 reads as true.
    #[serde(default = "default_clean_dictation")]
    pub clean_dictation: bool,
    /// Slice 2a: the profile the agent runs on. `None` means home. Required and nullable.
    #[serde(deserialize_with = "nullable")]
    pub run_profile: Option<ProfileName>,
}

/// The default of [`AgentSettings::clean_dictation`].
pub fn default_clean_dictation() -> bool {
    true
}

/// The answer of every `workspace_*` command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceList {
    /// The home profile. None until the user picks one.
    pub home: Option<ProfileName>,
    /// The config id of the one MOG lamp.
    pub lamp_mog_id: Option<MogId>,
    /// Home first, then by profile.
    pub workspaces: Vec<WorkspaceRow>,
    pub agent: AgentSettings,
}

// ---------- the ask loop (ADR 0122) ----------

/// One workspace in the catalog of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentWorkspace {
    /// The model alias of the workspace (`CatalogEntry.alias`), not a Databricks workspace ID.
    pub workspace_id: String,
    pub profile: ProfileName,
    pub name: String,
    pub is_home: bool,
    pub is_agent_workspace: bool,
}

/// The reply of `agent_turn_submit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTurnAccepted {
    pub turn_id: String,
    pub session_id: String,
    /// 0 is the live turn.
    pub position: u32,
    pub new_session: bool,
    pub budget: GenieCallBudget,
    pub cross_workspace: bool,
    pub catalog: Vec<AgentWorkspace>,
}

/// The state of one Genie call of an agent turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentCallState {
    Queued,
    Asking,
    Answered,
    NoAnswer,
    Failed,
    Timeout,
    SignedOut,
    Stopped,
}

/// The `mcp_call` reply, as `CommandReturns['mcp_call']` has it.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpCallReply {
    pub structured_content: serde_json::Value,
    pub text: String,
    pub is_error: bool,
    pub http_status: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rpc_error: Option<serde_json::Value>,
}

impl std::fmt::Debug for McpCallReply {
    /// Shows only the status and the error flag: no server text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpCallReply")
            .field("http_status", &self.http_status)
            .field("is_error", &self.is_error)
            .finish_non_exhaustive()
    }
}

impl From<&crate::mcp_transport::McpReply> for McpCallReply {
    fn from(r: &crate::mcp_transport::McpReply) -> Self {
        McpCallReply {
            structured_content: r.structured_content.clone(),
            text: r.text.clone(),
            is_error: r.is_error,
            http_status: r.http_status,
            rpc_error: r.rpc_error.clone(),
        }
    }
}

/// Why a reply is hidden from the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentWithheld {
    InjectionSuspected,
}

/// `agent_turn_call`: the question and the reply of one Genie call, for the card and history.
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTurnCall {
    pub call_id: String,
    pub turn_id: String,
    /// The model alias (`CatalogEntry.alias`), not a Databricks workspace ID.
    pub workspace_id: String,
    pub profile: ProfileName,
    pub question: String,
    pub state: AgentCallState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<McpCallReply>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withheld: Option<AgentWithheld>,
}

impl std::fmt::Debug for AgentTurnCall {
    /// The question and the reply are user content: `Debug` shows ids and the state only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentTurnCall")
            .field("call_id", &self.call_id)
            .field("turn_id", &self.turn_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// How the answer of a turn was made.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentAnswerKind {
    Note,
    Combined,
    Hidden,
}

/// One source of an agent answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSource {
    /// The model alias (`CatalogEntry.alias`), not a Databricks workspace ID.
    pub workspace_id: String,
    pub call_id: String,
    pub turn_id: String,
}

/// The token counts of one turn. Counts only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentUsage {
    pub input_tokens: u64,
    pub cached_input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
}

/// `agent_turn_result`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTurnResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    pub answer_kind: AgentAnswerKind,
    pub sources: Vec<AgentSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AgentUsage>,
}

impl std::fmt::Debug for AgentTurnResult {
    /// The answer is user content: `Debug` shows its size only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentTurnResult")
            .field("answer_kind", &self.answer_kind)
            .field(
                "answer",
                &format_args!(
                    "<redacted {} chars>",
                    self.answer.as_deref().map_or(0, |a| a.chars().count())
                ),
            )
            .finish_non_exhaustive()
    }
}

/// Why the one agent session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionEndReason {
    NewSession,
    Quit,
    Wipe,
    ProfileChanged,
    D1Changed,
    CatalogChanged,
    Tainted,
    Idle,
    Crashed,
    Lockdown,
    HardTimeout,
}

/// The phase of the one agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionPhase {
    None,
    Starting,
    Idle,
    Busy,
    Ending,
}

/// `agent_session_state`: for the hub panel at open. Counts only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSessionState {
    pub state: AgentSessionPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Unix ms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    /// Required and nullable.
    #[serde(deserialize_with = "nullable")]
    pub agent_profile: Option<ProfileName>,
    pub turns: u32,
    pub queued: u32,
    pub tokens: u64,
}

/// The `agent.session` payload (core to brain only, content-free).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentSessionEvent {
    Started {
        session_id: String,
    },
    Ended {
        session_id: String,
        reason: AgentSessionEndReason,
    },
}

/// Why a tool call of the model was refused. Model-facing codes, content-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentRefusal {
    WorkspaceNotAvailable,
    BudgetReached,
    CrossWorkspaceOff,
    InvalidArgs,
}

/// How an agent turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnOutcome {
    Answered,
    Stopped,
    Removed,
    Timeout,
    Offline,
    SignedOut,
    NotEligible,
    Failed,
    Lockdown,
    Refusals,
    SessionEnded,
}

/// The `agent.turn` payload (core to brain only, content-free).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentTurnEvent {
    Queued {
        turn_id: String,
        position: u32,
    },
    Started {
        turn_id: String,
    },
    Thinking {
        turn_id: String,
    },
    Call {
        turn_id: String,
        call_id: String,
        /// The model alias (`CatalogEntry.alias`), not a Databricks workspace ID.
        workspace_id: String,
        state: AgentCallState,
        used: u32,
        budget: u32,
    },
    Refused {
        turn_id: String,
        code: AgentRefusal,
        used: u32,
    },
    Slow {
        turn_id: String,
    },
    Ended {
        turn_id: String,
        outcome: AgentTurnOutcome,
        calls: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens: Option<u64>,
    },
}

/// A workspace name and colour on the card and in the hub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentWsChip {
    pub name: String,
    /// `mog.ui.variant` (`src/shared/mog-palette.ts`). Never a raw hex.
    pub color_variant: String,
}

/// The state of one Genie call row on the card and in the hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentCallViewState {
    Queued,
    Asking,
    Ready,
    NoAnswer,
    NeedsYou,
    Failed,
    Stopped,
}

/// One Genie call row of an agent turn view.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentCallView {
    pub call_id: String,
    pub workspace: AgentWsChip,
    pub question: String,
    pub state: AgentCallViewState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
}

impl std::fmt::Debug for AgentCallView {
    /// The question is user content: `Debug` shows the id and the state only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentCallView")
            .field("call_id", &self.call_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

/// One folded part (one Genie answer) of an agent turn view.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentPartView {
    pub call_id: String,
    pub workspace: AgentWsChip,
    pub sections: Vec<CardSection>,
    /// "From an earlier answer".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_earlier: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_shown_to_agent: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folded: Option<bool>,
}

impl std::fmt::Debug for AgentPartView {
    /// The sections are answer text: `Debug` shows the id and the section count only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentPartView")
            .field("call_id", &self.call_id)
            .field("sections", &self.sections.len())
            .finish_non_exhaustive()
    }
}

/// The state of an agent turn view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnViewState {
    Queued,
    Planning,
    Running,
    Slow,
    Done,
    Stopped,
    TimedOut,
    Offline,
    Failed,
    /// The turn ended `signed_out` and the question is kept (UX slice 5).
    NeedsYou,
}

/// Whether the summary of a turn is one note or a combined answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentSummaryKind {
    Note,
    Combined,
}

/// The summary of an agent turn view.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentSummaryView {
    pub md: String,
    pub kind: AgentSummaryKind,
    pub workspaces: Vec<AgentWsChip>,
    /// ADR 0122: the `[n]` markers of `md`. Absent when the summary cites nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub citations: Option<Vec<AgentCitation>>,
}

/// ADR 0122: one `[n]` citation in a combined summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentCitation {
    /// The number in the summary text, 1-based (an index into the turn result `sources`).
    pub n: u32,
    pub call_id: String,
    pub workspace: AgentWsChip,
    /// The Open in Databricks link of that part. A renderer must never use it without
    /// re-checking it at render time against `workspace_host`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    /// The workspace host that `deep_link` must belong to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_host: Option<String>,
}

impl std::fmt::Debug for AgentSummaryView {
    /// The summary is model text about user data: `Debug` shows its size only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSummaryView")
            .field("kind", &self.kind)
            .field(
                "md",
                &format_args!("<redacted {} chars>", self.md.chars().count()),
            )
            .finish_non_exhaustive()
    }
}

/// One agent turn as the card and the hub Agent panel show it.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTurnView {
    pub turn_id: String,
    pub question: String,
    pub state: AgentTurnViewState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<u32>,
    pub used: u32,
    pub budget: u32,
    pub agent_workspace: AgentWsChip,
    pub calls: Vec<AgentCallView>,
    pub parts: Vec<AgentPartView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<AgentSummaryView>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    /// When the user saw the answer, Unix ms (UX slice 4, Activity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen_at: Option<u64>,
}

impl std::fmt::Debug for AgentTurnView {
    /// The question and the answer are user content: `Debug` shows the id and the state only.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentTurnView")
            .field("turn_id", &self.turn_id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

// ---------- call recording (ADR 0103) ----------

/// The default notes model of a new install (ADR 0103). A Settings value.
pub const RECORDING_DEFAULT_ENDPOINT: &str = "databricks-claude-sonnet-5-5";
/// The default transcribe model of a new install (ADR 0103): a chat model that takes audio
/// (`audio_url` data URLs). A Settings value.
pub const RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT: &str = "databricks-gemini-3-8-flash";
/// The default recording length in minutes. The recording stops by itself at this length.
pub const RECORDING_DEFAULT_LENGTH_MIN: u16 = 60;
/// The shortest and the longest recording length that Settings accepts, in minutes.
pub const RECORDING_LENGTH_MIN_RANGE: (u16, u16) = (5, 120);
/// The longest instructions text, in characters.
pub const RECORDING_INSTRUCTIONS_MAX: usize = 16_000;
/// The default preset prompt of a new install (ADR 0103).
pub const RECORDING_DEFAULT_INSTRUCTIONS: &str = "You get the transcript of a call. Speakers are labeled Speaker 1, Speaker 2, and so on.
Write short notes in Markdown with these sections:
## Summary
Three to five sentences.
## Decisions
A bullet list. Write \"None\" if there are no decisions.
## Action items
One sub-heading per speaker. Under each, a bullet list of that speaker's action items, with an owner and a due date if the call names one.
Use only facts from the transcript.";

/// A string newtype that Rust checks when it parses, with the same rule in its JSON Schema.
macro_rules! checked_string {
    ($(#[$m:meta])* $name:ident, $pattern:expr, $check:expr) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// The JSON Schema pattern of the value.
            pub const PATTERN: &'static str = $pattern;
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = &'static str;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                let check: fn(&str) -> bool = $check;
                if check(&s) {
                    Ok(Self(s))
                } else {
                    Err(concat!("not a valid ", stringify!($name)))
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = &'static str;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::try_from(s.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(v: $name) -> String {
                v.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
                schemars::json_schema!({ "type": "string", "pattern": $pattern })
            }
        }
    };
}

fn ascii_id(s: &str, max: usize, extra: &[char]) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

checked_string!(
    /// A recording ID: 1-64 characters of `[A-Za-z0-9_-]`. Opaque on the wire.
    RecordingId,
    "^[A-Za-z0-9_-]{1,64}$",
    |s| ascii_id(s, 64, &['_', '-'])
);

checked_string!(
    /// A Unity Catalog volume path: `/Volumes/` plus at least three segments of
    /// `[A-Za-z0-9_.@-]`, with an optional end `/`. No `.` or `..` segment, no empty segment.
    /// File API paths use it, so the character set is strict.
    VolumePath,
    "^/Volumes(/[A-Za-z0-9_.@-]+){3,}/?$",
    |s| {
        let Some(rest) = s.strip_prefix("/Volumes/") else {
            return false;
        };
        let rest = rest.strip_suffix('/').unwrap_or(rest);
        let segs: Vec<&str> = rest.split('/').collect();
        s.len() <= 1024
            && segs.len() >= 3
            && segs.iter().all(|g| {
                !g.is_empty()
                    && *g != "."
                    && *g != ".."
                    && g.chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
            })
    }
);

checked_string!(
    /// A model endpoint name (the `model` of an AI Gateway chat call): 1-128 characters of
    /// `[A-Za-z0-9_-]`.
    EndpointName,
    "^[A-Za-z0-9_-]{1,128}$",
    |s| ascii_id(s, 128, &['_', '-'])
);

checked_string!(
    /// The orchestrator model (ADR 0122): `system.ai.` plus a name of 1-128 characters of
    /// `[A-Za-z0-9_-]` (the rule of `EndpointName`).
    OrchestratorModel,
    "^system\\.ai\\.[A-Za-z0-9_-]{1,128}$",
    |s| s
        .strip_prefix("system.ai.")
        .is_some_and(|n| ascii_id(n, 128, &['_', '-']))
);

/// One chat model the home AI Gateway serves, for the picker (ADR 0122).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestratorModelInfo {
    pub id: OrchestratorModel,
    pub label: String,
    /// There is an eval record for the model (NFR-22a).
    pub tested: bool,
    /// The model supports tool calls.
    pub tools: bool,
}

/// The `orchestrator_models` reply. No token, no profile name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestratorModels {
    pub models: Vec<OrchestratorModelInfo>,
    pub selected: OrchestratorModel,
    pub default_model: OrchestratorModel,
}

/// The state of the orchestrator for routing (ADR 0122).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OrchestratorState {
    Ready,
    NoHome,
    SignedOut,
    NoModel,
    Unreachable,
}

/// The `orchestrator_status` reply. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OrchestratorStatus {
    pub state: OrchestratorState,
    pub model: OrchestratorModel,
}

checked_string!(
    /// A Unity Catalog catalog or schema name for `recording_volume_create`: 1-255 characters
    /// of `[A-Za-z0-9_-]`. No dot, no quote, no back-tick.
    UcName,
    "^[A-Za-z0-9_-]{1,255}$",
    |s| ascii_id(s, 255, &['_', '-'])
);

/// The instructions for the notes model: the Settings preset or the text for one recording.
/// 1 to [`RECORDING_INSTRUCTIONS_MAX`] characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Instructions(String);

impl Instructions {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Instructions {
    type Error = &'static str;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        let n = s.chars().count();
        if n == 0 || n > RECORDING_INSTRUCTIONS_MAX || s.trim().is_empty() {
            Err("not valid instructions")
        } else {
            Ok(Self(s))
        }
    }
}

impl From<Instructions> for String {
    fn from(v: Instructions) -> String {
        v.0
    }
}

impl JsonSchema for Instructions {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Instructions".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": RECORDING_INSTRUCTIONS_MAX
        })
    }
}

/// The recording length in minutes, from 5 to 120 (ADR 0103). Settings offers 15, 30, 45, 60
/// and 90, or a custom value in the range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct LengthMin(u16);

impl LengthMin {
    pub fn get(self) -> u16 {
        self.0
    }
}

impl Default for LengthMin {
    fn default() -> Self {
        Self(RECORDING_DEFAULT_LENGTH_MIN)
    }
}

impl TryFrom<u16> for LengthMin {
    type Error = &'static str;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        let (lo, hi) = RECORDING_LENGTH_MIN_RANGE;
        if (lo..=hi).contains(&v) {
            Ok(Self(v))
        } else {
            Err("the length is out of range")
        }
    }
}

impl From<LengthMin> for u16 {
    fn from(v: LengthMin) -> u16 {
        v.0
    }
}

impl JsonSchema for LengthMin {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "LengthMin".into()
    }
    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "integer",
            "minimum": RECORDING_LENGTH_MIN_RANGE.0,
            "maximum": RECORDING_LENGTH_MIN_RANGE.1
        })
    }
}

/// Where the recordings go: the user's My Files volume, or a volume from Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStorageKind {
    MyFiles,
    Volume,
}

/// The volume root of the recordings. Files go under `<root>/Bottle/Recordings/<name>/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingStorage {
    pub kind: RecordingStorageKind,
    pub root: VolumePath,
}

/// The recording settings (ADR 0103): the `recording_settings_*` reply. No secret: Rust keeps
/// them in a small JSON file in the app config dir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingSettings {
    /// The knob. Off by default. When off, `recording_start` answers `recording_off`.
    pub enabled: bool,
    /// The user acknowledged the first-use consent notice once.
    pub consent_ack: bool,
    /// Required and nullable: `null` until My Files resolves or the user picks a volume.
    #[serde(deserialize_with = "nullable")]
    pub storage: Option<RecordingStorage>,
    /// The model that transcribes the audio parts.
    pub transcribe_endpoint: EndpointName,
    /// The model that writes the notes from the joined transcript.
    pub notes_endpoint: EndpointName,
    pub preset_instructions: Instructions,
    pub length_min: LengthMin,
    /// ADR 0118: the one-time "Your call can still hear you." note was shown. A settings file
    /// from before ADR 0118 has no such field, so it reads as false.
    #[serde(default)]
    pub held_note_shown: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            consent_ack: false,
            storage: None,
            transcribe_endpoint: EndpointName(RECORDING_DEFAULT_TRANSCRIBE_ENDPOINT.to_owned()),
            notes_endpoint: EndpointName(RECORDING_DEFAULT_ENDPOINT.to_owned()),
            preset_instructions: Instructions(RECORDING_DEFAULT_INSTRUCTIONS.to_owned()),
            length_min: LengthMin::default(),
            held_note_shown: false,
        }
    }
}

/// A partial `RecordingSettings` for `recording_settings_write`. An absent field keeps its
/// value. A value cannot be cleared to `null`: the user picks another one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingSettingsPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent_ack: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<RecordingStorage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcribe_endpoint: Option<EndpointName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_endpoint: Option<EndpointName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_instructions: Option<Instructions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub length_min: Option<LengthMin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_note_shown: Option<bool>,
}

impl RecordingSettings {
    /// The settings with each field of `patch` that is set.
    pub fn apply(mut self, patch: RecordingSettingsPatch) -> Self {
        let RecordingSettingsPatch {
            enabled,
            consent_ack,
            storage,
            transcribe_endpoint,
            notes_endpoint,
            preset_instructions,
            length_min,
            held_note_shown,
        } = patch;
        if let Some(v) = enabled {
            self.enabled = v;
        }
        if let Some(v) = consent_ack {
            self.consent_ack = v;
        }
        if let Some(v) = storage {
            self.storage = Some(v);
        }
        if let Some(v) = transcribe_endpoint {
            self.transcribe_endpoint = v;
        }
        if let Some(v) = notes_endpoint {
            self.notes_endpoint = v;
        }
        if let Some(v) = preset_instructions {
            self.preset_instructions = v;
        }
        if let Some(v) = length_min {
            self.length_min = v;
        }
        if let Some(v) = held_note_shown {
            self.held_note_shown = v;
        }
        self
    }
}

/// The phase of the one recording of the device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordingPhase {
    Idle,
    Starting,
    Recording,
    Stopping,
}

/// The system audio (the other side of the call) of a recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemAudio {
    Ok,
    /// This OS has no system audio capture (for example macOS before 14.4): mic only.
    Unavailable,
    /// 5 s of zero buffers: the permission is denied or the call is silent (CAP-10a).
    SilentOrDenied,
    /// No recording runs.
    Off,
}

/// The `recording.state` payload and the `recording_start`, `recording_stop` and
/// `recording_state` reply. Content-free: no audio, no transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingState {
    pub state: RecordingPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RecordingId>,
    /// The folder name, local start time `YYYY-MM-DD_HHMM` (plus `_2`.. on a clash).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Epoch milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at_ms: Option<u64>,
    /// Epoch milliseconds: the recording stops by itself at this time (`lengthMin`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ends_at_ms: Option<u64>,
    pub system_audio: SystemAudio,
}

impl RecordingState {
    /// No recording runs.
    pub fn idle() -> Self {
        Self {
            state: RecordingPhase::Idle,
            id: None,
            name: None,
            started_at_ms: None,
            ends_at_ms: None,
            system_audio: SystemAudio::Off,
        }
    }
}

/// The status of one recording job. `transcribing`: the transcribe model calls, part by part.
/// `writing_notes`: the notes model call and the writes of the result files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordingStatus {
    Recording,
    Uploading,
    Transcribing,
    WritingNotes,
    Done,
    Failed,
}

/// Why a recording failed or a recording command was refused (ADR 0103). One plain message and
/// one action each in the hub. `ipc_code` (in `crate::recording`) gives the IPC refusal code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecordingErrorKind {
    RecordingOff,
    ConsentRequired,
    RecordingBusy,
    MicBusy,
    NoStorage,
    VolumePermission,
    /// The transcribe endpoint does not exist, or the user may not query it.
    TranscribeEndpointMissing,
    /// The notes endpoint does not exist, or the user may not query it.
    NotesEndpointMissing,
    /// The audio is over the request budget of the model.
    TooLong,
    /// The audio file is not a valid encoded file (checked by its magic bytes), or the model
    /// could not read it.
    AudioUnreadable,
    UploadFailed,
    /// 429 again after the backoff of the rate bucket.
    RateLimited,
    Network,
    Auth,
    SystemAudioDenied,
    Internal,
}

/// One row of the recordings list, and the `recording.job` payload. Content-free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingSummary {
    pub id: RecordingId,
    pub name: String,
    /// Epoch milliseconds.
    pub started_at_ms: u64,
    pub duration_s: u32,
    pub status: RecordingStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<RecordingErrorKind>,
    /// The step that failed. Retry starts again at this step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_stage: Option<RecordingStatus>,
    /// Parts of the recording that are in the volume. Absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts_uploaded: Option<u32>,
    /// Parts of the recording so far (all of them once it has stopped). Absent when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts_total: Option<u32>,
    /// The audio is only on this device: no job.json in the volume and no upload runs now
    /// (an old build, or a crash with no way to resume).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub on_device_only: bool,
}

/// One transcript turn. Times are seconds from the start of the recording, across parts.
/// `speaker` is the speaker label from the transcribe model, with the part as a prefix when there is more than one
/// part (for example `2.1`), because the labels of two parts can differ. `text` is user content: `Debug` shows only its size (SEC-16c).
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingSegment {
    pub start: f64,
    pub end: f64,
    pub speaker: String,
    pub text: String,
    pub part: u32,
}

impl std::fmt::Debug for RecordingSegment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingSegment")
            .field("start", &self.start)
            .field("end", &self.end)
            .field("part", &self.part)
            .field(
                "text",
                &format_args!("<redacted {} chars>", self.text.chars().count()),
            )
            .finish()
    }
}

/// The `recording_detail` reply: the summary, the instructions used, the transcript turns and
/// the two Markdown files. The text is user content: `Debug` shows only sizes (SEC-16c).
#[derive(Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingDetail {
    pub summary: RecordingSummary,
    pub instructions: String,
    pub segments: Vec<RecordingSegment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_md: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_md: Option<String>,
}

impl std::fmt::Debug for RecordingDetail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let size = |s: &Option<String>| s.as_ref().map(|t| t.chars().count());
        f.debug_struct("RecordingDetail")
            .field("summary", &self.summary)
            .field(
                "instructions",
                &format_args!("<redacted {} chars>", self.instructions.chars().count()),
            )
            .field("segments", &self.segments)
            .field("transcript_md_chars", &size(&self.transcript_md))
            .field("notes_md_chars", &size(&self.notes_md))
            .finish()
    }
}

/// The `recording_storage_probe` reply: the My Files root, or `null` when it does not resolve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StorageProbe {
    #[serde(deserialize_with = "nullable")]
    pub my_files_root: Option<VolumePath>,
}

/// The `hotkey.record_toggle` payload (ADR 0103): the user pressed the record key (⌥R) in a
/// talk-key hold while the knob is on. The host ended the hold with a `hotkey.ptt` `cancel`
/// first, so no dictation and no paste follow. The brain starts or stops the recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordToggle {
    pub mog_id: MogId,
}
