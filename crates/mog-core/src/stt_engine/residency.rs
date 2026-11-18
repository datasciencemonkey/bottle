//! The model residency policy of R4 (CAP-51a, CAP-51b, D78, ADR 0082). Pure, so the tests
//! drive it with a simulated clock.
//!
//! - On AC power (or an unknown source, for example a desktop) the model stays loaded
//!   (CAP-51a).
//! - On battery the model unloads after [`BATTERY_IDLE_UNLOAD`] with no use (CAP-51b). A use is
//!   a preload, a lease start or a decode.
//! - A talk-key down preloads the model (CAP-51c); that is the engine's job, not this policy's.

use std::time::Duration;

use super::power::PowerSource;

/// CAP-51b: the idle time on battery after which the model unloads.
pub const BATTERY_IDLE_UNLOAD: Duration = Duration::from_secs(30 * 60);

/// The model must unload now: on battery and idle for `idle_limit` or more.
pub fn should_unload_after(power: PowerSource, idle: Duration, idle_limit: Duration) -> bool {
    power == PowerSource::Battery && idle >= idle_limit
}

/// [`should_unload_after`] with the CAP-51b limit of 30 min.
pub fn should_unload(power: PowerSource, idle: Duration) -> bool {
    should_unload_after(power, idle, BATTERY_IDLE_UNLOAD)
}

/// The model must be resident on this power source without a key-down (CAP-51a).
pub fn keeps_resident(power: PowerSource) -> bool {
    power != PowerSource::Battery
}
