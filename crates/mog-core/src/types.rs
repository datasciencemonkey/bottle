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
