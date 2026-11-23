//! Slice 2a (the MOG agent on one live Genie Code session).
//!
//! - [`pin`]: the pinned Genie Code version and binary digests.
//! - [`seam`]: the frozen internal seam between the session host and the guard. Only the
//!   orchestrator changes it.
//! - [`session`]: the Rust session host (R1): one `genie app-server` process, the turn queue,
//!   the deadlines and the wipe hook.
//! - [`guard`]: the `ask_genie` guard (R2): the shim channel, the ten checks, the Rust poll loop
//!   and the session call log.

pub mod guard;
pub mod pin;
pub mod seam;
pub mod session;
