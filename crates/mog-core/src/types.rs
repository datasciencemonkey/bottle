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
