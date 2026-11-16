//! R1 HotkeyService on Windows (#7): CAP-40a, CAP-40b, CAP-41, CAP-42.
//!
//! The decision logic is here, in platform-neutral code that the tests run on every OS:
//! which key events the `WH_KEYBOARD_LL` hook swallows ([`HookFilter`]), when the hook is
//! installed again ([`ReinstallPolicy`], [`HookSupervisor`]), how the callback latency is
//! measured ([`LatencyRecorder`]) and when the MOG shows the "elevated window" tooltip
//! ([`ElevationGate`]). The Win32 glue is in [`win32`] (Windows only).
//!
//! No signal, error or log carries the identity of a key the user pressed (SEC rules): a
//! [`HookSignal`] says only "talk key", "Esc" or "another key".

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

use crate::clock::MonotonicClock;
use crate::error::{CoreError, CoreResult};

#[cfg(windows)]
pub mod win32;

const ID: &str = "R1";

/// The hook is installed again after this interval (CAP-40b).
pub const REINSTALL_INTERVAL: Duration = Duration::from_secs(10 * 60);

/// The callback budget of CAP-40a: p99 under 1 ms.
pub const CALLBACK_BUDGET: Duration = Duration::from_millis(1);

/// Windows virtual-key codes that the filter knows. The low-level hook reports the left and
/// right modifier keys with their own codes.
pub mod vk {
    /// Esc.
    pub const ESCAPE: u32 = 0x1B;
    /// Left Windows key.
    pub const LWIN: u32 = 0x5B;
    /// Right Windows key.
    pub const RWIN: u32 = 0x5C;
    /// Left Ctrl.
    pub const LCONTROL: u32 = 0xA2;
    /// Right Ctrl.
    pub const RCONTROL: u32 = 0xA3;
    /// Left Alt.
    pub const LMENU: u32 = 0xA4;
    /// Right Alt (AltGr on some layouts).
    pub const RMENU: u32 = 0xA5;
    /// An unassigned code. A tap of it between a key-down and its key-up stops Windows from
    /// opening the Start menu or the menu bar (the "menu mask" key).
    pub const MASK: u32 = 0xE8;
}

/// The Windows talk key (D66: Right-Alt, else Right-Ctrl, else Ctrl+Win).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TalkKey {
    /// Right-Alt, the D66 default.
    #[default]
    RightAlt,
    /// Right-Ctrl.
    RightCtrl,
    /// Ctrl (either side) and Win (either side) held together.
    CtrlWin,
}

/// One event from the low-level keyboard hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyEvent {
    /// The virtual-key code.
    pub vk: u32,
    /// `true` for key-down (and auto-repeat), `false` for key-up.
    pub down: bool,
    /// `true` when the event came from `SendInput` (flag `LLKHF_INJECTED`).
    pub injected: bool,
}

impl KeyEvent {
    /// A physical key-down.
    pub fn down(vk: u32) -> Self {
        Self {
            vk,
            down: true,
            injected: false,
        }
    }

    /// A physical key-up.
    pub fn up(vk: u32) -> Self {
        Self {
            vk,
            down: false,
            injected: false,
        }
    }
}

/// What the hook callback does with one event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Call the next hook: the focused app gets the event.
    Pass,
    /// Return 1: no app sees the event (CAP-41).
    Swallow,
    /// Swallow the event, then inject a tap of [`vk::MASK`] followed by this key-up, so a
    /// Win key that went down before the chord does not open the Start menu.
    SwallowAndReplay {
        /// The key whose key-up is injected after the mask tap.
        vk: u32,
    },
}

/// What the hook tells the rest of the core. No signal names a key other than the talk key
/// and Esc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookSignal {
    /// The talk key went down. `at` is the monotonic time.
    TalkDown {
        /// Monotonic time of the event.
        at: Duration,
    },
    /// The talk key went up (or a reinstall found it is no longer down).
    TalkUp {
        /// Monotonic time of the event.
        at: Duration,
    },
    /// Esc went down during capture.
    Escape,
    /// Another key went down while the talk key is held (for CAP-44c).
    OtherKeyDown,
    /// An elevated window has the foreground, so the hook cannot see the talk key (CAP-42).
    TalkKeyBlocked,
    /// The hook was installed again (CAP-40b).
    HookReinstalled,
}

/// The CAP-41 key filter. It runs inside the hook callback, so every call is O(1) with no
/// allocation and no lock.
#[derive(Debug, Clone)]
pub struct HookFilter {
    talk: TalkKey,
    hands_free: bool,
    talk_active: bool,
    pressed: [bool; 256],
    swallowed: [bool; 256],
    mask_on_up: [bool; 256],
}

impl HookFilter {
    /// A filter for `talk`, with no key down and hands-free off.
    pub fn new(talk: TalkKey) -> Self {
        Self {
            talk,
            hands_free: false,
            talk_active: false,
            pressed: [false; 256],
            swallowed: [false; 256],
            mask_on_up: [false; 256],
        }
    }

    /// Hands-free capture is on (B3): Esc is swallowed and stops it (CAP-14b).
    pub fn set_hands_free(&mut self, on: bool) {
        self.hands_free = on;
    }

    /// Whether the talk key is held now.
    pub fn talk_active(&self) -> bool {
        self.talk_active
    }

    /// Decide one event at monotonic time `at`.
    pub fn on_key(&mut self, _ev: KeyEvent, _at: Duration) -> (Verdict, Option<HookSignal>) {
        (Verdict::Pass, None)
    }

    /// After a reinstall or a resume, read the physical key state again. If the talk key is
    /// active but no talk key is down, the key-up was lost: end the PTT.
    pub fn reconcile(
        &mut self,
        _physically_down: impl Fn(u32) -> bool,
        _at: Duration,
    ) -> Option<HookSignal> {
        None
    }
}

/// When the hook must be installed again (CAP-40b).
#[derive(Debug, Clone)]
pub struct ReinstallPolicy {
    interval: Duration,
    last_install: Option<Duration>,
}

impl Default for ReinstallPolicy {
    fn default() -> Self {
        Self::new(REINSTALL_INTERVAL)
    }
}

impl ReinstallPolicy {
    /// A policy with `interval` between installs.
    pub fn new(interval: Duration) -> Self {
        Self {
            interval,
            last_install: None,
        }
    }

    /// Record an install at `at`.
    pub fn installed(&mut self, at: Duration) {
        self.last_install = Some(at);
    }

    /// Whether the hook is due at `now`.
    pub fn due(&self, _now: Duration) -> bool {
        false
    }
}

/// Installs and removes the OS hook. The Win32 hook is one implementation; tests use a fake.
pub trait HookInstaller {
    /// Install the hook. The error reason is a fixed string, never key content.
    fn install(&mut self) -> Result<(), &'static str>;
    /// Remove the hook if it is installed.
    fn uninstall(&mut self);
}

/// Drives a [`HookInstaller`] by the [`ReinstallPolicy`].
#[derive(Debug)]
pub struct HookSupervisor<I: HookInstaller> {
    installer: I,
    policy: ReinstallPolicy,
    installed: bool,
}

impl<I: HookInstaller> HookSupervisor<I> {
    /// A supervisor with the default 10 min policy.
    pub fn new(installer: I) -> Self {
        Self::with_policy(installer, ReinstallPolicy::default())
    }

    /// A supervisor with `policy`.
    pub fn with_policy(installer: I, policy: ReinstallPolicy) -> Self {
        Self {
            installer,
            policy,
            installed: false,
        }
    }

    /// The installer, for inspection.
    pub fn installer(&self) -> &I {
        &self.installer
    }

    /// Whether the hook is installed now.
    pub fn is_installed(&self) -> bool {
        self.installed
    }

    /// Install the hook for the first time.
    pub fn start(&mut self, _now: Duration) -> CoreResult<()> {
        Ok(())
    }

    /// The periodic timer fired. Returns `true` when the hook was installed again.
    pub fn on_timer(&mut self, _now: Duration) -> CoreResult<bool> {
        Ok(false)
    }

    /// The device resumed from sleep: install the hook again now.
    pub fn on_resume(&mut self, _now: Duration) -> CoreResult<()> {
        Ok(())
    }

    /// Remove the hook.
    pub fn stop(&mut self) {}
}

/// A lock-free ring of hook-callback durations, for the CAP-40a p99 (AT-30).
#[derive(Debug)]
pub struct LatencyRecorder {
    samples: Box<[AtomicU32]>,
    next: AtomicUsize,
}

impl LatencyRecorder {
    /// A ring that keeps the last `capacity` samples (at least 1).
    pub fn with_capacity(capacity: usize) -> Self {
        let samples = (0..capacity.max(1)).map(|_| AtomicU32::new(0)).collect();
        Self {
            samples,
            next: AtomicUsize::new(0),
        }
    }

    /// Record one callback duration.
    pub fn record(&self, _d: Duration) {}

    /// The number of samples recorded so far (not capped by the ring size).
    pub fn count(&self) -> usize {
        0
    }

    /// The p99 of the kept samples, or `None` before the first sample.
    pub fn p99(&self) -> Option<Duration> {
        None
    }
}

/// Whether a process runs elevated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Elevation {
    /// The token is elevated.
    Elevated,
    /// The token is not elevated.
    NotElevated,
    /// The probe failed (for example access denied to a protected process).
    Unknown,
}

/// Decides when the MOG shows the "elevated window blocks the talk key" tooltip (CAP-42).
#[derive(Debug, Clone)]
pub struct ElevationGate {
    self_elevated: bool,
    shown: HashSet<u32>,
}

impl ElevationGate {
    /// A gate for a Bottle process that is (or is not) elevated.
    pub fn new(self_elevated: bool) -> Self {
        Self {
            self_elevated,
            shown: HashSet::new(),
        }
    }

    /// A window of process `pid` took the foreground.
    pub fn on_foreground(&mut self, _pid: u32, _elevation: Elevation) -> Option<HookSignal> {
        None
    }
}

/// The work of one hook callback: filter, send the signal without blocking, record latency.
pub struct HookCore {
    filter: HookFilter,
    hands_free: Arc<AtomicBool>,
    recorder: Arc<LatencyRecorder>,
    clock: Arc<dyn MonotonicClock>,
    signals: Sender<HookSignal>,
}

impl HookCore {
    /// A core for `talk` that sends signals on `signals`.
    pub fn new(
        talk: TalkKey,
        hands_free: Arc<AtomicBool>,
        recorder: Arc<LatencyRecorder>,
        clock: Arc<dyn MonotonicClock>,
        signals: Sender<HookSignal>,
    ) -> Self {
        Self {
            filter: HookFilter::new(talk),
            hands_free,
            recorder,
            clock,
            signals,
        }
    }

    /// Handle one hook event and return the verdict. It never blocks: a closed channel drops
    /// the signal.
    pub fn handle(&mut self, _ev: KeyEvent) -> Verdict {
        Verdict::Pass
    }

    /// See [`HookFilter::reconcile`]; the signal goes on the channel.
    pub fn reconcile(&mut self, _physically_down: impl Fn(u32) -> bool) {}

    /// Send a signal that does not come from a key event (reinstall, elevated foreground).
    pub fn send(&self, signal: HookSignal) {
        let _ = self.signals.send(signal);
    }
}

fn unavailable(reason: &'static str) -> CoreError {
    CoreError::Unavailable {
        component: ID,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::SimClock;
    use std::sync::mpsc;

    const T0: Duration = Duration::from_secs(1);

    fn run(f: &mut HookFilter, events: &[KeyEvent]) -> Vec<(Verdict, Option<HookSignal>)> {
        events.iter().map(|e| f.on_key(*e, T0)).collect()
    }

    // ---- CAP-41: swallow the talk key and Esc during capture ----

    #[test]
    fn cap_41_swallows_right_alt_down_repeat_and_up() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        let out = run(
            &mut f,
            &[
                KeyEvent::down(vk::RMENU),
                KeyEvent::down(vk::RMENU),
                KeyEvent::up(vk::RMENU),
            ],
        );
        assert_eq!(out[0], (Verdict::Swallow, Some(HookSignal::TalkDown { at: T0 })));
        assert_eq!(out[1], (Verdict::Swallow, None), "auto-repeat is swallowed, no new signal");
        assert_eq!(out[2], (Verdict::Swallow, Some(HookSignal::TalkUp { at: T0 })));
        assert!(!f.talk_active());
    }

    #[test]
    fn cap_41_no_menu_bar_in_100_presses() {
        // AT-30: an Alt press that reaches an app activates the Win32 menu bar. If no Right-Alt
        // event passes in 100 presses, no menu bar can activate.
        let mut f = HookFilter::new(TalkKey::RightAlt);
        for i in 0..100 {
            let down = f.on_key(KeyEvent::down(vk::RMENU), T0).0;
            let up = f.on_key(KeyEvent::up(vk::RMENU), T0).0;
            assert_eq!((down, up), (Verdict::Swallow, Verdict::Swallow), "press {i}");
        }
    }

    #[test]
    fn cap_41_swallows_right_ctrl_when_it_is_the_talk_key() {
        let mut f = HookFilter::new(TalkKey::RightCtrl);
        assert_eq!(f.on_key(KeyEvent::down(vk::RCONTROL), T0).0, Verdict::Swallow);
        assert_eq!(f.on_key(KeyEvent::up(vk::RCONTROL), T0).0, Verdict::Swallow);
        // Right-Alt is a normal key now.
        assert_eq!(f.on_key(KeyEvent::down(vk::RMENU), T0), (Verdict::Pass, None));
    }

    #[test]
    fn cap_41_passes_esc_when_not_capturing() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        assert_eq!(f.on_key(KeyEvent::down(vk::ESCAPE), T0), (Verdict::Pass, None));
        assert_eq!(f.on_key(KeyEvent::up(vk::ESCAPE), T0), (Verdict::Pass, None));
    }

    #[test]
    fn cap_41_swallows_esc_while_the_talk_key_is_held() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::ESCAPE), T0),
            (Verdict::Swallow, Some(HookSignal::Escape))
        );
        assert_eq!(f.on_key(KeyEvent::up(vk::ESCAPE), T0), (Verdict::Swallow, None));
    }

    #[test]
    fn cap_41_swallows_esc_during_hands_free() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.set_hands_free(true);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::ESCAPE), T0),
            (Verdict::Swallow, Some(HookSignal::Escape))
        );
        // Hands-free ends on that Esc; the key-up still matches the swallowed key-down.
        f.set_hands_free(false);
        assert_eq!(f.on_key(KeyEvent::up(vk::ESCAPE), T0), (Verdict::Swallow, None));
    }

    #[test]
    fn cap_41_never_swallows_a_key_up_whose_key_down_passed() {
        // Esc goes down before capture and up during capture: the app saw the key-down, so it
        // must see the key-up too, or the key sticks.
        let mut f = HookFilter::new(TalkKey::RightAlt);
        assert_eq!(f.on_key(KeyEvent::down(vk::ESCAPE), T0).0, Verdict::Pass);
        f.set_hands_free(true);
        assert_eq!(f.on_key(KeyEvent::up(vk::ESCAPE), T0).0, Verdict::Pass);
    }

    #[test]
    fn cap_41_passes_injected_events() {
        // Bottle's own paste (SendInput) and the mask replay must reach the app.
        let mut f = HookFilter::new(TalkKey::RightAlt);
        let injected = KeyEvent {
            vk: vk::RMENU,
            down: true,
            injected: true,
        };
        assert_eq!(f.on_key(injected, T0), (Verdict::Pass, None));
        assert!(!f.talk_active());
    }

    #[test]
    fn cap_41_other_keys_pass_and_report_no_identity() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        assert_eq!(f.on_key(KeyEvent::down(0x41), T0), (Verdict::Pass, None));
        f.on_key(KeyEvent::up(0x41), T0);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        assert_eq!(
            f.on_key(KeyEvent::down(0x41), T0),
            (Verdict::Pass, Some(HookSignal::OtherKeyDown))
        );
        assert_eq!(f.on_key(KeyEvent::up(0x41), T0), (Verdict::Pass, None));
    }

    #[test]
    fn cap_41_ignores_codes_outside_the_vk_range() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        assert_eq!(f.on_key(KeyEvent::down(0x1_0000), T0), (Verdict::Pass, None));
    }

    #[test]
    fn cap_41_ctrl_win_chord_ctrl_first() {
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        assert_eq!(f.on_key(KeyEvent::down(vk::LCONTROL), T0), (Verdict::Pass, None));
        assert_eq!(
            f.on_key(KeyEvent::down(vk::LWIN), T0),
            (Verdict::Swallow, Some(HookSignal::TalkDown { at: T0 }))
        );
        // Win up ends the chord; its key-down was swallowed, so the Start menu never sees it.
        assert_eq!(
            f.on_key(KeyEvent::up(vk::LWIN), T0),
            (Verdict::Swallow, Some(HookSignal::TalkUp { at: T0 }))
        );
        // The Ctrl key-down reached the app, so its key-up does too.
        assert_eq!(f.on_key(KeyEvent::up(vk::LCONTROL), T0), (Verdict::Pass, None));
    }

    #[test]
    fn cap_41_ctrl_win_chord_win_first_masks_the_start_menu() {
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        assert_eq!(f.on_key(KeyEvent::down(vk::RWIN), T0), (Verdict::Pass, None));
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RCONTROL), T0),
            (Verdict::Swallow, Some(HookSignal::TalkDown { at: T0 }))
        );
        assert_eq!(
            f.on_key(KeyEvent::up(vk::RCONTROL), T0),
            (Verdict::Swallow, Some(HookSignal::TalkUp { at: T0 }))
        );
        // Windows saw Win down and nothing else: a plain Win up would open Start.
        assert_eq!(
            f.on_key(KeyEvent::up(vk::RWIN), T0),
            (Verdict::SwallowAndReplay { vk: vk::RWIN }, None)
        );
    }

    #[test]
    fn cap_41_plain_win_press_is_not_masked() {
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        assert_eq!(f.on_key(KeyEvent::down(vk::LWIN), T0).0, Verdict::Pass);
        assert_eq!(f.on_key(KeyEvent::up(vk::LWIN), T0).0, Verdict::Pass);
    }

    // ---- CAP-40b: install again every 10 min and after resume ----

    #[test]
    fn cap_40b_policy_is_due_every_10_minutes() {
        let mut p = ReinstallPolicy::default();
        assert!(p.due(Duration::ZERO), "never installed is due");
        p.installed(T0);
        assert!(!p.due(T0 + Duration::from_secs(599)));
        assert!(p.due(T0 + Duration::from_secs(600)));
        assert_eq!(REINSTALL_INTERVAL, Duration::from_secs(600));
    }

    #[derive(Debug, Default)]
    struct FakeInstaller {
        installs: u32,
        uninstalls: u32,
        fail_next: bool,
        hooked: bool,
    }

    impl HookInstaller for FakeInstaller {
        fn install(&mut self) -> Result<(), &'static str> {
            if std::mem::take(&mut self.fail_next) {
                return Err("set_windows_hook_failed");
            }
            assert!(!self.hooked, "install while a hook is still installed");
            self.installs += 1;
            self.hooked = true;
            Ok(())
        }
        fn uninstall(&mut self) {
            if self.hooked {
                self.uninstalls += 1;
                self.hooked = false;
            }
        }
    }

    #[test]
    fn cap_40b_supervisor_installs_again_after_10_minutes() {
        let clock = SimClock::new();
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(clock.now()).unwrap();
        assert!(s.is_installed());
        clock.advance(Duration::from_secs(300));
        assert!(!s.on_timer(clock.now()).unwrap(), "not due at 5 min");
        clock.advance(Duration::from_secs(300));
        assert!(s.on_timer(clock.now()).unwrap(), "due at 10 min");
        assert_eq!((s.installer().installs, s.installer().uninstalls), (2, 1));
        assert!(s.is_installed());
    }

    #[test]
    fn cap_40b_supervisor_installs_again_on_resume() {
        let clock = SimClock::new();
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(clock.now()).unwrap();
        clock.advance(Duration::from_secs(5));
        s.on_resume(clock.now()).unwrap();
        assert_eq!(s.installer().installs, 2);
        // The resume restarts the 10 min interval.
        clock.advance(Duration::from_secs(599));
        assert!(!s.on_timer(clock.now()).unwrap());
    }

    #[test]
    fn cap_40b_8h_soak_reinstalls_48_times() {
        let clock = SimClock::new();
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(clock.now()).unwrap();
        let mut reinstalls = 0;
        for _ in 0..(8 * 60) {
            clock.advance(Duration::from_secs(60));
            if s.on_timer(clock.now()).unwrap() {
                reinstalls += 1;
            }
        }
        assert_eq!(reinstalls, 48);
        assert!(s.is_installed());
    }

    #[test]
    fn cap_40b_failed_install_reports_unavailable_and_retries_on_the_next_timer() {
        let clock = SimClock::new();
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(clock.now()).unwrap();
        clock.advance(REINSTALL_INTERVAL);
        s.installer.fail_next = true;
        let err = s.on_timer(clock.now()).unwrap_err();
        assert_eq!(err.code(), "unavailable");
        assert!(!s.is_installed());
        clock.advance(Duration::from_secs(1));
        assert!(s.on_timer(clock.now()).unwrap(), "retry at the next tick");
        assert!(s.is_installed());
    }

    #[test]
    fn cap_40b_start_failure_is_an_error() {
        let mut s = HookSupervisor::new(FakeInstaller {
            fail_next: true,
            ..FakeInstaller::default()
        });
        assert_eq!(s.start(T0).unwrap_err().code(), "unavailable");
        assert!(!s.is_installed());
    }

    #[test]
    fn cap_40b_stop_removes_the_hook_and_timers_do_nothing() {
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(T0).unwrap();
        s.stop();
        assert!(!s.is_installed());
        assert!(!s.on_timer(T0 + REINSTALL_INTERVAL).unwrap());
        assert_eq!(s.installer().installs, 1);
    }

    #[test]
    fn cap_40b_reconcile_ends_a_ptt_whose_key_up_was_lost() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        let later = T0 + Duration::from_secs(2);
        assert_eq!(f.reconcile(|_| false, later), Some(HookSignal::TalkUp { at: later }));
        assert!(!f.talk_active());
        // The next Right-Alt press starts a new PTT.
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RMENU), later).1,
            Some(HookSignal::TalkDown { at: later })
        );
    }

    #[test]
    fn cap_40b_reconcile_keeps_a_ptt_whose_key_is_still_down() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        assert_eq!(f.reconcile(|k| k == vk::RMENU, T0), None);
        assert!(f.talk_active());
    }

    // ---- CAP-40a: the callback returns in under 1 ms ----

    #[test]
    fn cap_40a_latency_recorder_p99() {
        let r = LatencyRecorder::with_capacity(1000);
        assert_eq!(r.p99(), None);
        for i in 1..=100u64 {
            r.record(Duration::from_micros(i));
        }
        assert_eq!(r.count(), 100);
        assert_eq!(r.p99(), Some(Duration::from_micros(99)));
    }

    #[test]
    fn cap_40a_latency_recorder_keeps_the_last_samples() {
        let r = LatencyRecorder::with_capacity(10);
        for _ in 0..10 {
            r.record(Duration::from_millis(5));
        }
        for _ in 0..10 {
            r.record(Duration::from_micros(1));
        }
        assert_eq!(r.count(), 20);
        assert_eq!(r.p99(), Some(Duration::from_micros(1)));
    }

    fn core(talk: TalkKey) -> (HookCore, mpsc::Receiver<HookSignal>, Arc<LatencyRecorder>) {
        let (tx, rx) = mpsc::channel();
        let rec = Arc::new(LatencyRecorder::with_capacity(20_000));
        let c = HookCore::new(
            talk,
            Arc::new(AtomicBool::new(false)),
            rec.clone(),
            Arc::new(SimClock::starting_at(T0)),
            tx,
        );
        (c, rx, rec)
    }

    #[test]
    fn cap_40a_core_sends_signals_and_records_every_callback() {
        let (mut c, rx, rec) = core(TalkKey::RightAlt);
        assert_eq!(c.handle(KeyEvent::down(vk::RMENU)), Verdict::Swallow);
        assert_eq!(c.handle(KeyEvent::up(vk::RMENU)), Verdict::Swallow);
        assert_eq!(c.handle(KeyEvent::down(0x41)), Verdict::Pass);
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkDown { at: T0 }));
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkUp { at: T0 }));
        assert!(rx.try_recv().is_err());
        assert_eq!(rec.count(), 3);
    }

    #[test]
    fn cap_40a_core_never_blocks_when_the_receiver_is_gone() {
        let (mut c, rx, _) = core(TalkKey::RightAlt);
        drop(rx);
        assert_eq!(c.handle(KeyEvent::down(vk::RMENU)), Verdict::Swallow);
    }

    #[test]
    fn cap_40a_core_reads_hands_free_on_each_event() {
        let (tx, rx) = mpsc::channel();
        let hands_free = Arc::new(AtomicBool::new(false));
        let mut c = HookCore::new(
            TalkKey::RightAlt,
            hands_free.clone(),
            Arc::new(LatencyRecorder::with_capacity(8)),
            Arc::new(SimClock::new()),
            tx,
        );
        hands_free.store(true, Ordering::SeqCst);
        assert_eq!(c.handle(KeyEvent::down(vk::ESCAPE)), Verdict::Swallow);
        assert_eq!(rx.try_recv(), Ok(HookSignal::Escape));
    }

    #[test]
    fn cap_40a_core_reconcile_sends_the_lost_key_up() {
        let (mut c, rx, _) = core(TalkKey::RightAlt);
        c.handle(KeyEvent::down(vk::RMENU));
        c.reconcile(|_| false);
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkDown { at: T0 }));
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkUp { at: T0 }));
    }

    #[test]
    fn cap_40a_callback_work_p99_is_under_1_ms() {
        // The platform-neutral part of the callback. The real hook p99 on a Windows device is
        // AT-30 (needs:windows, TODO(#5)).
        let (mut c, _rx, rec) = core(TalkKey::RightAlt);
        for i in 0..10_000u32 {
            let k = if i % 3 == 0 { vk::RMENU } else { 0x41 + (i % 20) };
            c.handle(KeyEvent::down(k));
            c.handle(KeyEvent::up(k));
        }
        assert_eq!(rec.count(), 20_000);
        let p99 = rec.p99().unwrap();
        assert!(p99 < CALLBACK_BUDGET, "p99 {p99:?}");
    }

    // ---- CAP-42: tooltip when an elevated window blocks the talk key ----

    #[test]
    fn cap_42_first_elevated_foreground_shows_the_tooltip_once() {
        let mut g = ElevationGate::new(false);
        assert_eq!(g.on_foreground(10, Elevation::NotElevated), None);
        assert_eq!(
            g.on_foreground(42, Elevation::Elevated),
            Some(HookSignal::TalkKeyBlocked)
        );
        assert_eq!(g.on_foreground(10, Elevation::NotElevated), None);
        assert_eq!(g.on_foreground(42, Elevation::Elevated), None, "once per process");
        assert_eq!(
            g.on_foreground(43, Elevation::Elevated),
            Some(HookSignal::TalkKeyBlocked)
        );
    }

    #[test]
    fn cap_42_no_tooltip_when_bottle_is_elevated() {
        let mut g = ElevationGate::new(true);
        assert_eq!(g.on_foreground(42, Elevation::Elevated), None);
    }

    #[test]
    fn cap_42_no_tooltip_when_elevation_is_unknown() {
        let mut g = ElevationGate::new(false);
        assert_eq!(g.on_foreground(4, Elevation::Unknown), None);
    }
}
