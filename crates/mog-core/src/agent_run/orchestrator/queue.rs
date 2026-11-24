//! The turn queue and the rate cap (plan Amendment 2, A3.3 and A5).
//!
//! There is one live turn and at most [`MAX_WAITING`] waiting turns. A second `turn/start` would
//! merge into the running turn, so Rust sends one turn at a time. This file holds only the
//! waiting line; the live turn belongs to the host.

use std::collections::VecDeque;
use std::time::Duration;

/// The most waiting turns behind the live turn.
pub const MAX_WAITING: usize = 3;
/// The most submitted turns in one window.
pub const RATE_CAP: usize = 6;
/// The length of the rate window.
pub const RATE_WINDOW: Duration = Duration::from_secs(60);

/// The waiting line.
#[derive(Debug)]
pub struct WaitQueue<T> {
    items: VecDeque<(String, T)>,
}

impl<T> Default for WaitQueue<T> {
    fn default() -> Self {
        Self {
            items: VecDeque::new(),
        }
    }
}

impl<T> WaitQueue<T> {
    /// Add a turn. The answer is its position: 1 is the first waiting turn (0 is the live one).
    /// `None` when the line is full.
    pub fn push(&mut self, id: String, t: T) -> Option<u32> {
        if self.items.len() >= MAX_WAITING {
            return None;
        }
        self.items.push_back((id, t));
        Some(self.items.len() as u32)
    }

    /// Remove a waiting turn.
    pub fn remove(&mut self, id: &str) -> Option<T> {
        let at = self.items.iter().position(|(i, _)| i == id)?;
        self.items.remove(at).map(|(_, t)| t)
    }

    /// The next turn to run.
    pub fn pop(&mut self) -> Option<T> {
        self.items.pop_front().map(|(_, t)| t)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|(i, _)| i == id)
    }

    /// Take every turn out, in order.
    pub fn drain(&mut self) -> Vec<(String, T)> {
        self.items.drain(..).collect()
    }

    /// The turns in order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &T)> {
        self.items.iter().map(|(i, t)| (i.as_str(), t))
    }
}

/// At most [`RATE_CAP`] turns in any [`RATE_WINDOW`].
#[derive(Debug, Default)]
pub struct RateCap {
    stamps: VecDeque<Duration>,
}

impl RateCap {
    /// True if a turn at `now` is allowed. An allowed turn is counted.
    pub fn allow(&mut self, now: Duration) -> bool {
        while self
            .stamps
            .front()
            .is_some_and(|t| now.saturating_sub(*t) >= RATE_WINDOW)
        {
            self.stamps.pop_front();
        }
        if self.stamps.len() >= RATE_CAP {
            return false;
        }
        self.stamps.push_back(now);
        true
    }

    /// True if a turn at `now` would be allowed. Nothing is counted.
    pub fn would_allow(&self, now: Duration) -> bool {
        self.stamps
            .iter()
            .filter(|t| now.saturating_sub(**t) < RATE_WINDOW)
            .count()
            < RATE_CAP
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fourth_waiting_turn_is_refused() {
        let mut q = WaitQueue::default();
        assert_eq!(q.push("a".into(), 1), Some(1));
        assert_eq!(q.push("b".into(), 2), Some(2));
        assert_eq!(q.push("c".into(), 3), Some(3));
        assert_eq!(q.push("d".into(), 4), None);
        assert_eq!(q.len(), 3);
    }

    #[test]
    fn remove_takes_a_waiting_turn_out_and_the_order_holds() {
        let mut q = WaitQueue::default();
        q.push("a".into(), 1);
        q.push("b".into(), 2);
        q.push("c".into(), 3);
        assert_eq!(q.remove("b"), Some(2));
        assert_eq!(q.remove("b"), None);
        assert_eq!(q.pop(), Some(1));
        assert_eq!(q.pop(), Some(3));
        assert_eq!(q.pop(), None);
    }

    #[test]
    fn six_turns_a_minute_and_the_window_slides() {
        let mut r = RateCap::default();
        for i in 0..6 {
            assert!(r.allow(Duration::from_secs(i)));
        }
        assert!(!r.would_allow(Duration::from_secs(10)));
        assert!(!r.allow(Duration::from_secs(10)));
        // The first stamp (t = 0) leaves the window at t = 60.
        assert!(!r.allow(Duration::from_secs(59)));
        assert!(r.allow(Duration::from_secs(60)));
        assert!(!r.allow(Duration::from_secs(60)));
    }
}
