//! R1 HotkeyService on Windows (#7, #313): CAP-40a, CAP-40b, CAP-41, CAP-42, CAP-45.
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
    /// Right Shift.
    pub const RSHIFT: u32 = 0xA1;
    /// Left Alt.
    pub const LMENU: u32 = 0xA4;
    /// Right Alt (AltGr on some layouts).
    pub const RMENU: u32 = 0xA5;
    /// An unassigned code. A tap of it between a key-down and its key-up stops Windows from
    /// opening the Start menu or the menu bar (the "menu mask" key).
    pub const MASK: u32 = 0xE8;
}

/// The Windows talk key (D66: Right-Alt, else Right-Ctrl, else Ctrl+Win).
///
/// D66 gives Windows only these three keys (#313, ADR 0010). Right-Win and Right-Shift are
/// not talk keys on Windows: a swallowed Right-Shift blocks the capitals typed with it, and a
/// swallowed Right-Win breaks the Win shortcuts.
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

impl TalkKey {
    /// Every Windows talk key, in the order of the D66 ladder.
    pub const ALL: [TalkKey; 3] = [TalkKey::RightAlt, TalkKey::RightCtrl, TalkKey::CtrlWin];

    /// The one virtual key of a single-key talk key; `None` for the Ctrl+Win chord.
    pub fn single_vk(self) -> Option<u32> {
        match self {
            TalkKey::RightAlt => Some(vk::RMENU),
            TalkKey::RightCtrl => Some(vk::RCONTROL),
            TalkKey::CtrlWin => None,
        }
    }
}

impl From<crate::hotkey_core::TalkKey> for TalkKey {
    /// The Windows key for the shared talk key setting of #6. A shared key that D66 does not
    /// give Windows (Right-Command, Right-Shift) falls back to the D66 default, Right-Alt.
    /// The conflict check of `hotkey_bindings` flags such a setting as invalid (CAP-43b).
    fn from(key: crate::hotkey_core::TalkKey) -> Self {
        use crate::hotkey_core::TalkKey as Core;
        match key {
            Core::RightControl => TalkKey::RightCtrl,
            Core::RightOption | Core::RightCommand | Core::RightShift => TalkKey::RightAlt,
        }
    }
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
    /// TODO(#294): the core-to-brain `hotkey.blocked` event that makes the MOG show the tooltip.
    TalkKeyBlocked,
    /// The hook was installed again (CAP-40b).
    HookReinstalled,
    /// A hook install or a Win32 registration failed. `reason` is a fixed string.
    HookFailed {
        /// A fixed, content-free reason, for example `"set_windows_hook_failed"`.
        reason: &'static str,
    },
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

    /// Whether `vk` is one key of the talk key.
    fn is_talk_part(&self, vk: u32) -> bool {
        match self.talk {
            TalkKey::CtrlWin => is_ctrl(vk) || is_win(vk),
            single => single.single_vk() == Some(vk),
        }
    }

    /// Whether the talk key is complete with the keys held now.
    fn talk_complete(&self) -> bool {
        match self.talk {
            TalkKey::RightAlt | TalkKey::RightCtrl => self
                .talk
                .single_vk()
                .is_some_and(|k| self.pressed[k as usize]),
            TalkKey::CtrlWin => {
                let held = |a: u32, b: u32| self.pressed[a as usize] || self.pressed[b as usize];
                held(vk::LCONTROL, vk::RCONTROL) && held(vk::LWIN, vk::RWIN)
            }
        }
    }

    /// Decide one event at monotonic time `at`.
    ///
    /// Rules: injected events pass and change nothing. The key-down that completes the talk
    /// key, and every later talk-key event, is swallowed. Esc is swallowed while the talk key
    /// is held or hands-free is on. A key-up is swallowed only when its key-down was, so no
    /// key sticks in the focused app.
    pub fn on_key(&mut self, ev: KeyEvent, at: Duration) -> (Verdict, Option<HookSignal>) {
        if ev.injected || ev.vk >= 256 {
            return (Verdict::Pass, None);
        }
        let i = ev.vk as usize;
        if ev.down {
            self.on_down(ev.vk, i, at)
        } else {
            self.on_up(ev.vk, i, at)
        }
    }

    fn on_down(&mut self, vk: u32, i: usize, at: Duration) -> (Verdict, Option<HookSignal>) {
        let repeat = self.pressed[i];
        self.pressed[i] = true;
        if self.swallowed[i] {
            // Auto-repeat of a key whose key-down was swallowed.
            return (Verdict::Swallow, None);
        }
        if self.is_talk_part(vk) {
            if self.talk_active {
                // A repeat of a key whose key-down passed stays unmarked, so its key-up
                // passes too and the key does not stick.
                if !repeat {
                    self.swallowed[i] = true;
                }
                return (Verdict::Swallow, None);
            }
            if self.talk_complete() {
                self.talk_active = true;
                self.swallowed[i] = true;
                // A Win key that went down before the chord reached Windows: mask its key-up.
                for w in [vk::LWIN, vk::RWIN] {
                    let w = w as usize;
                    if self.pressed[w] && !self.swallowed[w] {
                        self.mask_on_up[w] = true;
                    }
                }
                return (Verdict::Swallow, Some(HookSignal::TalkDown { at }));
            }
            // The first key of the chord: Windows sees it.
            return (Verdict::Pass, None);
        }
        if vk == vk::ESCAPE && (self.talk_active || self.hands_free) {
            self.swallowed[i] = true;
            return (Verdict::Swallow, Some(HookSignal::Escape));
        }
        if self.talk_active && !repeat {
            return (Verdict::Pass, Some(HookSignal::OtherKeyDown));
        }
        (Verdict::Pass, None)
    }

    fn on_up(&mut self, vk: u32, i: usize, at: Duration) -> (Verdict, Option<HookSignal>) {
        self.pressed[i] = false;
        let was_swallowed = std::mem::take(&mut self.swallowed[i]);
        let mask = std::mem::take(&mut self.mask_on_up[i]);
        let signal = if self.talk_active && self.is_talk_part(vk) {
            self.talk_active = false;
            Some(HookSignal::TalkUp { at })
        } else {
            None
        };
        let verdict = if was_swallowed {
            Verdict::Swallow
        } else if mask {
            Verdict::SwallowAndReplay { vk }
        } else {
            Verdict::Pass
        };
        (verdict, signal)
    }

    /// After a reinstall, read the key state again for the keys whose key-down reached
    /// Windows. A key the hook swallowed is skipped: Windows does not update its async key
    /// state, so "up" proves nothing (a stuck PTT is bounded by the 120 s hold stop). If a
    /// passed key of the talk key (the first key of Ctrl+Win) is up, the PTT ends.
    pub fn reconcile(
        &mut self,
        physically_down: impl Fn(u32) -> bool,
        at: Duration,
    ) -> Option<HookSignal> {
        for k in 0..256u32 {
            let i = k as usize;
            if self.pressed[i] && !self.swallowed[i] && !physically_down(k) {
                self.pressed[i] = false;
                self.swallowed[i] = false;
                self.mask_on_up[i] = false;
            }
        }
        if self.talk_active && !self.talk_complete() {
            self.talk_active = false;
            return Some(HookSignal::TalkUp { at });
        }
        None
    }
}

impl HookFilter {
    /// The session was unlocked (CAP-45). While the session is locked, the secure desktop
    /// gets the keys, so the hook can miss a key-up. The swallowed keys have no real async
    /// state, so their state clears. The passed keys get their physical state again. An
    /// active PTT then ends, because the key that completed it was swallowed.
    pub fn on_unlock(
        &mut self,
        physically_down: impl Fn(u32) -> bool,
        at: Duration,
    ) -> Option<HookSignal> {
        for i in 0..256usize {
            if self.swallowed[i] {
                self.pressed[i] = false;
                self.swallowed[i] = false;
                self.mask_on_up[i] = false;
            }
        }
        self.reconcile(physically_down, at)
    }

    /// The device resumed from sleep. A hold cannot last through sleep, so an active PTT
    /// ends and all key state is cleared.
    pub fn on_resume(&mut self, at: Duration) -> Option<HookSignal> {
        self.pressed = [false; 256];
        self.swallowed = [false; 256];
        self.mask_on_up = [false; 256];
        if std::mem::take(&mut self.talk_active) {
            return Some(HookSignal::TalkUp { at });
        }
        None
    }
}

/// What the hook thread does with the return value of `GetMessageW`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStep {
    /// A message came: handle it.
    Dispatch,
    /// `WM_QUIT` came: stop the loop.
    Quit,
    /// `GetMessageW` failed (it returned -1): stop the loop and report it.
    Failed,
}

impl LoopStep {
    /// Classify the return value of `GetMessageW`.
    pub fn from_get_message(got: i32) -> Self {
        match got {
            0 => LoopStep::Quit,
            -1 => LoopStep::Failed,
            _ => LoopStep::Dispatch,
        }
    }

    /// The signal the hook thread sends when the loop stops this way.
    pub fn exit_signal(self) -> Option<HookSignal> {
        match self {
            LoopStep::Failed => Some(HookSignal::HookFailed {
                reason: "message_loop_failed",
            }),
            LoopStep::Quit | LoopStep::Dispatch => None,
        }
    }
}

/// The state of the hook thread, as `start()` and `stop()` see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookThread {
    /// No thread.
    Stopped,
    /// The thread runs its message loop.
    Running,
    /// The thread ended by itself (for example after a loop failure). It counts as stopped.
    Finished,
}

impl HookThread {
    /// The state of a thread slot: `present` when a thread was started and not joined,
    /// `finished` when that thread has ended.
    pub fn of(present: bool, finished: bool) -> Self {
        match (present, finished) {
            (false, _) => HookThread::Stopped,
            (true, false) => HookThread::Running,
            (true, true) => HookThread::Finished,
        }
    }

    /// Whether `start()` starts a new hook thread (after it joins a finished one).
    pub fn start_spawns(self) -> bool {
        self != HookThread::Running
    }

    /// Whether `stop()` posts `WM_QUIT`. A finished thread is only joined.
    pub fn stop_posts_quit(self) -> bool {
        self == HookThread::Running
    }
}

/// The process id of the foreground window at start, read once (CAP-42). `0` means no
/// foreground window, so there is nothing to probe.
pub fn foreground_at_start(pid: u32) -> Option<u32> {
    (pid != 0).then_some(pid)
}

/// The body of the elevation worker (CAP-42): probe each foreground process id and send the
/// signal of the gate. The first id is the foreground at start ([`foreground_at_start`]), so
/// an elevated window that has the foreground before the hook starts is found at once.
pub fn run_elevation_gate(
    gate: &mut ElevationGate,
    pids: impl IntoIterator<Item = u32>,
    mut probe: impl FnMut(u32) -> Elevation,
    signals: &Sender<HookSignal>,
) {
    for pid in pids {
        if let Some(signal) = gate.on_foreground(pid, probe(pid)) {
            let _ = signals.send(signal);
        }
    }
}

/// The signal the hook thread sends for the result of a timer tick or a resume reinstall.
pub fn reinstall_signal(result: &CoreResult<bool>) -> Option<HookSignal> {
    match result {
        Ok(true) => Some(HookSignal::HookReinstalled),
        Ok(false) => None,
        Err(CoreError::Unavailable { reason, .. }) => Some(HookSignal::HookFailed { reason }),
        Err(_) => Some(HookSignal::HookFailed {
            reason: "reinstall_failed",
        }),
    }
}

impl Elevation {
    /// Classify a probe. `opened`: the process opened for limited query. `token_elevated`:
    /// the token elevation, or `None` when the token query was refused.
    pub fn from_probe(opened: bool, token_elevated: Option<bool>) -> Self {
        match (opened, token_elevated) {
            // A refused token query on a process that opened: most likely elevated.
            (true, Some(true)) | (true, None) => Elevation::Elevated,
            (true, Some(false)) => Elevation::NotElevated,
            (false, _) => Elevation::Unknown,
        }
    }
}

/// Feed a talk-key signal to the shared PTT machine of #6 (`hotkey_core`), so Windows and
/// macOS give the same `hotkey.ptt` phases. Another key during a hold goes to the CAP-44c
/// cancel rule. Signals with no shared equivalent (Esc, the elevated window, hook health)
/// return to the host.
pub fn forward_to_engine(
    signal: HookSignal,
    engine: &mut crate::hotkey_core::HotkeyEngine,
) -> Option<HookSignal> {
    use crate::hotkey_core::KeyEdge;
    match signal {
        HookSignal::TalkDown { .. } => engine.on_talk_key(KeyEdge::Down),
        HookSignal::TalkUp { .. } => engine.on_talk_key(KeyEdge::Up),
        HookSignal::OtherKeyDown => engine.on_other_key_down(),
        other => return Some(other),
    }
    None
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
    pub fn due(&self, now: Duration) -> bool {
        match self.last_install {
            None => true,
            Some(last) => now.saturating_sub(last) >= self.interval,
        }
    }
}

/// Installs and removes the OS hook. The Win32 hook is one implementation; tests use a fake.
pub trait HookInstaller {
    /// The handle of one installed hook.
    type Handle;
    /// Install a hook. The error reason is a fixed string, never key content.
    fn install(&mut self) -> Result<Self::Handle, &'static str>;
    /// Remove the hook `handle`.
    fn uninstall(&mut self, handle: Self::Handle);
}

/// Drives a [`HookInstaller`] by the [`ReinstallPolicy`].
pub struct HookSupervisor<I: HookInstaller> {
    installer: I,
    policy: ReinstallPolicy,
    current: Option<I::Handle>,
    running: bool,
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
            current: None,
            running: false,
        }
    }

    /// The installer, for inspection.
    pub fn installer(&self) -> &I {
        &self.installer
    }

    /// Whether the hook is installed now.
    pub fn is_installed(&self) -> bool {
        self.current.is_some()
    }

    /// Install the hook for the first time.
    pub fn start(&mut self, now: Duration) -> CoreResult<()> {
        self.running = true;
        self.reinstall(now)
    }

    /// The periodic timer fired. Returns `true` when the hook was installed again.
    pub fn on_timer(&mut self, now: Duration) -> CoreResult<bool> {
        if !self.running || (self.is_installed() && !self.policy.due(now)) {
            return Ok(false);
        }
        self.reinstall(now).map(|()| true)
    }

    /// The device resumed from sleep: install the hook again now.
    pub fn on_resume(&mut self, now: Duration) -> CoreResult<()> {
        if !self.running {
            return Ok(());
        }
        self.reinstall(now)
    }

    /// Remove the hook. Later timers and resumes do nothing.
    pub fn stop(&mut self) {
        self.running = false;
        if let Some(h) = self.current.take() {
            self.installer.uninstall(h);
        }
    }

    /// Install a new hook, then remove the old one. If the install fails, the old hook stays.
    /// Windows calls a low-level hook on the installing thread only, so no callback runs
    /// between the two calls.
    fn reinstall(&mut self, now: Duration) -> CoreResult<()> {
        let new = self.installer.install().map_err(unavailable)?;
        if let Some(old) = self.current.replace(new) {
            self.installer.uninstall(old);
        }
        self.policy.installed(now);
        Ok(())
    }
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
    pub fn record(&self, d: Duration) {
        let nanos = u32::try_from(d.as_nanos()).unwrap_or(u32::MAX);
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        self.samples[n % self.samples.len()].store(nanos, Ordering::Relaxed);
    }

    /// The number of samples recorded so far (not capped by the ring size).
    pub fn count(&self) -> usize {
        self.next.load(Ordering::Relaxed)
    }

    /// The p99 (nearest rank) of the kept samples, or `None` before the first sample.
    pub fn p99(&self) -> Option<Duration> {
        let kept = self.count().min(self.samples.len());
        if kept == 0 {
            return None;
        }
        let mut v: Vec<u32> = self.samples[..kept]
            .iter()
            .map(|s| s.load(Ordering::Relaxed))
            .collect();
        v.sort_unstable();
        let rank = (kept * 99).div_ceil(100).max(1);
        Some(Duration::from_nanos(u64::from(v[rank - 1])))
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
    pub fn on_foreground(&mut self, pid: u32, elevation: Elevation) -> Option<HookSignal> {
        if self.self_elevated || elevation != Elevation::Elevated {
            return None;
        }
        self.shown.insert(pid).then_some(HookSignal::TalkKeyBlocked)
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
    pub fn handle(&mut self, ev: KeyEvent) -> Verdict {
        let started = self.clock.now();
        self.filter
            .set_hands_free(self.hands_free.load(Ordering::Relaxed));
        let (verdict, signal) = self.filter.on_key(ev, started);
        if let Some(signal) = signal {
            self.send(signal);
        }
        self.recorder
            .record(self.clock.now().saturating_sub(started));
        verdict
    }

    /// See [`HookFilter::reconcile`]; the signal goes on the channel.
    pub fn reconcile(&mut self, physically_down: impl Fn(u32) -> bool) {
        if let Some(signal) = self.filter.reconcile(physically_down, self.clock.now()) {
            self.send(signal);
        }
    }

    /// The session was unlocked: see [`HookFilter::on_unlock`]; the signal goes on the channel.
    pub fn on_unlock(&mut self, physically_down: impl Fn(u32) -> bool) {
        if let Some(signal) = self.filter.on_unlock(physically_down, self.clock.now()) {
            self.send(signal);
        }
    }

    /// The device resumed: see [`HookFilter::on_resume`]; the signal goes on the channel.
    pub fn on_resume(&mut self) {
        if let Some(signal) = self.filter.on_resume(self.clock.now()) {
            self.send(signal);
        }
    }

    /// Send a signal that does not come from a key event (reinstall, elevated foreground).
    pub fn send(&self, signal: HookSignal) {
        let _ = self.signals.send(signal);
    }
}

fn is_ctrl(vk: u32) -> bool {
    vk == vk::LCONTROL || vk == vk::RCONTROL
}

fn is_win(vk: u32) -> bool {
    vk == vk::LWIN || vk == vk::RWIN
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
        assert_eq!(
            out[0],
            (Verdict::Swallow, Some(HookSignal::TalkDown { at: T0 }))
        );
        assert_eq!(
            out[1],
            (Verdict::Swallow, None),
            "auto-repeat is swallowed, no new signal"
        );
        assert_eq!(
            out[2],
            (Verdict::Swallow, Some(HookSignal::TalkUp { at: T0 }))
        );
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
            assert_eq!(
                (down, up),
                (Verdict::Swallow, Verdict::Swallow),
                "press {i}"
            );
        }
    }

    #[test]
    fn cap_41_swallows_right_ctrl_when_it_is_the_talk_key() {
        let mut f = HookFilter::new(TalkKey::RightCtrl);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RCONTROL), T0).0,
            Verdict::Swallow
        );
        assert_eq!(f.on_key(KeyEvent::up(vk::RCONTROL), T0).0, Verdict::Swallow);
        // Right-Alt is a normal key now.
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RMENU), T0),
            (Verdict::Pass, None)
        );
    }

    #[test]
    fn cap_41_passes_esc_when_not_capturing() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::ESCAPE), T0),
            (Verdict::Pass, None)
        );
        assert_eq!(
            f.on_key(KeyEvent::up(vk::ESCAPE), T0),
            (Verdict::Pass, None)
        );
    }

    #[test]
    fn cap_41_swallows_esc_while_the_talk_key_is_held() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::ESCAPE), T0),
            (Verdict::Swallow, Some(HookSignal::Escape))
        );
        assert_eq!(
            f.on_key(KeyEvent::up(vk::ESCAPE), T0),
            (Verdict::Swallow, None)
        );
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
        assert_eq!(
            f.on_key(KeyEvent::up(vk::ESCAPE), T0),
            (Verdict::Swallow, None)
        );
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
        assert_eq!(
            f.on_key(KeyEvent::down(0x1_0000), T0),
            (Verdict::Pass, None)
        );
    }

    #[test]
    fn cap_41_ctrl_win_chord_ctrl_first() {
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::LCONTROL), T0),
            (Verdict::Pass, None)
        );
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
        assert_eq!(
            f.on_key(KeyEvent::up(vk::LCONTROL), T0),
            (Verdict::Pass, None)
        );
    }

    #[test]
    fn cap_41_ctrl_win_chord_win_first_masks_the_start_menu() {
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RWIN), T0),
            (Verdict::Pass, None)
        );
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
    fn cap_41_ctrl_win_chord_repeat_of_the_first_key_does_not_stick() {
        // Ctrl reached the app and auto-repeats during the chord: its key-up must pass.
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        f.on_key(KeyEvent::down(vk::LCONTROL), T0);
        f.on_key(KeyEvent::down(vk::LWIN), T0);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::LCONTROL), T0).0,
            Verdict::Swallow
        );
        f.on_key(KeyEvent::up(vk::LWIN), T0);
        assert_eq!(
            f.on_key(KeyEvent::up(vk::LCONTROL), T0),
            (Verdict::Pass, None)
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
        /// Hook ids installed now, in install order.
        live: Vec<u32>,
        /// The most hooks installed at one time.
        max_live: usize,
    }

    impl HookInstaller for FakeInstaller {
        type Handle = u32;
        fn install(&mut self) -> Result<u32, &'static str> {
            if std::mem::take(&mut self.fail_next) {
                return Err("set_windows_hook_failed");
            }
            self.installs += 1;
            self.live.push(self.installs);
            self.max_live = self.max_live.max(self.live.len());
            Ok(self.installs)
        }
        fn uninstall(&mut self, handle: u32) {
            self.uninstalls += 1;
            self.live.retain(|h| *h != handle);
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
    fn cap_40b_failed_reinstall_keeps_the_old_hook_and_retries_on_the_next_timer() {
        let clock = SimClock::new();
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(clock.now()).unwrap();
        clock.advance(REINSTALL_INTERVAL);
        s.installer.fail_next = true;
        let err = s.on_timer(clock.now()).unwrap_err();
        assert_eq!(err.code(), "set_windows_hook_failed");
        assert!(
            s.is_installed(),
            "the old hook stays when the new one fails"
        );
        assert_eq!(s.installer().live, vec![1]);
        clock.advance(Duration::from_secs(1));
        assert!(s.on_timer(clock.now()).unwrap(), "retry at the next tick");
        assert_eq!(s.installer().live, vec![2]);
    }

    #[test]
    fn cap_40b_reinstall_installs_the_new_hook_before_it_removes_the_old_one() {
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(T0).unwrap();
        assert!(s.on_timer(T0 + REINSTALL_INTERVAL).unwrap());
        assert_eq!(s.installer().max_live, 2, "no gap without a hook");
        assert_eq!(s.installer().live, vec![2]);
    }

    #[test]
    fn cap_40b_reinstall_failure_becomes_a_hook_failed_signal() {
        let mut s = HookSupervisor::new(FakeInstaller::default());
        s.start(T0).unwrap();
        s.installer.fail_next = true;
        let result = s.on_timer(T0 + REINSTALL_INTERVAL);
        assert_eq!(
            reinstall_signal(&result),
            Some(HookSignal::HookFailed {
                reason: "set_windows_hook_failed"
            })
        );
        assert_eq!(
            reinstall_signal(&Ok(true)),
            Some(HookSignal::HookReinstalled)
        );
        assert_eq!(reinstall_signal(&Ok(false)), None);
    }

    #[test]
    fn cap_40b_start_failure_is_an_error() {
        let mut s = HookSupervisor::new(FakeInstaller {
            fail_next: true,
            ..FakeInstaller::default()
        });
        assert_eq!(
            s.start(T0).unwrap_err(),
            CoreError::Unavailable {
                component: "R1",
                reason: "set_windows_hook_failed"
            }
        );
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
    fn cap_40b_reconcile_keeps_a_swallowed_talk_key_whatever_the_async_state() {
        // Windows does not update the async key state for a key the hook swallowed, so
        // "up" from GetAsyncKeyState is not proof. The PTT must not split in two.
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        assert_eq!(f.reconcile(|_| false, T0), None);
        assert!(f.talk_active());
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RMENU), T0),
            (Verdict::Swallow, None)
        );
        assert_eq!(
            f.on_key(KeyEvent::up(vk::RMENU), T0),
            (Verdict::Swallow, Some(HookSignal::TalkUp { at: T0 }))
        );
    }

    #[test]
    fn cap_40b_reconcile_ends_a_chord_whose_passed_key_was_released() {
        // Ctrl reached Windows, so its async state is real: Ctrl up means the chord ended.
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        f.on_key(KeyEvent::down(vk::LCONTROL), T0);
        f.on_key(KeyEvent::down(vk::LWIN), T0);
        let later = T0 + Duration::from_secs(2);
        assert_eq!(
            f.reconcile(|_| false, later),
            Some(HookSignal::TalkUp { at: later })
        );
        assert!(!f.talk_active());
    }

    #[test]
    fn cap_40b_resume_ends_an_active_ptt() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        let later = T0 + Duration::from_secs(60);
        assert_eq!(f.on_resume(later), Some(HookSignal::TalkUp { at: later }));
        assert!(!f.talk_active());
        assert_eq!(f.on_resume(later), None, "a second resume event is a no-op");
        // The next press starts a new PTT.
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RMENU), later),
            (Verdict::Swallow, Some(HookSignal::TalkDown { at: later }))
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
    fn cap_40a_core_resume_sends_the_key_up() {
        let (mut c, rx, _) = core(TalkKey::RightAlt);
        c.handle(KeyEvent::down(vk::RMENU));
        c.on_resume();
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkDown { at: T0 }));
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkUp { at: T0 }));
    }

    #[test]
    fn cap_40a_callback_work_p99_is_under_1_ms() {
        // The platform-neutral part of the callback, timed on the real monotonic clock. The
        // real hook p99 on a Windows device is AT-30 (needs:windows, TODO(#5)).
        let (tx, _rx) = mpsc::channel();
        let rec = Arc::new(LatencyRecorder::with_capacity(20_000));
        let clock = Arc::new(crate::clock::SystemClock::new());
        let mut c = HookCore::new(
            TalkKey::RightAlt,
            Arc::new(AtomicBool::new(false)),
            rec.clone(),
            clock,
            tx,
        );
        for i in 0..10_000u32 {
            let k = if i % 3 == 0 {
                vk::RMENU
            } else {
                0x41 + (i % 20)
            };
            c.handle(KeyEvent::down(k));
            c.handle(KeyEvent::up(k));
        }
        assert_eq!(rec.count(), 20_000);
        let p99 = rec.p99().unwrap();
        assert!(p99 < CALLBACK_BUDGET, "p99 {p99:?}");
    }

    #[test]
    fn cap_41_shared_talk_keys_map_to_windows_keys() {
        use crate::hotkey_core::TalkKey as Core;
        assert_eq!(TalkKey::from(Core::RightOption), TalkKey::RightAlt);
        assert_eq!(TalkKey::from(Core::RightControl), TalkKey::RightCtrl);
        // D66 gives Windows only Right-Alt, Right-Ctrl and Ctrl+Win (#313). A shared key
        // outside that set falls back to the D66 default.
        assert_eq!(TalkKey::from(Core::RightCommand), TalkKey::RightAlt);
        assert_eq!(TalkKey::from(Core::RightShift), TalkKey::RightAlt);
    }

    #[test]
    fn cap_41_cap_43a_windows_never_swallows_right_shift_or_right_win() {
        // A swallowed Right-Shift blocks capitals; a swallowed Right-Win breaks Win shortcuts.
        use crate::hotkey_core::TalkKey as Core;
        for core in [
            Core::RightOption,
            Core::RightControl,
            Core::RightCommand,
            Core::RightShift,
        ] {
            for talk in [TalkKey::from(core), TalkKey::CtrlWin] {
                for code in [vk::RSHIFT, vk::RWIN] {
                    if talk == TalkKey::CtrlWin && code == vk::RWIN {
                        continue; // Win is one key of the Ctrl+Win chord.
                    }
                    let mut f = HookFilter::new(talk);
                    assert_eq!(
                        f.on_key(KeyEvent::down(code), T0),
                        (Verdict::Pass, None),
                        "{talk:?} {code:#x}"
                    );
                    assert_eq!(f.on_key(KeyEvent::up(code), T0).0, Verdict::Pass);
                }
            }
        }
    }

    #[test]
    fn cap_43a_windows_talk_keys_are_only_the_d66_keys() {
        assert_eq!(
            TalkKey::ALL,
            [TalkKey::RightAlt, TalkKey::RightCtrl, TalkKey::CtrlWin]
        );
        assert_eq!(TalkKey::RightAlt.single_vk(), Some(vk::RMENU));
        assert_eq!(TalkKey::RightCtrl.single_vk(), Some(vk::RCONTROL));
        assert_eq!(TalkKey::CtrlWin.single_vk(), None);
    }

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<crate::hotkey_core::HotkeyOutput>>);

    impl crate::hotkey_core::HotkeySink for RecordingSink {
        fn emit(&self, output: crate::hotkey_core::HotkeyOutput) {
            self.0.lock().unwrap().push(output);
        }
    }

    #[test]
    fn cap_41_talk_signals_drive_the_shared_ptt_machine() {
        use crate::hotkey_core::{HotkeyEngine, HotkeyOutput, PttPhase};
        let sink = Arc::new(RecordingSink::default());
        let clock = crate::clock::SimClock::new();
        let mut engine = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        assert_eq!(
            forward_to_engine(HookSignal::TalkDown { at: T0 }, &mut engine),
            None
        );
        // Hold longer than a tap (CAP-44a).
        clock.advance(crate::hotkey_core::TAP_MAX_HOLD);
        assert_eq!(
            forward_to_engine(HookSignal::TalkUp { at: T0 }, &mut engine),
            None
        );
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec![
                HotkeyOutput::Ptt(PttPhase::Down),
                HotkeyOutput::Ptt(PttPhase::Up)
            ]
        );
        // Signals with no shared equivalent go back to the host.
        assert_eq!(
            forward_to_engine(HookSignal::TalkKeyBlocked, &mut engine),
            Some(HookSignal::TalkKeyBlocked)
        );
        assert_eq!(
            forward_to_engine(HookSignal::Escape, &mut engine),
            Some(HookSignal::Escape)
        );
    }

    #[test]
    fn cap_44c_at_03_other_key_down_cancels_the_shared_ptt() {
        use crate::hotkey_core::{HotkeyEngine, HotkeyOutput, PttPhase};
        let sink = Arc::new(RecordingSink::default());
        let clock = crate::clock::SimClock::new();
        let mut engine = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        let mut f = HookFilter::new(TalkKey::RightAlt);
        let (_, sig) = f.on_key(KeyEvent::down(vk::RMENU), T0);
        assert_eq!(forward_to_engine(sig.unwrap(), &mut engine), None);
        clock.advance(Duration::from_millis(400));
        // The user types a key with Right-Alt held (an AltGr character): the PTT cancels.
        let (_, sig) = f.on_key(KeyEvent::down(0x45), T0);
        assert_eq!(sig, Some(HookSignal::OtherKeyDown));
        assert_eq!(forward_to_engine(sig.unwrap(), &mut engine), None);
        let (_, sig) = f.on_key(KeyEvent::up(vk::RMENU), T0);
        assert_eq!(forward_to_engine(sig.unwrap(), &mut engine), None);
        assert_eq!(
            *sink.0.lock().unwrap(),
            vec![
                HotkeyOutput::Ptt(PttPhase::Down),
                HotkeyOutput::Ptt(PttPhase::Cancel)
            ]
        );
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
        assert_eq!(
            g.on_foreground(42, Elevation::Elevated),
            None,
            "once per process"
        );
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
    fn cap_42_token_query_refused_counts_as_elevated() {
        // A non-elevated process may not query the token of an elevated one.
        assert_eq!(Elevation::from_probe(true, None), Elevation::Elevated);
        assert_eq!(Elevation::from_probe(true, Some(true)), Elevation::Elevated);
        assert_eq!(
            Elevation::from_probe(true, Some(false)),
            Elevation::NotElevated
        );
        assert_eq!(Elevation::from_probe(false, None), Elevation::Unknown);
    }

    #[test]
    fn cap_42_no_tooltip_when_elevation_is_unknown() {
        let mut g = ElevationGate::new(false);
        assert_eq!(g.on_foreground(4, Elevation::Unknown), None);
    }

    // ---- #313: message loop failure (CAP-40a, CAP-40b) ----

    #[test]
    fn cap_40b_get_message_minus_one_is_a_loop_failure() {
        assert_eq!(LoopStep::from_get_message(1), LoopStep::Dispatch);
        assert_eq!(LoopStep::from_get_message(0), LoopStep::Quit);
        assert_eq!(LoopStep::from_get_message(-1), LoopStep::Failed);
    }

    #[test]
    fn cap_40b_loop_failure_sends_hook_failed() {
        assert_eq!(
            LoopStep::Failed.exit_signal(),
            Some(HookSignal::HookFailed {
                reason: "message_loop_failed"
            })
        );
        assert_eq!(LoopStep::Quit.exit_signal(), None);
        assert_eq!(LoopStep::Dispatch.exit_signal(), None);
    }

    #[test]
    fn cap_40b_finished_hook_thread_counts_as_stopped() {
        assert_eq!(HookThread::of(false, false), HookThread::Stopped);
        assert_eq!(HookThread::of(true, false), HookThread::Running);
        assert_eq!(HookThread::of(true, true), HookThread::Finished);
        // start() starts a new hook when the old thread has ended.
        assert!(HookThread::Stopped.start_spawns());
        assert!(HookThread::Finished.start_spawns());
        assert!(!HookThread::Running.start_spawns());
        // stop() posts WM_QUIT only to a live thread, so it never fails with stop_post_failed
        // after the loop ended by itself.
        assert!(HookThread::Running.stop_posts_quit());
        assert!(!HookThread::Finished.stop_posts_quit());
        assert!(!HookThread::Stopped.stop_posts_quit());
    }

    // ---- #313: elevated foreground at start (CAP-42) ----

    #[test]
    fn cap_42_elevated_foreground_at_start_shows_the_tooltip_without_a_change() {
        let (tx, rx) = mpsc::channel();
        let mut gate = ElevationGate::new(false);
        // No foreground change event follows: only the start-time read.
        let pids = foreground_at_start(42).into_iter();
        run_elevation_gate(&mut gate, pids, |_| Elevation::Elevated, &tx);
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkKeyBlocked));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn cap_42_start_read_and_a_later_change_to_the_same_process_show_one_tooltip() {
        let (tx, rx) = mpsc::channel();
        let mut gate = ElevationGate::new(false);
        let pids = foreground_at_start(42).into_iter().chain([42, 7]);
        let probe = |pid| {
            if pid == 42 {
                Elevation::Elevated
            } else {
                Elevation::NotElevated
            }
        };
        run_elevation_gate(&mut gate, pids, probe, &tx);
        assert_eq!(
            rx.try_iter().collect::<Vec<_>>(),
            vec![HookSignal::TalkKeyBlocked]
        );
    }

    #[test]
    fn cap_42_no_foreground_window_at_start_sends_nothing() {
        assert_eq!(foreground_at_start(0), None);
        assert_eq!(foreground_at_start(9), Some(9));
    }

    // ---- #313: CAP-45 on Windows: read the key state again on session unlock ----

    #[test]
    fn cap_45_unlock_ends_an_active_ptt() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        let later = T0 + Duration::from_secs(30);
        // The swallowed key has no real async state: the PTT ends whatever it says.
        assert_eq!(
            f.on_unlock(|_| true, later),
            Some(HookSignal::TalkUp { at: later })
        );
        assert!(!f.talk_active());
        assert_eq!(
            f.on_unlock(|_| true, later),
            None,
            "a second unlock is a no-op"
        );
        // The next press starts a new PTT.
        assert_eq!(
            f.on_key(KeyEvent::down(vk::RMENU), later),
            (Verdict::Swallow, Some(HookSignal::TalkDown { at: later }))
        );
    }

    #[test]
    fn cap_45_unlock_keeps_a_passed_key_that_is_still_down() {
        // Ctrl reached Windows before the lock and is still held: Win then completes the chord.
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        f.on_key(KeyEvent::down(vk::LCONTROL), T0);
        assert_eq!(f.on_unlock(|k| k == vk::LCONTROL, T0), None);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::LWIN), T0),
            (Verdict::Swallow, Some(HookSignal::TalkDown { at: T0 }))
        );
    }

    #[test]
    fn cap_45_unlock_clears_a_passed_key_that_went_up() {
        // Ctrl went up on the secure desktop, so the hook never saw the key-up.
        let mut f = HookFilter::new(TalkKey::CtrlWin);
        f.on_key(KeyEvent::down(vk::LCONTROL), T0);
        assert_eq!(f.on_unlock(|_| false, T0), None);
        assert_eq!(
            f.on_key(KeyEvent::down(vk::LWIN), T0),
            (Verdict::Pass, None),
            "Win alone is not the chord"
        );
    }

    #[test]
    fn cap_45_unlock_lets_a_swallowed_key_up_pass() {
        // After unlock the hook forgets the swallowed key-down, so a late key-up passes and
        // no key sticks in the focused app.
        let mut f = HookFilter::new(TalkKey::RightAlt);
        f.on_key(KeyEvent::down(vk::RMENU), T0);
        f.on_unlock(|_| false, T0);
        assert_eq!(f.on_key(KeyEvent::up(vk::RMENU), T0), (Verdict::Pass, None));
    }

    #[test]
    fn cap_45_core_unlock_sends_the_key_up() {
        let (mut c, rx, _) = core(TalkKey::RightAlt);
        c.handle(KeyEvent::down(vk::RMENU));
        c.on_unlock(|_| false);
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkDown { at: T0 }));
        assert_eq!(rx.try_recv(), Ok(HookSignal::TalkUp { at: T0 }));
    }

    // ---- CAP-43b (#332, ADR 0025): the hook sees the Copilot key once ----

    /// Run `events` through a filter and a Copilot watch, as `HookCore` does. Returns how
    /// many times the watch fired and the filter verdicts.
    fn watch(events: &[KeyEvent]) -> (usize, Vec<Verdict>) {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        let mut w = CopilotWatch::default();
        let mut fired = 0;
        let mut verdicts = Vec::new();
        for e in events {
            verdicts.push(f.on_key(*e, T0).0);
            if w.on_key(*e, &f) {
                fired += 1;
            }
        }
        (fired, verdicts)
    }

    fn copilot_press(win: u32, shift: u32) -> [KeyEvent; 6] {
        [
            KeyEvent::down(win),
            KeyEvent::down(shift),
            KeyEvent::down(vk::F23),
            KeyEvent::up(vk::F23),
            KeyEvent::up(shift),
            KeyEvent::up(win),
        ]
    }

    #[test]
    fn cap_43b_the_copilot_chord_is_seen_once() {
        let mut events = copilot_press(vk::LWIN, vk::LSHIFT).to_vec();
        events.extend(copilot_press(vk::LWIN, vk::LSHIFT));
        events.extend(copilot_press(vk::RWIN, vk::RSHIFT));
        assert_eq!(watch(&events).0, 1);
    }

    #[test]
    fn cap_43b_either_win_and_either_shift_make_the_chord() {
        assert_eq!(watch(&copilot_press(vk::RWIN, vk::RSHIFT)).0, 1);
        assert_eq!(watch(&copilot_press(vk::LWIN, vk::RSHIFT)).0, 1);
    }

    #[test]
    fn cap_43b_f23_needs_win_and_shift() {
        let alone = [KeyEvent::down(vk::F23), KeyEvent::up(vk::F23)];
        let shift_only = [
            KeyEvent::down(vk::LSHIFT),
            KeyEvent::down(vk::F23),
            KeyEvent::up(vk::F23),
            KeyEvent::up(vk::LSHIFT),
        ];
        let win_only = [
            KeyEvent::down(vk::LWIN),
            KeyEvent::down(vk::F23),
            KeyEvent::up(vk::F23),
            KeyEvent::up(vk::LWIN),
        ];
        let not_f23 = [
            KeyEvent::down(vk::LWIN),
            KeyEvent::down(vk::LSHIFT),
            KeyEvent::down(0x85), // F22
            KeyEvent::up(0x85),
            KeyEvent::up(vk::LSHIFT),
            KeyEvent::up(vk::LWIN),
        ];
        for events in [&alone[..], &shift_only, &win_only, &not_f23] {
            assert_eq!(watch(events).0, 0, "{} events", events.len());
        }
    }

    #[test]
    fn cap_43b_an_injected_copilot_chord_is_ignored() {
        // SendInput (a remap tool, a test) is not a keyboard with the key.
        let injected: Vec<KeyEvent> = copilot_press(vk::LWIN, vk::LSHIFT)
            .iter()
            .map(|e| KeyEvent {
                injected: true,
                ..*e
            })
            .collect();
        assert_eq!(watch(&injected).0, 0);
    }

    #[test]
    fn cap_43b_the_copilot_chord_passes_to_windows() {
        // Copilot still opens: the hook only looks.
        let (_, verdicts) = watch(&copilot_press(vk::LWIN, vk::LSHIFT));
        assert!(verdicts.iter().all(|v| *v == Verdict::Pass), "{verdicts:?}");
    }

    #[test]
    fn cap_43b_a_watch_that_saw_the_key_stops_looking() {
        let mut f = HookFilter::new(TalkKey::RightAlt);
        let mut w = CopilotWatch::default();
        for e in copilot_press(vk::LWIN, vk::LSHIFT) {
            f.on_key(e, T0);
            w.on_key(e, &f);
        }
        assert!(w.seen());
    }

    #[test]
    fn cap_43b_hook_core_sends_copilot_key_seen_once() {
        let (mut c, rx, rec) = core(TalkKey::RightAlt);
        for _ in 0..2 {
            for e in copilot_press(vk::LWIN, vk::LSHIFT) {
                assert_eq!(c.handle(e), Verdict::Pass);
            }
        }
        assert_eq!(rx.try_recv(), Ok(HookSignal::CopilotKeySeen));
        assert!(rx.try_recv().is_err(), "one signal only");
        assert_eq!(rec.count(), 12, "every callback is still timed (CAP-40a)");
    }

    #[test]
    fn cap_43b_copilot_key_seen_goes_to_the_host_not_the_engine() {
        let sink = Arc::new(RecordingSink::default());
        let mut engine = crate::hotkey_core::HotkeyEngine::new(sink.clone());
        assert_eq!(
            forward_to_engine(HookSignal::CopilotKeySeen, &mut engine),
            Some(HookSignal::CopilotKeySeen)
        );
        assert!(sink.0.lock().unwrap().is_empty(), "no PTT output");
    }
}
