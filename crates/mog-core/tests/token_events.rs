//! R6 token drops on OS sleep, screen lock and sign-out, and `profile_status` (#465, ADR 0076).
//!
//! Doubles only: a fake CliRunner, a fake OS event source and simulated clocks. No live
//! workspace. Every token is a plainly fake value (`test-token-not-real-<n>`).

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mog_core::cli_runner::{refuse_token_command, CliRunner, SecretStdout};
use mog_core::clock::{SimClock, SimWallClock};
use mog_core::error::{CoreError, CoreResult};
use mog_core::http_transport::ProfileHosts;
use mog_core::ipc_gateway::{hardened_gateway_with, GatewayComponents, IpcGateway, NoLog};
use mog_core::token_broker::{CliTokenBroker, TokenBroker};
use mog_core::token_events::{
    self, mac_event, sign_out, watch_token_drops, win_power_event, win_session_event,
    ProfileStatusComponent, SessionEvent, SessionEventSource, SessionSink, SessionWatch,
    MAC_SCREEN_LOCKED, MAC_WILL_SLEEP, PBT_APMSUSPEND, WTS_SESSION_LOCK,
};
use mog_core::types::TokenState;
use serde_json::json;

const PROFILE: &str = "bottle-test";
const OTHER: &str = "bottle-other";
const HOST: &str = "h.cloud.databricks.com";
const OTHER_HOST: &str = "other.cloud.databricks.com";
/// 2026-11-17T12:00:00Z in Unix milliseconds.
const T0_MS: i64 = 1_794_916_800_000;

// ---------- doubles ----------

/// A fake R5: counts `auth token` runs per profile and answers a fake token that expires at
/// 13:00Z, or fails for a profile in `fail`.
#[derive(Default)]
struct CountingCli {
    runs: Mutex<Vec<String>>,
    fail: Mutex<Vec<String>>,
    n: AtomicUsize,
    /// Runs inside the next `auth token` run (for example an OS event during the run).
    during: Mutex<Option<Box<dyn FnOnce() + Send>>>,
}

impl CountingCli {
    fn runs_for(&self, profile: &str) -> usize {
        self.runs
            .lock()
            .unwrap()
            .iter()
            .filter(|p| *p == profile)
            .count()
    }
}

impl CliRunner for CountingCli {
    fn resolve(&self) -> CoreResult<PathBuf> {
        Ok(PathBuf::from("/usr/local/bin/databricks"))
    }
    fn run_json(&self, args: &[&str]) -> CoreResult<serde_json::Value> {
        refuse_token_command(args)?;
        Ok(json!({}))
    }
    fn run_auth_token(&self, profile: &str) -> CoreResult<SecretStdout> {
        self.runs.lock().unwrap().push(profile.to_owned());
        let during = self.during.lock().unwrap().take();
        if let Some(f) = during {
            f();
        }
        if self.fail.lock().unwrap().iter().any(|p| p == profile) {
            return Err(CoreError::Unavailable {
                component: "R5",
                reason: "cli_failed",
            });
        }
        let n = self.n.fetch_add(1, Ordering::SeqCst);
        Ok(SecretStdout::new(
            format!(
                r#"{{"access_token":"test-token-not-real-{n}","expiry":"2026-11-17T13:00:00Z"}}"#
            )
            .into_bytes(),
        ))
    }
}

struct Hosts;

impl ProfileHosts for Hosts {
    fn workspace_url(&self, profile: &str) -> CoreResult<String> {
        Ok(match profile {
            OTHER => format!("https://{OTHER_HOST}"),
            _ => format!("https://{HOST}"),
        })
    }
}

struct Rig {
    broker: Arc<CliTokenBroker>,
    cli: Arc<CountingCli>,
    clock: SimClock,
}

fn rig() -> Rig {
    let clock = SimClock::new();
    let cli = Arc::new(CountingCli::default());
    let broker = Arc::new(CliTokenBroker::new(
        cli.clone(),
        Arc::new(Hosts),
        Arc::new(clock.clone()),
        Arc::new(SimWallClock::at(T0_MS)),
    ));
    Rig { broker, cli, clock }
}

fn lend(b: &dyn TokenBroker, profile: &str, host: &str) -> String {
    let mut seen = String::new();
    b.with_token(profile, host, &mut |bytes| {
        seen = String::from_utf8(bytes.to_vec()).unwrap()
    })
    .unwrap();
    seen
}

/// A fake OS event source: it keeps every sink it gets for ever, and `fire` calls them all.
/// So a test proves that a dropped [`SessionWatch`] stops the drops by itself.
#[derive(Default)]
struct FakeSource {
    sinks: Mutex<Vec<SessionSink>>,
}

impl FakeSource {
    fn fire(&self, e: SessionEvent) {
        for s in self.sinks.lock().unwrap().iter() {
            s(e);
        }
    }
}

impl SessionEventSource for FakeSource {
    fn watch(&self, sink: SessionSink) -> CoreResult<SessionWatch> {
        self.sinks.lock().unwrap().push(sink);
        Ok(SessionWatch::default())
    }
}

// ---------- W-04b: sleep and lock drop every token ----------

fn event_makes_the_next_call_run_the_cli_again(event: SessionEvent) {
    let r = rig();
    let source = FakeSource::default();
    let _watch = watch_token_drops(&source, r.broker.clone()).unwrap();
    let first = lend(&*r.broker, PROFILE, HOST);
    lend(&*r.broker, OTHER, OTHER_HOST);
    assert_eq!(
        lend(&*r.broker, PROFILE, HOST),
        first,
        "cached before the event"
    );
    assert_eq!(r.cli.runs_for(PROFILE), 1);

    source.fire(event);

    assert_eq!(r.broker.token_state(PROFILE), TokenState::Unknown);
    assert_eq!(r.broker.token_state(OTHER), TokenState::Unknown);
    assert_ne!(lend(&*r.broker, PROFILE, HOST), first, "a new token");
    assert_eq!(r.cli.runs_for(PROFILE), 2, "the CLI ran again");
    lend(&*r.broker, OTHER, OTHER_HOST);
    assert_eq!(r.cli.runs_for(OTHER), 2, "every profile was dropped");
}

#[test]
fn w_04b_sleep_event_makes_the_next_call_run_the_cli_again() {
    event_makes_the_next_call_run_the_cli_again(SessionEvent::Sleep);
}

#[test]
fn w_04b_lock_event_makes_the_next_call_run_the_cli_again() {
    event_makes_the_next_call_run_the_cli_again(SessionEvent::ScreenLock);
}

#[test]
fn w_04b_no_drop_after_the_watch_is_dropped() {
    let r = rig();
    let source = FakeSource::default();
    let watch = watch_token_drops(&source, r.broker.clone()).unwrap();
    lend(&*r.broker, PROFILE, HOST);
    drop(watch);
    source.fire(SessionEvent::Sleep);
    assert_eq!(r.broker.token_state(PROFILE), TokenState::Valid);
    lend(&*r.broker, PROFILE, HOST);
    assert_eq!(r.cli.runs_for(PROFILE), 1, "no CLI run: the token stayed");
}

#[test]
fn w_04b_the_watch_does_not_keep_the_broker_alive() {
    let r = rig();
    let source = FakeSource::default();
    let _watch = watch_token_drops(&source, r.broker.clone()).unwrap();
    let weak = Arc::downgrade(&r.broker);
    drop(r.broker);
    assert!(weak.upgrade().is_none(), "the sink holds R6 weakly");
    // An event after R6 is gone does nothing and does not panic.
    source.fire(SessionEvent::ScreenLock);
}

#[test]
fn w_04b_a_sleep_during_a_cli_run_is_not_undone_by_that_run() {
    let r = rig();
    let source = Arc::new(FakeSource::default());
    let _watch = watch_token_drops(&*source, r.broker.clone()).unwrap();
    let s = source.clone();
    *r.cli.during.lock().unwrap() = Some(Box::new(move || s.fire(SessionEvent::Sleep)));
    let first = lend(&*r.broker, PROFILE, HOST);
    assert_eq!(
        r.broker.token_state(PROFILE),
        TokenState::Unknown,
        "the token from a run that the sleep overlapped is not kept"
    );
    assert_ne!(lend(&*r.broker, PROFILE, HOST), first);
    assert_eq!(r.cli.runs_for(PROFILE), 2);
}

// ---------- W-18c: sign-out drops the profile's token ----------

#[test]
fn w_18c_sign_out_drops_only_that_profile_and_the_next_call_runs_the_cli() {
    let r = rig();
    let first = lend(&*r.broker, PROFILE, HOST);
    lend(&*r.broker, OTHER, OTHER_HOST);

    sign_out(&*r.broker, PROFILE);

    assert_eq!(r.broker.token_state(PROFILE), TokenState::Unknown);
    assert_eq!(
        r.broker.token_state(OTHER),
        TokenState::Valid,
        "other profile kept"
    );
    assert_ne!(lend(&*r.broker, PROFILE, HOST), first);
    assert_eq!(r.cli.runs_for(PROFILE), 2);
    lend(&*r.broker, OTHER, OTHER_HOST);
    assert_eq!(r.cli.runs_for(OTHER), 1, "no CLI run for the other profile");
}

#[test]
fn w_18c_drop_profile_of_an_unknown_profile_does_nothing() {
    let r = rig();
    lend(&*r.broker, PROFILE, HOST);
    r.broker.drop_profile("bottle-never-seen");
    assert_eq!(r.broker.token_state(PROFILE), TokenState::Valid);
}

// ---------- W-04b: profile_status returns the state only ----------

fn status(r: &Rig, profile: &str) -> serde_json::Value {
    let gw = hardened_gateway_with(NoLog, ProfileStatusComponent::new(r.broker.clone()));
    gw.dispatch("hub", "profile_status", json!({ "profile": profile }))
        .unwrap()
}

#[test]
fn w_04b_profile_status_returns_each_state_and_no_token() {
    let r = rig();
    assert_eq!(
        status(&r, PROFILE),
        json!({"profile": PROFILE, "state": "unknown"})
    );

    let token = lend(&*r.broker, PROFILE, HOST);
    let valid = status(&r, PROFILE);
    assert_eq!(valid, json!({"profile": PROFILE, "state": "valid"}));
    assert!(!valid.to_string().contains(&token), "never a token");

    // 13:00Z expiry, refresh at 12:59Z: 59 min 30 s after 12:00Z is in the margin.
    r.clock.advance(Duration::from_secs(59 * 60 + 30));
    let expiring = status(&r, PROFILE);
    assert_eq!(expiring, json!({"profile": PROFILE, "state": "expiring"}));
    assert!(!expiring.to_string().contains("test-token-not-real"));

    r.cli.fail.lock().unwrap().push(OTHER.to_owned());
    assert!(r.broker.with_token(OTHER, OTHER_HOST, &mut |_| ()).is_err());
    assert_eq!(
        status(&r, OTHER),
        json!({"profile": OTHER, "state": "login_required"})
    );
}

#[test]
fn w_04b_profile_status_does_not_run_the_cli() {
    let r = rig();
    status(&r, PROFILE);
    assert_eq!(
        r.cli.runs_for(PROFILE),
        0,
        "a status read never fetches a token"
    );
}

#[test]
fn w_04b_profile_status_component_refuses_other_commands_and_bad_args() {
    let r = rig();
    let c = ProfileStatusComponent::new(r.broker.clone());
    let e = c.call("mcp_call", json!({})).unwrap_err();
    assert_eq!(e.code(), "not_implemented");
    let e = c.call("profile_status", json!({"nope": 1})).unwrap_err();
    assert_eq!(e.code(), "invalid_args");
}

#[test]
fn w_04b_profile_status_is_refused_for_a_mog_window() {
    let r = rig();
    let gw = hardened_gateway_with(NoLog, ProfileStatusComponent::new(r.broker.clone()));
    let e = gw
        .dispatch("mog-1", "profile_status", json!({ "profile": PROFILE }))
        .unwrap_err();
    assert_eq!(e.code(), "window_not_allowed");
}

// ---------- the OS event names and codes (every OS) ----------

#[test]
fn w_04b_macos_names_map_to_sleep_and_lock_only() {
    assert_eq!(MAC_WILL_SLEEP, "NSWorkspaceWillSleepNotification");
    assert_eq!(MAC_SCREEN_LOCKED, "com.apple.screenIsLocked");
    assert_eq!(mac_event(MAC_WILL_SLEEP), Some(SessionEvent::Sleep));
    assert_eq!(mac_event(MAC_SCREEN_LOCKED), Some(SessionEvent::ScreenLock));
    assert_eq!(mac_event("com.apple.screenIsUnlocked"), None);
    assert_eq!(mac_event("NSWorkspaceDidWakeNotification"), None);
}

#[test]
fn w_04b_windows_codes_map_to_sleep_and_lock_only() {
    assert_eq!(WTS_SESSION_LOCK, 0x7);
    assert_eq!(PBT_APMSUSPEND, 0x4);
    assert_eq!(
        win_session_event(WTS_SESSION_LOCK),
        Some(SessionEvent::ScreenLock)
    );
    assert_eq!(win_session_event(0x8), None, "WTS_SESSION_UNLOCK");
    assert_eq!(win_power_event(PBT_APMSUSPEND), Some(SessionEvent::Sleep));
    assert_eq!(win_power_event(0x7), None, "PBT_APMRESUMESUSPEND");
    assert_eq!(win_power_event(0x12), None, "PBT_APMRESUMEAUTOMATIC");
}

#[cfg(windows)]
#[test]
fn w_04b_windows_codes_match_windows_sys() {
    assert_eq!(
        WTS_SESSION_LOCK,
        windows_sys::Win32::UI::WindowsAndMessaging::WTS_SESSION_LOCK
    );
    assert_eq!(
        PBT_APMSUSPEND,
        windows_sys::Win32::UI::WindowsAndMessaging::PBT_APMSUSPEND
    );
}

// ---------- the real OS sources ----------

/// A sink that counts each event kind.
fn counting_sink() -> (SessionSink, Arc<Mutex<Vec<SessionEvent>>>) {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let sink: SessionSink = Arc::new(move |e| s.lock().unwrap().push(e));
    (sink, seen)
}

#[cfg(target_os = "macos")]
#[test]
fn w_04b_macos_source_gets_a_local_will_sleep_notification() {
    use objc2_app_kit::{NSWorkspace, NSWorkspaceWillSleepNotification};
    let (sink, seen) = counting_sink();
    let source = token_events::os_session_source();
    let watch = source.watch(sink).unwrap();
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    // SAFETY: a static AppKit name, posted only to this process's workspace center.
    let name = unsafe { NSWorkspaceWillSleepNotification };
    // SAFETY: no object is passed.
    unsafe { center.postNotificationName_object(name, None) };
    assert_eq!(*seen.lock().unwrap(), vec![SessionEvent::Sleep]);
    drop(watch);
    // SAFETY: as above.
    unsafe { center.postNotificationName_object(name, None) };
    assert_eq!(
        seen.lock().unwrap().len(),
        1,
        "no event after the watch ends"
    );
}

#[cfg(windows)]
#[test]
fn w_04b_windows_source_gets_a_session_lock_message() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        FindWindowExW, SendMessageW, HWND_MESSAGE, WM_WTSSESSION_CHANGE,
    };
    let (sink, seen) = counting_sink();
    let source = token_events::os_session_source();
    let watch = source.watch(sink).unwrap();
    let class: Vec<u16> = token_events::WIN_SESSION_CLASS
        .encode_utf16()
        .chain([0])
        .collect();
    // SAFETY: a NUL-terminated class name; HWND_MESSAGE finds message-only windows.
    let hwnd = unsafe {
        FindWindowExW(
            HWND_MESSAGE,
            std::ptr::null_mut(),
            class.as_ptr(),
            std::ptr::null(),
        )
    };
    assert!(!hwnd.is_null(), "the source made its message-only window");
    // SAFETY: a valid window of this process; SendMessageW waits for its thread.
    unsafe { SendMessageW(hwnd, WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK as usize, 0) };
    assert_eq!(*seen.lock().unwrap(), vec![SessionEvent::ScreenLock]);
    drop(watch);
    // SAFETY: as above; the window is gone, so FindWindowExW returns null.
    let gone = unsafe {
        FindWindowExW(
            HWND_MESSAGE,
            std::ptr::null_mut(),
            class.as_ptr(),
            std::ptr::null(),
        )
    };
    assert!(gone.is_null(), "the watch removed its window");
}

#[cfg(not(any(target_os = "macos", windows)))]
#[test]
fn w_04b_other_os_source_is_unavailable() {
    let (sink, _) = counting_sink();
    let e = token_events::os_session_source().watch(sink).unwrap_err();
    assert_eq!(e.code(), "session_events_unsupported");
}
