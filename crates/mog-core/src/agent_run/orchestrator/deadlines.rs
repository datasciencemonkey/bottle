//! The deadlines of a turn and of the session (ADR 0122).
//!
//! Pure functions of the elapsed time, so the host and the tests share one rule.

use std::time::Duration;

/// "Still working…" shows at 4 min.
pub const SLOW_AFTER: Duration = Duration::from_secs(4 * 60);
/// The hard stop of a turn: 10 min.
pub const HARD_AFTER: Duration = Duration::from_secs(10 * 60);
/// The session ends after this long with no turn (owner default).
pub const IDLE_AFTER: Duration = Duration::from_secs(30 * 60);
/// The settings of the session are compared at this interval.
pub const SETTINGS_POLL: Duration = Duration::from_secs(1);
/// The answers of an ended session stay in memory this long.
pub const RESULT_KEEP: Duration = Duration::from_secs(10 * 60);

/// What a live turn must do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDue {
    Nothing,
    /// Emit `slow` (once).
    Slow,
    /// Stop the turn with `Timeout`.
    HardStop,
}

/// The deadline action of a turn that started `elapsed` ago.
pub fn turn_due(elapsed: Duration, slow_sent: bool) -> TurnDue {
    if elapsed >= HARD_AFTER {
        TurnDue::HardStop
    } else if elapsed >= SLOW_AFTER && !slow_sent {
        TurnDue::Slow
    } else {
        TurnDue::Nothing
    }
}

/// True when a session with no turn has idled for `idle`.
pub fn idle_over(idle: Duration) -> bool {
    idle >= IDLE_AFTER
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    #[test]
    fn slow_at_four_minutes_hard_at_ten() {
        assert_eq!(turn_due(secs(239), false), TurnDue::Nothing);
        assert_eq!(turn_due(secs(240), false), TurnDue::Slow);
        assert_eq!(turn_due(secs(300), true), TurnDue::Nothing);
        assert_eq!(turn_due(secs(600), true), TurnDue::HardStop);
        assert_eq!(turn_due(secs(600), false), TurnDue::HardStop);
    }

    #[test]
    fn thirty_minutes_idle() {
        assert!(!idle_over(secs(30 * 60 - 1)));
        assert!(idle_over(secs(30 * 60)));
    }
}
