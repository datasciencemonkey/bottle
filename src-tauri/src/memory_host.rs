//! Local memory slice 1 (ADR 0113): the host side of the #574 user-action tickets.
//!
//! `src-tauri/src/ipc.rs` calls only these three hooks, so the ticket logic stays out of the
//! contract file:
//! - [`mint_for_view`]: on `view_emit`, mint a ticket for a real user click (a card "remember",
//!   a hub add, edit or rating). The relay carries it to the brain as `actionTicket`.
//! - [`gate_mem_exec`]: on `mem_local_exec`, redeem a ticket before a user-only write, and refuse
//!   a pin, a rating, a release or an edit that has none.
//! - [`attach_remember`]: on `stt_transcribe`, attach `remember { ticket, text }` when Rust's own
//!   grammar finds "remember this: ..." in its own transcript.
//!
//! TODO(memory slice 1, task A): this is the step-0 stub. It mints nothing and refuses every
//! ticket and every pin, which is the behaviour of main before slice 1.

use serde_json::Value;

use mog_core::action_ticket::ActionTickets;
use mog_core::error::{CoreError, CoreResult};
use mog_core::ipc::SttTranscribeArgs;
use mog_core::local_store::LocalStore;
use mog_core::types::Transcript;

fn invalid_args() -> CoreError {
    CoreError::IpcRefused {
        code: "invalid_args",
    }
}

/// The ticket for this `view_emit`, if it is a user click that needs one. Stub: none.
pub fn mint_for_view(
    _from: &str,
    _args: &Value,
    _tickets: Option<&ActionTickets>,
    _now_ms: u64,
) -> Option<String> {
    None
}

/// Run one brain `mem_local_exec`, with the ticket rules of ADR 0113. Stub: a ticket or a pin is
/// refused (`invalid_args`); every other call runs as before.
pub fn gate_mem_exec(
    _tickets: Option<&ActionTickets>,
    store: &dyn LocalStore,
    args: Value,
    _now_ms: u64,
) -> CoreResult<Value> {
    let pinned = args
        .get("params")
        .and_then(|p| p.get("pinned"))
        .is_some_and(|pin| pin != &Value::Bool(false) && !pin.is_null());
    if pinned || args.get("actionTicket").is_some() {
        return Err(invalid_args());
    }
    mog_core::local_store::mem_local_exec(store, args)
}

/// Attach the voice "remember" to a transcript. Stub: nothing.
pub fn attach_remember(
    _transcript: &mut Transcript,
    _args: &SttTranscribeArgs,
    _tickets: Option<&ActionTickets>,
    _now_ms: u64,
) {
}

/// The wall clock in ms, for ticket expiry.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
