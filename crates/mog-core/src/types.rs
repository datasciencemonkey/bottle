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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
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

/// `CaptureState.othersTrack`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OthersTrack {
    Off,
    Ok,
    ZeroBuffers,
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

/// The result of `stt_transcribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// A required field that may be `null`. Plain `Option` would also accept a missing key.
fn nullable<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}
