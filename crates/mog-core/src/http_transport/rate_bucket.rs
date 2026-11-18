//! The shared per-profile rate bucket of R7 (#364, ADR 0046; #138, ADR 0075; G-09a..G-09d,
//! G-10a).
//!
//! [`AimdBucket`] is the bucket behind the [`RateBucket`] seam. R7 and R8 share one per process:
//! - G-09a: 40 calls/min per profile in a sliding 60 s window.
//! - G-09b: a call that must wait takes a [`Ticket`] of its [`CallClass`]. A slot goes to the
//!   waiting ticket of the highest class (polls, then cancels, then new asks), and in one class
//!   to the oldest ticket. A call that does not wait ([`RateBucket::reserve`]) never jumps a
//!   waiting ticket.
//! - G-09c: a 429 halves the rate for 5 min. A 429 inside that hold starts the hold again but
//!   does not halve the rate again.
//! - G-09d: after the 5 min, the rate gains 5 calls/min each minute up to the limit.
//! - G-10a: `Retry-After` (seconds or HTTP date) or FMAPI `error.retry_after`, clamped to
//!   1-300 s, holds every call of the profile until it ends. A `Retry-After` on a 503 holds the
//!   profile the same way ([`RateBucket::hold`]) but does not cut the rate.
//!
//! The G-BACKOFF ask retry (G-10b) is in [`super::backoff`]; the blocking wait and the busy
//! states of the card are in [`super::rate_limit`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::clock::MonotonicClock;

/// G-09a: the default limit of each profile.
pub const DEFAULT_RATE_PER_MIN: u32 = 40;
/// G-09c: how long the halved rate holds after a 429.
pub const HALF_RATE_HOLD: Duration = Duration::from_secs(300);
/// G-09d: the rate added each minute after the hold.
pub const RECOVERY_STEP_PER_MIN: u32 = 5;
/// G-10a: the shortest retry wait.
pub const RETRY_AFTER_MIN: Duration = Duration::from_secs(1);
/// G-10a: the longest retry wait.
pub const RETRY_AFTER_MAX: Duration = Duration::from_secs(300);
/// G-09b: how soon a ticket asks again when a slot is free but a higher ticket waits for it.
pub const PRIORITY_RECHECK: Duration = Duration::from_millis(20);

const WINDOW: Duration = Duration::from_secs(60);

/// The priority class of one call (G-09b). The order of the variants is the priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CallClass {
    /// A poll of a running answer: `genie_poll_response`, `genie_get_query_result`.
    Poll,
    /// A cancel of a running answer: `genie_cancel_response`.
    Cancel,
    /// A new ask, and every other call.
    Ask,
}

impl CallClass {
    /// The class of a Genie One MCP tool (R8). An unknown name is an ask (the lowest class).
    pub fn of_tool(tool: &str) -> Self {
        match tool {
            "genie_poll_response" | "genie_get_query_result" => Self::Poll,
            "genie_cancel_response" => Self::Cancel,
            _ => Self::Ask,
        }
    }

    /// The class of an R7 route ID. An unknown route is an ask (the lowest class).
    pub fn of_route(route_id: &str) -> Self {
        match route_id {
            "genie.get_message" | "genie.query_result" => Self::Poll,
            _ => Self::Ask,
        }
    }
}

/// One waiting call on one profile (G-09b). It is not `Clone`: a ticket is spent once.
#[derive(Debug, PartialEq, Eq)]
pub struct Ticket {
    class: CallClass,
    seq: u64,
}

impl Ticket {
    /// The class of the waiting call.
    pub fn class(&self) -> CallClass {
        self.class
    }
}

/// The per-profile call budget that R7 and R8 ask before each call they send.
pub trait RateBucket: Send + Sync {
    /// Ask to send one call on `profile` without a place in the queue. `Duration::ZERO` means
    /// "send now" and counts the call. Any other value is the time to wait before asking again.
    fn reserve(&self, profile: &str) -> Duration;
    /// Record a 429 on `profile`, with the retry wait of the reply if it had one.
    fn on_rate_limited(&self, profile: &str, retry_after: Option<Duration>);
    /// G-09b: take a place in the queue of `profile` for one call of `class`.
    fn enqueue(&self, _profile: &str, class: CallClass) -> Ticket {
        Ticket { class, seq: 0 }
    }
    /// G-09b: ask to send the call of `ticket`. `Duration::ZERO` means "send now": the call is
    /// counted and the ticket is spent. Any other value is the time to wait before asking again.
    fn take(&self, profile: &str, _ticket: &Ticket) -> Duration {
        self.reserve(profile)
    }
    /// G-09b: give up the place of a ticket that was not spent.
    fn leave(&self, _profile: &str, _ticket: Ticket) {}
    /// G-10a: hold every call of `profile` for `wait` (clamped to 1-300 s) with no rate cut,
    /// for a `Retry-After` on a reply that is not a 429 (a 503).
    fn hold(&self, _profile: &str, _wait: Duration) {}
}

#[derive(Debug, Default)]
struct ProfileState {
    calls: VecDeque<Duration>,
    /// The time of the last 429 and the rate it set.
    cut: Option<(Duration, u32)>,
    retry_until: Duration,
    /// G-09b: the tickets that wait, in priority order.
    waiting: BTreeSet<(CallClass, u64)>,
}

impl ProfileState {
    /// Drop the calls that left the 60 s window.
    fn prune(&mut self, now: Duration) {
        while self
            .calls
            .front()
            .is_some_and(|t| t.saturating_add(WINDOW) <= now)
        {
            self.calls.pop_front();
        }
    }

    /// The time until the window has room at `rate`, or zero if it has room now.
    fn room_in(&self, rate: usize, now: Duration) -> Duration {
        if self.calls.len() < rate {
            return Duration::ZERO;
        }
        let oldest_to_expire = self.calls[self.calls.len() - rate];
        oldest_to_expire
            .saturating_add(WINDOW)
            .saturating_sub(now)
            .max(Duration::from_nanos(1))
    }

    /// One try to send. `key` is the place of the caller in the queue, or `None` for a call
    /// that does not queue (it goes behind every waiting ticket).
    fn try_send(&mut self, rate: usize, now: Duration, key: Option<(CallClass, u64)>) -> Duration {
        if now < self.retry_until {
            return self.retry_until - now;
        }
        self.prune(now);
        let room = self.room_in(rate, now);
        let ahead = match key {
            Some(k) => self.waiting.first().is_some_and(|first| *first < k),
            None => !self.waiting.is_empty(),
        };
        if ahead {
            return room.max(PRIORITY_RECHECK);
        }
        if !room.is_zero() {
            return room;
        }
        self.calls.push_back(now);
        if let Some(k) = key {
            self.waiting.remove(&k);
        }
        Duration::ZERO
    }
}

/// The AIMD bucket: see the module doc.
#[derive(Debug)]
pub struct AimdBucket {
    clock: Arc<dyn MonotonicClock>,
    limit: u32,
    profiles: Mutex<BTreeMap<String, ProfileState>>,
    next_seq: AtomicU64,
}

impl AimdBucket {
    /// A bucket at [`DEFAULT_RATE_PER_MIN`] on `clock`.
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            clock,
            limit: DEFAULT_RATE_PER_MIN,
            profiles: Mutex::new(BTreeMap::new()),
            next_seq: AtomicU64::new(1),
        }
    }

    fn rate_at(&self, st: &ProfileState, now: Duration) -> u32 {
        match st.cut {
            None => self.limit,
            Some((at, rate)) => {
                let end = at.saturating_add(HALF_RATE_HOLD);
                if now < end {
                    rate
                } else {
                    let steps = (now - end).as_secs() / 60;
                    let gained = u32::try_from(steps)
                        .unwrap_or(u32::MAX)
                        .saturating_mul(RECOVERY_STEP_PER_MIN);
                    rate.saturating_add(gained).min(self.limit)
                }
            }
        }
    }

    /// The current calls/min of `profile`.
    pub fn rate_per_min(&self, profile: &str) -> u32 {
        let now = self.clock.now();
        let map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        map.get(profile)
            .map_or(self.limit, |st| self.rate_at(st, now))
    }
}

impl AimdBucket {
    /// One try to send on `profile`, at the current rate.
    fn try_send(&self, profile: &str, key: Option<(CallClass, u64)>) -> Duration {
        let now = self.clock.now();
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        let rate = map
            .get(profile)
            .map_or(self.limit, |st| self.rate_at(st, now))
            .max(1) as usize;
        map.entry(profile.to_owned())
            .or_default()
            .try_send(rate, now, key)
    }
}

impl RateBucket for AimdBucket {
    fn reserve(&self, profile: &str) -> Duration {
        self.try_send(profile, None)
    }

    fn on_rate_limited(&self, profile: &str, retry_after: Option<Duration>) {
        let now = self.clock.now();
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        let current = map
            .get(profile)
            .map_or(self.limit, |st| self.rate_at(st, now));
        let st = map.entry(profile.to_owned()).or_default();
        // G-09c: one halving per hold. A 429 inside the 5 min hold starts the hold again at
        // the same rate, so a burst of 429s (or the retries of one ask) does not cut it again.
        let rate = match st.cut {
            Some((at, rate)) if now < at.saturating_add(HALF_RATE_HOLD) => rate,
            _ => (current / 2).max(1),
        };
        st.cut = Some((now, rate));
        if let Some(wait) = retry_after {
            st.retry_until = st.retry_until.max(now.saturating_add(clamp(wait)));
        }
    }

    fn enqueue(&self, profile: &str, class: CallClass) -> Ticket {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        map.entry(profile.to_owned())
            .or_default()
            .waiting
            .insert((class, seq));
        Ticket { class, seq }
    }

    fn take(&self, profile: &str, ticket: &Ticket) -> Duration {
        self.try_send(profile, Some((ticket.class, ticket.seq)))
    }

    fn leave(&self, profile: &str, ticket: Ticket) {
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(st) = map.get_mut(profile) {
            st.waiting.remove(&(ticket.class, ticket.seq));
        }
    }

    fn hold(&self, profile: &str, wait: Duration) {
        let now = self.clock.now();
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        let st = map.entry(profile.to_owned()).or_default();
        st.retry_until = st.retry_until.max(now.saturating_add(clamp(wait)));
    }
}

/// Clamp a retry wait to 1-300 s (G-10a).
pub fn clamp(d: Duration) -> Duration {
    d.clamp(RETRY_AFTER_MIN, RETRY_AFTER_MAX)
}

/// Parse a `Retry-After` value: delay seconds or an IMF-fixdate HTTP date
/// (`Wed, 21 Oct 2015 07:28:00 GMT`), clamped to 1-300 s. `None` if it is neither.
pub fn parse_retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let v = value.trim();
    if !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) {
        return Some(clamp(Duration::from_secs(
            v.parse::<u64>().unwrap_or(u64::MAX),
        )));
    }
    let at = parse_imf_fixdate(v)?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(clamp(Duration::from_secs(at.saturating_sub(now))))
}

/// FMAPI `error.retry_after` (seconds, number or digit string), clamped to 1-300 s.
pub fn retry_after_from_body(body: &Value) -> Option<Duration> {
    let v = body.get("error")?.get("retry_after")?;
    let secs = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) => {
            s.parse::<f64>().ok()?
        }
        _ => return None,
    };
    if !secs.is_finite() || secs < 0.0 {
        return None;
    }
    Some(clamp(Duration::from_secs_f64(secs.min(1e9))))
}

/// Seconds since the Unix epoch of an IMF-fixdate.
fn parse_imf_fixdate(s: &str) -> Option<u64> {
    let mut parts = s.split(' ');
    let _weekday = parts.next()?.strip_suffix(',')?;
    let day: u64 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':');
    let h: u64 = hms.next()?.parse().ok()?;
    let m: u64 = hms.next()?.parse().ok()?;
    let sec: u64 = hms.next()?.parse().ok()?;
    if parts.next()? != "GMT" || parts.next().is_some() || hms.next().is_some() {
        return None;
    }
    if !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 60 || year < 1970 {
        return None;
    }
    let days = u64::try_from(days_from_civil(year, month, day as i64)).ok()?;
    Some(days * 86_400 + h * 3_600 + m * 60 + sec)
}

/// Days from 1970-01-01 to a civil date (H. Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
