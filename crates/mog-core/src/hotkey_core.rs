//! Platform-neutral logic of R1 HotkeyService (#6, shared with the Windows hook of #7).
//!
//! The OS layer (`hotkey_macos`, and the Windows hook of #7) turns raw OS events into the
//! neutral inputs of this module. This module decides the PTT phases, the tap watchdog
//! actions, the Secure Event Input switch to the fallback chord and the key-state re-read
//! after a gap. It has no OS calls, so tests drive it with fakes and a simulated clock.
//!
//! Privacy: no input here carries a key code of typed text. The OS layer reports only the
//! talk key, the fallback chord and "the ask key went down", so key contents never reach this
//! module or any log.
//!
//! Ask key (RTR-17a, ADR 0098): while the talk key is held, the OS layer grabs talk key + G
//! through [`AskKeyControl`]. A press marks the hold as an ask, and its `up` becomes
//! [`HotkeyOutput::PttAskUp`]. The engine releases the grab on every path that ends the hold.
//!
//! Requirements: CAP-35b, CAP-36, CAP-37, CAP-38a, CAP-38b, CAP-39a, CAP-39b, CAP-39c, CAP-44a,
//! CAP-44c, CAP-45, RTR-17a.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use crate::clock::{MonotonicClock, SystemClock};
use crate::error::{CoreError, CoreResult};

const ID: &str = "R1";

/// The text of the "Talk key paused" pill (Appendix E, CAP-39c).
pub const TALK_KEY_PAUSED_PILL: &str = "Talk key paused: another app protects typing. Use ⌃⇧Space.";

/// How often the watchdog polls Secure Event Input, the session and the permission.
/// The OS has no Secure Event Input notification, so a poll is the only way (ADR 0011).
pub const WATCHDOG_TICK: Duration = Duration::from_millis(100);

/// The longest time from Secure Event Input start to the end of an active PTT (CAP-39a).
pub const SECURE_INPUT_DEADLINE: Duration = Duration::from_millis(250);

/// A slept gap longer than this counts as a wake (CAP-38b, CAP-45).
pub const WAKE_THRESHOLD: Duration = Duration::from_millis(500);

/// This many tap disables inside [`TAP_FAILURE_WINDOW`] make the watchdog create the tap again
/// instead of only turning it on again (CAP-38a).
pub const TAP_FAILURE_LIMIT: usize = 3;

/// The window for [`TAP_FAILURE_LIMIT`].
pub const TAP_FAILURE_WINDOW: Duration = Duration::from_secs(10);

/// While Input Monitoring is granted and no tap is alive, the watchdog asks for a new tap
/// this often (CAP-38b). `CGEventTapCreate` can fail for a short time after a wake.
pub const TAP_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// While Secure Event Input is on and the chord could not be registered, the watchdog tries
/// again this often (CAP-39b).
pub const CHORD_RETRY_INTERVAL: Duration = Duration::from_secs(1);

/// A talk-key hold shorter than this is a tap (CAP-44a).
pub const TAP_MAX_HOLD: Duration = Duration::from_millis(250);

/// Two taps whose key-downs are this close or closer make a double-tap (CAP-14a, ADR 0057).
pub const DOUBLE_TAP_MAX_GAP: Duration = Duration::from_millis(400);

/// Another key that goes down this soon after the talk key cancels the PTT (CAP-44c).
pub const OTHER_KEY_CANCEL_WINDOW: Duration = Duration::from_secs(1);

/// The ask key is grabbed for at most this long in one hold (RTR-17a, ADR 0098). It matches the
/// 120 s PTT safety stop of R2 (CAP-04a), so Option+G never stays grabbed after the capture ends.
pub const ASK_KEY_MAX_HOLD: Duration = Duration::from_secs(120);

/// The longest wait of a key-up for the queued ask-key presses (RTR-17a, ADR 0098). The ask key
/// is handled on the main thread and the key-up on the tap thread, so a G tapped just before the
/// release can come after the key-up. The engine waits for [`AskKeyControl::drain`]; this bound
/// keeps the key-up on time if the drain never comes. The watchdog tick checks it.
pub const ASK_UP_GRACE: Duration = Duration::from_millis(200);

/// A modifier-only talk key (CAP-35b, D66). The default is Right-Option.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TalkKey {
    /// Right-Option on macOS, Right-Alt on Windows (the D66 default).
    #[default]
    RightOption,
    /// Right-Control (the D66 second choice).
    RightControl,
    /// Right-Command.
    RightCommand,
    /// Right-Shift.
    RightShift,
}

/// A key goes down or comes up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEdge {
    /// The key went down.
    Down,
    /// The key came up.
    Up,
}

/// The phase of the `hotkey.ptt` event (section 4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PttPhase {
    /// The talk key went down: start PTT.
    Down,
    /// The talk key came up: end PTT and keep the audio.
    Up,
    /// PTT ends and the audio is dropped (for example when Secure Event Input starts).
    Cancel,
    /// A second tap came [`DOUBLE_TAP_MAX_GAP`] or less after the first (key-down to key-down):
    /// start hands-free (CAP-14a). It follows the second `Tap`.
    DoubleTap,
}

impl PttPhase {
    /// The `phase` value of the `hotkey.ptt` event.
    pub fn as_str(self) -> &'static str {
        match self {
            PttPhase::Down => "down",
            PttPhase::Up => "up",
            PttPhase::Cancel => "cancel",
            PttPhase::DoubleTap => "double_tap",
        }
    }
}

/// Which input drives PTT now (CAP-39b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TalkKeyMode {
    /// The listen-only tap sees the talk key.
    TalkKey,
    /// Secure Event Input is on: the Carbon chord Ctrl+Shift+Space drives PTT.
    Chord,
}

/// What R1 tells the rest of the app. It never carries key contents. The host sends the pill
/// outputs to the brain as the `hotkey.status` event (#297, ADR 0039).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyOutput {
    /// A `hotkey.ptt` phase.
    Ptt(PttPhase),
    /// Show the "Talk key paused" pill ([`TALK_KEY_PAUSED_PILL`], CAP-39c).
    TalkKeyPaused,
    /// Hide the pill: the talk key works again.
    TalkKeyRestored,
    /// Secure Event Input is on, but the OS refused the fallback chord (for example another
    /// app owns Ctrl+Shift+Space). PTT does not work until the chord registers or Secure
    /// Event Input ends. The watchdog keeps trying (CAP-39b). The host sends it as
    /// `hotkey.status` with reason `fallback_chord_unavailable`; the brain picks the text.
    FallbackChordUnavailable,
    /// The talk key was held for less than [`TAP_MAX_HOLD`] (CAP-44a). R1 emits it after
    /// `Ptt(Cancel)`, so the audio of the tap is dropped and no STT call starts (CAP-44b).
    /// The hands-free stop (CAP-14b) reads it. Two taps close together also give
    /// `Ptt(DoubleTap)` (CAP-14a).
    Tap,
    /// The talk key came up after an ask key press in the same hold (RTR-17a, ADR 0098). It
    /// ends the PTT as `Ptt(Up)` does; the host adds `ask: true` to the `hotkey.ptt` up. A tap,
    /// a cancel and a double-tap never carry it.
    PttAskUp,
}

impl HotkeyOutput {
    /// The pill text for this output, if it shows a pill.
    pub fn pill_text(self) -> Option<&'static str> {
        match self {
            HotkeyOutput::TalkKeyPaused => Some(TALK_KEY_PAUSED_PILL),
            _ => None,
        }
    }
}

/// Receives the outputs of R1. The host forwards them to the brain.
pub trait HotkeySink: Send + Sync {
    /// Take one output. It must return fast: the tap callback calls it.
    fn emit(&self, output: HotkeyOutput);
}

/// The physical key state that the OS reports now (CAP-45).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PhysicalState {
    /// The talk key is down.
    pub talk_key_down: bool,
    /// The fallback chord (Ctrl+Shift+Space) is down.
    pub chord_down: bool,
}

/// The PTT key-state machine (CAP-35b, CAP-39a, CAP-39b, CAP-45).
#[derive(Debug, Clone)]
pub struct PttMachine {
    mode: TalkKeyMode,
    held_by: Option<TalkKeyMode>,
}

impl Default for PttMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl PttMachine {
    /// A machine in talk-key mode with no PTT held.
    pub fn new() -> Self {
        Self {
            mode: TalkKeyMode::TalkKey,
            held_by: None,
        }
    }

    /// The input that drives PTT now.
    pub fn mode(&self) -> TalkKeyMode {
        self.mode
    }

    /// A PTT is active.
    pub fn is_held(&self) -> bool {
        self.held_by.is_some()
    }

    /// The talk key went down or came up. It is ignored in chord mode.
    pub fn on_talk_key(&mut self, edge: KeyEdge) -> Vec<HotkeyOutput> {
        self.on_edge(TalkKeyMode::TalkKey, edge)
    }

    /// The fallback chord was pressed or released. It is ignored in talk-key mode.
    pub fn on_chord(&mut self, edge: KeyEdge) -> Vec<HotkeyOutput> {
        self.on_edge(TalkKeyMode::Chord, edge)
    }

    fn on_edge(&mut self, source: TalkKeyMode, edge: KeyEdge) -> Vec<HotkeyOutput> {
        if self.mode != source {
            return Vec::new();
        }
        match (edge, self.held_by) {
            (KeyEdge::Down, None) => {
                self.held_by = Some(source);
                vec![HotkeyOutput::Ptt(PttPhase::Down)]
            }
            (KeyEdge::Up, Some(held)) if held == source => {
                self.held_by = None;
                vec![HotkeyOutput::Ptt(PttPhase::Up)]
            }
            _ => Vec::new(),
        }
    }

    /// Secure Event Input started or ended. `physical` is read after the change.
    ///
    /// Start: an active PTT ends with `cancel` (the focused field is protected, so nothing
    /// may go into it; ADR 0011), the chord takes over and the pill shows.
    /// End: a chord PTT ends with `up` (the chord is unregistered, so its release would be
    /// lost), the talk key takes over and the pill hides. A re-read never starts a PTT.
    pub fn on_secure_input(&mut self, active: bool, physical: PhysicalState) -> Vec<HotkeyOutput> {
        let target = if active {
            TalkKeyMode::Chord
        } else {
            TalkKeyMode::TalkKey
        };
        if self.mode == target {
            return self.on_gap(physical);
        }
        let mut out = Vec::new();
        if self.held_by.take().is_some() {
            let phase = if active {
                PttPhase::Cancel
            } else {
                PttPhase::Up
            };
            out.push(HotkeyOutput::Ptt(phase));
        }
        self.mode = target;
        out.push(if active {
            HotkeyOutput::TalkKeyPaused
        } else {
            HotkeyOutput::TalkKeyRestored
        });
        out
    }

    /// End an active PTT with `cancel` (CAP-44c). The later key-up gives nothing.
    pub fn cancel(&mut self) -> Vec<HotkeyOutput> {
        match self.held_by.take() {
            Some(_) => vec![HotkeyOutput::Ptt(PttPhase::Cancel)],
            None => Vec::new(),
        }
    }

    /// A gap (wake, unlock, session change, tap restart): read the physical state again.
    /// A PTT whose key is no longer down ends. A re-read never starts a PTT.
    pub fn on_gap(&mut self, physical: PhysicalState) -> Vec<HotkeyOutput> {
        let still_down = match self.held_by {
            None => return Vec::new(),
            Some(TalkKeyMode::TalkKey) => physical.talk_key_down,
            Some(TalkKeyMode::Chord) => physical.chord_down,
        };
        if still_down {
            return Vec::new();
        }
        self.held_by = None;
        vec![HotkeyOutput::Ptt(PttPhase::Up)]
    }
}

/// A signal about the health of the event tap (CAP-38a, CAP-38b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapSignal {
    /// The OS turned the tap off because a callback was slow.
    DisabledByTimeout,
    /// The OS turned the tap off for user input (for example secure entry).
    DisabledByUserInput,
    /// A periodic check of the tap state.
    HealthCheck {
        /// The tap is on.
        enabled: bool,
    },
    /// The device woke from sleep.
    Wake,
    /// The user session became active (fast user switch back, console).
    SessionActive,
    /// The user granted Input Monitoring.
    PermissionGranted,
}

/// What the watchdog wants done to the tap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TapAction {
    /// Nothing.
    None,
    /// Turn the tap on again.
    ReEnable,
    /// Remove the tap and create a new one.
    Recreate,
}

/// The tap watchdog decisions (CAP-38a, CAP-38b).
#[derive(Debug, Clone, Default)]
pub struct TapWatchdog {
    disables: VecDeque<Duration>,
}

impl TapWatchdog {
    /// A watchdog with no history.
    pub fn new() -> Self {
        Self::default()
    }

    /// Decide what to do for `signal` at monotonic time `now`.
    pub fn on_signal(&mut self, signal: TapSignal, now: Duration) -> TapAction {
        match signal {
            TapSignal::HealthCheck { enabled: true } => TapAction::None,
            TapSignal::Wake | TapSignal::SessionActive | TapSignal::PermissionGranted => {
                self.disables.clear();
                TapAction::Recreate
            }
            TapSignal::DisabledByTimeout
            | TapSignal::DisabledByUserInput
            | TapSignal::HealthCheck { enabled: false } => {
                while self
                    .disables
                    .front()
                    .is_some_and(|t| now.saturating_sub(*t) > TAP_FAILURE_WINDOW)
                {
                    self.disables.pop_front();
                }
                self.disables.push_back(now);
                if self.disables.len() >= TAP_FAILURE_LIMIT {
                    self.disables.clear();
                    TapAction::Recreate
                } else {
                    TapAction::ReEnable
                }
            }
        }
    }
}

/// Edge detection for the Secure Event Input poll.
#[derive(Debug, Clone, Default)]
pub struct SecureInputMonitor {
    active: bool,
}

impl SecureInputMonitor {
    /// A monitor that starts with Secure Event Input off.
    pub fn new() -> Self {
        Self::default()
    }

    /// The last polled state.
    pub fn active(&self) -> bool {
        self.active
    }

    /// Take one poll. Returns `Some(new_state)` on a change.
    pub fn poll(&mut self, active: bool) -> Option<bool> {
        if active == self.active {
            return None;
        }
        self.active = active;
        Some(active)
    }
}

/// An OS permission that R1 or text insertion needs (D70).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    /// Input Monitoring (macOS): the listen-only tap needs only this (CAP-36).
    InputMonitoring,
    /// Accessibility (macOS): only text insertion needs this (CAP-37).
    Accessibility,
}

/// The state of a permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStatus {
    /// Granted.
    Granted,
    /// Denied by the user.
    Denied,
    /// Not asked yet.
    Unknown,
}

/// The OS permission API. `status` must never show a prompt; `request` may.
pub trait PermissionApi: Send + Sync {
    /// The state now, with no prompt.
    fn status(&self, permission: Permission) -> PermissionStatus;
    /// Ask the user. This can show the OS prompt.
    fn request(&self, permission: Permission) -> PermissionStatus;
    /// Open the OS Settings pane of `permission` ([`settings_pane`]). It shows no prompt. An OS
    /// with no such pane does nothing.
    fn open_settings(&self, _permission: Permission) {}
}

/// The macOS System Settings pane of a permission (Privacy & Security). A fixed URL.
pub fn settings_pane(permission: Permission) -> &'static str {
    match permission {
        Permission::InputMonitoring => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        }
        Permission::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
    }
}

/// Proof that the user did something (a click in the hub or the pop-up) that asks for a
/// permission. Only UI handlers make one. The one prompt with no click is the host's launch
/// prompt, once per launch (user decision 2026-11-19, ADR 0101 decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserAction(());

impl UserAction {
    /// Make the proof. Call this only in the handler of a user click.
    pub fn from_user_click() -> Self {
        Self(())
    }
}

/// Check, with no prompt, that PTT can start. HotkeyService start calls this (CAP-36).
pub fn check_ptt_permission(api: &dyn PermissionApi) -> CoreResult<()> {
    match api.status(Permission::InputMonitoring) {
        PermissionStatus::Granted => Ok(()),
        PermissionStatus::Denied | PermissionStatus::Unknown => Err(CoreError::Unavailable {
            component: ID,
            reason: "input_monitoring_required",
        }),
    }
}

/// Ask for the PTT permission after a user action: Input Monitoring only (CAP-36).
pub fn request_ptt_permission(api: &dyn PermissionApi, _action: UserAction) -> PermissionStatus {
    api.request(Permission::InputMonitoring)
}

/// One user click for `permission` (CAP-36, CAP-37). The OS shows its prompt only once, so:
/// - granted: nothing to do;
/// - denied, or `asked_before` in this run: open the Settings pane (no prompt);
/// - else: show the OS prompt.
///
/// The caller passes only the permission that the clicked control names.
pub fn request_after_click(
    api: &dyn PermissionApi,
    permission: Permission,
    asked_before: bool,
    _action: UserAction,
) -> PermissionStatus {
    match api.status(permission) {
        PermissionStatus::Granted => PermissionStatus::Granted,
        PermissionStatus::Unknown if !asked_before => api.request(permission),
        status => {
            api.open_settings(permission);
            status
        }
    }
}

/// Ask for Accessibility after the user selects text insertion, and only then (CAP-37).
pub fn request_insertion_permission(
    api: &dyn PermissionApi,
    _action: UserAction,
) -> PermissionStatus {
    api.request(Permission::Accessibility)
}

/// What the watchdog reads from the OS on each tick. Every read must be cheap and prompt-free.
pub trait PlatformProbe: Send + Sync {
    /// Secure Event Input is on.
    fn secure_input_active(&self) -> bool;
    /// The physical key state now.
    fn physical_state(&self) -> PhysicalState;
    /// The total time the device has slept since boot. It grows only across a sleep.
    fn slept(&self) -> Duration;
    /// The session is on the console and the screen is not locked.
    fn session_active(&self) -> bool;
    /// Input Monitoring is granted (status only, no prompt).
    fn input_monitoring_granted(&self) -> bool;
}

/// What the engine asks the OS layer to do.
pub trait TapControl: Send + Sync {
    /// Turn the tap on again.
    fn re_enable(&self);
    /// Remove the tap and create a new one. The OS layer may do it later, off the tap thread.
    fn recreate(&self);
    /// Register (`true`) or unregister (`false`) the Carbon fallback chord. The OS layer
    /// reports the result of a register with [`HotkeyEngine::on_chord_registered`].
    fn set_chord(&self, on: bool);
    /// A tap exists now. When it is false and Input Monitoring is granted, the watchdog asks
    /// for a new tap every [`TAP_RETRY_INTERVAL`] (CAP-38b).
    fn tap_alive(&self) -> bool;
}

/// Grabs and releases the ask key: the talk key + G (RTR-17a, ADR 0098). A grabbed key goes to
/// Bottle only, so the focused app gets no "g" or "©". The engine calls it only at the start and
/// the end of a talk-key hold, so the key types as usual at any other time.
pub trait AskKeyControl: Send + Sync {
    /// Grab (`true`) or release (`false`) the ask key. It must return fast: the tap callback
    /// calls it with the engine lock held. The OS layer may do the work later, in order.
    fn set_ask_key(&self, on: bool);

    /// Handle every ask-key press that the OS has queued now, then call
    /// [`HotkeyEngine::on_ask_key_drained`]. The engine calls it at a key-up with the ask key
    /// grabbed and no G seen yet. It must return fast, as `set_ask_key` must.
    fn drain(&self);
}

/// The state of the fallback chord registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChordState {
    Off,
    Pending,
    On,
    Failed { at: Duration },
}

/// R1 core: the machine, the watchdog and the Secure Event Input monitor, wired to a sink.
pub struct HotkeyEngine {
    machine: PttMachine,
    watchdog: TapWatchdog,
    secure: SecureInputMonitor,
    sink: Arc<dyn HotkeySink>,
    slept: Option<Duration>,
    session_active: Option<bool>,
    granted: Option<bool>,
    chord: ChordState,
    chord_reported: bool,
    last_tap_retry: Option<Duration>,
    clock: Arc<dyn MonotonicClock>,
    held_since: Option<Duration>,
    /// The key-down time of the last tap, while it can still be the first of a double-tap.
    last_tap_down: Option<Duration>,
    /// The ask key was pressed in this hold (RTR-17a).
    ask: bool,
    /// The OS seam that grabs the ask key. None: this OS has no ask key (Windows, ADR 0098).
    ask_key: Option<Arc<dyn AskKeyControl>>,
    /// The time the ask key was grabbed, while it is grabbed.
    ask_key_since: Option<Duration>,
    /// The hold passed [`ASK_KEY_MAX_HOLD`]: the ask key stays released until the hold ends.
    ask_key_expired: bool,
    /// The time of a talk-key up that waits for the ask-key drain (RTR-17a, review M1).
    pending_up: Option<Duration>,
}

impl std::fmt::Debug for HotkeyEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotkeyEngine")
            .field("machine", &self.machine)
            .finish_non_exhaustive()
    }
}

impl HotkeyEngine {
    /// An engine that sends its outputs to `sink`. The tap and cancel rules (CAP-44a,
    /// CAP-44c) time the holds on the system monotonic clock.
    pub fn new(sink: Arc<dyn HotkeySink>) -> Self {
        Self::with_clock(sink, Arc::new(SystemClock::new()))
    }

    /// An engine that times the holds on `clock` (tests use `SimClock`).
    pub fn with_clock(sink: Arc<dyn HotkeySink>, clock: Arc<dyn MonotonicClock>) -> Self {
        Self {
            machine: PttMachine::new(),
            watchdog: TapWatchdog::new(),
            secure: SecureInputMonitor::new(),
            sink,
            slept: None,
            session_active: None,
            granted: None,
            chord: ChordState::Off,
            chord_reported: false,
            last_tap_retry: None,
            clock,
            held_since: None,
            last_tap_down: None,
            ask: false,
            ask_key: None,
            ask_key_since: None,
            ask_key_expired: false,
            pending_up: None,
        }
    }

    /// The same engine with `ctl`, which grabs the ask key during each talk-key hold (RTR-17a).
    pub fn with_ask_key(mut self, ctl: Arc<dyn AskKeyControl>) -> Self {
        self.ask_key = Some(ctl);
        self
    }

    /// The ask key went down (RTR-17a, ADR 0098): the grabbed hot key, or the other-key tap that
    /// compares the key code with G. During a talk-key hold it marks the hold as an ask; a
    /// second report sets the same flag. It never cancels the PTT (CAP-44c). With no talk-key
    /// hold it does nothing.
    pub fn on_ask_key(&mut self) {
        if self.machine.held_by == Some(TalkKeyMode::TalkKey) {
            self.ask = true;
        }
    }

    /// The OS layer has handled every ask-key press that was queued at the drain request
    /// ([`AskKeyControl::drain`]). A key-up that waits for it goes out now, as an ask if a G
    /// came. A late drain does nothing.
    pub fn on_ask_key_drained(&mut self) {
        self.resolve_pending_up();
    }

    /// A key-up waits for the drain only when it can still become an ask: a talk-key hold that
    /// is no tap, with the ask key grabbed and no G seen yet.
    fn up_waits_for_ask_key(&self) -> bool {
        self.ask_key.is_some()
            && self.ask_key_since.is_some()
            && !self.ask
            && self.machine.held_by == Some(TalkKeyMode::TalkKey)
            && self
                .held_since
                .is_some_and(|t| self.clock.now().saturating_sub(t) >= TAP_MAX_HOLD)
    }

    /// End a key-up that waits for the drain.
    fn resolve_pending_up(&mut self) {
        if self.pending_up.take().is_some() {
            let out = self.machine.on_talk_key(KeyEdge::Up);
            let out = self.timed(out);
            self.emit(out);
        }
    }

    /// Another key went down (CAP-44c). The OS layer gives no key identity. In the first
    /// [`OTHER_KEY_CANCEL_WINDOW`] of a hold the PTT ends with `cancel`, so no STT call
    /// starts; later the key is ignored.
    pub fn on_other_key_down(&mut self) {
        if self.other_key_window_open() {
            self.held_since = None;
            self.last_tap_down = None;
            let out = self.machine.cancel();
            self.emit(out);
        }
    }

    /// A PTT is held and it started less than [`OTHER_KEY_CANCEL_WINDOW`] ago. The macOS
    /// layer listens to other key-downs only while this is true (ADR 0017).
    pub fn other_key_window_open(&self) -> bool {
        // Only a talk-key hold: under Secure Event Input (chord mode) taps get no KeyDown. A
        // key-up that waits for the ask-key drain has ended the hold for the user.
        self.machine.held_by == Some(TalkKeyMode::TalkKey)
            && self.pending_up.is_none()
            && self
                .held_since
                .is_some_and(|t| self.clock.now().saturating_sub(t) < OTHER_KEY_CANCEL_WINDOW)
    }

    /// Apply the tap rule (CAP-44a) to the outputs of a key edge: a key-up less than
    /// [`TAP_MAX_HOLD`] after its key-down becomes `cancel` and `Tap`.
    fn timed(&mut self, out: Vec<HotkeyOutput>) -> Vec<HotkeyOutput> {
        let now = self.clock.now();
        let mut timed = Vec::with_capacity(out.len() + 1);
        for o in out {
            match o {
                HotkeyOutput::Ptt(PttPhase::Down) => {
                    self.held_since = Some(now);
                    timed.push(o);
                }
                HotkeyOutput::Ptt(PttPhase::Up) => {
                    let down = self.held_since.take();
                    match down.filter(|t| now.saturating_sub(*t) < TAP_MAX_HOLD) {
                        Some(down) => {
                            timed.push(HotkeyOutput::Ptt(PttPhase::Cancel));
                            timed.push(HotkeyOutput::Tap);
                            // CAP-14a: the second of two close taps is a double-tap. A third
                            // tap starts a new pair.
                            match self.last_tap_down.take() {
                                Some(first) if down.saturating_sub(first) <= DOUBLE_TAP_MAX_GAP => {
                                    timed.push(HotkeyOutput::Ptt(PttPhase::DoubleTap));
                                }
                                _ => self.last_tap_down = Some(down),
                            }
                        }
                        None => {
                            self.last_tap_down = None;
                            timed.push(o);
                        }
                    }
                }
                HotkeyOutput::Ptt(PttPhase::Cancel) => {
                    // A cancelled press (CAP-44c, Secure Event Input, a reset) is no tap.
                    self.held_since = None;
                    self.last_tap_down = None;
                    timed.push(o);
                }
                other => timed.push(other),
            }
        }
        timed
    }

    /// The tap rule for a PTT that the system ended (Secure Event Input): `cancel` goes through
    /// [`Self::timed`]; `up` ends the hold and any tap pair, but is never a tap.
    fn system_end(&mut self, out: Vec<HotkeyOutput>) -> Vec<HotkeyOutput> {
        let mut ended = Vec::with_capacity(out.len());
        for o in out {
            if o == HotkeyOutput::Ptt(PttPhase::Up) {
                self.held_since = None;
                self.last_tap_down = None;
                ended.push(o);
            } else {
                ended.extend(self.timed(vec![o]));
            }
        }
        ended
    }

    /// The OS layer tried to register the fallback chord. On a failure R1 emits
    /// [`HotkeyOutput::FallbackChordUnavailable`] once and retries (CAP-39b).
    pub fn on_chord_registered(&mut self, ok: bool, now: Duration) {
        if self.chord != ChordState::Pending {
            // A late result after Secure Event Input ended, or a duplicate.
            return;
        }
        if ok {
            self.chord = ChordState::On;
            if std::mem::take(&mut self.chord_reported) {
                // The chord works now: show the "Use ⌃⇧Space" pill again.
                self.emit(vec![HotkeyOutput::TalkKeyPaused]);
            }
        } else {
            self.chord = ChordState::Failed { at: now };
            if !std::mem::replace(&mut self.chord_reported, true) {
                self.emit(vec![HotkeyOutput::FallbackChordUnavailable]);
            }
        }
    }

    /// The PTT machine (read only).
    pub fn machine(&self) -> &PttMachine {
        &self.machine
    }

    /// The last polled Secure Event Input state.
    pub fn secure_input_active(&self) -> bool {
        self.secure.active()
    }

    /// The tap saw the talk key.
    pub fn on_talk_key(&mut self, edge: KeyEdge) {
        // A new edge ends a key-up that still waits for the drain.
        self.resolve_pending_up();
        // RTR-17a (review M1): a G tapped just before the release can still be queued on the
        // main thread. The key-up waits for the drain, at most ASK_UP_GRACE.
        if edge == KeyEdge::Up && self.up_waits_for_ask_key() {
            self.pending_up = Some(self.clock.now());
            if let Some(ctl) = &self.ask_key {
                ctl.drain();
            }
            return;
        }
        let out = self.machine.on_talk_key(edge);
        let out = self.timed(out);
        self.emit(out);
    }

    /// The Carbon chord fired.
    pub fn on_chord(&mut self, edge: KeyEdge) {
        let out = self.machine.on_chord(edge);
        let out = self.timed(out);
        self.emit(out);
    }

    /// The tap reported a health signal. A key-up can be lost while the tap is off, so every
    /// action is followed by a key-state re-read (CAP-45).
    pub fn on_tap_signal(
        &mut self,
        signal: TapSignal,
        now: Duration,
        probe: &dyn PlatformProbe,
        ctl: &dyn TapControl,
    ) {
        match self.watchdog.on_signal(signal, now) {
            TapAction::None => return,
            TapAction::ReEnable => ctl.re_enable(),
            TapAction::Recreate => ctl.recreate(),
        }
        let out = self.machine.on_gap(probe.physical_state());
        self.emit(out);
    }

    /// One watchdog tick, every [`WATCHDOG_TICK`]. The first tick records the baseline.
    pub fn tick(&mut self, now: Duration, probe: &dyn PlatformProbe, ctl: &dyn TapControl) {
        // RTR-17a: a key-up whose drain did not come in ASK_UP_GRACE goes out now.
        if self
            .pending_up
            .is_some_and(|t| now.saturating_sub(t) >= ASK_UP_GRACE)
        {
            self.resolve_pending_up();
        }
        // RTR-17a: a hold past the 120 s cap releases the ask key. The hold itself goes on.
        if self
            .ask_key_since
            .is_some_and(|t| now.saturating_sub(t) >= ASK_KEY_MAX_HOLD)
        {
            self.ask_key_expired = true;
            self.sync_ask_key();
        }
        // Secure Event Input first: it has the 250 ms deadline (CAP-39a).
        if let Some(active) = self.secure.poll(probe.secure_input_active()) {
            ctl.set_chord(active);
            self.chord = if active {
                ChordState::Pending
            } else {
                ChordState::Off
            };
            self.chord_reported = false;
            let out = self.machine.on_secure_input(active, probe.physical_state());
            // A Secure Event Input `cancel` goes through the tap rule: it is no tap and ends any
            // pair (CAP-14a). The `up` at its end is a lost key-up (CAP-45), never a tap.
            let out = self.system_end(out);
            self.emit(out);
        } else if let ChordState::Failed { at } = self.chord {
            if now.saturating_sub(at) >= CHORD_RETRY_INTERVAL {
                self.chord = ChordState::Pending;
                ctl.set_chord(true);
            }
        }

        let slept = probe.slept();
        let woke = self
            .slept
            .is_some_and(|before| slept.saturating_sub(before) > WAKE_THRESHOLD);
        self.slept = Some(slept);
        if woke {
            self.on_tap_signal(TapSignal::Wake, now, probe, ctl);
        }

        let session = probe.session_active();
        if let Some(before) = self.session_active.replace(session) {
            if before != session {
                if session {
                    self.on_tap_signal(TapSignal::SessionActive, now, probe, ctl);
                } else {
                    // Lock or switch away: the key-up can be lost (CAP-45).
                    let out = self.machine.on_gap(probe.physical_state());
                    self.emit(out);
                }
            }
        }

        let granted = probe.input_monitoring_granted();
        if self.granted.replace(granted) == Some(false) && granted {
            self.on_tap_signal(TapSignal::PermissionGranted, now, probe, ctl);
        }

        // A recreate can fail (for example just after a wake). Retry while no tap is alive.
        if granted && !ctl.tap_alive() {
            let due = self
                .last_tap_retry
                .is_none_or(|t| now.saturating_sub(t) >= TAP_RETRY_INTERVAL);
            if due {
                self.last_tap_retry = Some(now);
                ctl.recreate();
            }
        } else {
            self.last_tap_retry = None;
        }
    }

    /// Go back to the start state when the service stops. An active PTT ends with `cancel`
    /// and a shown pill hides, so a later start sees Secure Event Input as a new edge and
    /// registers the chord again (CAP-39b).
    pub fn reset(&mut self) {
        let mut out = Vec::new();
        if self.machine.is_held() {
            out.push(HotkeyOutput::Ptt(PttPhase::Cancel));
        }
        if self.machine.mode() == TalkKeyMode::Chord {
            out.push(HotkeyOutput::TalkKeyRestored);
        }
        self.machine = PttMachine::new();
        self.watchdog = TapWatchdog::new();
        self.secure = SecureInputMonitor::new();
        self.slept = None;
        self.session_active = None;
        self.granted = None;
        self.chord = ChordState::Off;
        self.chord_reported = false;
        self.last_tap_retry = None;
        self.held_since = None;
        self.last_tap_down = None;
        self.ask = false;
        self.emit(out);
    }

    /// Send `out` to the sink. Every state change ends here, so the ask flag and the ask key
    /// grab follow the hold on each path: a key edge, a cancel, Secure Event Input, a gap and
    /// a reset.
    fn emit(&mut self, out: Vec<HotkeyOutput>) {
        for o in out {
            let o = match o {
                HotkeyOutput::Ptt(PttPhase::Up) if std::mem::take(&mut self.ask) => {
                    HotkeyOutput::PttAskUp
                }
                HotkeyOutput::Ptt(PttPhase::Down | PttPhase::Cancel) => {
                    self.ask = false;
                    o
                }
                o => o,
            };
            self.sink.emit(o);
        }
        if !self.machine.is_held() {
            self.ask = false;
            self.pending_up = None;
        }
        self.sync_ask_key();
    }

    /// Grab the ask key while a talk-key hold is active and younger than [`ASK_KEY_MAX_HOLD`];
    /// release it otherwise. Each change calls the seam once, so grab and release pair up.
    fn sync_ask_key(&mut self) {
        let held = self.machine.held_by == Some(TalkKeyMode::TalkKey);
        if !held {
            self.ask_key_expired = false;
        }
        let want = held && !self.ask_key_expired;
        if want == self.ask_key_since.is_some() {
            return;
        }
        self.ask_key_since = want.then(|| self.clock.now());
        if let Some(ctl) = &self.ask_key {
            ctl.set_ask_key(want);
        }
    }
}

impl Drop for HotkeyEngine {
    /// An engine that goes away in a hold releases the ask key (RTR-17a).
    fn drop(&mut self) {
        if self.ask_key_since.take().is_some() {
            if let Some(ctl) = &self.ask_key {
                ctl.set_ask_key(false);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::{MonotonicClock, SimClock};
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecSink(Mutex<Vec<HotkeyOutput>>);
    impl HotkeySink for RecSink {
        fn emit(&self, output: HotkeyOutput) {
            self.0.lock().unwrap().push(output);
        }
    }
    impl RecSink {
        fn take(&self) -> Vec<HotkeyOutput> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    #[derive(Default)]
    struct FakeProbe {
        secure: Mutex<bool>,
        physical: Mutex<PhysicalState>,
        slept: Mutex<Duration>,
        session: Mutex<bool>,
        granted: Mutex<bool>,
    }
    impl FakeProbe {
        fn ready() -> Self {
            let p = Self::default();
            *p.session.lock().unwrap() = true;
            *p.granted.lock().unwrap() = true;
            p
        }
    }
    impl PlatformProbe for FakeProbe {
        fn secure_input_active(&self) -> bool {
            *self.secure.lock().unwrap()
        }
        fn physical_state(&self) -> PhysicalState {
            *self.physical.lock().unwrap()
        }
        fn slept(&self) -> Duration {
            *self.slept.lock().unwrap()
        }
        fn session_active(&self) -> bool {
            *self.session.lock().unwrap()
        }
        fn input_monitoring_granted(&self) -> bool {
            *self.granted.lock().unwrap()
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Ctl {
        ReEnable,
        Recreate,
        Chord(bool),
    }
    struct FakeCtl(Mutex<Vec<Ctl>>, Mutex<bool>);
    impl Default for FakeCtl {
        fn default() -> Self {
            Self(Mutex::new(Vec::new()), Mutex::new(true))
        }
    }
    impl TapControl for FakeCtl {
        fn tap_alive(&self) -> bool {
            *self.1.lock().unwrap()
        }
        fn re_enable(&self) {
            self.0.lock().unwrap().push(Ctl::ReEnable);
        }
        fn recreate(&self) {
            self.0.lock().unwrap().push(Ctl::Recreate);
        }
        fn set_chord(&self, on: bool) {
            self.0.lock().unwrap().push(Ctl::Chord(on));
        }
    }
    impl FakeCtl {
        fn take(&self) -> Vec<Ctl> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    #[derive(Default)]
    struct FakePerms {
        statuses: Mutex<Vec<(Permission, PermissionStatus)>>,
        requested: Mutex<Vec<Permission>>,
        status_calls: Mutex<Vec<Permission>>,
        opened: Mutex<Vec<Permission>>,
    }
    impl FakePerms {
        fn with(p: Permission, s: PermissionStatus) -> Self {
            let f = Self::default();
            f.statuses.lock().unwrap().push((p, s));
            f
        }
        fn requested(&self) -> Vec<Permission> {
            self.requested.lock().unwrap().clone()
        }
    }
    impl PermissionApi for FakePerms {
        fn status(&self, permission: Permission) -> PermissionStatus {
            self.status_calls.lock().unwrap().push(permission);
            self.statuses
                .lock()
                .unwrap()
                .iter()
                .find(|(p, _)| *p == permission)
                .map(|(_, s)| *s)
                .unwrap_or(PermissionStatus::Unknown)
        }
        fn request(&self, permission: Permission) -> PermissionStatus {
            self.requested.lock().unwrap().push(permission);
            PermissionStatus::Granted
        }
        fn open_settings(&self, permission: Permission) {
            self.opened.lock().unwrap().push(permission);
        }
    }

    use HotkeyOutput::{Ptt, TalkKeyPaused, TalkKeyRestored};
    use KeyEdge::{Down, Up};
    use PttPhase::{Cancel, Down as PDown, Up as PUp};

    fn down() -> PhysicalState {
        PhysicalState {
            talk_key_down: true,
            chord_down: false,
        }
    }
    fn up() -> PhysicalState {
        PhysicalState::default()
    }

    #[test]
    fn cap_35b_phase_names_match_the_hotkey_ptt_event() {
        assert_eq!(PDown.as_str(), "down");
        assert_eq!(PUp.as_str(), "up");
        assert_eq!(Cancel.as_str(), "cancel");
    }

    #[test]
    fn cap_35b_down_then_up_gives_one_ptt() {
        let mut m = PttMachine::new();
        assert_eq!(m.on_talk_key(Down), vec![Ptt(PDown)]);
        assert!(m.is_held());
        assert_eq!(m.on_talk_key(Up), vec![Ptt(PUp)]);
        assert!(!m.is_held());
    }

    #[test]
    fn cap_35b_repeated_edges_do_not_double_report() {
        let mut m = PttMachine::new();
        assert_eq!(m.on_talk_key(Up), vec![]);
        m.on_talk_key(Down);
        assert_eq!(m.on_talk_key(Down), vec![]);
        m.on_talk_key(Up);
        assert_eq!(m.on_talk_key(Up), vec![]);
    }

    #[test]
    fn cap_35b_at_30_every_key_up_of_1000_cycles_is_reported() {
        let sink = Arc::new(RecSink::default());
        let clock = SimClock::new();
        let mut e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        for _ in 0..1000 {
            e.on_talk_key(Down);
            // Hold longer than a tap (CAP-44a); a shorter hold ends with `cancel` + `Tap`.
            clock.advance(TAP_MAX_HOLD);
            e.on_talk_key(Up);
        }
        let out = sink.take();
        assert_eq!(out.iter().filter(|o| **o == Ptt(PUp)).count(), 1000);
        assert_eq!(out.iter().filter(|o| **o == Ptt(PDown)).count(), 1000);
        assert!(!e.machine().is_held());
    }

    #[test]
    fn cap_36_start_check_never_prompts() {
        let perms = FakePerms::with(Permission::InputMonitoring, PermissionStatus::Granted);
        assert_eq!(check_ptt_permission(&perms), Ok(()));
        assert!(perms.requested().is_empty());
    }

    #[test]
    fn cap_36_start_check_fails_without_input_monitoring_and_does_not_prompt() {
        for s in [PermissionStatus::Unknown, PermissionStatus::Denied] {
            let perms = FakePerms::with(Permission::InputMonitoring, s);
            let err = check_ptt_permission(&perms).unwrap_err();
            assert_eq!(err.code(), "input_monitoring_required");
            assert!(perms.requested().is_empty());
            assert!(!perms
                .status_calls
                .lock()
                .unwrap()
                .contains(&Permission::Accessibility));
        }
    }

    #[test]
    fn cap_36_ptt_request_asks_input_monitoring_only() {
        let perms = FakePerms::default();
        let s = request_ptt_permission(&perms, UserAction::from_user_click());
        assert_eq!(s, PermissionStatus::Granted);
        assert_eq!(perms.requested(), vec![Permission::InputMonitoring]);
    }

    #[test]
    fn cap_36_cap_37_a_click_prompts_once_then_opens_the_settings_pane() {
        for p in [Permission::InputMonitoring, Permission::Accessibility] {
            let perms = FakePerms::default();
            // Not asked yet: the click shows the OS prompt, and only for `p`.
            let s = request_after_click(&perms, p, false, UserAction::from_user_click());
            assert_eq!(s, PermissionStatus::Granted);
            assert_eq!(perms.requested(), vec![p]);
            assert!(perms.opened.lock().unwrap().is_empty());
            // A later click in the same run: the OS shows no new prompt, so open the pane.
            let s = request_after_click(&perms, p, true, UserAction::from_user_click());
            assert_eq!(s, PermissionStatus::Unknown);
            assert_eq!(perms.requested(), vec![p]);
            assert_eq!(*perms.opened.lock().unwrap(), vec![p]);
        }
    }

    #[test]
    fn cap_36_a_denied_permission_opens_the_pane_and_shows_no_prompt() {
        let perms = FakePerms::with(Permission::InputMonitoring, PermissionStatus::Denied);
        let s = request_after_click(
            &perms,
            Permission::InputMonitoring,
            false,
            UserAction::from_user_click(),
        );
        assert_eq!(s, PermissionStatus::Denied);
        assert!(perms.requested().is_empty());
        assert_eq!(
            *perms.opened.lock().unwrap(),
            vec![Permission::InputMonitoring]
        );
    }

    #[test]
    fn cap_36_a_granted_permission_neither_prompts_nor_opens_the_pane() {
        let perms = FakePerms::with(Permission::InputMonitoring, PermissionStatus::Granted);
        for asked in [false, true] {
            let s = request_after_click(
                &perms,
                Permission::InputMonitoring,
                asked,
                UserAction::from_user_click(),
            );
            assert_eq!(s, PermissionStatus::Granted);
        }
        assert!(perms.requested().is_empty());
        assert!(perms.opened.lock().unwrap().is_empty());
    }

    #[test]
    fn cap_36_cap_37_the_settings_panes_are_the_privacy_panes() {
        assert_eq!(
            settings_pane(Permission::InputMonitoring),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
        );
        assert_eq!(
            settings_pane(Permission::Accessibility),
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        );
    }

    #[test]
    fn cap_37_accessibility_is_asked_only_by_the_insertion_choice() {
        let perms = FakePerms::default();
        let _ = check_ptt_permission(&perms);
        request_ptt_permission(&perms, UserAction::from_user_click());
        assert!(!perms.requested().contains(&Permission::Accessibility));
        request_insertion_permission(&perms, UserAction::from_user_click());
        assert_eq!(
            perms.requested(),
            vec![Permission::InputMonitoring, Permission::Accessibility]
        );
    }

    #[test]
    fn cap_38a_tap_timeout_turns_the_tap_on_again() {
        let mut w = TapWatchdog::new();
        assert_eq!(
            w.on_signal(TapSignal::DisabledByTimeout, Duration::from_secs(1)),
            TapAction::ReEnable
        );
        assert_eq!(
            w.on_signal(TapSignal::DisabledByUserInput, Duration::from_secs(60)),
            TapAction::ReEnable
        );
        assert_eq!(
            w.on_signal(
                TapSignal::HealthCheck { enabled: false },
                Duration::from_secs(120)
            ),
            TapAction::ReEnable
        );
        assert_eq!(
            w.on_signal(
                TapSignal::HealthCheck { enabled: true },
                Duration::from_secs(121)
            ),
            TapAction::None
        );
    }

    #[test]
    fn cap_38a_repeated_tap_failures_recreate_the_tap() {
        let mut w = TapWatchdog::new();
        let t = Duration::from_secs(100);
        assert_eq!(
            w.on_signal(TapSignal::DisabledByTimeout, t),
            TapAction::ReEnable
        );
        assert_eq!(
            w.on_signal(TapSignal::DisabledByTimeout, t + Duration::from_secs(1)),
            TapAction::ReEnable
        );
        assert_eq!(
            w.on_signal(TapSignal::DisabledByTimeout, t + Duration::from_secs(2)),
            TapAction::Recreate
        );
        // After the recreate, the count starts again.
        assert_eq!(
            w.on_signal(TapSignal::DisabledByTimeout, t + Duration::from_secs(3)),
            TapAction::ReEnable
        );
        // Old failures fall out of the window.
        let mut w = TapWatchdog::new();
        w.on_signal(TapSignal::DisabledByTimeout, t);
        w.on_signal(TapSignal::DisabledByTimeout, t + Duration::from_secs(5));
        assert_eq!(
            w.on_signal(TapSignal::DisabledByTimeout, t + Duration::from_secs(11)),
            TapAction::ReEnable
        );
    }

    #[test]
    fn cap_38a_ptt_works_after_a_tap_timeout() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let clock = SimClock::new();
        let mut e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        e.on_talk_key(Down);
        // The key-up is lost while the tap is off; the re-read ends the PTT.
        e.on_tap_signal(
            TapSignal::DisabledByTimeout,
            Duration::from_secs(1),
            &probe,
            &ctl,
        );
        assert_eq!(ctl.take(), vec![Ctl::ReEnable]);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
        e.on_talk_key(Down);
        // Hold longer than a tap (CAP-44a).
        clock.advance(TAP_MAX_HOLD);
        e.on_talk_key(Up);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn cap_38b_wake_session_and_grant_recreate_the_tap() {
        for s in [
            TapSignal::Wake,
            TapSignal::SessionActive,
            TapSignal::PermissionGranted,
        ] {
            let mut w = TapWatchdog::new();
            assert_eq!(w.on_signal(s, Duration::from_secs(1)), TapAction::Recreate);
        }
    }

    #[test]
    fn cap_38b_engine_recreates_on_wake_session_and_grant() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let clock = SimClock::new();
        let mut e = HotkeyEngine::new(sink.clone());
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(
            ctl.take(),
            vec![],
            "the first tick only records the baseline"
        );

        // Sleep and wake.
        *probe.slept.lock().unwrap() = Duration::from_secs(3600);
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Recreate]);

        // A short slept delta is noise.
        *probe.slept.lock().unwrap() += Duration::from_millis(10);
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![]);

        // Session goes away and comes back.
        *probe.session.lock().unwrap() = false;
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![]);
        *probe.session.lock().unwrap() = true;
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Recreate]);

        // Permission revoked and granted again.
        *probe.granted.lock().unwrap() = false;
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        *probe.granted.lock().unwrap() = true;
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Recreate]);
        assert_eq!(sink.take(), vec![]);
    }

    #[test]
    fn cap_38b_failed_recreate_is_retried_until_a_tap_is_alive() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let clock = SimClock::new();
        let mut e = HotkeyEngine::new(sink.clone());
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![]);
        // The recreate after a wake failed: no tap is alive.
        *ctl.1.lock().unwrap() = false;
        let mut recreates = 0;
        for _ in 0..30 {
            clock.advance(WATCHDOG_TICK);
            e.tick(clock.now(), &probe, &ctl);
            recreates += ctl.take().iter().filter(|c| **c == Ctl::Recreate).count();
        }
        // 3 s of ticks with a 1 s retry interval.
        assert_eq!(recreates, 3);
        // The tap comes back: no more retries.
        *ctl.1.lock().unwrap() = true;
        for _ in 0..30 {
            clock.advance(WATCHDOG_TICK);
            e.tick(clock.now(), &probe, &ctl);
        }
        assert_eq!(ctl.take(), vec![]);
    }

    #[test]
    fn cap_38b_no_tap_retry_without_input_monitoring() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        *probe.granted.lock().unwrap() = false;
        let ctl = FakeCtl::default();
        *ctl.1.lock().unwrap() = false;
        let mut e = HotkeyEngine::new(sink);
        for i in 0..30 {
            e.tick(WATCHDOG_TICK * i, &probe, &ctl);
        }
        assert_eq!(ctl.take(), vec![]);
    }

    #[test]
    fn cap_39b_failed_chord_registration_is_reported_once_and_retried() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let clock = SimClock::new();
        let mut e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        e.tick(clock.now(), &probe, &ctl);
        *probe.secure.lock().unwrap() = true;
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Chord(true)]);
        assert_eq!(sink.take(), vec![TalkKeyPaused]);
        // The OS refuses the chord.
        e.on_chord_registered(false, clock.now());
        assert_eq!(sink.take(), vec![HotkeyOutput::FallbackChordUnavailable]);
        // The watchdog retries every second; a second failure is not reported again.
        let mut retries = 0;
        for _ in 0..20 {
            clock.advance(WATCHDOG_TICK);
            e.tick(clock.now(), &probe, &ctl);
            let n = ctl
                .take()
                .iter()
                .filter(|c| **c == Ctl::Chord(true))
                .count();
            if n > 0 {
                retries += n;
                e.on_chord_registered(false, clock.now());
            }
        }
        assert_eq!(retries, 2);
        assert_eq!(sink.take(), vec![]);
        // A retry works: the pill shows again and the chord drives PTT.
        clock.advance(CHORD_RETRY_INTERVAL);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Chord(true)]);
        e.on_chord_registered(true, clock.now());
        assert_eq!(sink.take(), vec![TalkKeyPaused]);
        clock.advance(WATCHDOG_TICK * 20);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(ctl.take(), vec![], "no retry once the chord is on");
        e.on_chord(Down);
        clock.advance(TAP_MAX_HOLD);
        e.on_chord(Up);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn cap_39b_late_chord_result_after_secure_input_ends_is_ignored() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let mut e = HotkeyEngine::new(sink.clone());
        e.tick(Duration::ZERO, &probe, &ctl);
        *probe.secure.lock().unwrap() = true;
        e.tick(WATCHDOG_TICK, &probe, &ctl);
        *probe.secure.lock().unwrap() = false;
        e.tick(WATCHDOG_TICK * 2, &probe, &ctl);
        sink.take();
        e.on_chord_registered(false, WATCHDOG_TICK * 3);
        assert_eq!(sink.take(), vec![]);
        for i in 4..40 {
            e.tick(WATCHDOG_TICK * i, &probe, &ctl);
        }
        assert_eq!(ctl.take(), vec![Ctl::Chord(true), Ctl::Chord(false)]);
    }

    #[test]
    fn cap_39a_secure_input_ends_an_active_ptt() {
        let mut m = PttMachine::new();
        m.on_talk_key(Down);
        let out = m.on_secure_input(true, down());
        assert_eq!(out.first(), Some(&Ptt(Cancel)));
        assert!(!m.is_held());
    }

    #[test]
    fn cap_39a_at_30_secure_input_ends_ptt_within_250_ms() {
        assert!(WATCHDOG_TICK < SECURE_INPUT_DEADLINE);
        // Secure Event Input starts at every 10 ms offset inside one tick period.
        for offset_ms in (0..100).step_by(10) {
            let sink = Arc::new(RecSink::default());
            let probe = FakeProbe::ready();
            let ctl = FakeCtl::default();
            let clock = SimClock::new();
            let mut e = HotkeyEngine::new(sink.clone());
            e.tick(clock.now(), &probe, &ctl);
            e.on_talk_key(Down);
            *probe.physical.lock().unwrap() = down();
            clock.advance(Duration::from_millis(offset_ms));
            let started = clock.now();
            *probe.secure.lock().unwrap() = true;
            // The watchdog ticks on its fixed grid.
            let mut ended_at = None;
            let mut t = WATCHDOG_TICK;
            while t <= Duration::from_secs(1) {
                if t >= started {
                    e.tick(t, &probe, &ctl);
                    if sink.take().contains(&Ptt(Cancel)) {
                        ended_at = Some(t);
                        break;
                    }
                }
                t += WATCHDOG_TICK;
            }
            let ended_at = ended_at.expect("PTT must end");
            assert!(
                ended_at - started <= SECURE_INPUT_DEADLINE,
                "offset {offset_ms}"
            );
        }
    }

    #[test]
    fn cap_39b_secure_input_switches_to_the_chord() {
        let mut m = PttMachine::new();
        m.on_secure_input(true, up());
        assert_eq!(m.mode(), TalkKeyMode::Chord);
        // The talk key is ignored in chord mode.
        assert_eq!(m.on_talk_key(Down), vec![]);
        assert_eq!(m.on_chord(Down), vec![Ptt(PDown)]);
        assert_eq!(m.on_chord(Up), vec![Ptt(PUp)]);
        // Back to the talk key when Secure Event Input ends.
        m.on_secure_input(false, up());
        assert_eq!(m.mode(), TalkKeyMode::TalkKey);
        assert_eq!(m.on_chord(Down), vec![]);
        assert_eq!(m.on_talk_key(Down), vec![Ptt(PDown)]);
    }

    #[test]
    fn cap_39b_engine_registers_the_chord_only_while_secure_input_is_on() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let clock = SimClock::new();
        let mut e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        e.tick(Duration::ZERO, &probe, &ctl);
        assert_eq!(ctl.take(), vec![]);
        *probe.secure.lock().unwrap() = true;
        e.tick(WATCHDOG_TICK, &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Chord(true)]);
        assert!(e.secure_input_active());
        e.on_chord(Down);
        clock.advance(TAP_MAX_HOLD);
        e.on_chord(Up);
        *probe.secure.lock().unwrap() = false;
        e.tick(WATCHDOG_TICK * 2, &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Chord(false)]);
        assert_eq!(
            sink.take(),
            vec![TalkKeyPaused, Ptt(PDown), Ptt(PUp), TalkKeyRestored]
        );
    }

    #[test]
    fn cap_39b_reset_after_stop_registers_the_chord_again() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let mut e = HotkeyEngine::new(sink.clone());
        *probe.secure.lock().unwrap() = true;
        e.tick(Duration::ZERO, &probe, &ctl);
        e.on_chord(Down);
        assert_eq!(ctl.take(), vec![Ctl::Chord(true)]);
        sink.take();
        e.reset();
        assert_eq!(sink.take(), vec![Ptt(Cancel), TalkKeyRestored]);
        assert!(!e.machine().is_held());
        e.tick(WATCHDOG_TICK, &probe, &ctl);
        assert_eq!(ctl.take(), vec![Ctl::Chord(true)]);
    }

    #[test]
    fn cap_39c_pill_text_matches_appendix_e() {
        assert_eq!(
            TALK_KEY_PAUSED_PILL,
            "Talk key paused: another app protects typing. Use ⌃⇧Space."
        );
        assert_eq!(TalkKeyPaused.pill_text(), Some(TALK_KEY_PAUSED_PILL));
        assert_eq!(TalkKeyRestored.pill_text(), None);
        assert_eq!(HotkeyOutput::FallbackChordUnavailable.pill_text(), None);
        assert_eq!(Ptt(PDown).pill_text(), None);
    }

    #[test]
    fn cap_39c_secure_input_start_shows_the_pill_once() {
        let mut m = PttMachine::new();
        m.on_talk_key(Down);
        assert_eq!(
            m.on_secure_input(true, down()),
            vec![Ptt(Cancel), TalkKeyPaused]
        );
        assert_eq!(m.on_secure_input(true, down()), vec![]);
        assert_eq!(m.on_secure_input(false, up()), vec![TalkKeyRestored]);
    }

    #[test]
    fn cap_45_gap_ends_a_ptt_whose_key_is_up() {
        let mut m = PttMachine::new();
        m.on_talk_key(Down);
        assert_eq!(m.on_gap(up()), vec![Ptt(PUp)]);
        assert!(!m.is_held());
    }

    #[test]
    fn cap_45_gap_keeps_a_ptt_whose_key_is_still_down() {
        let mut m = PttMachine::new();
        m.on_talk_key(Down);
        assert_eq!(m.on_gap(down()), vec![]);
        assert!(m.is_held());
    }

    #[test]
    fn cap_45_gap_never_starts_a_ptt() {
        let mut m = PttMachine::new();
        assert_eq!(m.on_gap(down()), vec![]);
        assert!(!m.is_held());
    }

    #[test]
    fn cap_45_chord_ptt_ends_when_secure_input_ends() {
        let mut m = PttMachine::new();
        m.on_secure_input(true, up());
        m.on_chord(Down);
        let chord_held = PhysicalState {
            talk_key_down: false,
            chord_down: true,
        };
        assert_eq!(m.on_gap(chord_held), vec![]);
        assert_eq!(
            m.on_secure_input(false, chord_held),
            vec![Ptt(PUp), TalkKeyRestored]
        );
        assert!(!m.is_held());
    }

    #[test]
    fn cap_45_secure_input_end_rereads_the_talk_key() {
        // The talk key went down while the chord was active; it does not start a PTT.
        let mut m = PttMachine::new();
        m.on_secure_input(true, up());
        assert_eq!(m.on_secure_input(false, down()), vec![TalkKeyRestored]);
        assert!(!m.is_held());
    }

    #[test]
    fn cap_45_no_ptt_stays_active_after_wake_or_unlock() {
        let sink = Arc::new(RecSink::default());
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        let mut e = HotkeyEngine::new(sink.clone());
        e.tick(Duration::ZERO, &probe, &ctl);
        // Wake with the key up.
        e.on_talk_key(Down);
        *probe.slept.lock().unwrap() = Duration::from_secs(60);
        e.tick(WATCHDOG_TICK, &probe, &ctl);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
        // Lock then unlock with the key up.
        e.on_talk_key(Down);
        *probe.session.lock().unwrap() = false;
        e.tick(WATCHDOG_TICK * 2, &probe, &ctl);
        *probe.session.lock().unwrap() = true;
        e.tick(WATCHDOG_TICK * 3, &probe, &ctl);
        let out = sink.take();
        assert_eq!(out.last(), Some(&Ptt(PUp)));
        assert!(!e.machine().is_held());
    }

    // ---- CAP-44a: a hold shorter than 250 ms is a tap; CAP-44c: another key cancels ----

    fn engine_at_zero() -> (Arc<RecSink>, SimClock, HotkeyEngine) {
        let sink = Arc::new(RecSink::default());
        let clock = SimClock::new();
        let e = HotkeyEngine::with_clock(sink.clone(), Arc::new(clock.clone()));
        (sink, clock, e)
    }

    #[test]
    fn cap_44a_at_03_a_200_ms_hold_gives_a_tap_event_and_no_up() {
        let (sink, clock, mut e) = engine_at_zero();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(200));
        e.on_talk_key(Up);
        let out = sink.take();
        assert_eq!(out, vec![Ptt(PDown), Ptt(Cancel), HotkeyOutput::Tap]);
        // No `up`, so no STT call starts for the tap (CAP-44b drops its audio).
        assert!(!out.contains(&Ptt(PUp)));
        assert!(!e.machine().is_held());
    }

    #[test]
    fn cap_44a_holds_of_250_ms_or_more_are_not_taps() {
        for ms in [250u64, 300, 2_000] {
            let (sink, clock, mut e) = engine_at_zero();
            e.on_talk_key(Down);
            clock.advance(Duration::from_millis(ms));
            e.on_talk_key(Up);
            assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)], "{ms} ms");
        }
    }

    #[test]
    fn cap_44a_holds_just_under_250_ms_are_taps() {
        for ms in [0u64, 1, 120, 249] {
            let (sink, clock, mut e) = engine_at_zero();
            e.on_talk_key(Down);
            clock.advance(Duration::from_millis(ms));
            e.on_talk_key(Up);
            assert_eq!(
                sink.take(),
                vec![Ptt(PDown), Ptt(Cancel), HotkeyOutput::Tap],
                "{ms} ms"
            );
        }
    }

    #[test]
    fn cap_44a_a_short_chord_hold_is_a_tap_too() {
        let (sink, clock, mut e) = engine_at_zero();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        *probe.secure.lock().unwrap() = true;
        e.tick(clock.now(), &probe, &ctl);
        sink.take();
        e.on_chord(Down);
        clock.advance(Duration::from_millis(100));
        e.on_chord(Up);
        assert_eq!(
            sink.take(),
            vec![Ptt(PDown), Ptt(Cancel), HotkeyOutput::Tap]
        );
    }

    #[test]
    fn cap_44a_a_gap_re_read_is_not_a_tap() {
        // A lost key-up found by a re-read ends the PTT with `up`; only a real key-up edge
        // can be a tap.
        let (sink, clock, mut e) = engine_at_zero();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(100));
        e.on_tap_signal(TapSignal::DisabledByTimeout, clock.now(), &probe, &ctl);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn cap_44a_tap_has_no_pill_text() {
        assert_eq!(HotkeyOutput::Tap.pill_text(), None);
    }

    // ---- CAP-14a: a double-tap (key-downs 400 ms or less apart) starts hands-free ----

    fn tap_at(e: &mut HotkeyEngine, clock: &SimClock, hold_ms: u64) {
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(hold_ms));
        e.on_talk_key(Up);
    }

    #[test]
    fn cap_14a_two_taps_with_key_downs_400_ms_apart_give_double_tap() {
        for gap in [0u64, 150, 300] {
            let (sink, clock, mut e) = engine_at_zero();
            tap_at(&mut e, &clock, 100);
            clock.advance(Duration::from_millis(gap));
            tap_at(&mut e, &clock, 100);
            assert_eq!(
                sink.take(),
                vec![
                    Ptt(PDown),
                    Ptt(Cancel),
                    HotkeyOutput::Tap,
                    Ptt(PDown),
                    Ptt(Cancel),
                    HotkeyOutput::Tap,
                    Ptt(PttPhase::DoubleTap),
                ],
                "key-downs {} ms apart",
                100 + gap
            );
        }
        assert_eq!(PttPhase::DoubleTap.as_str(), "double_tap");
        assert_eq!(DOUBLE_TAP_MAX_GAP, Duration::from_millis(400));
    }

    #[test]
    fn cap_14a_key_downs_more_than_400_ms_apart_are_two_single_taps() {
        let (sink, clock, mut e) = engine_at_zero();
        tap_at(&mut e, &clock, 100);
        clock.advance(Duration::from_millis(301));
        tap_at(&mut e, &clock, 100);
        assert!(!sink.take().contains(&Ptt(PttPhase::DoubleTap)));
    }

    #[test]
    fn cap_14a_a_hold_between_taps_and_a_third_tap_give_no_double_tap() {
        let (sink, clock, mut e) = engine_at_zero();
        // A tap, then a PTT hold: the hold resets the double-tap rule.
        tap_at(&mut e, &clock, 50);
        tap_at(&mut e, &clock, 300);
        tap_at(&mut e, &clock, 50);
        assert!(!sink.take().contains(&Ptt(PttPhase::DoubleTap)));
        // A double-tap, then a third tap: the third tap is a new single tap.
        let (sink, clock, mut e) = engine_at_zero();
        tap_at(&mut e, &clock, 50);
        tap_at(&mut e, &clock, 50);
        tap_at(&mut e, &clock, 50);
        let doubles = sink
            .take()
            .into_iter()
            .filter(|o| *o == Ptt(PttPhase::DoubleTap))
            .count();
        assert_eq!(doubles, 1);
    }

    #[test]
    fn cap_14a_a_cancelled_press_is_not_the_first_tap() {
        // CAP-44c: another key cancels the press. It is no tap, so no double-tap follows.
        let (sink, clock, mut e) = engine_at_zero();
        e.on_talk_key(Down);
        e.on_other_key_down();
        e.on_talk_key(Up);
        clock.advance(Duration::from_millis(50));
        tap_at(&mut e, &clock, 50);
        assert!(!sink.take().contains(&Ptt(PttPhase::DoubleTap)));
    }

    #[test]
    fn cap_45_cap_14a_a_short_chord_hold_ended_by_secure_input_is_an_up_and_no_tap() {
        // Secure Event Input ends during a chord hold: the key-up is lost, so R1 sends `up`
        // (CAP-45) even for a short hold. It is never a tap, and it ends any tap pair.
        for hold_ms in [10u64, 100, 1_000] {
            let (sink, clock, mut e) = engine_at_zero();
            let probe = FakeProbe::ready();
            let ctl = FakeCtl::default();
            e.tick(clock.now(), &probe, &ctl);
            *probe.secure.lock().unwrap() = true;
            e.tick(clock.now(), &probe, &ctl);
            e.on_chord(Down);
            clock.advance(Duration::from_millis(5));
            e.on_chord(Up);
            clock.advance(Duration::from_millis(5));
            e.on_chord(Down);
            clock.advance(Duration::from_millis(hold_ms));
            *probe.secure.lock().unwrap() = false;
            e.tick(clock.now(), &probe, &ctl);
            let out = sink.take();
            assert!(
                out.ends_with(&[Ptt(PUp), HotkeyOutput::TalkKeyRestored]),
                "{hold_ms} ms: {out:?}"
            );
            assert!(
                !out.contains(&Ptt(PttPhase::DoubleTap)),
                "{hold_ms} ms: {out:?}"
            );
        }
    }

    #[test]
    fn cap_14a_cap_39a_a_secure_input_cancel_between_two_taps_gives_no_double_tap() {
        // A machine cancel (Secure Event Input starts during the second press) is no tap. The
        // chord tap after it is a new first tap, not the second tap of the earlier pair.
        let (sink, clock, mut e) = engine_at_zero();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        e.tick(clock.now(), &probe, &ctl);
        tap_at(&mut e, &clock, 50);
        clock.advance(Duration::from_millis(50));
        e.on_talk_key(Down);
        *probe.physical.lock().unwrap() = down();
        *probe.secure.lock().unwrap() = true;
        e.tick(clock.now(), &probe, &ctl);
        assert!(sink
            .take()
            .ends_with(&[Ptt(PDown), Ptt(Cancel), HotkeyOutput::TalkKeyPaused]));
        clock.advance(Duration::from_millis(50));
        e.on_chord(Down);
        clock.advance(Duration::from_millis(50));
        e.on_chord(Up);
        let out = sink.take();
        assert!(
            out.contains(&HotkeyOutput::Tap),
            "the chord press is a tap: {out:?}"
        );
        assert!(!out.contains(&Ptt(PttPhase::DoubleTap)), "{out:?}");
    }

    #[test]
    fn cap_44c_at_03_another_key_in_the_first_second_cancels_the_ptt() {
        for ms in [0u64, 300, 999] {
            let (sink, clock, mut e) = engine_at_zero();
            e.on_talk_key(Down);
            clock.advance(Duration::from_millis(ms));
            e.on_other_key_down();
            assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(Cancel)], "{ms} ms");
            assert!(!e.machine().is_held());
            // The later key-up gives nothing: no `up`, so no STT call.
            clock.advance(Duration::from_millis(500));
            e.on_talk_key(Up);
            assert_eq!(sink.take(), vec![], "{ms} ms");
        }
    }

    #[test]
    fn cap_44c_another_key_after_one_second_does_not_cancel() {
        for ms in [1_000u64, 1_500, 30_000] {
            let (sink, clock, mut e) = engine_at_zero();
            e.on_talk_key(Down);
            clock.advance(Duration::from_millis(ms));
            e.on_other_key_down();
            assert!(e.machine().is_held(), "{ms} ms");
            e.on_talk_key(Up);
            assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)], "{ms} ms");
        }
    }

    #[test]
    fn cap_44c_another_key_with_no_hold_does_nothing() {
        let (sink, _clock, mut e) = engine_at_zero();
        e.on_other_key_down();
        assert_eq!(sink.take(), vec![]);
        // The next hold is a normal PTT.
        assert!(!e.other_key_window_open());
    }

    #[test]
    fn cap_44c_the_next_hold_after_a_cancel_works() {
        let (sink, clock, mut e) = engine_at_zero();
        e.on_talk_key(Down);
        e.on_other_key_down();
        e.on_talk_key(Up);
        sink.take();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(400));
        e.on_talk_key(Up);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn cap_44c_other_key_window_is_open_only_in_the_first_second_of_a_hold() {
        let (_sink, clock, mut e) = engine_at_zero();
        assert!(!e.other_key_window_open());
        e.on_talk_key(Down);
        assert!(e.other_key_window_open());
        clock.advance(Duration::from_millis(999));
        assert!(e.other_key_window_open());
        clock.advance(Duration::from_millis(1));
        assert!(!e.other_key_window_open());
        e.on_talk_key(Up);
        assert!(!e.other_key_window_open());
    }

    #[test]
    fn cap_44c_other_key_window_stays_closed_for_a_chord_hold() {
        let (sink, clock, mut e) = engine_at_zero();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        *probe.secure.lock().unwrap() = true;
        e.tick(clock.now(), &probe, &ctl);
        e.on_chord(Down);
        assert!(!e.other_key_window_open());
        e.on_other_key_down();
        clock.advance(TAP_MAX_HOLD);
        e.on_chord(Up);
        assert_eq!(sink.take(), vec![TalkKeyPaused, Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn cap_44c_machine_cancel_ends_only_an_active_ptt() {
        let mut m = PttMachine::new();
        assert_eq!(m.cancel(), vec![]);
        m.on_talk_key(Down);
        assert_eq!(m.cancel(), vec![Ptt(Cancel)]);
        assert!(!m.is_held());
        assert_eq!(m.on_talk_key(Up), vec![]);
    }

    // ---- RTR-17a (ADR 0098): the talk key + G marks the hold as an ask ----

    /// The ask-key seam double: it records each register (`true`) and unregister (`false`).
    /// The drain requests are counted apart: the test plays the OS layer and calls
    /// `on_ask_key_drained` itself.
    #[derive(Default)]
    struct RecAskKey(Mutex<Vec<bool>>, Mutex<usize>);
    impl AskKeyControl for RecAskKey {
        fn set_ask_key(&self, on: bool) {
            self.0.lock().unwrap().push(on);
        }
        fn drain(&self) {
            *self.1.lock().unwrap() += 1;
        }
    }
    impl RecAskKey {
        fn take(&self) -> Vec<bool> {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
        fn drains(&self) -> usize {
            std::mem::take(&mut *self.1.lock().unwrap())
        }
    }

    fn ask_engine() -> (Arc<RecSink>, SimClock, Arc<RecAskKey>, HotkeyEngine) {
        let (sink, clock, e) = engine_at_zero();
        let keys = Arc::new(RecAskKey::default());
        let e = e.with_ask_key(keys.clone());
        (sink, clock, keys, e)
    }

    use HotkeyOutput::PttAskUp;

    #[test]
    fn rtr_17a_g_during_the_hold_puts_ask_on_the_up() {
        for ms in [300u64, 1_500, 30_000] {
            let (sink, clock, _keys, mut e) = ask_engine();
            e.on_talk_key(Down);
            clock.advance(Duration::from_millis(ms));
            e.on_ask_key();
            assert!(e.machine().is_held(), "{ms} ms");
            clock.advance(Duration::from_millis(400));
            e.on_talk_key(Up);
            assert_eq!(sink.take(), vec![Ptt(PDown), PttAskUp], "{ms} ms");
        }
    }

    #[test]
    fn rtr_17a_a_hold_without_g_is_a_plain_up_and_the_flag_is_per_hold() {
        let (sink, clock, _keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(400));
        e.on_talk_key(Up);
        e.on_ask_key_drained();
        e.on_talk_key(Down);
        e.on_ask_key();
        clock.advance(Duration::from_millis(400));
        e.on_talk_key(Up);
        // The next hold has no G: no ask.
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(400));
        e.on_talk_key(Up);
        e.on_ask_key_drained();
        assert_eq!(
            sink.take(),
            vec![
                Ptt(PDown),
                Ptt(PUp),
                Ptt(PDown),
                PttAskUp,
                Ptt(PDown),
                Ptt(PUp)
            ]
        );
    }

    #[test]
    fn rtr_17a_an_early_g_from_both_paths_marks_the_ask_once() {
        // The race: G comes at 0 ms, before the Carbon hot key is registered. The other-key
        // tap reports it, then the hot key reports a repeat. Both set the same flag.
        let (sink, clock, _keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        e.on_ask_key();
        e.on_ask_key();
        assert!(e.machine().is_held());
        clock.advance(Duration::from_millis(600));
        e.on_talk_key(Up);
        assert_eq!(sink.take(), vec![Ptt(PDown), PttAskUp]);
    }

    #[test]
    fn rtr_17a_g_with_no_hold_does_nothing() {
        let (sink, clock, keys, mut e) = ask_engine();
        e.on_ask_key();
        assert_eq!(sink.take(), vec![]);
        assert_eq!(keys.take(), Vec::<bool>::new());
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(400));
        e.on_talk_key(Up);
        e.on_ask_key_drained();
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn rtr_17a_a_tap_a_cancel_and_a_double_tap_never_carry_ask() {
        let (sink, clock, _keys, mut e) = ask_engine();
        // A tap with G: still a tap (CAP-44a).
        e.on_talk_key(Down);
        e.on_ask_key();
        clock.advance(Duration::from_millis(100));
        e.on_talk_key(Up);
        // The second tap makes a double-tap.
        clock.advance(Duration::from_millis(100));
        e.on_talk_key(Down);
        e.on_ask_key();
        clock.advance(Duration::from_millis(100));
        e.on_talk_key(Up);
        // A CAP-44c cancel after G.
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Down);
        e.on_ask_key();
        e.on_other_key_down();
        e.on_talk_key(Up);
        let out = sink.take();
        assert!(!out.contains(&PttAskUp), "{out:?}");
        assert!(out.contains(&Ptt(PttPhase::DoubleTap)), "{out:?}");
        assert_eq!(out.last(), Some(&Ptt(Cancel)), "{out:?}");
    }

    #[test]
    fn rtr_17a_a_chord_hold_never_asks() {
        let (sink, clock, keys, mut e) = ask_engine();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        *probe.secure.lock().unwrap() = true;
        e.tick(clock.now(), &probe, &ctl);
        e.on_chord(Down);
        e.on_ask_key();
        clock.advance(TAP_MAX_HOLD);
        e.on_chord(Up);
        assert_eq!(sink.take(), vec![TalkKeyPaused, Ptt(PDown), Ptt(PUp)]);
        // No ask key under Secure Event Input: the chord drives PTT.
        assert_eq!(keys.take(), Vec::<bool>::new());
    }

    // ---- CAP-44c (ADR 0098): G does not cancel; every other key still cancels ----

    #[test]
    fn cap_44c_g_in_the_first_second_does_not_cancel() {
        for ms in [0u64, 300, 999] {
            let (sink, clock, _keys, mut e) = ask_engine();
            e.on_talk_key(Down);
            clock.advance(Duration::from_millis(ms));
            e.on_ask_key();
            assert!(e.machine().is_held(), "{ms} ms");
            clock.advance(Duration::from_millis(500));
            e.on_talk_key(Up);
            assert_eq!(sink.take(), vec![Ptt(PDown), PttAskUp], "{ms} ms");
        }
    }

    #[test]
    fn cap_44c_another_key_still_cancels_in_an_ask_hold() {
        let (sink, clock, _keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(100));
        e.on_ask_key();
        clock.advance(Duration::from_millis(100));
        e.on_other_key_down();
        assert!(!e.machine().is_held());
        clock.advance(Duration::from_millis(500));
        e.on_talk_key(Up);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(Cancel)]);
    }

    // ---- RTR-17a (ADR 0098): the ask key is grabbed only during a talk-key hold ----

    fn paired(calls: &[bool]) {
        assert_eq!(calls.first(), Some(&true), "{calls:?}");
        assert_eq!(calls.last(), Some(&false), "{calls:?}");
        let on = calls.iter().filter(|c| **c).count();
        assert_eq!(on, calls.len() - on, "{calls:?}");
        assert!(calls.windows(2).all(|w| w[0] != w[1]), "{calls:?}");
    }

    #[test]
    fn rtr_17a_the_ask_key_is_registered_at_the_hold_and_unregistered_at_the_up() {
        let (_sink, clock, keys, mut e) = ask_engine();
        assert_eq!(keys.take(), Vec::<bool>::new());
        e.on_talk_key(Down);
        assert_eq!(keys.take(), vec![true]);
        clock.advance(Duration::from_millis(400));
        e.on_ask_key();
        assert_eq!(keys.take(), Vec::<bool>::new());
        e.on_talk_key(Up);
        assert_eq!(keys.take(), vec![false]);
        // Outside a hold, Option+G types "©" again: no register.
        e.on_ask_key();
        e.on_other_key_down();
        assert_eq!(keys.take(), Vec::<bool>::new());
    }

    #[test]
    fn rtr_17a_the_ask_key_is_released_on_a_tap_and_on_a_cap_44c_cancel() {
        let (_sink, clock, keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(100));
        e.on_talk_key(Up);
        paired(&keys.take());
        // Esc, or any other key, in the first second cancels and releases the grab.
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Down);
        e.on_other_key_down();
        paired(&keys.take());
        e.on_talk_key(Up);
        assert_eq!(keys.take(), Vec::<bool>::new());
    }

    #[test]
    fn rtr_17a_the_ask_key_is_released_when_secure_input_pauses_the_talk_key() {
        let (sink, clock, keys, mut e) = ask_engine();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        e.tick(clock.now(), &probe, &ctl);
        e.on_talk_key(Down);
        e.on_ask_key();
        *probe.secure.lock().unwrap() = true;
        *probe.physical.lock().unwrap() = down();
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        paired(&keys.take());
        assert!(!sink.take().contains(&PttAskUp));
    }

    #[test]
    fn rtr_17a_the_ask_key_is_released_on_a_tap_disable_gap_and_on_a_lock() {
        // A tap disable with the key no longer down ends the hold (CAP-45).
        let (sink, clock, keys, mut e) = ask_engine();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        e.on_talk_key(Down);
        e.on_ask_key();
        clock.advance(Duration::from_secs(2));
        *probe.physical.lock().unwrap() = up();
        e.on_tap_signal(TapSignal::DisabledByTimeout, clock.now(), &probe, &ctl);
        paired(&keys.take());
        // The lost key-up of an ask hold still asks: the user tapped G.
        assert_eq!(sink.take(), vec![Ptt(PDown), PttAskUp]);

        // A re-enable with the key still down keeps the hold and the grab.
        e.on_talk_key(Down);
        *probe.physical.lock().unwrap() = down();
        e.on_tap_signal(TapSignal::DisabledByUserInput, clock.now(), &probe, &ctl);
        assert_eq!(keys.take(), vec![true]);

        // A lock or a user switch with the key up ends the hold (focus loss).
        e.tick(clock.now(), &probe, &ctl);
        *probe.session.lock().unwrap() = false;
        *probe.physical.lock().unwrap() = up();
        clock.advance(WATCHDOG_TICK);
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(keys.take(), vec![false]);
        assert!(!e.machine().is_held());
    }

    #[test]
    fn rtr_17a_the_ask_key_is_released_at_the_120_s_hold_cap() {
        let (sink, clock, keys, mut e) = ask_engine();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        *probe.physical.lock().unwrap() = down();
        e.on_talk_key(Down);
        e.on_ask_key();
        clock.advance(ASK_KEY_MAX_HOLD - Duration::from_millis(1));
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(keys.take(), vec![true]);
        clock.advance(Duration::from_millis(1));
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(keys.take(), vec![false]);
        // The cap only releases the grab: the hold and its ask stay.
        assert!(e.machine().is_held());
        clock.advance(Duration::from_secs(5));
        e.tick(clock.now(), &probe, &ctl);
        e.on_talk_key(Up);
        assert_eq!(keys.take(), Vec::<bool>::new());
        assert_eq!(sink.take(), vec![Ptt(PDown), PttAskUp]);
        // The next hold grabs the key again.
        e.on_talk_key(Down);
        assert_eq!(keys.take(), vec![true]);
        assert_eq!(ASK_KEY_MAX_HOLD, Duration::from_secs(120));
    }

    #[test]
    fn rtr_17a_the_ask_key_is_released_on_stop_and_on_drop() {
        let (_sink, _clock, keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        e.reset();
        paired(&keys.take());
        e.on_talk_key(Down);
        drop(e);
        paired(&keys.take());
        // An engine that is not in a hold unregisters nothing at drop.
        let (_sink, _clock, keys, e) = ask_engine();
        drop(e);
        assert_eq!(keys.take(), Vec::<bool>::new());
    }

    // ---- RTR-17a (ADR 0098, review M1): the key-up waits for the queued ask-key presses ----

    #[test]
    fn rtr_17a_a_g_handled_after_the_up_edge_still_asks() {
        // The user taps G, then releases. The main thread is busy, so the hot-key press is
        // handled only after the tap thread has seen the key-up edge.
        let (sink, clock, keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_secs(3));
        e.on_talk_key(Up);
        assert_eq!(keys.drains(), 1);
        assert_eq!(sink.take(), vec![Ptt(PDown)]);
        e.on_ask_key();
        e.on_ask_key_drained();
        assert_eq!(sink.take(), vec![PttAskUp]);
        assert!(!e.machine().is_held());
        // The grab is released after the decision, not before it.
        paired(&keys.take());
    }

    #[test]
    fn rtr_17a_a_plain_up_waits_only_for_the_drain() {
        let (sink, clock, keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Up);
        assert_eq!(keys.drains(), 1);
        // No clock time passes: the up goes out as soon as the drain comes back.
        e.on_ask_key_drained();
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
        paired(&keys.take());
    }

    #[test]
    fn rtr_17a_a_lost_drain_ends_the_up_at_the_grace_deadline() {
        assert_eq!(ASK_UP_GRACE, Duration::from_millis(200));
        let (sink, clock, keys, mut e) = ask_engine();
        let probe = FakeProbe::ready();
        let ctl = FakeCtl::default();
        *probe.physical.lock().unwrap() = down();
        e.tick(clock.now(), &probe, &ctl);
        e.on_talk_key(Down);
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Up);
        clock.advance(ASK_UP_GRACE - Duration::from_millis(1));
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(sink.take(), vec![Ptt(PDown)]);
        clock.advance(Duration::from_millis(1));
        e.tick(clock.now(), &probe, &ctl);
        assert_eq!(sink.take(), vec![Ptt(PUp)]);
        paired(&keys.take());
        // A drain or a G that comes after the decision does nothing.
        e.on_ask_key();
        e.on_ask_key_drained();
        assert_eq!(sink.take(), vec![]);
        assert_eq!(keys.take(), Vec::<bool>::new());
    }

    #[test]
    fn rtr_17a_no_wait_for_a_tap_a_marked_ask_or_an_os_with_no_ask_key() {
        let (sink, clock, keys, mut e) = ask_engine();
        // A tap: always `cancel` and `Tap`.
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(100));
        e.on_talk_key(Up);
        // G is already marked: the up is an ask at once.
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Down);
        e.on_ask_key();
        clock.advance(Duration::from_secs(1));
        e.on_talk_key(Up);
        assert_eq!(keys.drains(), 0);
        assert_eq!(
            sink.take(),
            vec![
                Ptt(PDown),
                Ptt(Cancel),
                HotkeyOutput::Tap,
                Ptt(PDown),
                PttAskUp
            ]
        );
        // No ask key (Windows): the up goes out at once.
        let (sink, clock, mut e) = engine_at_zero();
        e.on_talk_key(Down);
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Up);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }

    #[test]
    fn rtr_17a_a_new_press_during_the_wait_ends_the_old_up_first() {
        let (sink, clock, keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_secs(2));
        e.on_talk_key(Up);
        e.on_talk_key(Down);
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp), Ptt(PDown)]);
        // The late drain of the first press does not end the second one.
        e.on_ask_key_drained();
        assert!(e.machine().is_held());
        assert_eq!(keys.take(), vec![true, false, true]);
    }

    #[test]
    fn cap_44c_a_key_after_the_up_edge_does_not_cancel() {
        let (sink, clock, _keys, mut e) = ask_engine();
        e.on_talk_key(Down);
        clock.advance(Duration::from_millis(500));
        e.on_talk_key(Up);
        assert!(!e.other_key_window_open());
        e.on_other_key_down();
        e.on_ask_key_drained();
        assert_eq!(sink.take(), vec![Ptt(PDown), Ptt(PUp)]);
    }
}
