//! Event names of 4.5 (ARC-5). Tauri allows only alphanumerics, `-`, `/`, `:` and `_` in
//! event names, so each dotted name of the requirements maps `.` to `:` (ADR 0003).
//! Core-to-brain streams (level, partial text) are Channels, not events: see
//! `CaptureSubscribeArgs`. The brain passes level frames on to one MOG view as `mog:level`.
//! Core-to-brain items travel as `{ event, payload }` messages on the brain Channel.

use serde::Serialize;

/// Who emits the event and who gets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Rust core to the brain only, on the brain's private Channel (`brain_subscribe`).
    /// Never a Tauri event: any window that may listen gets every Tauri event (ADR 0003).
    CoreToBrain,
    /// Brain to a view window as a Tauri event (`emitTo("mog-<id>" | "card-<id>" | "hub", ..)`).
    /// Every view can listen to these; they carry only what views render.
    BrainToView,
}

/// One event of 4.5.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventSpec {
    /// The Tauri event name.
    pub name: &'static str,
    /// The name in the requirements.
    pub spec_name: &'static str,
    pub direction: Direction,
}

const fn core(name: &'static str, spec_name: &'static str) -> EventSpec {
    EventSpec {
        name,
        spec_name,
        direction: Direction::CoreToBrain,
    }
}

pub const HOTKEY_PTT: &str = "hotkey:ptt";
pub const HOTKEY_PANIC: &str = "hotkey:panic";
/// The talk-key status for the "Talk key paused" pill (CAP-39c) and the elevated-window
/// tooltip (CAP-42). Payload: `TalkKeyStatus` (#297, #294, ADR 0039). Not in the 4.5 list yet.
pub const HOTKEY_STATUS: &str = "hotkey:status";
pub const CAPTURE_STATE: &str = "capture:state";
pub const CAPTURE_SEGMENT: &str = "capture:segment";
/// One capture lease stopped, with its reason (`types::CaptureStopped`, #425, ADR 0055).
/// Not in the 4.5 list of the requirements.
pub const CAPTURE_STOPPED: &str = "capture:stopped";
pub const SESSION_STOP_SIGNAL: &str = "session:stop_signal";
pub const AUTH_STATUS: &str = "auth:status";
/// Online or offline (ARC-F7). Payload: `types::NetStatus` (#466). Sticky.
pub const NET_STATUS: &str = "net:status";
pub const MEM_REMOTE_STATUS: &str = "mem:remote_status";
pub const AGENT_TOOL_CALL: &str = "agent:tool_call";
pub const AGENT_STATUS: &str = "agent:status";
pub const MOG_VIEW_STATE: &str = "mog:view_state";
pub const CARD_UPDATE: &str = "card:update";
/// Mic frames for the MOG level meter (UX-06a), sent to `mog-<owner>` (#285, ADR 0040).
/// Not in the 4.5 list of the requirements.
pub const MOG_LEVEL: &str = "mog:level";
/// `view_emit` arrives in the brain under this name.
pub const VIEW_INPUT: &str = "view:input";
/// ADR 0103: the record key (⌥R) in a talk-key hold, with the knob on. Payload:
/// `types::RecordToggle`. The host sends a `hotkey:ptt` `cancel` first. Not in 4.5.
pub const HOTKEY_RECORD_TOGGLE: &str = "hotkey:record_toggle";
/// ADR 0103: the one recording of the device. Payload: `types::RecordingState`. Not sticky:
/// a brain that subscribes later calls `recording_state`. Not in 4.5.
pub const RECORDING_STATE: &str = "recording:state";
/// ADR 0103: the status of one recording job changed. Payload: `types::RecordingSummary`.
/// Not in 4.5.
pub const RECORDING_JOB: &str = "recording:job";

/// Every event of 4.5, in the order of the requirements, plus `mog.level` (ADR 0040) and
/// `hotkey.status` (ADR 0039) and `capture.stopped` (ADR 0055), plus the call recording events
/// (ADR 0103).
pub const ALL: &[EventSpec] = &[
    core(HOTKEY_PTT, "hotkey.ptt"),
    core(HOTKEY_PANIC, "hotkey.panic"),
    core(HOTKEY_STATUS, "hotkey.status"),
    core(CAPTURE_STATE, "capture.state"),
    core(CAPTURE_SEGMENT, "capture.segment"),
    core(CAPTURE_STOPPED, "capture.stopped"),
    core(SESSION_STOP_SIGNAL, "session.stop_signal"),
    core(AUTH_STATUS, "auth.status"),
    core(NET_STATUS, "net.status"),
    core(MEM_REMOTE_STATUS, "mem.remote_status"),
    core(AGENT_TOOL_CALL, "agent.tool_call"),
    core(AGENT_STATUS, "agent.status"),
    EventSpec {
        name: MOG_VIEW_STATE,
        spec_name: "mog.view_state",
        direction: Direction::BrainToView,
    },
    EventSpec {
        name: CARD_UPDATE,
        spec_name: "card.update",
        direction: Direction::BrainToView,
    },
    EventSpec {
        name: MOG_LEVEL,
        spec_name: "mog.level",
        direction: Direction::BrainToView,
    },
    core(VIEW_INPUT, "view.input"),
    core(HOTKEY_RECORD_TOGGLE, "hotkey.record_toggle"),
    core(RECORDING_STATE, "recording.state"),
    core(RECORDING_JOB, "recording.job"),
];

/// The core-to-brain events whose last payload the host keeps and sends again to a brain
/// Channel that subscribes later (ADR 0039). A state, not a stream: the brain can subscribe
/// after it changed (for example a talk key that failed at setup).
pub const STICKY: &[&str] = &[HOTKEY_STATUS];

/// True for an event of [`STICKY`].
pub fn is_sticky(event: &str) -> bool {
    STICKY.contains(&event)
}
