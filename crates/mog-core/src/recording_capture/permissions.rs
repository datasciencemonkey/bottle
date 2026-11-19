//! The permission ask when the user turns the recording knob ON (ADR 0103, ADR 0101 d7
//! "just ask").
//!
//! macOS has a Microphone prompt and a System Audio Recording prompt. There is no preflight
//! API for the audio tap, so a short tap start shows the second prompt; nothing here reads or
//! changes TCC state by any private API (CAP-10b). Windows has no prompt for loopback, and
//! its mic prompt is a privacy setting, so nothing is asked there.

/// The OS asks. The real ones are in the host; tests use a fake.
pub trait PermissionAsker {
    /// Show the Microphone prompt if the OS has not asked yet.
    fn ask_mic(&self);
    /// Start a short system audio tap, so the OS shows its prompt if it has not asked yet.
    fn ask_system_audio(&self);
}

/// Ask for both permissions when the knob goes from OFF (`was`) to ON (`now`); otherwise ask
/// nothing. A knob that stays ON never asks again.
pub fn ask_on_enable(was: bool, now: bool, asker: &dyn PermissionAsker) {
    if !was && now {
        asker.ask_mic();
        asker.ask_system_audio();
    }
}
