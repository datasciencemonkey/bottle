//! R6 TokenBroker (section 4.3).
//!
//! Responsibilities: In-memory tokens per profile in buffers that it overwrites with zeros. Refresh at expiry − 60 s, single-flight, drop on 401. Resolves `workspace_id` from `x-databricks-org-id`.
//!
//! Dependency rule (ARC-6, section 4.6): only this module names the token type [`SecretToken`].
//! Other components never hold a token. They lend the bytes for one call through
//! [`TokenBroker::with_token`]. A test fails if any other source file names the type.
//!
//! [`CliTokenBroker`] is the real R6 (#18, ADR 0066):
//! - W-04a: tokens come only from `databricks auth token --profile <p> --output json`, through
//!   [`CliRunner::run_auth_token`]. The caller passes the profile; R6 never picks one.
//! - W-01b, W-04b: a token is only in memory, in a [`SecretToken`] that is zeroed on drop. It is
//!   never written to disk, a log, `Debug` output, an error or the agent harness.
//! - W-05a: one refresh per profile at expiry - 60 s ([`REFRESH_MARGIN`]); concurrent callers wait
//!   for the one CLI run in flight (single-flight), and share its result.
//! - W-05b, W-05c: [`TokenBroker::invalidate`] drops the token; the next call runs the CLI once.
//!   R7 retries the failed request once (W-05d).
//! - 4.6 rule 2: each token is bound to the host of its profile when R6 gets it, and R6 lends it
//!   only for that exact host.
//! - W-06a, W-06b: `workspace_id` is the `x-databricks-org-id` header of
//!   `GET /api/2.0/preview/scim/v2/Me` (the [`ScimMe`] seam, R7), cached per profile per launch.
//!
//! The refresh margin and the CLI reply fields follow the token cache of the reference plugin.
// Adapted from datasciencemonkey/antigravity-bricks-plugin plugins/databricks-genie/scripts/genie_bridge.py @ a31fd2c (MIT).

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{compiler_fence, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use reqwest::Url;
use serde::Deserialize;

use crate::cli_runner::CliRunner;
use crate::clock::{MonotonicClock, WallClock};
use crate::error::{CoreError, CoreResult};
use crate::genie_endpoint::{valid_workspace_id, GenieOneEndpoint};
use crate::http_transport::ProfileHosts;
use crate::types::TokenState;

const ID: &str = "R6";

/// R6 refreshes a token this long before it expires (W-05a, ARC-F5).
pub const REFRESH_MARGIN: Duration = Duration::from_secs(60);
/// The lifetime R6 assumes when the CLI reply has no expiry (ADR 0066). R6 then refreshes
/// after this time, without the margin.
pub const DEFAULT_LIFETIME: Duration = Duration::from_secs(300);

/// The CLI could not give a token (ARC-F5): the UI shows "Sign in to <profile>".
pub const LOGIN_REQUIRED: &str = "login_required";
/// The CLI reply is not a usable token reply (bad JSON, no token, bad or past expiry).
pub const TOKEN_REPLY_INVALID: &str = "token_reply_invalid";
/// The request host is not the host this token is bound to (4.6 rule 2).
pub const TOKEN_HOST_MISMATCH: &str = "token_host_mismatch";
/// The profile name is not a CLI profile name (it could be read as a flag).
pub const PROFILE_INVALID: &str = "profile_invalid";
/// The profile host is not a bare workspace URL.
pub const PROFILE_HOST_INVALID: &str = "profile_host_invalid";
/// The SCIM reply had no `x-databricks-org-id` header (W-06a).
pub const ORG_ID_MISSING: &str = "org_id_missing";
/// The `x-databricks-org-id` value is not a workspace ID.
pub const WORKSPACE_ID_INVALID: &str = "workspace_id_invalid";
/// No [`ScimMe`] seam is bound, or it is gone.
pub const SCIM_UNAVAILABLE: &str = "scim_unavailable";

fn unavailable(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

/// A Databricks token in memory.
///
/// It is not `Clone` and not `Serialize`, its `Debug` output is redacted, and its buffer is
/// overwritten with zeros when it drops.
///
/// ```compile_fail
/// let t = mog_core::token_broker::SecretToken::new(String::from("x"));
/// let _copy = t.clone();
/// ```
pub struct SecretToken {
    bytes: Vec<u8>,
}

impl SecretToken {
    /// Take ownership of a token string. The string buffer moves in; no copy is made.
    pub fn new(value: String) -> Self {
        Self {
            bytes: value.into_bytes(),
        }
    }

    fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The token length in bytes.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True if the token is empty.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Overwrite the buffer with zeros now. `Drop` calls this too.
    pub fn wipe(&mut self) {
        for b in self.bytes.iter_mut() {
            *b = 0;
        }
        // Keep the compiler from removing the zero writes as dead stores.
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut self.bytes);
    }

    /// True if every byte is zero (after [`SecretToken::wipe`]).
    pub fn is_zeroed(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }
}

impl Drop for SecretToken {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretToken(<redacted>)")
    }
}

/// Holds the tokens per profile and lends them for one call at a time.
pub trait TokenBroker: Send + Sync {
    /// Lend the current token of `profile` to `f` for one request to `host`. The bytes must not
    /// be kept. `host` is the exact host the request goes to (no scheme, port or path); R6
    /// refuses any host other than the one the token is bound to (4.6 rule 2).
    fn with_token(&self, profile: &str, host: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<()>;
    /// Drop the token of `profile` (for example after a 401).
    fn invalidate(&self, profile: &str) -> CoreResult<()>;
    /// The `workspace_id` of `profile`, from `x-databricks-org-id`.
    fn workspace_id(&self, profile: &str) -> CoreResult<String>;
}

/// The R6 stub for tests and unwired builds. It holds no token. Every method returns
/// [`CoreError::NotImplemented`]. The real R6 is [`CliTokenBroker`].
#[derive(Debug, Default, Clone, Copy)]
pub struct StubTokenBroker;

impl TokenBroker for StubTokenBroker {
    fn with_token(&self, _profile: &str, _host: &str, _f: &mut dyn FnMut(&[u8])) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn invalidate(&self, _profile: &str) -> CoreResult<()> {
        Err(CoreError::NotImplemented { component: ID })
    }

    fn workspace_id(&self, _profile: &str) -> CoreResult<String> {
        Err(CoreError::NotImplemented { component: ID })
    }
}

/// The SCIM `Me` call that R6 needs for `workspace_id` (W-06a). R7 implements it: it sends
/// `GET /api/2.0/preview/scim/v2/Me` for `profile` (with the token from R6, to the profile host)
/// and returns the `x-databricks-org-id` header, or `None` if the reply has none.
pub trait ScimMe: Send + Sync {
    /// The `x-databricks-org-id` header of `GET /api/2.0/preview/scim/v2/Me` for `profile`.
    fn org_id_header(&self, profile: &str) -> CoreResult<Option<String>>;
}

/// One token in memory, bound to its host.
struct Held {
    token: SecretToken,
    /// The lowercase host of the profile when R6 got the token.
    host: String,
    /// Monotonic time of the refresh (expiry - 60 s).
    refresh_at: Duration,
    /// Monotonic time of the expiry.
    expires_at: Duration,
}

/// The per-profile state. A token in use by a caller stays alive (in its `Arc`) until that
/// one call ends, then it drops and its buffer is zeroed.
#[derive(Default)]
struct Slot {
    token: Option<Arc<Held>>,
    /// A CLI run is in flight for this profile.
    fetching: bool,
    /// Callers waiting for that run.
    waiting: usize,
    /// Counts finished CLI runs, so a waiter knows a run ended while it waited.
    generation: u64,
    /// The code of the last run, if it failed.
    failed: Option<&'static str>,
}

/// The real R6 TokenBroker (#18). See the module text for the rules it keeps.
pub struct CliTokenBroker {
    cli: Arc<dyn CliRunner>,
    hosts: Arc<dyn ProfileHosts>,
    mono: Arc<dyn MonotonicClock>,
    wall: Arc<dyn WallClock>,
    scim: OnceLock<Weak<dyn ScimMe>>,
    slots: Mutex<HashMap<String, Slot>>,
    done: Condvar,
    ids: Mutex<HashMap<String, Arc<Mutex<Option<String>>>>>,
}

impl fmt::Debug for CliTokenBroker {
    /// Shows the profile count only: no token, host or profile name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CliTokenBroker")
            .field("profiles", &self.lock().len())
            .finish_non_exhaustive()
    }
}

/// A Databricks CLI profile name: a letter, digit or `_` first (so never a flag), then letters,
/// digits, `_`, `.` or `-`, at most 64 characters.
fn valid_profile(p: &str) -> bool {
    let b = p.as_bytes();
    !b.is_empty()
        && b.len() <= 64
        && (b[0].is_ascii_alphanumeric() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'.' | b'-'))
}

/// The CLI reply. The token borrows from the stdout buffer, so R6 makes one copy only.
#[derive(Deserialize)]
struct Reply<'a> {
    #[serde(borrow)]
    access_token: &'a str,
    #[serde(default)]
    expires_in: Option<f64>,
    #[serde(default, borrow)]
    expiry: Option<&'a str>,
}

impl CliTokenBroker {
    /// R6 on R5 (`cli`), the profile hosts, a monotonic clock for the refresh time and a wall
    /// clock to read the CLI expiry. Bind the SCIM seam with [`CliTokenBroker::bind_scim`].
    pub fn new(
        cli: Arc<dyn CliRunner>,
        hosts: Arc<dyn ProfileHosts>,
        mono: Arc<dyn MonotonicClock>,
        wall: Arc<dyn WallClock>,
    ) -> Self {
        Self {
            cli,
            hosts,
            mono,
            wall,
            scim: OnceLock::new(),
            slots: Mutex::new(HashMap::new()),
            done: Condvar::new(),
            ids: Mutex::new(HashMap::new()),
        }
    }

    /// Bind the SCIM seam (R7). R7 holds R6, so R6 holds R7 weakly. The first bind wins.
    pub fn bind_scim(&self, scim: Weak<dyn ScimMe>) {
        let _ = self.scim.set(scim);
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Drop every token of every profile now (W-04b: sleep, lock, sign-out).
    /// TODO(#163): the sign-out flow and the OS sleep and lock events call this.
    pub fn drop_all(&self) {
        let mut slots = self.lock();
        for slot in slots.values_mut() {
            slot.token = None;
            slot.failed = None;
        }
    }

    /// The token state of `profile` for TS (`profile_status`): never a token.
    pub fn token_state(&self, profile: &str) -> TokenState {
        let slots = self.lock();
        let Some(slot) = slots.get(profile) else {
            return TokenState::Unknown;
        };
        match (&slot.token, slot.failed) {
            (Some(h), _) if self.mono.now() < h.refresh_at => TokenState::Valid,
            (Some(h), _) if self.mono.now() < h.expires_at => TokenState::Expiring,
            (_, Some(_)) => TokenState::LoginRequired,
            _ => TokenState::Unknown,
        }
    }

    /// The Genie One MCP endpoint of `profile`, always with `?o=<workspace_id>` (W-06a, G-02).
    pub fn genie_one_endpoint(&self, profile: &str) -> CoreResult<GenieOneEndpoint> {
        let host = self.profile_host(profile)?;
        let id = self.workspace_id(profile)?;
        GenieOneEndpoint::new(&host, &id).map_err(|_| unavailable(PROFILE_HOST_INVALID))
    }

    /// TEST SEAM: the number of callers waiting for the CLI run in flight for `profile`.
    #[doc(hidden)]
    pub fn waiting(&self, profile: &str) -> usize {
        self.lock().get(profile).map_or(0, |s| s.waiting)
    }

    /// The lowercase host of the profile URL (a bare `scheme://host[:port]` URL only).
    fn profile_host(&self, profile: &str) -> CoreResult<String> {
        let text = self.hosts.workspace_url(profile)?;
        let u = Url::parse(text.trim()).map_err(|_| unavailable(PROFILE_HOST_INVALID))?;
        let bare = (u.path().is_empty() || u.path() == "/")
            && u.query().is_none()
            && u.fragment().is_none()
            && u.username().is_empty()
            && u.password().is_none();
        match u.host_str() {
            Some(h) if bare && !h.is_empty() => Ok(h.to_ascii_lowercase()),
            _ => Err(unavailable(PROFILE_HOST_INVALID)),
        }
    }

    /// The current token of `profile`: the cached one, or one new CLI run shared by every
    /// caller that arrives while it is in flight (W-05a).
    fn current(&self, profile: &str) -> CoreResult<Arc<Held>> {
        if !valid_profile(profile) {
            return Err(unavailable(PROFILE_INVALID));
        }
        let mut slots = self.lock();
        let start = slots.entry(profile.to_owned()).or_default().generation;
        loop {
            let now = self.mono.now();
            let slot = slots.entry(profile.to_owned()).or_default();
            if let Some(h) = &slot.token {
                if now < h.refresh_at {
                    return Ok(Arc::clone(h));
                }
            }
            if slot.generation != start {
                // A run ended while this caller waited: share its failure, do not run again.
                if let Some(code) = slot.failed {
                    return Err(unavailable(code));
                }
            }
            if slot.fetching {
                slot.waiting += 1;
                slots = self.done.wait(slots).unwrap_or_else(|e| e.into_inner());
                if let Some(s) = slots.get_mut(profile) {
                    s.waiting -= 1;
                }
                continue;
            }
            slot.fetching = true;
            drop(slots);
            let got = self.fetch(profile);
            slots = self.lock();
            let slot = slots.entry(profile.to_owned()).or_default();
            slot.fetching = false;
            slot.generation += 1;
            let out = match got {
                Ok(held) => {
                    let held = Arc::new(held);
                    slot.token = Some(Arc::clone(&held));
                    slot.failed = None;
                    Ok(held)
                }
                Err(e) => {
                    slot.token = None;
                    slot.failed = Some(e.code());
                    Err(e)
                }
            };
            self.done.notify_all();
            return out;
        }
    }

    /// One CLI run: the token, bound to the profile host, with its refresh time.
    fn fetch(&self, profile: &str) -> CoreResult<Held> {
        let host = self.profile_host(profile)?;
        let mut stdout = match self.cli.run_auth_token(profile) {
            Ok(b) => b,
            Err(e @ CoreError::NotImplemented { .. }) => return Err(e),
            Err(_) => return Err(unavailable(LOGIN_REQUIRED)),
        };
        let parsed = self.parse(&stdout);
        // The stdout buffer held the token: overwrite it before it drops.
        stdout.fill(0);
        compiler_fence(Ordering::SeqCst);
        std::hint::black_box(&mut stdout);
        let (token, lifetime) = parsed?;
        let now = self.mono.now();
        let refresh_in = match lifetime {
            Some(l) => l.saturating_sub(REFRESH_MARGIN),
            None => DEFAULT_LIFETIME,
        };
        Ok(Held {
            token,
            host,
            refresh_at: now + refresh_in,
            expires_at: now + lifetime.unwrap_or(DEFAULT_LIFETIME),
        })
    }

    /// The token and its remaining lifetime (`None`: the reply has no expiry).
    fn parse(&self, stdout: &[u8]) -> CoreResult<(SecretToken, Option<Duration>)> {
        let bad = || unavailable(TOKEN_REPLY_INVALID);
        let r: Reply<'_> = serde_json::from_slice(stdout).map_err(|_| bad())?;
        if r.access_token.is_empty() {
            return Err(bad());
        }
        let lifetime = match (r.expires_in, r.expiry) {
            (Some(secs), _) if secs.is_finite() && secs > 0.0 => {
                Some(Duration::from_secs_f64(secs.min(1.0e9)))
            }
            (Some(_), _) => return Err(bad()),
            (None, Some(text)) => {
                let left = rfc3339_ms(text).ok_or_else(bad)? - self.wall.now_ms();
                if left <= 0 {
                    return Err(bad());
                }
                Some(Duration::from_millis(left as u64))
            }
            (None, None) => None,
        };
        Ok((SecretToken::new(r.access_token.to_owned()), lifetime))
    }
}

impl TokenBroker for CliTokenBroker {
    fn with_token(&self, profile: &str, host: &str, f: &mut dyn FnMut(&[u8])) -> CoreResult<()> {
        let held = self.current(profile)?;
        if !held.host.eq_ignore_ascii_case(host) {
            return Err(unavailable(TOKEN_HOST_MISMATCH));
        }
        f(held.token.bytes());
        Ok(())
    }

    fn invalidate(&self, profile: &str) -> CoreResult<()> {
        if let Some(slot) = self.lock().get_mut(profile) {
            slot.token = None;
        }
        Ok(())
    }

    fn workspace_id(&self, profile: &str) -> CoreResult<String> {
        if !valid_profile(profile) {
            return Err(unavailable(PROFILE_INVALID));
        }
        let cell = {
            let mut ids = self.ids.lock().unwrap_or_else(|e| e.into_inner());
            Arc::clone(ids.entry(profile.to_owned()).or_default())
        };
        // One lookup per profile at a time, so concurrent first calls make one SCIM call.
        let mut id = cell.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(known) = &*id {
            return Ok(known.clone());
        }
        let scim = self
            .scim
            .get()
            .and_then(Weak::upgrade)
            .ok_or(unavailable(SCIM_UNAVAILABLE))?;
        let header = scim
            .org_id_header(profile)?
            .ok_or(unavailable(ORG_ID_MISSING))?;
        let value = header.trim();
        if !valid_workspace_id(value) {
            return Err(unavailable(WORKSPACE_ID_INVALID));
        }
        *id = Some(value.to_owned());
        Ok(value.to_owned())
    }
}

/// Unix milliseconds of an RFC 3339 time such as `2026-11-17T13:00:00.123456-05:00` or
/// `...Z` (the `expiry` of `databricks auth token --output json`).
fn rfc3339_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let t = s.get(r)?;
        if t.bytes().all(|c| c.is_ascii_digit()) {
            t.parse().ok()
        } else {
            None
        }
    };
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    let mut i = 19;
    let mut frac_ms = 0i64;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return None;
        }
        let digits = &s[start..i.min(start + 3)];
        frac_ms = digits.parse::<i64>().ok()? * 10i64.pow(3 - digits.len() as u32);
    }
    let offset_min = match b.get(i)? {
        b'Z' | b'z' if i + 1 == b.len() => 0,
        sign @ (b'+' | b'-') if i + 6 == b.len() && b[i + 3] == b':' => {
            let (oh, om) = (num(i + 1..i + 3)?, num(i + 4..i + 6)?);
            if oh > 23 || om > 59 {
                return None;
            }
            let m = oh * 60 + om;
            if *sign == b'-' {
                -m
            } else {
                m
            }
        }
        _ => return None,
    };
    // Days from civil (Howard Hinnant's algorithm).
    let y2 = if mo <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (mo + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + h * 3_600 + mi * 60 + sec - offset_min * 60;
    Some(secs * 1_000 + frac_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w_05a_rfc3339_expiry_parses_utc_offsets_and_fractions() {
        let t0 = 1_794_916_800_000; // 2026-11-17T12:00:00Z
        assert_eq!(rfc3339_ms("2026-11-17T12:00:00Z"), Some(t0));
        assert_eq!(rfc3339_ms("2026-11-17T07:00:00-05:00"), Some(t0));
        assert_eq!(rfc3339_ms("2026-11-17T13:00:00.5+01:00"), Some(t0 + 500));
        assert_eq!(rfc3339_ms("2026-11-17T12:00:00.123456789Z"), Some(t0 + 123));
        assert_eq!(rfc3339_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_ms("2024-02-29T00:00:00Z"), Some(1_709_164_800_000));
        for bad in [
            "",
            "yesterday",
            "2026-11-17",
            "2026-11-17T12:00:00",
            "2026-13-17T12:00:00Z",
            "2026-11-17T12:00:00.Z",
            "2026-11-17T12:00:00+0100",
            "2026-11-17T12:00:00Zjunk",
            "+026-11-17T12:00:00Z",
        ] {
            assert_eq!(rfc3339_ms(bad), None, "{bad}");
        }
    }

    #[test]
    fn w_04a_profile_names_that_could_be_flags_are_not_profiles() {
        for ok in ["bottle-test", "DEFAULT", "_x", "a.b-c_1"] {
            assert!(valid_profile(ok), "{ok}");
        }
        for bad in ["", "-p", ".x", "a b", "a/b", "é"] {
            assert!(!valid_profile(bad), "{bad}");
        }
    }
}
