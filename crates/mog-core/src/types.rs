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
