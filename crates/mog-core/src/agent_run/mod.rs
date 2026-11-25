//! The ask loop of ADR 0122.
//!
//! - [`seam`]: the internal seam between the orchestrator host and the guard. Only the
//!   orchestrator changes it.
//! - [`orchestrator`]: the ask loop (one orchestrator model, in process).
//! - [`guard`]: the `ask_genie` guard (R2): the ten checks, the Rust poll loop and the session
//!   call log.
//! - [`reauth`]: the home-first "sign in, then retry" rule (ADR 0122 amendment, owner decision
//!   2026-11-24).

pub mod guard;
pub mod orchestrator;
pub mod reauth;
pub mod seam;
