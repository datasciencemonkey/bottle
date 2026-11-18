//! R6 token drops on OS sleep, screen lock and sign-out, and the `profile_status` reply
//! (#465, ADR 0076).
//!
//! - W-04b: on an OS sleep or screen-lock event, [`watch_token_drops`] calls
//!   [`CliTokenBroker::drop_all`], so the next call runs `databricks auth token` again.
//! - W-18c: [`sign_out`] drops the token of one profile ([`CliTokenBroker::drop_profile`]). Only
//!   the user's sign-out action calls it (#163). Bottle never signs out by itself.
//! - `profile_status`: [`ProfileStatusComponent`] returns `TokenStatus { profile, state }` from
//!   [`CliTokenBroker::token_state`]. It never holds or returns a token (ARC-6), and it never
//!   runs the CLI.
//!
//! The OS events ([`os_session_source`]):
//! - macOS: `NSWorkspaceWillSleepNotification` (sleep) and the `com.apple.screenIsLocked`
//!   distributed notification (lock).
//! - Windows: `PBT_APMSUSPEND` from `PowerRegisterSuspendResumeNotification` (sleep) and
//!   `WTS_SESSION_LOCK` to a message-only window (lock).
//!
//! The host routes `profile_status` here and keeps the OS watch for the app lifetime, on the
//! R6 instance that #467 builds: TODO(#486) (`src-tauri/src/ipc.rs` is a contract file).

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::Value;

use crate::error::{CoreError, CoreResult};
use crate::ipc::ProfileArgs;
use crate::ipc_gateway::GatewayComponents;
use crate::token_broker::CliTokenBroker;
use crate::types::{TokenState, TokenStatus};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

const ID: &str = "R6";

/// An OS event after which R6 drops every token (W-04b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEvent {
    /// The computer is about to sleep.
    Sleep,
    /// The screen is locked.
    ScreenLock,
}

/// Where an OS event source sends its events. It may run on any thread.
pub type SessionSink = Arc<dyn Fn(SessionEvent) + Send + Sync>;

/// A source of OS sleep and lock events.
pub trait SessionEventSource: Send + Sync {
    /// Send each event to `sink` until the returned watch drops.
    fn watch(&self, sink: SessionSink) -> CoreResult<SessionWatch>;
}

/// A live watch. Dropping it stops the events: [`watch_token_drops`] does no drop after that,
/// and the OS source removes its observers.
#[must_use = "dropping the watch stops the token drops"]
#[derive(Default)]
pub struct SessionWatch {
    live: Option<Arc<AtomicBool>>,
    stop: Option<Box<dyn FnOnce() + Send>>,
}

impl SessionWatch {
    /// A watch that runs `stop` when it drops.
    pub fn on_drop(stop: impl FnOnce() + Send + 'static) -> Self {
        Self {
            live: None,
            stop: Some(Box::new(stop)),
        }
    }

    fn with_live(mut self, live: Arc<AtomicBool>) -> Self {
        self.live = Some(live);
        self
    }
}

impl Drop for SessionWatch {
    fn drop(&mut self) {
        if let Some(live) = &self.live {
            live.store(false, Ordering::Release);
        }
        if let Some(stop) = self.stop.take() {
            stop();
        }
    }
}

impl fmt::Debug for SessionWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionWatch").finish_non_exhaustive()
    }
}

/// The R6 drops that the OS events and the sign-out call.
pub trait TokenDrops: Send + Sync {
    /// Drop every token of every profile.
    fn drop_all(&self);
    /// Drop the token of `profile` only.
    fn drop_profile(&self, profile: &str);
}

impl TokenDrops for CliTokenBroker {
    fn drop_all(&self) {
        CliTokenBroker::drop_all(self);
    }
    fn drop_profile(&self, profile: &str) {
        CliTokenBroker::drop_profile(self, profile);
    }
}

/// The R6 token state per profile, for `profile_status`. Never a token.
pub trait TokenStates: Send + Sync {
    /// The state of the token of `profile`.
    fn token_state(&self, profile: &str) -> TokenState;
}

impl TokenStates for CliTokenBroker {
    fn token_state(&self, profile: &str) -> TokenState {
        CliTokenBroker::token_state(self, profile)
    }
}

/// W-04b: each sleep or lock event from `source` drops every token of `drops`. The sink holds
/// `drops` weakly, so the watch does not keep R6 alive. Keep the watch for the app lifetime.
pub fn watch_token_drops<D>(
    source: &dyn SessionEventSource,
    drops: Arc<D>,
) -> CoreResult<SessionWatch>
where
    D: TokenDrops + ?Sized + 'static,
{
    let live = Arc::new(AtomicBool::new(true));
    let alive = Arc::clone(&live);
    let weak = Arc::downgrade(&drops);
    drop(drops);
    let sink: SessionSink = Arc::new(move |_event| {
        if alive.load(Ordering::Acquire) {
            if let Some(d) = weak.upgrade() {
                d.drop_all();
            }
        }
    });
    Ok(source.watch(sink)?.with_live(live))
}

/// W-18c: the user signed out of `profile`, so R6 drops its token now. Call this only from
/// the user's sign-out action (#163); Bottle never signs out by itself.
pub fn sign_out(drops: &dyn TokenDrops, profile: &str) {
    drops.drop_profile(profile);
}

/// The `profile_status` component on the R12 gateway seam. Its reply is the token state only.
pub struct ProfileStatusComponent {
    states: Arc<dyn TokenStates>,
}

impl ProfileStatusComponent {
    pub fn new(states: Arc<dyn TokenStates>) -> Self {
        Self { states }
    }
}

impl fmt::Debug for ProfileStatusComponent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProfileStatusComponent")
            .finish_non_exhaustive()
    }
}

fn refused(code: &'static str) -> CoreError {
    CoreError::IpcRefused { code }
}

impl GatewayComponents for ProfileStatusComponent {
    fn call(&self, command: &'static str, args: Value) -> CoreResult<Value> {
        if command != "profile_status" {
            return Err(refused("not_implemented"));
        }
        let a: ProfileArgs = serde_json::from_value(args).map_err(|_| refused("invalid_args"))?;
        let state = self.states.token_state(&a.profile);
        let status = TokenStatus {
            profile: a.profile,
            state,
            expires_at: None,
        };
        serde_json::to_value(status).map_err(|_| refused("host_failed"))
    }
}

// ---------- the OS event names and codes ----------

/// The AppKit name of the sleep notification (`NSWorkspaceWillSleepNotification`).
pub const MAC_WILL_SLEEP: &str = "NSWorkspaceWillSleepNotification";
/// The distributed notification that macOS posts when the screen locks.
pub const MAC_SCREEN_LOCKED: &str = "com.apple.screenIsLocked";
/// `WTS_SESSION_LOCK`, the `wParam` of `WM_WTSSESSION_CHANGE` when the session locks.
pub const WTS_SESSION_LOCK: u32 = 0x7;
/// `PBT_APMSUSPEND`, the power event before the computer sleeps.
pub const PBT_APMSUSPEND: u32 = 0x4;
/// The class name of the Windows message-only window that gets the lock event.
pub const WIN_SESSION_CLASS: &str = "BottleTokenSessionEvents";

/// The event of a macOS notification name, if it is one R6 drops on.
pub fn mac_event(name: &str) -> Option<SessionEvent> {
    match name {
        MAC_WILL_SLEEP => Some(SessionEvent::Sleep),
        MAC_SCREEN_LOCKED => Some(SessionEvent::ScreenLock),
        _ => None,
    }
}

/// The event of a `WM_WTSSESSION_CHANGE` `wParam`, if it is one R6 drops on.
pub fn win_session_event(code: u32) -> Option<SessionEvent> {
    (code == WTS_SESSION_LOCK).then_some(SessionEvent::ScreenLock)
}

/// The event of a power broadcast type, if it is one R6 drops on.
pub fn win_power_event(kind: u32) -> Option<SessionEvent> {
    (kind == PBT_APMSUSPEND).then_some(SessionEvent::Sleep)
}

/// The OS sleep and lock source of this OS. On an OS other than macOS and Windows, `watch`
/// returns `session_events_unsupported` (Bottle ships on macOS and Windows only).
pub fn os_session_source() -> Box<dyn SessionEventSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacSessionEvents)
    }
    #[cfg(windows)]
    {
        Box::new(windows::WinSessionEvents)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        Box::new(NoSessionEvents)
    }
}

/// No OS source on this OS.
#[cfg(not(any(target_os = "macos", windows)))]
struct NoSessionEvents;

#[cfg(not(any(target_os = "macos", windows)))]
impl SessionEventSource for NoSessionEvents {
    fn watch(&self, _sink: SessionSink) -> CoreResult<SessionWatch> {
        Err(unavailable("session_events_unsupported"))
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn unavailable(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}
