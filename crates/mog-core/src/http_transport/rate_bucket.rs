//! The shared per-profile rate bucket of R7 (#364, ADR 0046; G-09a, G-09c, G-09d, G-10a).
//!
//! [`AimdBucket`] is the simplest correct version behind the [`RateBucket`] seam:
//! - G-09a: 40 calls/min per profile in a sliding 60 s window.
//! - G-09c: a 429 halves the rate for 5 min.
//! - G-09d: after the 5 min, the rate gains 5 calls/min each minute up to the limit.
//! - G-10a: `Retry-After` (seconds or HTTP date) or FMAPI `error.retry_after`, clamped to
//!   1-300 s, holds every call of the profile until it ends.
//!
//! TODO(#138): G-09b priority (polls, then cancels, then new asks) and G-10b ask backoff
//! (Table G-BACKOFF) need the call class from the bridge; #138 adds them behind this seam.

use std::collections::{BTreeMap, VecDeque};
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

const WINDOW: Duration = Duration::from_secs(60);

/// The per-profile call budget that R7 asks before each call it sends.
pub trait RateBucket: Send + Sync {
    /// Ask to send one call on `profile`. `Duration::ZERO` means "send now" and counts the
    /// call. Any other value is the time to wait before asking again.
    fn reserve(&self, profile: &str) -> Duration;
    /// Record a 429 on `profile`, with the retry wait of the reply if it had one.
    fn on_rate_limited(&self, profile: &str, retry_after: Option<Duration>);
}

#[derive(Debug, Default)]
struct ProfileState {
    calls: VecDeque<Duration>,
    /// The time of the last 429 and the rate it set.
    cut: Option<(Duration, u32)>,
    retry_until: Duration,
}

/// The AIMD bucket: see the module doc.
#[derive(Debug)]
pub struct AimdBucket {
    clock: Arc<dyn MonotonicClock>,
    limit: u32,
    profiles: Mutex<BTreeMap<String, ProfileState>>,
}

impl AimdBucket {
    /// A bucket at [`DEFAULT_RATE_PER_MIN`] on `clock`.
    pub fn new(clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            clock,
            limit: DEFAULT_RATE_PER_MIN,
            profiles: Mutex::new(BTreeMap::new()),
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

impl RateBucket for AimdBucket {
    fn reserve(&self, profile: &str) -> Duration {
        let now = self.clock.now();
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        let rate = map
            .get(profile)
            .map_or(self.limit, |st| self.rate_at(st, now))
            .max(1) as usize;
        let st = map.entry(profile.to_owned()).or_default();
        if now < st.retry_until {
            return st.retry_until - now;
        }
        while st
            .calls
            .front()
            .is_some_and(|t| t.saturating_add(WINDOW) <= now)
        {
            st.calls.pop_front();
        }
        if st.calls.len() < rate {
            st.calls.push_back(now);
            return Duration::ZERO;
        }
        let oldest_to_expire = st.calls[st.calls.len() - rate];
        oldest_to_expire.saturating_add(WINDOW).saturating_sub(now)
    }

    fn on_rate_limited(&self, profile: &str, retry_after: Option<Duration>) {
        let now = self.clock.now();
        let mut map = self.profiles.lock().unwrap_or_else(PoisonError::into_inner);
        let current = map
            .get(profile)
            .map_or(self.limit, |st| self.rate_at(st, now));
        let st = map.entry(profile.to_owned()).or_default();
        st.cut = Some((now, (current / 2).max(1)));
        if let Some(wait) = retry_after {
            st.retry_until = st.retry_until.max(now.saturating_add(clamp(wait)));
        }
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
